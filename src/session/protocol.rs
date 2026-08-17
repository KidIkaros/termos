//! The daemon/client wire protocol — ported from TUIOS `internal/session`
//! (`protocol.go`, `codec.go`), using the JSON codec with a length-prefixed
//! frame.

use std::io::{self, Read, Write};

use serde::{Deserialize, Serialize};

use super::model::{SessionInfo, WindowInfo};

/// Maximum accepted frame size (defends against a corrupt length prefix).
pub const MAX_FRAME: usize = 64 * 1024 * 1024;

/// The protocol version string.
pub const VERSION: &str = "0.1.0";

/// A single control message between the client and daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum Message {
    /// Client → daemon: handshake.
    Hello { name: String },
    /// Daemon → client: handshake reply + current sessions.
    Welcome {
        version: String,
        sessions: Vec<SessionInfo>,
    },
    /// Client → daemon: request the session list.
    List,
    /// Daemon → client: the session list.
    ListResult { sessions: Vec<SessionInfo> },
    /// Client → daemon: create a session (with its first window).
    New { name: String, shell: String },
    /// Client → daemon: attach to a session (starts streaming).
    Attach { name: String },
    /// Daemon → client: attach acknowledged + window list.
    Attached { windows: Vec<WindowInfo> },
    /// Client → daemon: detach (stops streaming).
    Detach,
    /// Client → daemon: kill a session.
    Kill { name: String },
    /// Client → daemon: spawn a window in the attached session.
    NewWindow { shell: String, workspace: i32 },
    /// Client → daemon: close a window.
    CloseWindow { window: String },
    /// Daemon → client: a window was spawned (reply to `NewWindow`).
    WindowAdded { window: WindowInfo },
    /// Daemon → client: a window was closed.
    WindowClosed { window: String },
    /// Client → daemon: forward encoded bytes to a window's PTY.
    Input { window: String, data: Vec<u8> },
    /// Client → daemon: resize a window's PTY.
    Resize {
        window: String,
        cols: u16,
        rows: u16,
    },
    /// Daemon → client: a window's PTY output chunk.
    PtyOutput { window: String, data: Vec<u8> },
    /// Daemon → client: a window's shell exited.
    PtyClosed { window: String },
    /// Client → daemon: report a window's agent state (`set-agent-state`).
    /// `window: None` targets the session's most recently active window (the
    /// port's approximation of "focused", since focus lives client-side).
    SetAgentState {
        session: Option<String>,
        window: Option<String>,
        state: String,
        message: String,
        harness: String,
    },
    /// Daemon → client: a window's agent state changed (broadcast).
    AgentStateChanged {
        window: String,
        state: String,
        message: String,
        harness: String,
    },
    /// Client → daemon: read a window's agent state (`get-agent-state`).
    GetAgentState {
        session: Option<String>,
        window: Option<String>,
    },
    /// Daemon → client: the window's agent state.
    AgentStateResult {
        window: String,
        state: String,
        message: String,
        harness: String,
    },
    /// Client → daemon: write raw bytes to a window's PTY (`send-keys` /
    /// `send-text`; the client does key encoding).
    WriteInput {
        session: Option<String>,
        window: Option<String>,
        data: Vec<u8>,
    },
    /// Client → daemon: capture a window's recent output (`capture-pane`).
    CapturePane {
        session: Option<String>,
        window: Option<String>,
    },
    /// Daemon → client: the captured output (raw bytes as lossy UTF-8).
    PaneCapture { window: String, content: String },
    /// Client → daemon: wait until a window's output matches a regex
    /// (`wait-for`). The daemon polls its output ring until the deadline.
    WaitFor {
        session: Option<String>,
        window: Option<String>,
        pattern: String,
        timeout_ms: u64,
    },
    /// Daemon → client: the wait outcome.
    WaitResult { window: String, matched: bool },
    /// Client → daemon: execute a tape script in a session (`tape exec`).
    /// The daemon parses it and streams the commands to attached clients.
    TapeExecute { session: String, script: String },
    /// Daemon → client: one tape command from a remote `tape exec`.
    TapeCommand {
        index: usize,
        total: usize,
        command: crate::tape::command::Command,
    },
    /// Daemon → client: a remote tape finished.
    TapeFinished { total: usize },
    /// Daemon → client: an error reply.
    Error { message: String },
}

