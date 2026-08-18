//! Config file watcher — ported from Go TUIOS `internal/config/watcher.go`.
//!
//! Uses a polling approach (checks mtime every 2 seconds) since we don't have
//! a file watcher dependency in this module. Provides a `ConfigWatcher` struct
//! with `new(path)`, `check_changed() -> bool`, and `stop()`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::SystemTime;

use super::userconfig::UserConfig;

/// A polling-based config file watcher.
///
/// Call `check_changed()` periodically (e.g. every 2 seconds) to detect
/// whether the config file has been modified since the last check.
pub struct ConfigWatcher {
    path: PathBuf,
    last_mtime: Arc<std::sync::Mutex<Option<SystemTime>>>,
    stopped: Arc<AtomicBool>,
}

impl ConfigWatcher {
    /// Create a new watcher for the config file at `path`.
    /// Records the current mtime so the first `check_changed()` only fires
    /// on a modification after creation.
    pub fn new(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref().to_path_buf();
        let last_mtime = file_mtime(&path);
        Self {
            path,
            last_mtime: Arc::new(std::sync::Mutex::new(last_mtime)),
            stopped: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Returns `true` if the file's mtime has changed since the last call.
    /// Updates the stored mtime on each call.
    pub fn check_changed(&self) -> bool {
        if self.stopped.load(Ordering::Relaxed) {
            return false;
        }
        let current = file_mtime(&self.path);
        let mut guard = self.last_mtime.lock().unwrap();
        if current != *guard {
            *guard = current;
            true
        } else {
            false
        }
    }

    /// Stop the watcher. Subsequent `check_changed()` calls always return
    /// `false`.
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
    }

    /// Reload the config from the watched path.
    pub fn reload(&self) -> UserConfig {
        UserConfig::load_from(&self.path)
    }

    /// The path being watched.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Get the mtime of a file, or `None` if it doesn't exist.
fn file_mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
}

/// The recommended polling interval (2 seconds).
pub const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watcher_new_records_mtime() {
        let dir = std::env::temp_dir().join("termos_test_watcher_new");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.toml");

        let cfg = UserConfig::default_config();
        std::fs::write(&path, toml::to_string(&cfg).unwrap()).unwrap();

        let watcher = ConfigWatcher::new(&path);
        // No change yet.
        assert!(!watcher.check_changed());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn watcher_detects_change() {
        let dir = std::env::temp_dir().join("termos_test_watcher_change");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.toml");

        let cfg = UserConfig::default_config();
        std::fs::write(&path, toml::to_string(&cfg).unwrap()).unwrap();

        let watcher = ConfigWatcher::new(&path);
        assert!(!watcher.check_changed());

        // Modify the file. We need to ensure the mtime actually changes,
        // which may require a small sleep on some filesystems.
        std::thread::sleep(std::time::Duration::from_millis(50));
        let mut cfg2 = cfg.clone();
        cfg2.appearance.border_style = "double".into();
        std::fs::write(&path, toml::to_string(&cfg2).unwrap()).unwrap();

        assert!(watcher.check_changed());
        // Second call should not fire again.
        assert!(!watcher.check_changed());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn watcher_stop_prevents_detection() {
        let dir = std::env::temp_dir().join("termos_test_watcher_stop");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.toml");

        let cfg = UserConfig::default_config();
        std::fs::write(&path, toml::to_string(&cfg).unwrap()).unwrap();

        let watcher = ConfigWatcher::new(&path);
        watcher.stop();

        std::thread::sleep(std::time::Duration::from_millis(50));
        let mut cfg2 = cfg.clone();
        cfg2.appearance.border_style = "double".into();
        std::fs::write(&path, toml::to_string(&cfg2).unwrap()).unwrap();

        assert!(!watcher.check_changed());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn watcher_nonexistent_file() {
        let watcher = ConfigWatcher::new("/nonexistent/path/config.toml");
        assert!(!watcher.check_changed());
    }

    #[test]
    fn watcher_reload_returns_config() {
        let dir = std::env::temp_dir().join("termos_test_watcher_reload");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.toml");

        let cfg = UserConfig::default_config();
        std::fs::write(&path, toml::to_string(&cfg).unwrap()).unwrap();

        let watcher = ConfigWatcher::new(&path);
        let loaded = watcher.reload();
        assert_eq!(loaded.appearance.border_style, "rounded");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn poll_interval_is_two_seconds() {
        assert_eq!(POLL_INTERVAL, std::time::Duration::from_secs(2));
    }
}
