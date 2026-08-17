//! Tape trust store — the security foundation for `.tuios.tape` autorun,
//! ported from TUIOS `internal/tape/trust/trust.go`.
//!
//! The store is direnv's model. A tape is inert until the user explicitly
//! trusts it; trust is bound to the (canonical path, content hash) pair, so
//! any edit to the file reverts it to untrusted and re-prompts. Denial is
//! keyed by path alone, so a hostile edit cannot nag the user back into a
//! prompt.
//!
//! The single-read [`Store::check`] API is deliberately shaped so a later
//! stage can execute the exact bytes that were hashed, defeating a swap of
//! the file (or its symlink target) between the trust check and the run.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Bounds the bytes read from a tape.
pub const MAX_TAPE_SIZE: u64 = 64 * 1024;

/// The fixed basename of a project tape.
pub const TAPE_FILE_NAME: &str = ".tuios.tape";

/// The trust verdict for a tape encountered at a directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Fails the hygiene preconditions (not regular, not owned, group/world
    /// writable, or too large). Never offered for trust.
    Ineligible,
    /// Eligible but not trusted: never seen, or edited since trust.
    Untrusted,
    /// Path trusted and the content hash matches.
    Trusted,
    /// The user chose "never for this path"; survives edits by design.
    Denied,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Ineligible => "ineligible",
            Status::Untrusted => "untrusted",
            Status::Trusted => "trusted",
            Status::Denied => "denied",
        }
    }
}

