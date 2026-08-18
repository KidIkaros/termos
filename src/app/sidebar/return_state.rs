//! Return-to-session navigation after leaving the rail.
//!
//! Ported from Go TUIOS `internal/app/sidebar_return.go`. When the user
//! enters the sidebar and moves the cursor to a different session, pressing
//! the leader key again (or Esc) returns them to the session they came from
//! rather than the one the cursor is on.

/// The return state: whether return is armed and what to return to.
#[derive(Debug, Clone, Default)]
pub struct ReturnState {
    /// Whether return is armed (the user entered the rail with the leader key).
    pub armed: bool,
    /// Whether we're in return mode (the user pressed the leader key again).
    pub mode: bool,
    /// The window ID to return to.
    pub window_id: String,
    /// The session ID to return to.
    pub session_id: String,
}

impl ReturnState {
    /// Create a new return state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Arm the return state when entering the rail.
    pub fn arm(&mut self, window_id: &str, session_id: &str) {
        self.armed = true;
        self.mode = false;
        self.window_id = window_id.to_string();
        self.session_id = session_id.to_string();
    }

    /// Disarm when leaving the rail normally.
    pub fn disarm(&mut self) {
        self.armed = false;
        self.mode = false;
    }

    /// Enter return mode (the user pressed the leader key again).
    pub fn enter_return_mode(&mut self) {
        if self.armed {
            self.mode = true;
        }
    }

    /// Whether the user should return to their original session.
    pub fn should_return(&self) -> bool {
        self.armed && self.mode
    }

    /// The session to return to.
    pub fn return_session(&self) -> &str {
        &self.session_id
    }

    /// The window to return to.
    pub fn return_window(&self) -> &str {
        &self.window_id
    }

    /// Consume the return state, returning whether a return should happen.
    pub fn take_return(&mut self) -> bool {
        let r = self.should_return();
        self.disarm();
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_state_is_disarmed() {
        let s = ReturnState::new();
        assert!(!s.armed);
        assert!(!s.should_return());
    }

    #[test]
    fn arm_sets_window_and_session() {
        let mut s = ReturnState::new();
        s.arm("w0", "work");
        assert!(s.armed);
        assert_eq!(s.return_window(), "w0");
        assert_eq!(s.return_session(), "work");
    }

    #[test]
    fn enter_return_mode_only_when_armed() {
        let mut s = ReturnState::new();
        s.enter_return_mode();
        assert!(!s.should_return());
        s.arm("w0", "work");
        s.enter_return_mode();
        assert!(s.should_return());
    }

    #[test]
    fn take_return_consumes_state() {
        let mut s = ReturnState::new();
        s.arm("w0", "work");
        s.enter_return_mode();
        assert!(s.take_return());
        assert!(!s.armed);
        assert!(!s.should_return());
    }

    #[test]
    fn disarm_clears_everything() {
        let mut s = ReturnState::new();
        s.arm("w0", "work");
        s.enter_return_mode();
        s.disarm();
        assert!(!s.armed);
        assert!(!s.mode);
    }
}