/// Write a length-prefixed (u32 BE) JSON frame.
pub fn write_message<W: Write>(w: &mut W, msg: &Message) -> io::Result<()> {
    let bytes = serde_json::to_vec(msg).map_err(io::Error::other)?;
    if bytes.len() > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too large"));
    }
    w.write_all(&(bytes.len() as u32).to_be_bytes())?;
    w.write_all(&bytes)?;
    w.flush()
}

/// Read a length-prefixed JSON frame.
pub fn read_message<R: Read>(r: &mut R) -> io::Result<Message> {
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too large"));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    serde_json::from_slice(&buf).map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(msg: Message) -> Message {
        let mut buf = Vec::new();
        write_message(&mut buf, &msg).unwrap();
        let mut cursor = io::Cursor::new(buf);
        read_message(&mut cursor).unwrap()
    }

    #[test]
    fn input_message_round_trips() {
        let msg = Message::Input {
            window: "w0".to_string(),
            data: b"\x1b[A\x1b[B".to_vec(),
        };
        match round_trip(msg) {
            Message::Input { window, data } => {
                assert_eq!(window, "w0");
                assert_eq!(data, b"\x1b[A\x1b[B");
            }
            other => panic!("wrong message: {other:?}"),
        }
    }

    #[test]
    fn welcome_round_trips_with_sessions() {
        let msg = Message::Welcome {
            version: VERSION.to_string(),
            sessions: vec![SessionInfo {
                id: "id-1".to_string(),
                name: "dev".to_string(),
                created_at: 123,
                attached: false,
                windows: 2,
                restored: false,
            }],
        };
        match round_trip(msg) {
            Message::Welcome { sessions, .. } => {
                assert_eq!(sessions.len(), 1);
                assert_eq!(sessions[0].name, "dev");
            }
            other => panic!("wrong message: {other:?}"),
        }
    }

    #[test]
    fn attached_round_trips_window_list() {
        let msg = Message::Attached {
            windows: vec![WindowInfo {
                id: "w0".to_string(),
                title: "Terminal".to_string(),
                workspace: 1,
                cols: 80,
                rows: 24,
                agent_state: String::new(),
                agent_message: String::new(),
                agent_harness: String::new(),
            }],
        };
        match round_trip(msg) {
            Message::Attached { windows } => assert_eq!(windows[0].cols, 80),
            other => panic!("wrong message: {other:?}"),
        }
    }

    #[test]
    fn agent_state_messages_round_trip() {
        let msg = Message::SetAgentState {
            session: Some("dev".to_string()),
            window: Some("w1".to_string()),
            state: "needs_input".to_string(),
            message: "awaiting approval".to_string(),
            harness: "claude-code".to_string(),
        };
        match round_trip(msg) {
            Message::SetAgentState {
                session,
                window,
                state,
                message,
                harness,
            } => {
                assert_eq!(session.as_deref(), Some("dev"));
                assert_eq!(window.as_deref(), Some("w1"));
                assert_eq!(state, "needs_input");
                assert_eq!(message, "awaiting approval");
                assert_eq!(harness, "claude-code");
            }
            other => panic!("wrong message: {other:?}"),
        }

        let changed = Message::AgentStateChanged {
            window: "w0".to_string(),
            state: "done".to_string(),
            message: String::new(),
            harness: "claude".to_string(),
        };
        match round_trip(changed) {
            Message::AgentStateChanged {
                window,
                state,
                harness,
                ..
            } => {
                assert_eq!(window, "w0");
                assert_eq!(state, "done");
                assert_eq!(harness, "claude");
            }
            other => panic!("wrong message: {other:?}"),
        }
    }

    #[test]
    fn truncated_frame_errors() {
        let mut buf = vec![0u8, 0, 0, 10, b'{', b'"'];
        let mut cursor = io::Cursor::new(&mut buf);
        assert!(read_message(&mut cursor).is_err());
    }
}
