//! The JSON verb protocol — ported from Go TUIOS `internal/session/verb_protocol.go`,
//! `verb_hints.go`, and `verb_compat.go`.
//!
//! A typed, line-delimited JSON protocol layered additively on the daemon
//! socket. One request per line:
//!
//! ```json
//! {"id": 1, "verb": "list-windows", "params": {"session": "work"}}
//! ```
//!
//! and one response per line, either:
//!
//! ```json
//! {"id": 1, "result": {"type": "window_list", ...}}
//! ```
//!
//! or:
//!
//! ```json
//! {"id": 1, "error": {"code": "session_not_found", "message": "..."}}
//! ```

#![allow(clippy::result_large_err)]

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// The version of the JSON verb protocol. Bump only on an incompatible change
/// to the envelope or to an existing verb's contract; adding a new verb is
/// backward compatible.
pub const VERB_PROTOCOL_VERSION: u32 = 1;

/// The oldest protocol version this daemon still serves.
pub const MIN_VERB_PROTOCOL_VERSION: u32 = 1;

// Stable string error codes returned in the response error envelope.
pub const ERR_INVALID_REQUEST: &str = "invalid_request";
pub const ERR_UNKNOWN_VERB: &str = "unknown_verb";
pub const ERR_INVALID_PARAMS: &str = "invalid_params";
pub const ERR_SESSION_NOT_FOUND: &str = "session_not_found";
pub const ERR_WINDOW_NOT_FOUND: &str = "window_not_found";
pub const ERR_NO_WINDOWS: &str = "no_windows";
pub const ERR_PTY_NOT_FOUND: &str = "pty_not_found";
pub const ERR_NEEDS_CLIENT: &str = "needs_client";
pub const ERR_OPTION_NOT_FOUND: &str = "option_not_found";
pub const ERR_COMMAND_FAILED: &str = "command_failed";
pub const ERR_TIMEOUT: &str = "timeout";
pub const ERR_INTERNAL: &str = "internal";
pub const ERR_PROTOCOL_MISMATCH: &str = "protocol_mismatch";

/// The closed value sets the protocol accepts.
pub const CAPTURE_SOURCES: &[&str] = &["visible", "recent"];
pub const WAIT_CONDITIONS: &[&str] = &[
    "session-exists",
    "window-output",
    "window-exit",
    "window-idle",
];
pub const KNOWN_EVENT_TYPES: &[&str] = &[
    "window-created",
    "window-closed",
    "window-exit",
    "window-retitled",
    "window-focused",
    "window-moved",
    "window-minimized",
    "window-restored",
    "workspace-switched",
    "output",
    "bell",
    "mode-changed",
    "session-created",
    "session-closed",
    "gap",
];

/// Agent state names accepted by `set-agent-state`.
pub const AGENT_STATE_NAMES: &[&str] =
    &["none", "working", "needs_input", "idle", "done", "errored"];

/// Agent source names accepted by `set-agent-state`.
pub const AGENT_SOURCE_NAMES: &[&str] = &["report", "osc", "harness", "override"];

/// One decoded request line. `id` is opaque (number, string, or absent) and
/// echoed back on the response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerbRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    pub verb: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// The error envelope with a stable string code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerbError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<Box<VerbHint>>,
}

impl VerbError {
    /// Build a `VerbError` with the given code and message.
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
            hint: None,
        }
    }

    /// Build a `VerbError` carrying a hint.
    pub fn with_hint(code: &str, message: impl Into<String>, hint: VerbHint) -> Self {
        let mut e = Self::new(code, message);
        if !hint.is_empty() {
            e.hint = Some(Box::new(hint));
        }
        e
    }
}

impl std::fmt::Display for VerbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for VerbError {}

/// One response line. Exactly one of `result` or `error` is set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerbResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<VerbError>,
}

impl VerbResponse {
    /// A successful response carrying a result value.
    pub fn ok(id: Option<Value>, result: Value) -> Self {
        Self {
            id,
            result: Some(result),
            error: None,
        }
    }

    /// An error response.
    pub fn err(id: Option<Value>, error: VerbError) -> Self {
        Self {
            id,
            result: None,
            error: Some(error),
        }
    }

    /// Serialize as one newline-terminated JSON line.
    pub fn to_line(&self) -> String {
        let mut s = serde_json::to_string(self).unwrap_or_else(|_| {
            r#"{"error":{"code":"internal","message":"failed to encode response"}}"#.to_string()
        });
        s.push('\n');
        s
    }
}

/// The structured remedy attached to an error envelope. Every field is
/// optional; a hint is only attached when at least one field is meaningful.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VerbHint {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub verb: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub command: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub param: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accepted: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub did_you_mean: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub available: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
}

impl VerbHint {
    /// Reports whether the hint carries nothing worth serializing.
    pub fn is_empty(&self) -> bool {
        self.verb.is_empty()
            && self.command.is_empty()
            && self.param.is_empty()
            && self.accepted.is_empty()
            && self.did_you_mean.is_empty()
            && self.available.is_empty()
            && self.detail.is_empty()
    }
}

