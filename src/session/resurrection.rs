//! Session resurrection — ported from Go TUIOS `internal/session/resurrection.go`.
//!
//! Persists session state to disk so it can be restored after a daemon crash
//! or restart. State is written atomically (temp file + rename) and stamped
//! with a schema version so a newer file is archived rather than loaded.

use crate::session::model::{validate_session_name, SessionState};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The subdirectory under the state home where session files live.
const RESURRECTION_DIR: &str = "tuios/sessions";

/// How often a session is saved regardless of whether anything changed.
pub const RESURRECTION_INTERVAL: Duration = Duration::from_secs(30);

/// How often the saver looks for a structural change.
pub const RESURRECTION_DIRTY_INTERVAL: Duration = Duration::from_secs(2);

/// The current on-disk resurrection schema version.
pub const RESURRECTION_VERSION: u32 = 1;

/// Marker shown on a session that came back from saved state.
pub const RESTORED_TAG: &str = "restored";
pub const RESTORED_NOTE: &str = "layout came back from saved state; the shells are new";

/// Marker for state that is on disk with no daemon holding it.
pub const SAVED_TAG: &str = "saved";
pub const SAVED_NOTE: &str = "on disk only, with no daemon running to hold it";

/// How long an archived state file is kept.
const ARCHIVE_RETENTION: Duration = Duration::from_secs(14 * 24 * 60 * 60);

/// Return the directory for session resurrection files.
pub fn resurrection_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("TUIOS_SESSIONS_DIR") {
        return PathBuf::from(dir);
    }
    if let Ok(state_home) = std::env::var("XDG_STATE_HOME") {
        return PathBuf::from(state_home).join(RESURRECTION_DIR);
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home)
            .join(".local")
            .join("state")
            .join(RESURRECTION_DIR);
    }
    PathBuf::from(RESURRECTION_DIR)
}

/// Return the path for a specific session's resurrection file.
pub fn resurrection_path(session_name: &str) -> PathBuf {
    resurrection_dir().join(format!("{session_name}.json"))
}

/// Return the archive directory for corrupt or incompatible state files.
pub fn resurrection_archive_dir() -> PathBuf {
    resurrection_dir().join("archive")
}

/// Move a bad state file into the archive directory, tagged with a timestamp.
/// Best effort: on any failure the original file is removed. Returns the
/// destination path, or an empty string if the move failed.
pub fn archive_resurrection_file(path: &Path) -> String {
    let archive_dir = resurrection_archive_dir();
    if fs::create_dir_all(&archive_dir).is_err() {
        let _ = fs::remove_file(path);
        return String::new();
    }
    let base = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dest = archive_dir.join(format!("{base}.{ts}.bak"));
    if fs::rename(path, &dest).is_err() {
        let _ = fs::remove_file(path);
        return String::new();
    }
    dest.to_string_lossy().to_string()
}

/// Render where an archived state file was moved to.
fn archived_note(dest: &str) -> String {
    if dest.is_empty() {
        "it could not be archived and was removed".to_string()
    } else {
        format!("archived to {dest}")
    }
}

/// Remove leftover temp files and archived state past retention.
pub fn clean_resurrection_dir() {
    let dir = resurrection_dir();
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".json.tmp") {
                let _ = fs::remove_file(entry.path());
            }
        }
    }

    let archive_dir = resurrection_archive_dir();
    if let Ok(entries) = fs::read_dir(&archive_dir) {
        let cutoff = SystemTime::now() - ARCHIVE_RETENTION;
        for entry in entries.flatten() {
            if let Ok(meta) = entry.metadata() {
                if let Ok(modified) = meta.modified() {
                    if modified < cutoff {
                        let _ = fs::remove_file(entry.path());
                    }
                }
            }
        }
    }
}

/// Persist session state to disk atomically.
pub fn save_session_for_resurrection(state: &SessionState) -> io::Result<()> {
    if state.name.is_empty() {
        return Ok(());
    }

    let dir = resurrection_dir();
    fs::create_dir_all(&dir)?;

    let data = serde_json::to_vec_pretty(state)
        .map_err(|e| io::Error::other(format!("failed to marshal session state: {e}")))?;

    let path = resurrection_path(&state.name);
    let tmp_path = path.with_extension("json.tmp");

    fs::write(&tmp_path, &data)?;
    fs::rename(&tmp_path, &path)?;

    Ok(())
}

