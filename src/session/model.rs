//! The session data model — ported from TUIOS `internal/session` (the
//! `Session`, `SessionInfo`, and name-validation types).

use serde::{Deserialize, Serialize};

/// A persistent session: a named set of windows (shell processes) managed
/// together. The daemon owns the live PTYs; this struct is the metadata every
/// control surface addresses the session by.
#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub name: String,
    /// Unix seconds at creation.
    pub created_at: u64,
    /// True when the session was rebuilt from saved state at daemon start.
    pub restored: bool,
}

impl Session {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.into(),
            created_at: now_secs(),
            restored: false,
        }
    }

    pub fn restored(name: impl Into<String>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.into(),
            created_at: now_secs(),
            restored: true,
        }
    }
}

/// A snapshot of a session for listing and the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub name: String,
    pub created_at: u64,
    pub attached: bool,
    pub windows: usize,
    pub restored: bool,
}

/// One window of a session, as reported to an attaching client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowInfo {
    pub id: String,
    pub title: String,
    pub workspace: i32,
    pub cols: u16,
    pub rows: u16,
}

/// Per-session spawn configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionConfig {
    /// The shell used for the first window (empty = `$SHELL` or `/bin/sh`).
    pub shell: String,
    pub cwd: Option<String>,
}

/// Persisted window state for resurrection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowState {
    pub title: String,
    pub shell: String,
    pub workspace: i32,
}

/// Persisted session state for resurrection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionState {
    pub name: String,
    pub windows: Vec<WindowState>,
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Validate a session name. Names are the identity every keyed map, every
/// switch and the daemon's addressing use, so they are restricted to a safe,
/// filesystem- and shell-friendly charset.
pub fn validate_session_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("session name cannot be empty".into());
    }
    if name.len() > 64 {
        return Err("session name too long (max 64 chars)".into());
    }
    if name == "." || name == ".." {
        return Err("invalid session name".into());
    }
    if name
        .chars()
        .any(|c| c.is_whitespace() || c == '/' || c == '\\')
    {
        return Err("session name cannot contain whitespace or path separators".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_names_pass() {
        assert!(validate_session_name("dev").is_ok());
        assert!(validate_session_name("session-0").is_ok());
        assert!(validate_session_name("Payments-API").is_ok());
    }

    #[test]
    fn invalid_names_are_rejected() {
        assert!(validate_session_name("").is_err());
        assert!(validate_session_name("has space").is_err());
        assert!(validate_session_name("has/slash").is_err());
        assert!(validate_session_name(".").is_err());
        assert!(validate_session_name(&"x".repeat(65)).is_err());
    }
}
