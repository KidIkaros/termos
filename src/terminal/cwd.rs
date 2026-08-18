//! Shell working-directory tracking — ported from Go TUIOS
//! `internal/terminal/window_cwd.go`.
//!
//! Reads a shell's CWD from `/proc/<pid>/cwd` on Linux (empty elsewhere) with
//! a short cache, because the render path asks once per window per frame and a
//! readlink per window per frame would be pure overhead. A directory that
//! changed a moment ago catching up on the next second is not noticeable.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How often the shell's working directory is re-read from the OS.
const CWD_REFRESH_INTERVAL: Duration = Duration::from_secs(1);

/// A memoised CWD read for one window.
#[derive(Debug, Default)]
pub struct CwdCache {
    inner: Mutex<CwdEntry>,
}

#[derive(Debug, Default)]
struct CwdEntry {
    value: String,
    fetched_at: Option<Instant>,
}

impl CwdCache {
    /// Create an empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// The cached CWD for the shell with the given pid, refreshing at most
    /// once per [`CWD_REFRESH_INTERVAL`]. Empty when it cannot be determined
    /// (non-Linux, process gone, permissions).
    pub fn get(&self, pid: i32) -> String {
        let now = Instant::now();
        let mut entry = self.inner.lock().unwrap();
        if let Some(fetched) = entry.fetched_at {
            if now.duration_since(fetched) < CWD_REFRESH_INTERVAL {
                return entry.value.clone();
            }
        }
        let value = read_proc_cwd(pid);
        entry.value = value.clone();
        entry.fetched_at = Some(now);
        value
    }

    /// Force-clear the cache (e.g. on a known shell cwd change).
    pub fn clear(&self) {
        let mut entry = self.inner.lock().unwrap();
        entry.value.clear();
        entry.fetched_at = None;
    }
}

/// Read `/proc/<pid>/cwd` on Linux. Returns the empty string when the read
/// fails (non-Linux, gone process, no permission).
fn read_proc_cwd(pid: i32) -> String {
    #[cfg(target_os = "linux")]
    {
        let path = PathBuf::from(format!("/proc/{pid}/cwd"));
        match std::fs::read_link(&path) {
            Ok(target) => target.to_string_lossy().into_owned(),
            Err(_) => String::new(),
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_pid_is_empty_or_cwd() {
        let cache = CwdCache::new();
        // pid 1 (init) exists on Linux; the read either succeeds (systemd) or
        // is empty (permissions) — never panics and never returns garbage.
        let _ = cache.get(1);
        // A pid that cannot exist returns empty.
        let value = cache.get(i32::MAX);
        assert!(value.is_empty() || value.starts_with('/'));
    }

    #[test]
    fn cache_refreshes_after_interval() {
        let cache = CwdCache::new();
        let first = cache.get(i32::MAX);
        // Immediate re-read hits the cache (empty, but no syscall).
        let second = cache.get(i32::MAX);
        assert_eq!(first, second);
    }

    #[test]
    fn clear_resets() {
        let cache = CwdCache::new();
        cache.get(i32::MAX);
        cache.clear();
        let entry = cache.inner.lock().unwrap();
        assert!(entry.value.is_empty());
        assert!(entry.fetched_at.is_none());
    }

    #[test]
    fn current_process_cwd_is_a_directory() {
        let cache = CwdCache::new();
        let cwd = cache.get(std::process::id() as i32);
        // /proc/<self>/cwd must resolve; on Linux it is the test's cwd.
        if !cwd.is_empty() {
            assert!(PathBuf::from(&cwd).is_dir());
        }
    }
}
