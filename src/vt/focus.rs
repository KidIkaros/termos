//! Focus event reporting — ported from Go TUIOS `internal/vt/focus.go`.
//!
//! Implements mode 1004: focus/blur event reporting.

/// Tracks whether focus events (mode 1004) are enabled.
#[derive(Debug, Clone, Default)]
pub struct FocusEventMode {
    enabled: bool,
}

impl FocusEventMode {
    /// Create with focus events disabled.
    pub fn new() -> Self {
        Self::default()
    }

    /// Enable focus event reporting.
    pub fn enable(&mut self) {
        self.enabled = true;
    }

    /// Disable focus event reporting.
    pub fn disable(&mut self) {
        self.enabled = false;
    }

    /// Whether focus events are currently enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

/// The escape sequence sent when the terminal gains focus: `ESC[I`.
pub fn focus_sequence() -> &'static str {
    "\x1b[I"
}

/// The escape sequence sent when the terminal loses focus: `ESC[O`.
pub fn blur_sequence() -> &'static str {
    "\x1b[O"
}

/// Encode the appropriate focus/blur sequence.
pub fn encode_focus(focused: bool) -> Vec<u8> {
    if focused {
        focus_sequence().as_bytes().to_vec()
    } else {
        blur_sequence().as_bytes().to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_disabled() {
        let m = FocusEventMode::new();
        assert!(!m.is_enabled());
    }

    #[test]
    fn enable_disable() {
        let mut m = FocusEventMode::new();
        m.enable();
        assert!(m.is_enabled());
        m.disable();
        assert!(!m.is_enabled());
    }

    #[test]
    fn focus_seq() {
        assert_eq!(focus_sequence(), "\x1b[I");
    }

    #[test]
    fn blur_seq() {
        assert_eq!(blur_sequence(), "\x1b[O");
    }

    #[test]
    fn encode_focus_true() {
        assert_eq!(encode_focus(true), b"\x1b[I");
    }

    #[test]
    fn encode_focus_false() {
        assert_eq!(encode_focus(false), b"\x1b[O");
    }
}