/// Load a saved session state from disk. Corrupt or version-incompatible files
/// are archived (not deleted) and returned as an error.
pub fn load_resurrection_state(session_name: &str) -> Result<SessionState, String> {
    let path = resurrection_path(session_name);
    let data = fs::read_to_string(&path)
        .map_err(|e| format!("no resurrection data for session {session_name:?}: {e}"))?;

    let state: SessionState = serde_json::from_str(&data).map_err(|e| {
        let dest = archive_resurrection_file(&path);
        format!(
            "saved state for session {session_name:?} is corrupt and cannot be restored ({}): {e}",
            archived_note(&dest)
        )
    })?;

    if state.resurrection_version > RESURRECTION_VERSION {
        let dest = archive_resurrection_file(&path);
        return Err(format!(
            "saved state for session {session_name:?} was written by a newer TermOS (state version {}, this build reads up to {}) and cannot be restored ({})",
            state.resurrection_version,
            RESURRECTION_VERSION,
            archived_note(&dest)
        ));
    }

    Ok(state)
}

/// Metadata for a resurrectable session, for listing.
#[derive(Debug, Clone)]
pub struct ResurrectableInfo {
    pub name: String,
    pub window_count: usize,
    pub saved_at: Option<SystemTime>,
}

/// Return metadata for every resurrectable session, sorted by name.
pub fn list_resurrectable_infos() -> Vec<ResurrectableInfo> {
    let names = match list_resurrectable_sessions() {
        Ok(n) => n,
        Err(_) => return vec![],
    };
    let mut infos: Vec<ResurrectableInfo> = names
        .iter()
        .filter_map(|name| {
            let state = load_resurrection_state(name).ok()?;
            let saved_at = fs::metadata(resurrection_path(name))
                .and_then(|m| m.modified())
                .ok();
            Some(ResurrectableInfo {
                name: name.clone(),
                window_count: state.windows.len(),
                saved_at,
            })
        })
        .collect();
    infos.sort_by(|a, b| a.name.cmp(&b.name));
    infos
}

/// Return names of sessions that can be restored, sorted.
pub fn list_resurrectable_sessions() -> Result<Vec<String>, io::Error> {
    let dir = resurrection_dir();
    let mut names = Vec::new();
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.ends_with(".json") {
                continue;
            }
            let stem = &name[..name.len() - 5]; // strip .json
            names.push(stem.to_string());
        }
    }
    names.sort();
    Ok(names)
}

/// Delete the resurrection file for a session.
pub fn remove_resurrection_state(session_name: &str) {
    let _ = fs::remove_file(resurrection_path(session_name));
}

/// Return the working directory of the process with the given PID by reading
/// `/proc/<pid>/cwd`. On platforms without procfs, returns `None`.
pub fn process_cwd(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }
    let cwd = fs::read_link(format!("/proc/{pid}/cwd")).ok()?;
    cwd.to_str().map(|s| s.to_string())
}

