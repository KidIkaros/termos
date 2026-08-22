//! Notification channels and event types.

/// Supported notification delivery channels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Channel {
    /// Display in the TUI overlay (toast/popup).
    TuiOverlay,
    /// Desktop system notification (via notify-rust).
    Desktop,
    /// Log to the event log (for later review).
    EventLog,
    /// Send to attached web/SSH clients.
    Remote,
}

/// A raw notification event before template rendering.
#[derive(Debug, Clone)]
pub struct NotificationEvent {
    /// Template name to use.
    pub template: String,
    /// Variables for template substitution.
    pub variables: std::collections::HashMap<String, String>,
    /// Target channels.
    pub channels: Vec<Channel>,
}
