//! End-to-end daemon test: create a session over the Unix socket, attach,
//! forward input, and read the PTY's output back.

use std::sync::{Arc, Once};
use std::time::Duration;

use termos::session::{Daemon, DaemonClient, Message};

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
                if buf
                    .windows(b"daemon-stage1".len())
                    .any(|w| w == b"daemon-stage1")
                {
                    found = true;
                    break;
                }
            }
            Ok(Message::PtyClosed { .. }) => break,
            Ok(_) => {}
            Err(_) => break,
        }
    }
    assert!(
        found,
        "daemon did not echo the marker back; got: {:?}",
        String::from_utf8_lossy(&buf)
    );

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
                if data
                    .windows(b"broadcast-stage1".len())
                    .any(|w| w == b"broadcast-stage1")
                {
                    got_first = true;
                }
            }
        }
        if !got_second {
            if let Ok(Message::PtyOutput { data, .. }) = second.recv() {
                if data
                    .windows(b"broadcast-stage1".len())
                    .any(|w| w == b"broadcast-stage1")
                {
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

/// The daemon fires window-lifecycle hooks for the windows it spawns and
/// closes (the authoritative fire sites for daemon-mode windows).
#[test]
fn daemon_fires_window_lifecycle_hooks() {
    use std::sync::Mutex;
    use std::time::Instant;

    isolate_state_dir();
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("tuios.sock");

    let daemon = Arc::new(Daemon::new());
    // Register placeholder commands so `fire` runs (the runner replaces their
    // execution) — mirroring the CLI's `daemon.load_hooks(&config.hooks)`.
    let mut hook_cfg = std::collections::HashMap::new();
    for ev in [
        "after-new-window",
        "after-close-window",
        "after-focus-change",
        "after-workspace-switch",
        "after-attach",
        "after-detach",
        "after-layout-change",
        "after-resize",
        "after-agent-state",
    ] {
        hook_cfg.insert(ev.to_string(), toml::Value::String("dummy".into()));
    }
    daemon.load_hooks(&hook_cfg);
    let seen: Arc<Mutex<Vec<(termos::hooks::Event, String, String)>>> =
        Arc::new(Mutex::new(Vec::new()));
    let seen2 = Arc::clone(&seen);
    daemon.set_hook_runner(move |_, ctx| {
        if let Some(ev) = ctx.event {
            seen2
                .lock()
                .unwrap()
                .push((ev, ctx.session_id.clone(), ctx.window_id.clone()));
        }
    });
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

    let wait_until = |pred: &dyn Fn(&(termos::hooks::Event, String, String)) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let hit = seen.lock().unwrap().iter().any(pred);
            if hit || Instant::now() >= deadline {
                return hit;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    };

    // Creating a session spawns its first window (w0) → after-new-window.
    client.new_session("hooks", "/bin/sh").unwrap();
    assert!(wait_until(&|(e, sess, wid)| {
        *e == termos::hooks::Event::AfterNewWindow && sess == "hooks" && wid == "w0"
    }));

    // Adding a window over the protocol → after-new-window for the new id.
    client.attach("hooks").unwrap();
    client
        .send(&Message::NewWindow {
            shell: "/bin/sh".to_string(),
            workspace: 1,
        })
        .unwrap();
    client.set_read_timeout(Duration::from_secs(3)).unwrap();
    let mut added = false;
    for _ in 0..40 {
        if let Ok(Message::WindowAdded { .. }) = client.recv() {
            added = true;
            break;
        }
    }
    assert!(added, "no WindowAdded reply");
    assert!(wait_until(&|(e, sess, wid)| {
        *e == termos::hooks::Event::AfterNewWindow && sess == "hooks" && wid == "w1"
    }));

    // Closing a window → after-close-window for that id.
    client
        .send(&Message::CloseWindow {
            window: "w1".to_string(),
        })
        .unwrap();
    let mut closed = false;
    for _ in 0..40 {
        if let Ok(Message::WindowClosed { window }) = client.recv() {
            if window == "w1" {
                closed = true;
                break;
            }
        }
    }
    assert!(closed, "no WindowClosed reply");
    assert!(wait_until(&|(e, sess, wid)| {
        *e == termos::hooks::Event::AfterCloseWindow && sess == "hooks" && wid == "w1"
    }));
}

/// `set-agent-state` reports a window's agent state; attached clients receive
/// the `AgentStateChanged` broadcast.
#[test]
fn daemon_set_agent_state_broadcasts_to_clients() {
    use std::sync::Mutex;

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
    client.new_session("agent", "/bin/sh").unwrap();
    client.attach("agent").unwrap();

    // The reporting CLI is a separate connection (not attached).
    let reporter = DaemonClient::connect_to(&socket).unwrap();
    reporter
        .send(&Message::SetAgentState {
            session: Some("agent".to_string()),
            window: None, // targets the most recently active / first window
            state: "needs_input".to_string(),
            message: "awaiting approval".to_string(),
            harness: "claude-code".to_string(),
        })
        .unwrap();

    // The attached client sees the broadcast with the full payload.
    let got: Arc<Mutex<Option<Message>>> = Arc::new(Mutex::new(None));
    client.set_read_timeout(Duration::from_secs(3)).unwrap();
    for _ in 0..40 {
        match client.recv() {
            Ok(Message::AgentStateChanged {
                window,
                state,
                message,
                harness,
            }) if state == "needs_input" => {
                assert_eq!(window, "w0");
                assert_eq!(message, "awaiting approval");
                assert_eq!(harness, "claude-code");
                *got.lock().unwrap() = Some(Message::AgentStateChanged {
                    window,
                    state,
                    message,
                    harness,
                });
                break;
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    assert!(
        got.lock().unwrap().is_some(),
        "attached client did not receive the AgentStateChanged broadcast"
    );

    // An attached client also sees the state in a fresh attach (WindowInfo).
    let fresh = DaemonClient::connect_to(&socket).unwrap();
    let windows = fresh.attach("agent").unwrap();
    assert_eq!(windows[0].agent_state, "needs_input");
    assert_eq!(windows[0].agent_harness, "claude-code");

    // Invalid state is rejected at the CLI layer; the daemon rejects an
    // unknown window.
    let bad = DaemonClient::connect_to(&socket).unwrap();
    bad.send(&Message::SetAgentState {
        session: Some("agent".to_string()),
        window: Some("nope".to_string()),
        state: "done".to_string(),
        message: String::new(),
        harness: String::new(),
    })
    .unwrap();
    bad.set_read_timeout(Duration::from_secs(3)).unwrap();
    let mut rejected = false;
    for _ in 0..10 {
        if let Ok(Message::Error { .. }) = bad.recv() {
            rejected = true;
            break;
        }
    }
    assert!(rejected, "unknown window should error");
}

/// The agent verbs (write-input, capture-pane, wait-for, get-agent-state)
/// work headlessly against the daemon's output rings.
#[test]
fn daemon_agent_verbs_work_headlessly() {
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
    client.new_session("verbs", "/bin/sh").unwrap();

    // send-text writes bytes to the PTY (the shell echoes them back into the
    // ring).
    client
        .send(&Message::WriteInput {
            session: Some("verbs".to_string()),
            window: None,
            data: b"echo verb-stage1\r".to_vec(),
        })
        .unwrap();
    // Wait for the echo to land in the ring.
    client
        .send(&Message::WaitFor {
            session: Some("verbs".to_string()),
            window: None,
            pattern: "verb-stage1".to_string(),
            timeout_ms: 5000,
        })
        .unwrap();
    client.set_read_timeout(Duration::from_secs(3)).unwrap();
    let mut matched = false;
    for _ in 0..40 {
        if let Ok(Message::WaitResult { matched: m, .. }) = client.recv() {
            matched = m;
            break;
        }
    }
    assert!(matched, "wait-for did not observe the echoed output");

    // capture-pane returns the ring content.
    client
        .send(&Message::CapturePane {
            session: Some("verbs".to_string()),
            window: None,
        })
        .unwrap();
    let mut content = String::new();
    for _ in 0..40 {
        if let Ok(Message::PaneCapture { content: c, .. }) = client.recv() {
            content = c;
            break;
        }
    }
    assert!(
        content.contains("verb-stage1"),
        "capture-pane missing the echo; got: {content:?}"
    );

    // get-agent-state starts at none and reflects set-agent-state.

    client
        .send(&Message::GetAgentState {
            session: Some("verbs".to_string()),
            window: None,
        })
        .unwrap();
    let mut state = String::new();
    for _ in 0..40 {
        if let Ok(Message::AgentStateResult { state: s, .. }) = client.recv() {
            state = s;
            break;
        }
    }
    assert_eq!(state, "", "fresh window should report no agent state");

    client
        .send(&Message::SetAgentState {
            session: Some("verbs".to_string()),
            window: None,
            state: "working".to_string(),
            message: String::new(),
            harness: String::new(),
        })
        .unwrap();
    client
        .send(&Message::GetAgentState {
            session: Some("verbs".to_string()),
            window: None,
        })
        .unwrap();
    let mut state = String::new();
    for _ in 0..40 {
        if let Ok(Message::AgentStateResult { state: s, .. }) = client.recv() {
            state = s;
            break;
        }
    }
    assert_eq!(state, "working");
}

/// `tape exec` streams parsed commands to the session's attached clients.
#[test]
fn daemon_tape_exec_broadcasts_commands() {
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
    client.new_session("exec", "/bin/sh").unwrap();
    client.attach("exec").unwrap();

    let reporter = DaemonClient::connect_to(&socket).unwrap();
    reporter
        .send(&Message::TapeExecute {
            session: "exec".to_string(),
            script: "Type \"hi\"\nEnter\nSleep 100ms\n".to_string(),
        })
        .unwrap();

    client.set_read_timeout(Duration::from_secs(3)).unwrap();
    let mut commands = 0usize;
    let mut finished = false;
    for _ in 0..40 {
        match client.recv() {
            Ok(Message::TapeCommand { index, total, .. }) => {
                commands += 1;
                assert!(index < total);
            }
            Ok(Message::TapeFinished { total }) => {
                finished = true;
                assert_eq!(total, 3);
                break;
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    assert_eq!(commands, 3, "expected 3 TapeCommand frames");
    assert!(finished, "no TapeFinished frame");
}