/// Validate a session name for resurrection (delegates to the model validator).
pub fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Ok(());
    }
    validate_session_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::model::WindowState;
    use std::sync::Mutex;

    // Serialize tests so they don't race on the shared override dir.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn with_temp_dir<F: FnOnce(&Path)>(f: F) {
        let _guard = TEST_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("TUIOS_SESSIONS_DIR", tmp.path());
        f(tmp.path());
        std::env::remove_var("TUIOS_SESSIONS_DIR");
    }

    #[test]
    fn save_and_load_round_trip() {
        with_temp_dir(|_dir| {
            let state = SessionState {
                name: "test-session".into(),
                windows: vec![WindowState {
                    title: "shell".into(),
                    shell: "/bin/bash".into(),
                    workspace: 1,
                }],
                resurrection_version: RESURRECTION_VERSION,
            };
            save_session_for_resurrection(&state).unwrap();
            let loaded = load_resurrection_state("test-session").unwrap();
            assert_eq!(loaded.name, "test-session");
            assert_eq!(loaded.windows.len(), 1);
            assert_eq!(loaded.windows[0].title, "shell");
        });
    }

    #[test]
    fn list_sessions_round_trip() {
        with_temp_dir(|_dir| {
            let state = SessionState {
                name: "alpha".into(),
                windows: vec![],
                resurrection_version: RESURRECTION_VERSION,
            };
            save_session_for_resurrection(&state).unwrap();
            let state2 = SessionState {
                name: "beta".into(),
                windows: vec![],
                resurrection_version: RESURRECTION_VERSION,
            };
            save_session_for_resurrection(&state2).unwrap();
            let names = list_resurrectable_sessions().unwrap();
            assert_eq!(names, vec!["alpha", "beta"]);
        });
    }

    #[test]
    fn list_infos_round_trip() {
        with_temp_dir(|_dir| {
            let state = SessionState {
                name: "work".into(),
                windows: vec![
                    WindowState {
                        title: "a".into(),
                        shell: "/bin/sh".into(),
                        workspace: 0,
                    },
                    WindowState {
                        title: "b".into(),
                        shell: "/bin/sh".into(),
                        workspace: 1,
                    },
                ],
                resurrection_version: RESURRECTION_VERSION,
            };
            save_session_for_resurrection(&state).unwrap();
            let infos = list_resurrectable_infos();
            assert_eq!(infos.len(), 1);
            assert_eq!(infos[0].name, "work");
            assert_eq!(infos[0].window_count, 2);
        });
    }

    #[test]
    fn remove_state_deletes_file() {
        with_temp_dir(|_dir| {
            let state = SessionState {
                name: "doomed".into(),
                windows: vec![],
                resurrection_version: RESURRECTION_VERSION,
            };
            save_session_for_resurrection(&state).unwrap();
            assert!(resurrection_path("doomed").exists());
            remove_resurrection_state("doomed");
            assert!(!resurrection_path("doomed").exists());
        });
    }

    #[test]
    fn load_missing_returns_error() {
        with_temp_dir(|_dir| {
            let result = load_resurrection_state("nonexistent");
            assert!(result.is_err());
        });
    }

    #[test]
    fn load_corrupt_archives_and_errors() {
        with_temp_dir(|_dir| {
            let path = resurrection_path("corrupt");
            fs::create_dir_all(resurrection_dir()).unwrap();
            fs::write(&path, "not valid json").unwrap();
            let result = load_resurrection_state("corrupt");
            assert!(result.is_err());
            assert!(result.unwrap_err().contains("corrupt"));
            // The file should have been archived (moved).
            assert!(!path.exists());
        });
    }

    #[test]
    fn version_mismatch_archives() {
        with_temp_dir(|_dir| {
            let state = SessionState {
                name: "future".into(),
                windows: vec![],
                resurrection_version: RESURRECTION_VERSION + 100,
            };
            save_session_for_resurrection(&state).unwrap();
            let result = load_resurrection_state("future");
            assert!(result.is_err());
            assert!(result.unwrap_err().contains("newer TermOS"));
        });
    }

    #[test]
    fn clean_resurrection_dir_removes_tmp_files() {
        with_temp_dir(|dir| {
            let tmp = dir.join("leftover.json.tmp");
            fs::write(&tmp, "incomplete").unwrap();
            clean_resurrection_dir();
            assert!(!tmp.exists());
        });
    }

    #[test]
    fn validate_name_accepts_empty() {
        assert!(validate_name("").is_ok());
    }

    #[test]
    fn validate_name_rejects_slash() {
        assert!(validate_name("has/slash").is_err());
    }

    #[test]
    fn resurrection_dir_uses_override() {
        with_temp_dir(|dir| {
            let d = resurrection_dir();
            assert_eq!(d, dir);
        });
    }

    #[test]
    fn archive_dir_is_subdir() {
        with_temp_dir(|dir| {
            let archive = resurrection_archive_dir();
            assert_eq!(archive, dir.join("archive"));
        });
    }
}
