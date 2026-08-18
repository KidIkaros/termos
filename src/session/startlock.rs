//! Daemon start lock — ported from Go TUIOS `internal/session/daemon_startlock.go`.
//!
//! Guards daemon startup against two starters racing to bind the socket: the
//! first starter binds and listens; a second starter's probe is refused, which
//! reads exactly like the crashed-daemon case that stale-socket recovery is
//! for — so without a lock the second starter would unlink a live daemon's
//! socket and bind its own. The lock file sits beside the socket and is never
//! removed: a lock is an inode, and deleting it would let two starters hold
//! locks on two different inodes.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;

/// Another daemon holds the start lock (mid-way through binding the socket).
#[derive(Debug)]
pub struct DaemonStarting;

impl std::fmt::Display for DaemonStarting {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "another TermOS daemon is starting")
    }
}

impl std::error::Error for DaemonStarting {}

/// The lock file path: the socket path plus `.lock`.
pub fn start_lock_path(socket_path: &std::path::Path) -> PathBuf {
    let mut p = socket_path.as_os_str().to_owned();
    p.push(".lock");
    PathBuf::from(p)
}

/// Take the non-blocking exclusive flock on a file. The lock is tied to the
/// open file description, so it is dropped by close and by process exit,
/// including a SIGKILL: a daemon that dies never leaves the lock held.
fn lock_exclusive(file: &File) -> Result<(), DaemonStarting> {
    let fd = file.as_raw_fd();
    let rc = unsafe { nix::libc::flock(fd, nix::libc::LOCK_EX | nix::libc::LOCK_NB) };
    if rc == 0 {
        Ok(())
    } else {
        let err = io::Error::last_os_error();
        if err.kind() == io::ErrorKind::WouldBlock {
            Err(DaemonStarting)
        } else {
            panic!("flock failed: {err}");
        }
    }
}

/// The held start lock. Dropping it (or process exit) releases the lock.
#[derive(Debug)]
pub struct StartLock {
    _file: File,
}

impl StartLock {
    /// Acquire the exclusive start lock beside `socket_path`, or fail with
    /// [`DaemonStarting`] if another process holds it. The returned guard
    /// must live for the daemon's lifetime.
    pub fn acquire(socket_path: &std::path::Path) -> Result<Self, DaemonStarting> {
        let path = start_lock_path(socket_path);
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .open(&path)
            .map_err(|e| {
                eprintln!(
                    "termos: failed to open daemon start lock {}: {e}",
                    path.display()
                );
                DaemonStarting
            })?;
        lock_exclusive(&file)?;
        Ok(Self { _file: file })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_path_is_socket_plus_dot_lock() {
        let p = start_lock_path(std::path::Path::new("/tmp/termos.sock"));
        assert_eq!(p, PathBuf::from("/tmp/termos.sock.lock"));
    }

    #[test]
    fn second_lock_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("termos.sock");
        let _first = StartLock::acquire(&socket).expect("first lock");
        // A second acquisition must fail while the first is held.
        let err = StartLock::acquire(&socket).unwrap_err();
        assert!(matches!(err, DaemonStarting));
    }

    #[test]
    fn lock_released_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("termos.sock");
        {
            let _first = StartLock::acquire(&socket).unwrap();
        }
        // After the guard drops, the lock is free again.
        let _second = StartLock::acquire(&socket).expect("reacquire after drop");
    }

    #[test]
    fn distinct_sockets_do_not_conflict() {
        let dir = tempfile::tempdir().unwrap();
        let a = StartLock::acquire(&dir.path().join("a.sock")).unwrap();
        let b = StartLock::acquire(&dir.path().join("b.sock")).unwrap();
        assert!(a._file.as_raw_fd() != b._file.as_raw_fd());
    }
}