/// The outcome of checking a tape file. `content` holds the exact bytes that
/// `hash` was computed over, read in a single pass from the same descriptor
/// the hygiene checks were applied to; a later stage must execute `content`,
/// never re-read the path.
#[derive(Debug, Clone)]
pub struct CheckResult {
    pub status: Status,
    /// Canonical path (symlinks resolved), the trust key.
    pub path: String,
    /// Hex SHA-256 of `content`; empty when `content` is empty.
    pub hash: String,
    /// Exact hashed bytes, reused verbatim by a later run.
    pub content: Vec<u8>,
    pub size: u64,
    /// Why a tape is ineligible, for the notice.
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TrustEntry {
    path: String,
    sha256: String,
    trusted_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DenyEntry {
    path: String,
    denied_at: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct TrustFile {
    #[serde(default)]
    trusted: Vec<TrustEntry>,
    #[serde(default)]
    denied: Vec<DenyEntry>,
}

/// The in-memory trust store backed by a TOML file. All methods are safe for
/// concurrent use.
#[derive(Debug)]
pub struct Store {
    path: PathBuf,
    trusted: HashMap<String, TrustEntry>,
    denied: HashMap<String, DenyEntry>,
    /// Non-empty when the store file was present but failed its own integrity
    /// checks; its contents are then deliberately not loaded.
    pub warning: String,
}

impl Store {
    /// Open (or create) the trust store at the default XDG location.
    pub fn load() -> std::result::Result<Self, String> {
        let base = dirs::data_dir().ok_or("no data dir")?;
        Self::load_from_path(base.join("termos").join("tape-trust.toml"))
    }

    /// Open (or create) the trust store at `path`. A missing store is created
    /// empty with 0600 permissions and a 0700 parent. A present store is
    /// validated: if group/world-accessible or not owned by the current user,
    /// its contents are ignored and `warning` is set, so tampering downgrades
    /// trust to nothing rather than being honored.
    pub fn load_from_path(path: PathBuf) -> std::result::Result<Self, String> {
        let mut s = Self {
            path,
            trusted: HashMap::new(),
            denied: HashMap::new(),
            warning: String::new(),
        };

        std::fs::create_dir_all(s.path.parent().unwrap_or(Path::new(".")))
            .map_err(|e| format!("creating tape trust store directory: {e}"))?;

        let info = match std::fs::metadata(&s.path) {
            Ok(info) => info,
            Err(_) => {
                // First run: materialize an empty, correctly-permissioned store.
                s.save()?;
                return Ok(s);
            }
        };

        if !owned_by_current_user(&info) || is_group_or_world_accessible(&info) {
            s.warning = format!(
                "tape trust store {} has unsafe owner or permissions (want 0600, owned by you); ignoring its contents",
                s.path.display()
            );
            let _ = std::fs::set_permissions(&s.path, std::fs::Permissions::from_mode(0o600));
            return Ok(s);
        }

        let data = std::fs::read(&s.path).map_err(|e| format!("reading tape trust store: {e}"))?;
        let file: TrustFile = match toml::from_str(&String::from_utf8_lossy(&data)) {
            Ok(f) => f,
            Err(e) => {
                s.warning = format!(
                    "tape trust store {} is corrupt ({e}); ignoring its contents",
                    s.path.display()
                );
                return Ok(s);
            }
        };
        for e in file.trusted {
            if !e.path.is_empty() && !e.sha256.is_empty() {
                s.trusted.insert(e.path.clone(), e);
            }
        }
        for e in file.denied {
            if !e.path.is_empty() {
                s.denied.insert(e.path.clone(), e);
            }
        }
        Ok(s)
    }

    /// Resolve, hygiene-check, read, and hash the tape at `tape_path` in a
    /// single pass, returning its trust status. Denial is evaluated first and
    /// by canonical path alone, so a denied tape never causes a read.
    pub fn check(&self, tape_path: &str) -> std::result::Result<CheckResult, String> {
        let real =
            std::fs::canonicalize(tape_path).map_err(|e| format!("resolving tape path: {e}"))?;
        let real_str = real.to_string_lossy().into_owned();

        let denied = self.denied.contains_key(&real_str);
        let trusted = self.trusted.get(&real_str).cloned();
        if denied {
            return Ok(CheckResult {
                status: Status::Denied,
                path: real_str,
                hash: String::new(),
                content: Vec::new(),
                size: 0,
                reason: String::new(),
            });
        }

        // Open once and stat the descriptor: hygiene on the fd ties the checks
        // to the same inode the bytes come from.
        let f = std::fs::File::open(&real).map_err(|e| format!("opening tape: {e}"))?;
        let info = f.metadata().map_err(|e| format!("stat tape: {e}"))?;
        if let Some(reason) = hygiene_reason(&info) {
            return Ok(CheckResult {
                status: Status::Ineligible,
                path: real_str,
                hash: String::new(),
                content: Vec::new(),
                size: info.size(),
                reason,
            });
        }

        // Read from the same descriptor, capped one byte past the limit.
        let mut content = Vec::new();
        f.take(MAX_TAPE_SIZE + 1)
            .read_to_end(&mut content)
            .map_err(|e| format!("reading tape: {e}"))?;
        if content.len() as u64 > MAX_TAPE_SIZE {
            return Ok(CheckResult {
                status: Status::Ineligible,
                path: real_str,
                hash: String::new(),
                content: Vec::new(),
                size: content.len() as u64,
                reason: format!("larger than {} KiB", MAX_TAPE_SIZE / 1024),
            });
        }

        let hash = hex_hash(&content);
        let size = content.len() as u64;
        let status = match trusted {
            Some(t) if t.sha256 == hash => Status::Trusted,
            _ => Status::Untrusted,
        };
        Ok(CheckResult {
            status,
            path: real_str,
            hash,
            content,
            size,
            reason: String::new(),
        })
    }

    /// Record trust for a (canonical path, hash) pair, replacing any earlier
    /// hash for that path and clearing any deny entry.
    pub fn trust(&mut self, path: &str, hash: &str) -> std::result::Result<(), String> {
        if path.is_empty() || hash.is_empty() {
            return Err("trust requires a path and a hash".into());
        }
        self.trusted.insert(
            path.to_string(),
            TrustEntry {
                path: path.to_string(),
                sha256: hash.to_string(),
                trusted_at: now_rfc3339(),
            },
        );
        self.denied.remove(path);
        self.save()
    }

    /// Record "never for this path", clearing any trust entry.
    pub fn deny(&mut self, path: &str) -> std::result::Result<(), String> {
        if path.is_empty() {
            return Err("deny requires a path".into());
        }
        self.denied.insert(
            path.to_string(),
            DenyEntry {
                path: path.to_string(),
                denied_at: now_rfc3339(),
            },
        );
        self.trusted.remove(path);
        self.save()
    }

    /// Remove both trust and deny entries for a path.
    pub fn forget(&mut self, path: &str) -> std::result::Result<(), String> {
        self.trusted.remove(path);
        self.denied.remove(path);
        self.save()
    }

    /// The stored trusted hash for a canonical path, if any (lets a caller
    /// tell a never-seen tape from one trusted and since edited).
    pub fn trusted_hash(&self, path: &str) -> Option<String> {
        self.trusted.get(path).map(|e| e.sha256.clone())
    }

    /// Persist the store atomically (temp file plus rename) with 0600.
    fn save(&self) -> std::result::Result<(), String> {
        let file = TrustFile {
            trusted: self.trusted.values().cloned().collect(),
            denied: self.denied.values().cloned().collect(),
        };
        let data = toml::to_string(&file).map_err(|e| format!("encoding tape trust store: {e}"))?;
        let dir = self.path.parent().unwrap_or(Path::new("."));
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("creating tape trust store directory: {e}"))?;

        let tmp = dir.join(format!(".tape-trust-{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| -> std::result::Result<(), String> {
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp)
                .map_err(|e| format!("creating tape trust store temp file: {e}"))?;
            f.set_permissions(std::fs::Permissions::from_mode(0o600))
                .map_err(|e| format!("setting tape trust store permissions: {e}"))?;
            f.write_all(data.as_bytes())
                .map_err(|e| format!("writing tape trust store: {e}"))?;
            f.sync_all().ok();
            Ok(())
        })();
        match result {
            Ok(()) => std::fs::rename(&tmp, &self.path)
                .map_err(|e| format!("replacing tape trust store: {e}")),
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                Err(e)
            }
        }
    }
}

