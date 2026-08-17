//! End-to-end daemon test: create a session over the Unix socket, attach,
//! forward input, and read the PTY's output back.

use std::sync::{Arc, Once};
use std::time::Duration;

use tuios::session::{Daemon, DaemonClient, Message};

/// Point persistence at a throwaway directory so tests never touch (or
/// resurrect from) the user's real state directory.
static ISOLATE_STATE: Once = Once::new();
fn isolate_state_dir() {
    ISOLATE_STATE.call_once(|| {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.keep();
        std::env::set_var("XDG_STATE_HOME", path);
    });
}

/// Spawn the daemon on a temp socket and connect, retrying until it is up.
fn start_daemon() -> (tempfile::TempDir, DaemonClient) {
    isolate_state_dir();
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("tuios.sock");

    let daemon = Arc::new(Daemon::new());
    let path = socket.clone();
    std::thread::spawn(move || {
        let _ = daemon.run(&path);
    });

    let client = loop {
        match DaemonClient::connect_to(&socket) {
            Ok(c) => break c,
            Err(_) => std::thread::sleep(Duration::from_millis(20)),
        }
    };

    (dir, client)
}

#[test]
fn daemon_create_attach_input_output_kill() {
    let (_dir, client) = start_daemon();

    // Create a session whose first window is a shell.
    let sessions = client.new_session("dev", "/bin/sh").unwrap();
    assert!(sessions.iter().any(|s| s.name == "dev"));

    // Attach and confirm the window list.
    let windows = client.attach("dev").unwrap();
    assert_eq!(windows.len(), 1);
    assert_eq!(windows[0].id, "w0");

    // Forward a command and read the echo back.
    client.send_input("w0", b"echo daemon-stage1\r").unwrap();
    client.set_read_timeout(Duration::from_secs(3)).unwrap();
    let mut buf = Vec::new();
    let mut found = false;
    for _ in 0..40 {
        match client.recv() {
            Ok(Message::PtyOutput { data, .. }) => {
                buf.extend_from_slice(&data);
                if buf.windows(b"daemon-stage1".len()).any(|w| w == b"daemon-stage1") {
                    found = true;
                    break;
                }
            }
            Ok(Message::PtyClosed { .. }) => break,
            Ok(_) => {}
            Err(_) => break,
        }
    }
    assert!(found, "daemon did not echo the marker back; got: {:?}", String::from_utf8_lossy(&buf));

    // The session shows as attached while we are on it.
    let list = client.list().unwrap();
    assert!(list.iter().any(|s| s.name == "dev" && s.attached));

    // Kill removes it.
    let after = client.kill("dev").unwrap();
    assert!(!after.iter().any(|s| s.name == "dev"));
}

#[test]
fn daemon_rejects_duplicate_session_and_broadcasts_to_two_clients() {
    let (dir, client) = start_daemon();
    let socket = dir.path().join("tuios.sock");
    client.new_session("dup", "/bin/sh").unwrap();
    assert!(client.new_session("dup", "/bin/sh").is_err());

    // Two clients attach to the same session (multi-client broadcast).
    let first = DaemonClient::connect_to(&socket).unwrap();
    first.attach("dup").unwrap();
    let second = DaemonClient::connect_to(&socket).unwrap();
    second.attach("dup").unwrap();

    // The session shows attached while either client is on it.
    let list = client.list().unwrap();
    assert!(list.iter().any(|s| s.name == "dup" && s.attached));

    // Input from one client is broadcast to both.
    first.send_input("w0", b"echo broadcast-stage1\r").unwrap();
    first.set_read_timeout(Duration::from_secs(1)).unwrap();
    second.set_read_timeout(Duration::from_secs(1)).unwrap();

    let mut got_first = false;
    let mut got_second = false;
    for _ in 0..60 {
        if !got_first {
            if let Ok(Message::PtyOutput { data, .. }) = first.recv() {
                if data.windows(b"broadcast-stage1".len()).any(|w| w == b"broadcast-stage1") {
                    got_first = true;
                }
            }
        }
        if !got_second {
            if let Ok(Message::PtyOutput { data, .. }) = second.recv() {
                if data.windows(b"broadcast-stage1".len()).any(|w| w == b"broadcast-stage1") {
                    got_second = true;
                }
            }
        }
        if got_first && got_second {
            break;
        }
    }
    assert!(got_first, "first client did not receive the echo");
    assert!(got_second, "second client did not receive the echo");
}

#[test]
fn daemon_lists_and_kills_missing_session() {
    let (_dir, client) = start_daemon();
    assert!(client.kill("nope").is_err());
    let list = client.list().unwrap();
    assert!(list.is_empty());
}
