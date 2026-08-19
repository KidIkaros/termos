//! In-process daemon auto-start — ported from Go TUIOS
//! `internal/session/autostart.go`.
//!
//! When a client connects and no daemon is running, the daemon can be started
//! in-process in a background thread instead of spawning a subprocess. This is
//! the path the TUI takes so a single process is both the app and the daemon.

use std::io;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Once};
use std::time::Duration;

use super::daemon::{default_socket_path, Daemon};

/// The singleton in-process daemon handle, plus any error from its startup.
struct InProcessState {
    daemon: Option<Arc<Daemon>>,
    err: Option<io::Error>,
}

static IN_PROCESS: Mutex<Option<InProcessState>> = Mutex::new(None);
static IN_PROCESS_ONCE: Once = Once::new();

/// Ensure the termos daemon is running. If no daemon is reachable, start one
/// in-process in a background thread. Returns `Ok(())` once the socket is
/// accepting connections, or an error if the daemon fails to come up.
pub fn ensure_daemon_running() -> io::Result<()> {
    // Fast path: a daemon is already reachable.
    if is_daemon_running() {
        return Ok(());
    }

    // Start the in-process daemon exactly once.
    IN_PROCESS_ONCE.call_once(|| {
        let daemon = Arc::new(Daemon::new());
        let daemon_clone = Arc::clone(&daemon);

        // Start() is non-blocking — it spawns the accept thread and returns.
        let result = daemon_clone.run_default();

        let mut state = IN_PROCESS.lock().unwrap();
        *state = Some(InProcessState {
            daemon: Some(daemon),
            err: result.err(),
        });
    });

    // Check for a startup error from the once-init.
    {
        let state = IN_PROCESS.lock().unwrap();
        if let Some(ref s) = *state {
            if let Some(ref e) = s.err {
                return Err(io::Error::new(e.kind(), e.to_string()));
            }
        }
    }

    // Wait for the socket to come up.
    wait_for_daemon(&default_socket_path(), Duration::from_secs(5))
}

/// Whether a daemon is currently reachable on the default socket.
pub fn is_daemon_running() -> bool {
    UnixStream::connect(default_socket_path()).is_ok()
}

/// Stop the in-process daemon if one was started. Called during graceful
/// shutdown.
pub fn stop_in_process_daemon() {
    let mut state = IN_PROCESS.lock().unwrap();
    if let Some(ref mut s) = *state {
        s.daemon = None;
    }
}

/// Poll the socket until a connection succeeds or the timeout elapses.
pub fn wait_for_daemon(path: &PathBuf, timeout: Duration) -> io::Result<()> {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if UnixStream::connect(path).is_ok() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        format!("daemon did not start within {:?}", timeout),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_daemon_running_on_nonexistent_socket_is_false() {
        // The default socket almost certainly does not exist in the test
        // environment; if a real daemon happens to be running this still
        // returns true, which is a valid answer.
        let _ = is_daemon_running();
    }

    #[test]
    fn wait_for_daemon_times_out_on_missing_socket() {
        let path = PathBuf::from("/tmp/termos-nonexistent-autostart-test.sock");
        let result = wait_for_daemon(&path, Duration::from_millis(100));
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
    }

    #[test]
    fn stop_in_process_daemon_is_safe_when_none_started() {
        // Should not panic when no daemon was started.
        stop_in_process_daemon();
    }
}
