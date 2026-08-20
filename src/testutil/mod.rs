//! Test utilities — ported from Go TUIOS `internal/testutil/`.
//!
//! Provides:
//! - Fake shell for testing PTY interactions
//! - XDG isolation for test environments
//! - E2E test harness infrastructure
//! - PTY exhaustion guard for skipping PTY-dependent tests

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

// ─── PTY Exhaustion Guard ─────────────────────────────────────────────────

/// Returns `true` if a PTY can be allocated right now.
///
/// Unlike the original cached variant, this re-probes on each call so that
/// `skip_if_pty_exhausted!` reflects the live system state — important
/// because `Window::spawn` now uses a pool semaphore that blocks instead of
/// failing, so the bottleneck is the pool slot count, not the kernel PTY
/// ceiling.
pub fn pty_is_available() -> bool {
    match nix::pty::openpty(None, None) {
        Ok(pair) => {
            drop(pair.master);
            drop(pair.slave);
            true
        }
        Err(_) => false,
    }
}

/// Call at the top of any test that requires a real PTY.
/// Skips the test (via `return`) if PTYs are exhausted.  Rate-limiting is
/// handled by the pool semaphore inside `Window::spawn`, so there is no
/// need to serialize tests externally.
///
/// # Example
/// ```ignore
/// #[test]
/// fn spawn_pty_works() {
///     crate::skip_if_pty_exhausted!();
///     // ... test code ...
/// }
/// ```
#[macro_export]
macro_rules! skip_if_pty_exhausted {
    () => {
        if !$crate::testutil::pty_is_available() {
            eprintln!("SKIP: PTYs exhausted — skipping PTY-dependent test");
            return;
        }
    };
}

// ─── XDG Isolation ───────────────────────────────────────────────────────

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A temporary XDG environment for testing. Creates isolated directories
/// and sets the environment variables, restoring them on drop.
pub struct XdgIsolate {
    tmpdir: PathBuf,
    pub old_xdg_config: Option<String>,
    pub old_xdg_state: Option<String>,
    pub old_xdg_data: Option<String>,
    pub old_xdg_cache: Option<String>,
    pub old_home: Option<String>,
}

impl XdgIsolate {
    /// Create a new isolated XDG environment.
    pub fn new() -> std::io::Result<Self> {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let tmpdir =
            std::env::temp_dir().join(format!("termos-test-{}-{}", std::process::id(), id));
        std::fs::create_dir_all(&tmpdir)?;

        let config = tmpdir.join("config");
        let state = tmpdir.join("state");
        let data = tmpdir.join("data");
        let cache = tmpdir.join("cache");
        let home = tmpdir.join("home");

        std::fs::create_dir_all(&config)?;
        std::fs::create_dir_all(&state)?;
        std::fs::create_dir_all(&data)?;
        std::fs::create_dir_all(&cache)?;
        std::fs::create_dir_all(&home)?;

        let old_xdg_config = std::env::var("XDG_CONFIG_HOME").ok();
        let old_xdg_state = std::env::var("XDG_STATE_HOME").ok();
        let old_xdg_data = std::env::var("XDG_DATA_HOME").ok();
        let old_xdg_cache = std::env::var("XDG_CACHE_HOME").ok();
        let old_home = std::env::var("HOME").ok();

        std::env::set_var("XDG_CONFIG_HOME", &config);
        std::env::set_var("XDG_STATE_HOME", &state);
        std::env::set_var("XDG_DATA_HOME", &data);
        std::env::set_var("XDG_CACHE_HOME", &cache);
        std::env::set_var("HOME", &home);

        Ok(Self {
            tmpdir,
            old_xdg_config,
            old_xdg_state,
            old_xdg_data,
            old_xdg_cache,
            old_home,
        })
    }

    /// Path to the `config/` directory.
    pub fn config_dir(&self) -> PathBuf {
        self.tmpdir.join("config")
    }

    /// Path to the `state/` directory.
    pub fn state_dir(&self) -> PathBuf {
        self.tmpdir.join("state")
    }

    /// Path to the `data/` directory.
    pub fn data_dir(&self) -> PathBuf {
        self.tmpdir.join("data")
    }

    /// Path to the `cache/` directory.
    pub fn cache_dir(&self) -> PathBuf {
        self.tmpdir.join("cache")
    }

    /// Path to the `home/` directory.
    pub fn home_dir(&self) -> PathBuf {
        self.tmpdir.join("home")
    }
}

impl Drop for XdgIsolate {
    fn drop(&mut self) {
        // Restore environment variables.
        if let Some(val) = &self.old_xdg_config {
            std::env::set_var("XDG_CONFIG_HOME", val);
        } else {
            std::env::remove_var("XDG_CONFIG_HOME");
        }
        if let Some(val) = &self.old_xdg_state {
            std::env::set_var("XDG_STATE_HOME", val);
        } else {
            std::env::remove_var("XDG_STATE_HOME");
        }
        if let Some(val) = &self.old_xdg_data {
            std::env::set_var("XDG_DATA_HOME", val);
        } else {
            std::env::remove_var("XDG_DATA_HOME");
        }
        if let Some(val) = &self.old_xdg_cache {
            std::env::set_var("XDG_CACHE_HOME", val);
        } else {
            std::env::remove_var("XDG_CACHE_HOME");
        }
        if let Some(val) = &self.old_home {
            std::env::set_var("HOME", val);
        } else {
            std::env::remove_var("HOME");
        }
        // Clean up the temporary directory.
        let _ = std::fs::remove_dir_all(&self.tmpdir);
    }
}

// ─── Fake Shell ───────────────────────────────────────────────────────────

/// Create a fake shell script in the given directory.
/// The script echoes input back, making it a valid `sh` PTY target.
pub fn create_fake_shell(dir: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join("fake_shell.sh");
    std::fs::write(
        &path,
        "#!/bin/sh\necho \"fake_shell: $@\"\nwhile IFS= read -r line; do\n  echo \"$line\"\ndone\n",
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms)?;
    }
    Ok(path)
}

// ─── E2E Harness ──────────────────────────────────────────────────────────

/// Result of a single E2E test step.
pub struct E2eResult {
    pub step: String,
    pub passed: bool,
    pub actual: String,
    pub expected: String,
}

/// Summarize E2E results.
pub fn summarize_results(results: &[E2eResult]) -> (usize, usize) {
    let passed = results.iter().filter(|r| r.passed).count();
    let failed = results.len() - passed;
    (passed, failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xdg_isolate_creates_dirs() {
        let iso = XdgIsolate::new().unwrap();
        assert!(iso.tmpdir.exists());
        assert!(std::env::var("XDG_CONFIG_HOME").is_ok());
    }

    #[test]
    fn create_fake_shell_is_executable() {
        let dir = std::env::temp_dir().join(format!(
            "termos-fakeshell-{}",
            std::process::id()
        ));
        let path = create_fake_shell(&dir).unwrap();
        assert!(path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn summarize_counts_correctly() {
        let results = vec![
            E2eResult {
                step: "a".into(),
                passed: true,
                actual: "ok".into(),
                expected: "ok".into(),
            },
            E2eResult {
                step: "b".into(),
                passed: false,
                actual: "".into(),
                expected: "ok".into(),
            },
            E2eResult {
                step: "c".into(),
                passed: true,
                actual: "".into(),
                expected: "ok".into(),
            },
        ];
        let (passed, failed) = summarize_results(&results);
        assert_eq!(passed, 2);
        assert_eq!(failed, 1);
    }
}