/// Build an `invalid_params` error naming the offending parameter.
pub fn invalid_param(param: &str, message: impl Into<String>, accepted: &[&str]) -> VerbError {
    VerbError::with_hint(
        ERR_INVALID_PARAMS,
        message,
        VerbHint {
            param: param.to_string(),
            accepted: accepted.iter().map(|s| s.to_string()).collect(),
            verb: "list-verbs".to_string(),
            ..Default::default()
        },
    )
}

/// One parameter of a verb, for the `list-verbs` introspection output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerbParam {
    pub name: String,
    /// `string` | `int` | `bool` | `[]string`
    #[serde(rename = "type")]
    pub ty: String,
    #[serde(skip_serializing_if = "is_false")]
    pub required: bool,
    pub description: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub accepted: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub default: String,
}

fn is_false(b: &bool) -> bool {
    !b
}

/// The serialized form of a verb entry in the `list-verbs` result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerbDoc {
    pub verb: String,
    pub description: String,
    pub params: Vec<VerbParam>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<String>,
}

/// The shared session selector parameter.
pub fn session_param() -> VerbParam {
    VerbParam {
        name: "session".into(),
        ty: "string".into(),
        required: false,
        description: "Session name. Omit to target the most recently active session.".into(),
        accepted: vec![],
        default: String::new(),
    }
}

/// The shared window selector parameter.
pub fn window_param() -> VerbParam {
    VerbParam {
        name: "window".into(),
        ty: "string".into(),
        required: false,
        description: "Window id or name. Omit to target the focused window.".into(),
        accepted: vec![],
        default: String::new(),
    }
}

/// A verb entry: documentation plus handler.
#[derive(Clone)]
pub struct VerbEntry {
    pub description: String,
    pub params: Vec<VerbParam>,
    pub examples: Vec<String>,
    pub handler: VerbHandler,
}

/// A verb handler executes one verb. `params` carries the raw JSON of the
/// request's params object (may be null). It returns a result value to
/// serialize, or a boxed `VerbError` describing why it failed.
pub type VerbHandler = Arc<dyn Fn(&Value) -> Result<Value, VerbError> + Send + Sync>;

use std::sync::Arc;

/// The dispatch table for every JSON verb the daemon supports.
pub struct VerbRegistry {
    entries: BTreeMap<String, VerbEntry>,
}

