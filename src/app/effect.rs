//! Side effects returned by `Os::update`.
//!
//! The message pump keeps the model pure: `update` mutates `Os` state and
//! returns a list of `Effect`s describing what the outside world must do —
//! flush host-terminal sequences, switch or kill a daemon session, or quit.
//! The event loop executes them; the model never touches the socket or the
//! host terminal itself.

/// One side effect for the event loop to execute after an `update`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// The application should quit.
    Quit,
    /// Bytes to flush to the host terminal (OSC 9 / OSC 22 / BEL / kitty
    /// queries), always after the frame so they never interleave a draw.
    WriteHost(Vec<u8>),
    /// The session switcher asked to attach to this daemon session.
    RequestAttach(String),
    /// The session switcher asked to kill this daemon session.
    RequestKill(String),
    /// No effect.
    None,
}

impl Effect {
    /// Whether this is `Effect::None`.
    pub fn is_none(&self) -> bool {
        matches!(self, Effect::None)
    }
}