fn hex_hash(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// The tape eligibility preconditions; `None` means eligible.
fn hygiene_reason(info: &std::fs::Metadata) -> Option<String> {
    if !info.is_file() {
        return Some("not a regular file".into());
    }
    if !owned_by_current_user(info) {
        return Some("not owned by you".into());
    }
    if info.mode() & 0o022 != 0 {
        return Some("group- or world-writable".into());
    }
    if info.size() > MAX_TAPE_SIZE {
        return Some(format!("larger than {} KiB", MAX_TAPE_SIZE / 1024));
    }
    None
}

fn owned_by_current_user(info: &std::fs::Metadata) -> bool {
    let uid = unsafe { nix::libc::getuid() };
    info.uid() == uid
}

fn is_group_or_world_accessible(info: &std::fs::Metadata) -> bool {
    info.mode() & 0o077 != 0
}

/// RFC3339 UTC timestamp for the store entries.
fn now_rfc3339() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_epoch_utc(secs)
}

fn format_epoch_utc(secs: u64) -> String {
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days as i64 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mth = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if mth <= 2 { y + 1 } else { y };
    format!("{year:04}-{mth:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tape-trust.toml");
        let store = Store::load_from_path(path).unwrap();
        (dir, store)
    }

    fn write_tape(dir: &Path, content: &[u8]) -> String {
        let path = dir.join(".tuios.tape");
        std::fs::write(&path, content).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn first_run_creates_0600_store() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tape-trust.toml");
        let store = Store::load_from_path(path.clone()).unwrap();
        assert!(store.warning.is_empty());
        assert!(path.exists());
        let mode = std::fs::metadata(&path).unwrap().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn check_round_trip_trust_edit_deny() {
        let (dir, mut store) = temp_store();
        let tape = write_tape(dir.path(), b"Enter\n");
        let dir2 = dir.path().to_path_buf();

        // Untrusted first.
        let r1 = store.check(&tape).unwrap();
        assert_eq!(r1.status, Status::Untrusted);
        assert_eq!(r1.content, b"Enter\n");

        // Trust it: now trusted.
        store.trust(&r1.path, &r1.hash).unwrap();
        let r2 = store.check(&tape).unwrap();
        assert_eq!(r2.status, Status::Trusted);

        // Editing reverts to untrusted.
        std::fs::write(dir2.join(".tuios.tape"), b"Enter\nEnter\n").unwrap();
        let r3 = store.check(&tape).unwrap();
        assert_eq!(r3.status, Status::Untrusted);

        // Denial is by path and survives edits.
        store.deny(&r3.path).unwrap();
        let r4 = store.check(&tape).unwrap();
        assert_eq!(r4.status, Status::Denied);
        assert!(r4.content.is_empty(), "denied tapes are never read");

        // Forget returns to untrusted.
        store.forget(&r3.path).unwrap();
        let r5 = store.check(&tape).unwrap();
        assert_eq!(r5.status, Status::Untrusted);
    }

    #[test]
    fn ineligible_tapes_are_rejected() {
        let (dir, store) = temp_store();
        // World-writable.
        let tape = dir.path().join(".tuios.tape");
        std::fs::write(&tape, b"Enter\n").unwrap();
        std::fs::set_permissions(&tape, std::fs::Permissions::from_mode(0o666)).unwrap();
        let r = store.check(&tape.to_string_lossy()).unwrap();
        assert_eq!(r.status, Status::Ineligible);
        assert!(r.reason.contains("writable"), "reason: {}", r.reason);

        // Oversized.
        std::fs::set_permissions(&tape, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::write(&tape, vec![b'x'; (MAX_TAPE_SIZE + 100) as usize]).unwrap();
        let r = store.check(&tape.to_string_lossy()).unwrap();
        assert_eq!(r.status, Status::Ineligible);
        assert!(r.reason.contains("larger than"), "reason: {}", r.reason);
    }

    #[test]
    fn tampered_store_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tape-trust.toml");
        let _ = Store::load_from_path(path.clone()).unwrap();
        // Make it world-accessible: contents must be ignored.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::write(&path, "trusted = [{ path = 'x', sha256 = 'y' }]").unwrap();
        let store = Store::load_from_path(path.clone()).unwrap();
        assert!(!store.warning.is_empty(), "warning expected");
        assert!(store.trusted.is_empty());
    }

    #[test]
    fn trust_round_trips_persistence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tape-trust.toml");
        let tape = write_tape(dir.path(), b"Enter\n");
        {
            let mut store = Store::load_from_path(path.clone()).unwrap();
            let r = store.check(&tape).unwrap();
            store.trust(&r.path, &r.hash).unwrap();
        }
        // A fresh store from disk still trusts the tape.
        let store = Store::load_from_path(path).unwrap();
        let r = store.check(&tape).unwrap();
        assert_eq!(r.status, Status::Trusted);
    }
}