impl VerbRegistry {
    /// Build the registry with all standard verbs registered.
    #[allow(clippy::result_large_err)]
    pub fn new() -> Self {
        let mut entries = BTreeMap::new();

        // hello
        entries.insert(
            "hello".to_string(),
            VerbEntry {
                description: "Handshake: report the protocol version this daemon speaks and the version range it accepts.".into(),
                params: vec![
                    VerbParam { name: "client".into(), ty: "string".into(), required: false, description: "Name of the calling program, for the daemon log.".into(), accepted: vec![], default: String::new() },
                    VerbParam { name: "version".into(), ty: "string".into(), required: false, description: "Version string of the calling program.".into(), accepted: vec![], default: String::new() },
                    VerbParam { name: "protocol".into(), ty: "int".into(), required: false, description: "Protocol version the caller speaks.".into(), accepted: vec![], default: String::new() },
                ],
                examples: vec![r#"{"id":1,"verb":"hello","params":{"client":"tuios","version":"1.2.3","protocol":1}}"#.into()],
                handler: Arc::new(|_params| {
                    Ok(serde_json::json!({
                        "type": "hello",
                        "protocol": VERB_PROTOCOL_VERSION,
                        "min_protocol": MIN_VERB_PROTOCOL_VERSION,
                    }))
                }),
            },
        );

        // list-verbs
        entries.insert(
            "list-verbs".to_string(),
            VerbEntry {
                description: "List every supported verb with its parameter schema and examples, plus the protocol version and error-code catalog.".into(),
                params: vec![VerbParam { name: "verb".into(), ty: "string".into(), required: false, description: "Describe only this verb. Omit to describe all of them.".into(), accepted: vec![], default: String::new() }],
                examples: vec![
                    r#"{"id":1,"verb":"list-verbs"}"#.into(),
                    r#"{"id":1,"verb":"list-verbs","params":{"verb":"capture-pane"}}"#.into(),
                ],
                handler: Arc::new(|_params| {
                    // Returns a placeholder; the daemon overrides this handler
                    // with one that has access to the registry.
                    Ok(serde_json::json!({"type": "verb_list", "version": VERB_PROTOCOL_VERSION, "verbs": []}))
                }),
            },
        );

        // list-sessions
        entries.insert(
            "list-sessions".to_string(),
            VerbEntry {
                description: "List all sessions the daemon holds.".into(),
                params: vec![],
                examples: vec![r#"{"id":1,"verb":"list-sessions"}"#.into()],
                handler: Arc::new(|_params| {
                    Ok(serde_json::json!({"type": "session_list", "sessions": []}))
                }),
            },
        );

        // session-info
        entries.insert(
            "session-info".to_string(),
            VerbEntry {
                description: "Report details about one session.".into(),
                params: vec![session_param()],
                examples: vec![
                    r#"{"id":1,"verb":"session-info","params":{"session":"work"}}"#.into(),
                ],
                handler: Arc::new(|_params| {
                    Err(VerbError::new(ERR_SESSION_NOT_FOUND, "session not found"))
                }),
            },
        );

        // list-windows
        entries.insert(
            "list-windows".to_string(),
            VerbEntry {
                description: "List the windows in a session.".into(),
                params: vec![session_param()],
                examples: vec![
                    r#"{"id":1,"verb":"list-windows","params":{"session":"work"}}"#.into(),
                ],
                handler: Arc::new(|_params| {
                    Ok(serde_json::json!({"type": "window_list", "windows": []}))
                }),
            },
        );

        // new-window
        entries.insert(
            "new-window".to_string(),
            VerbEntry {
                description: "Create a new window.".into(),
                params: vec![
                    session_param(),
                    VerbParam {
                        name: "name".into(),
                        ty: "string".into(),
                        required: false,
                        description: "Name for the new window. Omit to use the shell's title."
                            .into(),
                        accepted: vec![],
                        default: String::new(),
                    },
                ],
                examples: vec![
                    r#"{"id":1,"verb":"new-window","params":{"session":"work","name":"build"}}"#
                        .into(),
                ],
                handler: Arc::new(|_params| {
                    Err(VerbError::new(ERR_SESSION_NOT_FOUND, "session not found"))
                }),
            },
        );

        // close-window
        entries.insert(
            "close-window".to_string(),
            VerbEntry {
                description: "Close a window.".into(),
                params: vec![session_param(), window_param()],
                examples: vec![r#"{"id":1,"verb":"close-window","params":{"session":"work","window":"build"}}"#.into()],
                handler: Arc::new(|_params| {
                    Err(VerbError::new(ERR_SESSION_NOT_FOUND, "session not found"))
                }),
            },
        );

        // send-keys
        entries.insert(
            "send-keys".to_string(),
            VerbEntry {
                description: "Send parsed key tokens to a window.".into(),
                params: vec![
                    session_param(),
                    window_param(),
                    VerbParam { name: "keys".into(), ty: "string".into(), required: true, description: r#"Key sequence, e.g. "ctrl+b,n" or "Hello World"."#.into(), accepted: vec![], default: String::new() },
                    VerbParam { name: "literal".into(), ty: "bool".into(), required: false, description: "Send the keys to the PTY without parsing them as key names.".into(), accepted: vec![], default: "false".into() },
                    VerbParam { name: "raw".into(), ty: "bool".into(), required: false, description: "Treat every character as its own key instead of splitting on spaces and commas.".into(), accepted: vec![], default: "false".into() },
                ],
                examples: vec![r#"{"id":1,"verb":"send-keys","params":{"session":"work","keys":"ls,Enter"}}"#.into()],
                handler: Arc::new(|_params| {
                    Err(VerbError::new(ERR_SESSION_NOT_FOUND, "session not found"))
                }),
            },
        );

        // send-text
        entries.insert(
            "send-text".to_string(),
            VerbEntry {
                description: "Send literal text to a window's PTY.".into(),
                params: vec![
                    session_param(),
                    window_param(),
                    VerbParam {
                        name: "text".into(),
                        ty: "string".into(),
                        required: true,
                        description: "Text written verbatim to the PTY.".into(),
                        accepted: vec![],
                        default: String::new(),
                    },
                ],
                examples: vec![
                    r#"{"id":1,"verb":"send-text","params":{"session":"work","text":"echo hi\n"}}"#
                        .into(),
                ],
                handler: Arc::new(|_params| {
                    Err(VerbError::new(ERR_SESSION_NOT_FOUND, "session not found"))
                }),
            },
        );

        // capture-pane
        entries.insert(
            "capture-pane".to_string(),
            VerbEntry {
                description: "Capture a pane's content.".into(),
                params: vec![
                    session_param(),
                    window_param(),
                    VerbParam { name: "source".into(), ty: "string".into(), required: false, description: "Which buffer to capture.".into(), accepted: CAPTURE_SOURCES.iter().map(|s| s.to_string()).collect(), default: "visible".into() },
                    VerbParam { name: "styled".into(), ty: "bool".into(), required: false, description: "Include ANSI styling in the captured text.".into(), accepted: vec![], default: "false".into() },
                    VerbParam { name: "lines".into(), ty: "int".into(), required: false, description: "Keep only the last N non-empty-tailed lines.".into(), accepted: vec![], default: String::new() },
                    VerbParam { name: "start".into(), ty: "int".into(), required: false, description: "1-based inclusive first line of the region to keep.".into(), accepted: vec![], default: String::new() },
                    VerbParam { name: "end".into(), ty: "int".into(), required: false, description: "1-based inclusive last line of the region to keep.".into(), accepted: vec![], default: String::new() },
                ],
                examples: vec![r#"{"id":1,"verb":"capture-pane","params":{"session":"work","source":"recent","lines":50}}"#.into()],
                handler: Arc::new(|_params| {
                    Err(VerbError::new(ERR_SESSION_NOT_FOUND, "session not found"))
                }),
            },
        );

        // resize
        entries.insert(
            "resize".to_string(),
            VerbEntry {
                description: "Resize a window's PTY.".into(),
                params: vec![
                    session_param(),
                    window_param(),
                    VerbParam { name: "width".into(), ty: "int".into(), required: true, description: "New width in columns. Must be positive.".into(), accepted: vec![], default: String::new() },
                    VerbParam { name: "height".into(), ty: "int".into(), required: true, description: "New height in rows. Must be positive.".into(), accepted: vec![], default: String::new() },
                ],
                examples: vec![r#"{"id":1,"verb":"resize","params":{"session":"work","width":120,"height":40}}"#.into()],
                handler: Arc::new(|_params| {
                    Err(VerbError::new(ERR_SESSION_NOT_FOUND, "session not found"))
                }),
            },
        );

        // kill-session
        entries.insert(
            "kill-session".to_string(),
            VerbEntry {
                description: "Terminate a session and every window in it.".into(),
                params: vec![VerbParam {
                    name: "session".into(),
                    ty: "string".into(),
                    required: true,
                    description: "Session to terminate.".into(),
                    accepted: vec![],
                    default: String::new(),
                }],
                examples: vec![
                    r#"{"id":1,"verb":"kill-session","params":{"session":"work"}}"#.into(),
                ],
                handler: Arc::new(|_params| {
                    Err(VerbError::new(ERR_SESSION_NOT_FOUND, "session not found"))
                }),
            },
        );

        // set-option
        entries.insert(
            "set-option".to_string(),
            VerbEntry {
                description: "Set a session option, applied live when a client is attached.".into(),
                params: vec![
                    session_param(),
                    VerbParam { name: "key".into(), ty: "string".into(), required: true, description: r#"Option path, e.g. "appearance.dockbar_position"."#.into(), accepted: vec![], default: String::new() },
                    VerbParam { name: "value".into(), ty: "string".into(), required: false, description: "New value, as a string.".into(), accepted: vec![], default: String::new() },
                ],
                examples: vec![r#"{"id":1,"verb":"set-option","params":{"session":"work","key":"appearance.dockbar_position","value":"top"}}"#.into()],
                handler: Arc::new(|_params| {
                    Err(VerbError::new(ERR_SESSION_NOT_FOUND, "session not found"))
                }),
            },
        );

        // get-option
        entries.insert(
            "get-option".to_string(),
            VerbEntry {
                description: "Read a session option previously set with set-option.".into(),
                params: vec![
                    session_param(),
                    VerbParam { name: "key".into(), ty: "string".into(), required: true, description: "Option path to read.".into(), accepted: vec![], default: String::new() },
                ],
                examples: vec![r#"{"id":1,"verb":"get-option","params":{"session":"work","key":"appearance.dockbar_position"}}"#.into()],
                handler: Arc::new(|_params| {
                    Err(VerbError::new(ERR_OPTION_NOT_FOUND, "option not found"))
                }),
            },
        );

        // subscribe
        entries.insert(
            "subscribe".to_string(),
            VerbEntry {
                description: "Open a long-lived event stream on this connection.".into(),
                params: vec![
                    session_param(),
                    window_param(),
                    VerbParam { name: "types".into(), ty: "[]string".into(), required: false, description: "Only deliver these event types. Omit for all of them.".into(), accepted: KNOWN_EVENT_TYPES.iter().map(|s| s.to_string()).collect(), default: String::new() },
                    VerbParam { name: "queue".into(), ty: "int".into(), required: false, description: "Buffered events before the stream marks a gap.".into(), accepted: vec![], default: "256".into() },
                ],
                examples: vec![r#"{"id":1,"verb":"subscribe","params":{"session":"work","types":["window-created","window-closed"]}}"#.into()],
                handler: Arc::new(|_params| {
                    Ok(serde_json::json!({"type": "subscribed", "seq": 0}))
                }),
            },
        );

        // unsubscribe
        entries.insert(
            "unsubscribe".to_string(),
            VerbEntry {
                description: "Close this connection's event stream.".into(),
                params: vec![],
                examples: vec![r#"{"id":1,"verb":"unsubscribe"}"#.into()],
                handler: Arc::new(|_params| Ok(serde_json::json!({"type": "unsubscribed"}))),
            },
        );

        // set-session-name
        entries.insert(
            "set-session-name".to_string(),
            VerbEntry {
                description: "Set a session's display name.".into(),
                params: vec![
                    session_param(),
                    VerbParam { name: "name".into(), ty: "string".into(), required: false, description: "Display label for the session.".into(), accepted: vec![], default: String::new() },
                ],
                examples: vec![r#"{"id":1,"verb":"set-session-name","params":{"session":"work","name":"Payments API"}}"#.into()],
                handler: Arc::new(|_params| {
                    Err(VerbError::new(ERR_SESSION_NOT_FOUND, "session not found"))
                }),
            },
        );

        // set-session-accent
        entries.insert(
            "set-session-accent".to_string(),
            VerbEntry {
                description: "Set a session's accent colour.".into(),
                params: vec![
                    session_param(),
                    VerbParam { name: "accent".into(), ty: "string".into(), required: false, description: "Colour name or #rrggbb literal.".into(), accepted: vec![], default: String::new() },
                ],
                examples: vec![r#"{"id":1,"verb":"set-session-accent","params":{"session":"work","accent":"cyan"}}"#.into()],
                handler: Arc::new(|_params| {
                    Err(VerbError::new(ERR_SESSION_NOT_FOUND, "session not found"))
                }),
            },
        );

        // set-workspace-name
        entries.insert(
            "set-workspace-name".to_string(),
            VerbEntry {
                description: "Name a workspace.".into(),
                params: vec![
                    session_param(),
                    VerbParam { name: "workspace".into(), ty: "int".into(), required: true, description: "Workspace number to name.".into(), accepted: vec![], default: String::new() },
                    VerbParam { name: "name".into(), ty: "string".into(), required: false, description: "Label for the workspace.".into(), accepted: vec![], default: String::new() },
                ],
                examples: vec![r#"{"id":1,"verb":"set-workspace-name","params":{"session":"work","workspace":2,"name":"review"}}"#.into()],
                handler: Arc::new(|_params| {
                    Err(VerbError::new(ERR_SESSION_NOT_FOUND, "session not found"))
                }),
            },
        );

        // set-agent-state
        entries.insert(
            "set-agent-state".to_string(),
            VerbEntry {
                description: "Set the agent state a window's pane reports.".into(),
                params: vec![
                    session_param(),
                    window_param(),
                    VerbParam { name: "state".into(), ty: "string".into(), required: true, description: "The agent state to record.".into(), accepted: AGENT_STATE_NAMES.iter().map(|s| s.to_string()).collect(), default: String::new() },
                    VerbParam { name: "message".into(), ty: "string".into(), required: false, description: "Optional short note reported with the state.".into(), accepted: vec![], default: String::new() },
                    VerbParam { name: "source".into(), ty: "string".into(), required: false, description: "Where the state came from.".into(), accepted: AGENT_SOURCE_NAMES.iter().map(|s| s.to_string()).collect(), default: "report".into() },
                    VerbParam { name: "harness".into(), ty: "string".into(), required: false, description: "Optional id of the harness the state is about.".into(), accepted: vec![], default: String::new() },
                ],
                examples: vec![
                    r#"{"id":1,"verb":"set-agent-state","params":{"session":"work","state":"needs_input","message":"awaiting approval"}}"#.into(),
                    r#"{"id":1,"verb":"set-agent-state","params":{"session":"work","state":"working","source":"osc","harness":"claude-code"}}"#.into(),
                ],
                handler: Arc::new(|_params| {
                    Err(VerbError::new(ERR_SESSION_NOT_FOUND, "session not found"))
                }),
            },
        );

        // get-agent-state
        entries.insert(
            "get-agent-state".to_string(),
            VerbEntry {
                description: "Read the agent state a window's pane last reported.".into(),
                params: vec![session_param(), window_param()],
                examples: vec![r#"{"id":1,"verb":"get-agent-state","params":{"session":"work","window":"build"}}"#.into()],
                handler: Arc::new(|_params| {
                    Err(VerbError::new(ERR_SESSION_NOT_FOUND, "session not found"))
                }),
            },
        );

        // explain-agent-screen
        entries.insert(
            "explain-agent-screen".to_string(),
            VerbEntry {
                description: "Dump a pane's screen tail exactly as the harness screen rules read it.".into(),
                params: vec![
                    session_param(),
                    window_param(),
                    VerbParam { name: "harness".into(), ty: "string".into(), required: false, description: "Run this harness's rules instead of the attributed one.".into(), accepted: vec![], default: String::new() },
                    VerbParam { name: "lines".into(), ty: "int".into(), required: false, description: "Read this many lines from the bottom.".into(), accepted: vec![], default: String::new() },
                ],
                examples: vec![
                    r#"{"id":1,"verb":"explain-agent-screen","params":{"session":"work","window":"build"}}"#.into(),
                    r#"{"id":1,"verb":"explain-agent-screen","params":{"session":"work","harness":"codex","lines":20}}"#.into(),
                ],
                handler: Arc::new(|_params| {
                    Err(VerbError::new(ERR_SESSION_NOT_FOUND, "session not found"))
                }),
            },
        );

        // wait-for
        entries.insert(
            "wait-for".to_string(),
            VerbEntry {
                description: "Block until a condition matches, or fail with the timeout code.".into(),
                params: vec![
                    VerbParam { name: "condition".into(), ty: "string".into(), required: true, description: "Condition to wait for.".into(), accepted: WAIT_CONDITIONS.iter().map(|s| s.to_string()).collect(), default: String::new() },
                    session_param(),
                    window_param(),
                    VerbParam { name: "pattern".into(), ty: "string".into(), required: false, description: "Regular expression, required by window-output.".into(), accepted: vec![], default: String::new() },
                    VerbParam { name: "idle".into(), ty: "int".into(), required: false, description: "Milliseconds of silence that count as idle.".into(), accepted: vec![], default: "500".into() },
                    VerbParam { name: "timeout".into(), ty: "int".into(), required: false, description: "Milliseconds to wait before failing with the timeout code.".into(), accepted: vec![], default: "30000".into() },
                ],
                examples: vec![r#"{"id":1,"verb":"wait-for","params":{"condition":"window-output","session":"work","pattern":"done","timeout":10000}}"#.into()],
                handler: Arc::new(|_params| {
                    Err(VerbError::new(ERR_TIMEOUT, "condition did not match before timeout"))
                }),
            },
        );

        // diagnose
        entries.insert(
            "diagnose".to_string(),
            VerbEntry {
                description: "Report daemon health: session count, client count, uptime, memory usage, and per-session detail.".into(),
                params: vec![],
                examples: vec![r#"{"id":1,"verb":"diagnose"}"#.into()],
                handler: Arc::new(|_params| {
                    Err(VerbError::new(ERR_INTERNAL, "diagnose requires a daemon connection"))
                }),
            },
        );

        // headless-command
        entries.insert(
            "headless-command".to_string(),
            VerbEntry {
                description: "Execute a command without a TUI (headless). Supported: list-sessions, list-windows, capture-pane, send-text, kill-session, diagnose.".into(),
                params: vec![
                    VerbParam { name: "command".into(), ty: "string".into(), required: true, description: "The headless command to execute.".into(), accepted: vec![], default: String::new() },
                    VerbParam { name: "args".into(), ty: "[]string".into(), required: false, description: "Arguments for the command.".into(), accepted: vec![], default: String::new() },
                    session_param(),
                ],
                examples: vec![
                    r#"{"id":1,"verb":"headless-command","params":{"command":"list-sessions"}}"#.into(),
                    r#"{"id":1,"verb":"headless-command","params":{"command":"capture-pane","session":"work","args":["w0"]}}"#.into(),
                ],
                handler: Arc::new(|_params| {
                    Err(VerbError::new(ERR_INTERNAL, "headless-command requires a daemon connection"))
                }),
            },
        );

        Self { entries }
    }

    /// Look up a verb by name.
    pub fn get(&self, name: &str) -> Option<&VerbEntry> {
        self.entries.get(name)
    }

    /// Every registered verb name, sorted.
    pub fn names(&self) -> Vec<String> {
        self.entries.keys().cloned().collect()
    }

    /// Build the `list-verbs` result document.
    pub fn list_verbs(&self, filter: Option<&str>) -> Value {
        let verbs: Vec<VerbDoc> = match filter {
            Some(name) => {
                if let Some(entry) = self.entries.get(name) {
                    vec![describe_verb(name, entry)]
                } else {
                    vec![]
                }
            }
            None => self
                .entries
                .iter()
                .map(|(name, entry)| describe_verb(name, entry))
                .collect(),
        };
        serde_json::json!({
            "type": "verb_list",
            "version": VERB_PROTOCOL_VERSION,
            "min_version": MIN_VERB_PROTOCOL_VERSION,
            "verbs": verbs,
            "error_codes": error_code_catalog(),
        })
    }

    /// Dispatch one request line, returning the response.
    pub fn dispatch(&self, req: &VerbRequest) -> VerbResponse {
        if req.verb.is_empty() {
            return VerbResponse::err(
                req.id.clone(),
                VerbError::with_hint(
                    ERR_INVALID_REQUEST,
                    r#"request is missing the "verb" field"#,
                    VerbHint {
                        param: "verb".into(),
                        verb: "list-verbs".into(),
                        available: self.names(),
                        detail: r#"Every request line is an object of the form {"id":1,"verb":"list-verbs","params":{}}."#.into(),
                        ..Default::default()
                    },
                ),
            );
        }

        let entry = match self.entries.get(&req.verb) {
            Some(e) => e,
            None => {
                let known = self.names();
                return VerbResponse::err(
                    req.id.clone(),
                    VerbError::with_hint(
                        ERR_UNKNOWN_VERB,
                        format!("unknown verb {}", echo_name(&req.verb)),
                        VerbHint {
                            verb: "list-verbs".into(),
                            command: "tuios list-verbs".into(),
                            did_you_mean: closest_match(&req.verb, &known),
                            available: known,
                            detail: "Call list-verbs for every verb with its parameter schema and examples.".into(),
                            ..Default::default()
                        },
                    ),
                );
            }
        };

        let params = req.params.clone().unwrap_or(Value::Null);
        match (entry.handler)(&params) {
            Ok(result) => VerbResponse::ok(req.id.clone(), result),
            Err(e) => VerbResponse::err(req.id.clone(), e),
        }
    }
}

impl Default for VerbRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Render one registry entry as its documented form.
fn describe_verb(name: &str, entry: &VerbEntry) -> VerbDoc {
    VerbDoc {
        verb: name.to_string(),
        description: entry.description.clone(),
        params: entry.params.clone(),
        examples: entry.examples.clone(),
    }
}

/// The error-code catalog, documenting every stable code.
pub fn error_code_catalog() -> Vec<Value> {
    vec![
        serde_json::json!({"code": ERR_INVALID_REQUEST, "description": "The line was not a valid request envelope, or the connection is in the wrong state for the verb."}),
        serde_json::json!({"code": ERR_UNKNOWN_VERB, "description": "No such verb. The hint carries the closest match and the full verb list."}),
        serde_json::json!({"code": ERR_INVALID_PARAMS, "description": "A parameter was missing, malformed, or outside its accepted set."}),
        serde_json::json!({"code": ERR_SESSION_NOT_FOUND, "description": "The named session does not exist. The hint lists the sessions that do."}),
        serde_json::json!({"code": ERR_WINDOW_NOT_FOUND, "description": "The window target did not resolve. The hint lists the addressable windows."}),
        serde_json::json!({"code": ERR_NO_WINDOWS, "description": "The session exists but holds no windows to act on."}),
        serde_json::json!({"code": ERR_PTY_NOT_FOUND, "description": "The target window has no live PTY; its shell has already exited."}),
        serde_json::json!({"code": ERR_NEEDS_CLIENT, "description": "The verb needs an attached client to render it, and none is attached."}),
        serde_json::json!({"code": ERR_OPTION_NOT_FOUND, "description": "The option was never set on this session."}),
        serde_json::json!({"code": ERR_COMMAND_FAILED, "description": "The verb was routed to the attached client and came back failed."}),
        serde_json::json!({"code": ERR_TIMEOUT, "description": "A wait-for condition did not match before its timeout elapsed."}),
        serde_json::json!({"code": ERR_PROTOCOL_MISMATCH, "description": "The caller's protocol version is outside the range this daemon accepts."}),
        serde_json::json!({"code": ERR_INTERNAL, "description": "Unexpected server-side failure."}),
    ]
}

/// Maximum length of a caller-supplied name echoed back in an error message.
const MAX_ECHOED_NAME: usize = 128;

/// Render a caller-supplied name for an error message, truncating it so the
/// response stays proportional to the request.
pub fn echo_name(name: &str) -> String {
    if name.len() <= MAX_ECHOED_NAME {
        return name.to_string();
    }
    format!("{}... ({} bytes)", &name[..MAX_ECHOED_NAME], name.len())
}

/// Returns the candidate closest to `target` by edit distance, or `""` when
/// nothing is close enough to suggest.
pub fn closest_match(target: &str, candidates: &[String]) -> String {
    if target.is_empty() || candidates.is_empty() {
        return String::new();
    }
    let limit = (target.chars().count() / 4 + 1).min(3) as i32;
    let target_len = target.chars().count() as i32;

    let mut best = String::new();
    let mut best_dist = limit + 1;
    for c in candidates {
        if c == target {
            continue;
        }
        let c_len = c.chars().count() as i32;
        if (target_len - c_len).abs() > limit {
            continue;
        }
        let d = edit_distance(&target.to_lowercase(), &c.to_lowercase());
        if d < best_dist || (d == best_dist && c < &best) {
            best_dist = d;
            best = c.clone();
        }
    }
    if best_dist > limit {
        return String::new();
    }
    best
}

/// Levenshtein distance using a single rolling row.
pub fn edit_distance(a: &str, b: &str) -> i32 {
    let ar: Vec<char> = a.chars().collect();
    let br: Vec<char> = b.chars().collect();
    if ar.is_empty() {
        return br.len() as i32;
    }
    if br.is_empty() {
        return ar.len() as i32;
    }
    let mut prev: Vec<i32> = (0..=br.len()).map(|i| i as i32).collect();
    let mut cur = vec![0i32; br.len() + 1];
    for i in 1..=ar.len() {
        cur[0] = i as i32;
        for j in 1..=br.len() {
            let cost = if ar[i - 1] == br[j - 1] { 0 } else { 1 };
            cur[j] = (cur[j - 1] + 1).min(prev[j] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[br.len()]
}

/// Detect whether a connection's first byte indicates a JSON verb-protocol
/// client (`{` or whitespace) vs a binary client.
pub fn is_json_first_byte(b: u8) -> bool {
    matches!(b, b'{' | b' ' | b'\t' | b'\n' | b'\r')
}

/// Parse one newline-delimited request line.
pub fn parse_request_line(line: &str) -> Result<VerbRequest, VerbError> {
    serde_json::from_str(line)
        .map_err(|e| VerbError::new(ERR_INVALID_REQUEST, format!("malformed JSON request: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_has_all_verbs() {
        let reg = VerbRegistry::new();
        let names = reg.names();
        assert!(names.contains(&"hello".to_string()));
        assert!(names.contains(&"list-verbs".to_string()));
        assert!(names.contains(&"list-sessions".to_string()));
        assert!(names.contains(&"new-window".to_string()));
        assert!(names.contains(&"send-keys".to_string()));
        assert!(names.contains(&"capture-pane".to_string()));
        assert!(names.contains(&"wait-for".to_string()));
        assert!(names.contains(&"set-agent-state".to_string()));
        assert!(names.contains(&"subscribe".to_string()));
        assert!(names.contains(&"unsubscribe".to_string()));
        assert!(names.contains(&"diagnose".to_string()));
        assert!(names.contains(&"headless-command".to_string()));
    }

    #[test]
    fn dispatch_unknown_verb() {
        let reg = VerbRegistry::new();
        let req = VerbRequest {
            id: Some(Value::from(1)),
            verb: "nonexistent".into(),
            params: None,
        };
        let resp = reg.dispatch(&req);
        assert!(resp.error.is_some());
        let e = resp.error.unwrap();
        assert_eq!(e.code, ERR_UNKNOWN_VERB);
        assert!(e.hint.is_some());
        let h = e.hint.unwrap();
        assert!(!h.available.is_empty());
    }

    #[test]
    fn dispatch_missing_verb() {
        let reg = VerbRegistry::new();
        let req = VerbRequest {
            id: Some(Value::from(1)),
            verb: String::new(),
            params: None,
        };
        let resp = reg.dispatch(&req);
        let e = resp.error.unwrap();
        assert_eq!(e.code, ERR_INVALID_REQUEST);
    }

    #[test]
    fn dispatch_hello() {
        let reg = VerbRegistry::new();
        let req = VerbRequest {
            id: Some(Value::from(1)),
            verb: "hello".into(),
            params: Some(serde_json::json!({"protocol": 1})),
        };
        let resp = reg.dispatch(&req);
        assert!(resp.result.is_some());
        let r = resp.result.unwrap();
        assert_eq!(r["type"], "hello");
        assert_eq!(r["protocol"], VERB_PROTOCOL_VERSION);
    }

    #[test]
    fn dispatch_list_verbs() {
        let reg = VerbRegistry::new();
        // The default handler returns a placeholder; test list_verbs directly.
        let r = reg.list_verbs(None);
        assert_eq!(r["type"], "verb_list");
        assert!(r["verbs"].is_array());
        assert!(r["verbs"].as_array().unwrap().len() > 10);
    }

    #[test]
    fn dispatch_list_verbs_filtered() {
        let reg = VerbRegistry::new();
        let req = VerbRequest {
            id: Some(Value::from(1)),
            verb: "list-verbs".into(),
            params: Some(serde_json::json!({"verb": "hello"})),
        };
        let resp = reg.dispatch(&req);
        // The default handler returns a placeholder; the daemon overrides it.
        assert!(resp.result.is_some());
    }

    #[test]
    fn parse_request_line_valid() {
        let req = parse_request_line(r#"{"id":1,"verb":"hello","params":{}}"#).unwrap();
        assert_eq!(req.verb, "hello");
    }

    #[test]
    fn parse_request_line_invalid() {
        let err = parse_request_line("not json").unwrap_err();
        assert_eq!(err.code, ERR_INVALID_REQUEST);
    }

    #[test]
    fn response_to_line_ok() {
        let resp = VerbResponse::ok(Some(Value::from(1)), serde_json::json!({"type": "hello"}));
        let line = resp.to_line();
        assert!(line.ends_with('\n'));
        assert!(line.contains("\"result\""));
    }

    #[test]
    fn response_to_line_err() {
        let resp = VerbResponse::err(
            Some(Value::from(1)),
            VerbError::new(ERR_SESSION_NOT_FOUND, "no such session"),
        );
        let line = resp.to_line();
        assert!(line.contains("\"error\""));
        assert!(line.contains("session_not_found"));
    }

    #[test]
    fn closest_match_finds_typo() {
        let candidates = vec![
            "list-windows".to_string(),
            "list-sessions".to_string(),
            "list-verbs".to_string(),
        ];
        assert_eq!(closest_match("list-window", &candidates), "list-windows");
    }

    #[test]
    fn closest_match_no_match_for_garbage() {
        let candidates = vec!["hello".to_string(), "list-verbs".to_string()];
        assert_eq!(closest_match("zzzzzzzz", &candidates), "");
    }

    #[test]
    fn edit_distance_basic() {
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(edit_distance("same", "same"), 0);
        assert_eq!(edit_distance("", "abc"), 3);
    }

    #[test]
    fn echo_name_short() {
        assert_eq!(echo_name("hello"), "hello");
    }

    #[test]
    fn echo_name_long_truncates() {
        let long = "x".repeat(200);
        let echoed = echo_name(&long);
        assert!(echoed.starts_with("x"));
        assert!(echoed.contains("..."));
    }

    #[test]
    fn is_json_first_byte_detects() {
        assert!(is_json_first_byte(b'{'));
        assert!(is_json_first_byte(b' '));
        assert!(is_json_first_byte(b'\n'));
        assert!(!is_json_first_byte(b'\x00'));
        assert!(!is_json_first_byte(b'A'));
    }

    #[test]
    fn invalid_param_builds_hint() {
        let e = invalid_param("source", "unknown source", CAPTURE_SOURCES);
        assert_eq!(e.code, ERR_INVALID_PARAMS);
        assert!(e.hint.is_some());
        let h = e.hint.unwrap();
        assert_eq!(h.param, "source");
        assert_eq!(
            h.accepted,
            CAPTURE_SOURCES
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn verb_hint_empty_check() {
        let h = VerbHint::default();
        assert!(h.is_empty());
        let h = VerbHint {
            verb: "list-verbs".into(),
            ..Default::default()
        };
        assert!(!h.is_empty());
    }

    #[test]
    fn error_code_catalog_complete() {
        let catalog = error_code_catalog();
        assert!(catalog.len() >= 13);
        let codes: Vec<&str> = catalog.iter().filter_map(|v| v["code"].as_str()).collect();
        assert!(codes.contains(&ERR_INVALID_REQUEST));
        assert!(codes.contains(&ERR_UNKNOWN_VERB));
        assert!(codes.contains(&ERR_TIMEOUT));
        assert!(codes.contains(&ERR_PROTOCOL_MISMATCH));
    }
}
