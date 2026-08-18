//! Kitty keyboard protocol state — ported from Go TUIOS `internal/vt/kitty_keyboard.go`.
//!
//! Tracks the kitty keyboard protocol flag stack that applications push/pop
//! to control key reporting behavior.

/// Flag: Disambiguate escape sequences (mode 1).
pub const FLAG_REPORT_ALL_KEYS: i32 = 1;
/// Flag: Report associated text with key events.
pub const FLAG_REPORT_ASSOCIATED_TEXT: i32 = 2;
/// Flag: Report event types (press/release/repeat).
pub const FLAG_REPORT_EVENT_TYPES: i32 = 4;
/// Flag: Report alternate keys for non-US layouts.
pub const FLAG_REPORT_ALTERNATE_KEYS: i32 = 8;
/// Flag: Report modifier keys as escape codes.
pub const FLAG_REPORT_MODIFIERS_AS_KEYS: i32 = 16;

/// Kitty keyboard protocol state with a stack of flag bitmasks.
#[derive(Debug, Clone)]
pub struct KittyKeyboardState {
    stack: Vec<i32>,
}

impl KittyKeyboardState {
    /// Create new state with a base entry [0].
    pub fn new() -> Self {
        Self { stack: vec![0] }
    }

    /// Current flags (top of stack).
    pub fn current_flags(&self) -> i32 {
        *self.stack.last().unwrap_or(&0)
    }

    /// Push a new flag set onto the stack.
    pub fn push(&mut self, flags: i32) {
        self.stack.push(flags);
    }

    /// Pop `n` entries from the stack (minimum 1 entry remains).
    pub fn pop(&mut self, n: usize) {
        for _ in 0..n {
            if self.stack.len() > 1 {
                self.stack.pop();
            }
        }
    }

    /// Replace the top of stack with new flags.
    pub fn set_flags(&mut self, flags: i32) {
        if self.stack.is_empty() {
            self.stack.push(flags);
        } else {
            *self.stack.last_mut().unwrap() = flags;
        }
    }

    /// Whether disambiguation mode is on.
    pub fn is_disambiguate(&self) -> bool {
        (self.current_flags() & FLAG_REPORT_ALL_KEYS) != 0
    }

    /// Whether event type reporting is on.
    pub fn is_report_event_types(&self) -> bool {
        (self.current_flags() & FLAG_REPORT_EVENT_TYPES) != 0
    }

    /// Whether alternate key reporting is on.
    pub fn is_report_alternate_keys(&self) -> bool {
        (self.current_flags() & FLAG_REPORT_ALTERNATE_KEYS) != 0
    }

    /// Whether all-keys-as-escape-codes is on.
    pub fn is_report_all_keys(&self) -> bool {
        (self.current_flags() & FLAG_REPORT_ALL_KEYS) != 0
    }

    /// Whether associated text reporting is on.
    pub fn is_report_associated_text(&self) -> bool {
        (self.current_flags() & FLAG_REPORT_ASSOCIATED_TEXT) != 0
    }

    /// Stack depth.
    pub fn depth(&self) -> usize {
        self.stack.len()
    }
}

impl Default for KittyKeyboardState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_starts_with_base() {
        let s = KittyKeyboardState::new();
        assert_eq!(s.current_flags(), 0);
        assert_eq!(s.depth(), 1);
    }

    #[test]
    fn push_pop() {
        let mut s = KittyKeyboardState::new();
        s.push(FLAG_REPORT_ALL_KEYS | FLAG_REPORT_EVENT_TYPES);
        assert_eq!(
            s.current_flags(),
            FLAG_REPORT_ALL_KEYS | FLAG_REPORT_EVENT_TYPES
        );
        assert_eq!(s.depth(), 2);
        s.pop(1);
        assert_eq!(s.current_flags(), 0);
        assert_eq!(s.depth(), 1);
    }

    #[test]
    fn pop_keeps_minimum_one() {
        let mut s = KittyKeyboardState::new();
        s.pop(10);
        assert_eq!(s.depth(), 1);
    }

    #[test]
    fn set_flags_replaces_top() {
        let mut s = KittyKeyboardState::new();
        s.push(0);
        s.set_flags(FLAG_REPORT_ASSOCIATED_TEXT);
        assert_eq!(s.current_flags(), FLAG_REPORT_ASSOCIATED_TEXT);
        s.pop(1);
        assert_eq!(s.current_flags(), 0);
    }

    #[test]
    fn flag_checks() {
        let mut s = KittyKeyboardState::new();
        s.set_flags(FLAG_REPORT_ALL_KEYS | FLAG_REPORT_ALTERNATE_KEYS);
        assert!(s.is_report_all_keys());
        assert!(s.is_report_alternate_keys());
        assert!(!s.is_report_event_types());
        assert!(!s.is_report_associated_text());
    }
}
