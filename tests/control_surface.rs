//! End-to-end test of the public scriptable control surface: drive a real
//! daemon over the line-delimited JSON verb protocol (the same protocol the
//! `termos action` / `termos subscribe` / `termos block-until-exit` CLI
//! commands speak).
//!
//! Verifies: session creation, window creation with ID capture, input +
//! capture + wait-for, output streaming (`subscribe`), exit-status reporting
//! (`block-until-exit`), and error envelopes for bad targets.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::{Arc, Once};
use std::time::Duration;

use termos::session::{Daemon, DaemonClient, VerbClient};

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

/// Spawn the daemon on a temp socket, returning the socket path.
fn start_daemon() -> (tempfile::TempDir, std::path::PathBuf) {
    isolate_state_dir();
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("tuios.sock");
    let daemon = Arc::new(Daemon::new());
    let path = socket.clone();
    std::thread::spawn(move || {
        let _ = daemon.run(&path);
    });
    // Wait for the socket to come up.
    for _ in 0..100 {
        if UnixStream::connect(&socket).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    (dir, socket)
}

/// A bare verb-protocol connection (line-delimited JSON).
struct VerbConn {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
}

impl VerbConn {
    fn connect(path: &Path) -> Self {
        let stream = UnixStream::connect(path).unwrap();
        let reader = BufReader::new(stream.try_clone().unwrap());
        Self { stream, reader }
    }

    /// Send one verb request and return the parsed response line.
    fn call(&mut self, verb: &str, params: serde_json::Value) -> serde_json::Value {
        let req = serde_json::json!({ "verb": verb, "params": params });
        let line = serde_json::to_string(&req).unwrap();
        self.stream.write_all(line.as_bytes()).unwrap();
        self.stream.write_all(b"\n").unwrap();
        self.stream.flush().unwrap();
        let mut buf = String::new();
        self.reader.read_line(&mut buf).unwrap();
        serde_json::from_str(buf.trim()).unwrap()
    }

    /// Read the next streamed line (subscribe mode).
    fn read_line(&mut self) -> serde_json::Value {
        let mut buf = String::new();
        self.reader.read_line(&mut buf).unwrap();
        serde_json::from_str(buf.trim()).unwrap()
    }
}

/// Collect a pane's streamed output until the `closed` event.
fn subscribe_until_closed(path: &Path, session: &str, window: &str) -> String {
    let mut conn = VerbConn::connect(path);
    // Ack line first.
    let ack = conn.call(
        "subscribe",
        serde_json::json!({ "session": session, "window": window }),
    );
    assert_eq!(ack["result"]["subscribed"], true);
    let mut data = String::new();
    loop {
        let line = conn.read_line();
        if line["result"]["closed"].as_bool().unwrap_or(false) {
            break;
        }
        data.push_str(line["result"]["data"].as_str().unwrap_or(""));
    }
    data
}

#[test]
fn control_surface_end_to_end() {
    let (_dir, socket) = start_daemon();
    let mut conn = VerbConn::connect(&socket);

    // Create a session through the verb protocol.
    let resp = conn.call(
        "new-session",
        serde_json::json!({ "name": "ci", "shell": "/bin/sh" }),
    );
    assert!(resp["result"]["session"]["name"].as_str() == Some("ci"));

    // List sessions.
    let resp = conn.call("list-sessions", serde_json::json!({}));
    let sessions = resp["result"]["sessions"].as_array().unwrap();
    assert_eq!(sessions.len(), 1);

    // Open a second window — ID capture is the returned window id.
    let resp = conn.call(
        "new-window",
        serde_json::json!({ "session": "ci", "shell": "/bin/sh" }),
    );
    let wid = resp["result"]["window"]["id"].as_str().unwrap().to_string();
    assert_eq!(wid, "w1");

    // Send text, then capture and wait for it.
    conn.call(
        "send-text",
        serde_json::json!({ "session": "ci", "window": wid, "text": "echo e2e-marker\r" }),
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let captured = loop {
        let resp = conn.call(
            "capture-pane",
            serde_json::json!({ "session": "ci", "window": wid }),
        );
        let content = resp["result"]["content"].as_str().unwrap_or("");
        if content.contains("e2e-marker") {
            break content.to_string();
        }
        assert!(std::time::Instant::now() < deadline, "marker never appeared");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(captured.contains("e2e-marker"));

    let resp = conn.call(
        "wait-for",
        serde_json::json!({ "session": "ci", "window": wid, "pattern": "e2e-marker" }),
    );
    assert_eq!(resp["result"]["matched"], true);

    // Subscribe: stream output, then have the shell exit.
    let sub_path = socket.clone();
    let sub_wid = wid.clone();
    let sub_session = "ci".to_string();
    let subscriber = std::thread::spawn(move || {
        subscribe_until_closed(&sub_path, &sub_session, &sub_wid)
    });
    std::thread::sleep(Duration::from_millis(200));
    conn.call(
        "send-text",
        serde_json::json!({ "session": "ci", "window": wid, "text": "echo streamed-line\r" }),
    );
    std::thread::sleep(Duration::from_millis(200));
    conn.call(
        "send-text",
        serde_json::json!({ "session": "ci", "window": wid, "text": "exit 7\r" }),
    );
    let streamed = subscriber.join().unwrap();
    assert!(
        streamed.contains("streamed-line"),
        "subscribed stream should contain the echoed marker, got: {streamed:?}"
    );

    // block-until-exit reports the shell's real exit code.
    let resp = conn.call(
        "block-until-exit",
        serde_json::json!({ "session": "ci", "window": wid, "timeout": "5000" }),
    );
    assert_eq!(resp["result"]["exit_code"], 7);
    assert_eq!(resp["result"]["success"], false);

    // A live window times out with a structured error. The timeout is sent
    // as a JSON number (the typed `termos action timeout=300` form) to
    // exercise the daemon's string coercion.
    let resp = conn.call(
        "block-until-exit",
        serde_json::json!({ "session": "ci", "window": "w0", "timeout": 300 }),
    );
    assert_eq!(resp["error"]["code"], "timeout");

    // A bad window yields the window-not-found envelope.
    let resp = conn.call(
        "capture-pane",
        serde_json::json!({ "session": "ci", "window": "nope" }),
    );
    assert_eq!(resp["error"]["code"], "window_not_found");

    // Clean up through the protocol.
    let resp = conn.call("kill-session", serde_json::json!({ "session": "ci" }));
    assert_eq!(resp["result"]["killed"], "ci");
}

#[test]
fn window_ids_stay_unique_across_close_and_reopen() {
    // Regression: window ids were derived from the *live* window count, so
    // closing a window made the next spawn reuse its id (two live windows
    // both named "w2" — scripting targets silently hit the wrong pane).
    // Ids must be monotonic per session and never repeat.
    let (_dir, socket) = start_daemon();
    let mut conn = VerbConn::connect(&socket);

    let resp = conn.call(
        "new-session",
        serde_json::json!({ "name": "ids", "shell": "/bin/sh" }),
    );
    assert_eq!(resp["result"]["session"]["name"], "ids");
    assert_eq!(resp["result"]["session"]["windows"], 1);

    // Session's first window is w0; open four more (w1..w4).
    let resp = conn.call("list-windows", serde_json::json!({ "session": "ids" }));
    let mut created: Vec<String> = resp["result"]["windows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(created, vec!["w0"]);
    for _ in 0..4 {
        let resp = conn.call(
            "new-window",
            serde_json::json!({ "session": "ids", "shell": "/bin/sh" }),
        );
        created.push(resp["result"]["window"]["id"].as_str().unwrap().to_string());
    }
    assert_eq!(created, vec!["w0", "w1", "w2", "w3", "w4"]);

    // Close w1 and w3 — the count drops, which used to trigger id reuse.
    for close_id in ["w1", "w3"] {
        let resp = conn.call(
            "close-window",
            serde_json::json!({ "session": "ids", "window": close_id }),
        );
        assert_eq!(resp["result"]["closed"], true);
    }

    // Reopen two windows: they must NOT be w1/w3 (or any prior id).
    let mut reopened = Vec::new();
    for _ in 0..2 {
        let resp = conn.call(
            "new-window",
            serde_json::json!({ "session": "ids", "shell": "/bin/sh" }),
        );
        reopened.push(resp["result"]["window"]["id"].as_str().unwrap().to_string());
    }
    assert_eq!(reopened, vec!["w5", "w6"], "ids must not be reused after close");

    // The full live list has no duplicates and covers w0..w6 minus w1/w3.
    let resp = conn.call("list-windows", serde_json::json!({ "session": "ids" }));
    let live: Vec<String> = resp["result"]["windows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(live.len(), 5, "w0, w2, w4, w5, w6 should be live");
    let mut sorted = live.clone();
    sorted.sort();
    assert_eq!(sorted, vec!["w0", "w2", "w4", "w5", "w6"]);
    let unique: std::collections::HashSet<&String> = live.iter().collect();
    assert_eq!(unique.len(), live.len(), "duplicate window ids: {live:?}");

    // Scripting still works against the reopened windows by id.
    conn.call(
        "send-text",
        serde_json::json!({ "session": "ids", "window": "w5", "text": "echo post-reopen\r" }),
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let captured = loop {
        let resp = conn.call(
            "capture-pane",
            serde_json::json!({ "session": "ids", "window": "w5" }),
        );
        let content = resp["result"]["content"].as_str().unwrap_or("");
        if content.contains("post-reopen") {
            break content.to_string();
        }
        assert!(std::time::Instant::now() < deadline, "marker never appeared");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(captured.contains("post-reopen"));

    let resp = conn.call("kill-session", serde_json::json!({ "session": "ids" }));
    assert_eq!(resp["result"]["killed"], "ids");
}

#[test]
fn verb_client_round_trip_over_a_real_socket() {
    let (_dir, socket) = start_daemon();
    let mut client = VerbClient::connect_to(&socket).unwrap();
    let result = client
        .request_json("hello", serde_json::json!({}))
        .unwrap();
    assert_eq!(result["daemon"], "termos");
    // list-sessions before any session exists is an empty list, not an error.
    let result = client
        .request_json("list-sessions", serde_json::json!({}))
        .unwrap();
    assert!(result["sessions"].as_array().unwrap().is_empty());
    // Unknown verbs surface as structured errors through the client too.
    let err = client
        .request_json("definitely-not-a-verb", serde_json::json!({}))
        .unwrap_err();
    match err {
        termos::session::verb_client::VerbClientError::Verb(e) => {
            assert_eq!(e.code, "unknown_verb");
        }
        other => panic!("expected a verb error, got {other:?}"),
    }
    // And the binary client still works on the same socket (protocols coexist).
    let bin = DaemonClient::connect_to(&socket).unwrap();
    assert!(bin.list().unwrap().is_empty());
}
