//! The unified message enum for the message-pump architecture.
//!
//! Every event source in the application — the local crossterm poll loop, the
//! daemon socket reader thread, the config watcher, and the adaptive tick —
//! produces a `Msg`. A single `Os::update` consumes them, which is what makes
//! the whole input surface deterministic and unit-testable the same way Go's
//! Bubble Tea `Update` is.
//!
//! Two daemon events are deliberately handled at the loop level and never
//! become `Msg`s: `WindowAdded` (the loop must register the window's output
//! channel with the reader thread's registry before the window exists) and
//! `WindowClosed` (the loop must drop that channel from the registry).

use crossterm::event::{KeyEvent, MouseEvent};

use crate::config::userconfig::UserConfig;
use crate::session::model::SessionInfo;
use crate::tape::command::Command;

/// One event for the application to process.
#[derive(Debug, Clone)]
pub enum Msg {
    /// A key press (or repeat) from any input source.
    Key(KeyEvent),
    /// A mouse event from the local terminal.
    Mouse(MouseEvent),
    /// The terminal was resized.
    Resize { cols: u16, rows: u16 },
    /// The adaptive maintenance tick (agent alerts, script playback, layout).
    Tick,
    /// The config watcher published a reloaded configuration.
    ConfigReloaded(Box<UserConfig>),
    /// A window's agent state changed (daemon broadcast).
    RemoteAgentStateChanged {
        window: String,
        state: String,
        message: String,
        harness: String,
    },
    /// One command from a remote `tape exec`.
    RemoteTapeCommand {
        index: usize,
        total: usize,
        command: Command,
    },
    /// A remote tape finished.
    RemoteTapeFinished { total: usize },
    /// The daemon replied to a session `List`.
    RemoteListResult { sessions: Vec<SessionInfo> },
    /// The daemon reported an error.
    RemoteError(String),
    /// A no-op message.
    None,
}

/// Build a `Msg` from a daemon control event.
///
/// `WindowAdded` and `WindowClosed` map to `Msg::None`: the remote event loop
/// intercepts them because it must keep its output-channel registry in sync
/// (see the module docs).
pub fn from_remote_event(ev: crate::session::remote::RemoteEvent) -> Msg {
    match ev {
        crate::session::remote::RemoteEvent::WindowAdded(_) => Msg::None,
        crate::session::remote::RemoteEvent::WindowClosed(_) => Msg::None,
        crate::session::remote::RemoteEvent::AgentStateChanged {
            window,
            state,
            message,
            harness,
        } => Msg::RemoteAgentStateChanged {
            window,
            state,
            message,
            harness,
        },
        crate::session::remote::RemoteEvent::TapeCommand {
            index,
            total,
            command,
        } => Msg::RemoteTapeCommand {
            index,
            total,
            command,
        },
        crate::session::remote::RemoteEvent::TapeFinished { total } => {
            Msg::RemoteTapeFinished { total }
        }
        crate::session::remote::RemoteEvent::Attached { .. } => Msg::None,
        crate::session::remote::RemoteEvent::ListResult { sessions } => {
            Msg::RemoteListResult { sessions }
        }
        crate::session::remote::RemoteEvent::Error(msg) => Msg::RemoteError(msg),
    }
}
