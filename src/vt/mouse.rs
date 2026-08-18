//! Mouse tracking modes — ported from Go TUIOS `internal/vt/mouse.go`.
//!
//! Full mouse tracking mode support: X10, button-event, all-event, SGR,
//! URXVT, and pixel modes.

// ---------------------------------------------------------------------------
// Mode bitflags
// ---------------------------------------------------------------------------

pub const MOUSE_MODE_NONE: i32 = 0;
pub const MOUSE_MODE_CLICK: i32 = 1; // 1000
pub const MOUSE_MODE_HIGHLIGHT: i32 = 2; // 1001
pub const MOUSE_MODE_BUTTON_EVENT: i32 = 4; // 1002
pub const MOUSE_MODE_ALL_EVENT: i32 = 8; // 1003
pub const MOUSE_MODE_SGR: i32 = 16; // 1006
pub const MOUSE_MODE_URXVT: i32 = 32; // 1015
pub const MOUSE_MODE_PIXEL: i32 = 64; // 1016

// ---------------------------------------------------------------------------
// MouseState
// ---------------------------------------------------------------------------

/// Tracks the current mouse reporting mode.
#[derive(Debug, Clone, Default)]
pub struct MouseState {
    mode: i32,
}

impl MouseState {
    /// Create with no mouse reporting.
    pub fn new() -> Self {
        Self::default()
    }

    /// Enable or disable a mode bit.
    pub fn set_mode(&mut self, mode: i32, enable: bool) {
        if enable {
            self.mode |= mode;
        } else {
            self.mode &= !mode;
        }
    }

    /// Whether click reporting (1000) is on.
    pub fn is_click(&self) -> bool {
        (self.mode & MOUSE_MODE_CLICK) != 0
    }

    /// Whether button-event reporting (1002) is on.
    pub fn is_button_event(&self) -> bool {
        (self.mode & MOUSE_MODE_BUTTON_EVENT) != 0
    }

    /// Whether all-event reporting (1003) is on.
    pub fn is_all_event(&self) -> bool {
        (self.mode & MOUSE_MODE_ALL_EVENT) != 0
    }

    /// Whether SGR encoding (1006) is on.
    pub fn is_sgr(&self) -> bool {
        (self.mode & MOUSE_MODE_SGR) != 0
    }

    /// Whether URXVT encoding (1015) is on.
    pub fn is_urxvt(&self) -> bool {
        (self.mode & MOUSE_MODE_URXVT) != 0
    }

    /// Whether pixel reporting (1016) is on.
    pub fn is_pixel(&self) -> bool {
        (self.mode & MOUSE_MODE_PIXEL) != 0
    }

    /// Whether any mouse reporting is active.
    pub fn any_mouse(&self) -> bool {
        self.mode
            & (MOUSE_MODE_CLICK
                | MOUSE_MODE_HIGHLIGHT
                | MOUSE_MODE_BUTTON_EVENT
                | MOUSE_MODE_ALL_EVENT)
            != 0
    }

    /// Raw mode bitmask.
    pub fn raw_mode(&self) -> i32 {
        self.mode
    }
}

// ---------------------------------------------------------------------------
// MouseButton
// ---------------------------------------------------------------------------

/// Mouse button identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
    WheelLeft,
    WheelRight,
    Button8,
    Button9,
    NoButton,
}

impl MouseButton {
    /// The X10 button code (0-255 range, before +32 offset).
    pub fn code(&self) -> u8 {
        match self {
            Self::Left => 0,
            Self::Middle => 1,
            Self::Right => 2,
            Self::WheelUp => 64,
            Self::WheelDown => 65,
            Self::WheelLeft => 66,
            Self::WheelRight => 67,
            Self::Button8 => 128,
            Self::Button9 => 129,
            Self::NoButton => 3,
        }
    }
}

// ---------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------

/// Encode a mouse event as an escape sequence.
///
/// `button` — which button was pressed/released.
/// `x`, `y` — 1-based terminal coordinates.
/// `pressed` — true for press, false for release.
/// `state` — current mouse mode state.
pub fn encode_mouse(
    button: MouseButton,
    x: i32,
    y: i32,
    pressed: bool,
    state: &MouseState,
) -> Vec<u8> {
    if state.is_sgr() {
        encode_sgr(button, x, y, pressed)
    } else if state.is_urxvt() {
        encode_urxvt(button, x, y, pressed)
    } else {
        encode_x10(button, x, y, pressed)
    }
}

fn encode_x10(button: MouseButton, x: i32, y: i32, pressed: bool) -> Vec<u8> {
    let btn_code = if !pressed
        && !matches!(
            button,
            MouseButton::WheelUp
                | MouseButton::WheelDown
                | MouseButton::WheelLeft
                | MouseButton::WheelRight
        ) {
        3 // release
    } else {
        button.code()
    };
    let b = btn_code + 32;
    let cx = (x + 32).min(255) as u8;
    let cy = (y + 32).min(255) as u8;
    vec![0x1b, b'[', b'M', b, cx, cy]
}

fn encode_sgr(button: MouseButton, x: i32, y: i32, pressed: bool) -> Vec<u8> {
    let btn = button.code();
    let suffix = if pressed { 'M' } else { 'm' };
    format!("\x1b[<{};{};{}{}", btn, x, y, suffix).into_bytes()
}

fn encode_urxvt(button: MouseButton, x: i32, y: i32, pressed: bool) -> Vec<u8> {
    let btn_code = if !pressed
        && !matches!(
            button,
            MouseButton::WheelUp
                | MouseButton::WheelDown
                | MouseButton::WheelLeft
                | MouseButton::WheelRight
        ) {
        3
    } else {
        button.code()
    };
    let b = (btn_code + 32) as i32;
    format!("\x1b[<{};{};{}M", b, x + 1, y + 1).into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_no_mouse() {
        let s = MouseState::new();
        assert!(!s.any_mouse());
        assert!(!s.is_click());
    }

    #[test]
    fn set_click_mode() {
        let mut s = MouseState::new();
        s.set_mode(MOUSE_MODE_CLICK, true);
        assert!(s.is_click());
        assert!(s.any_mouse());
        s.set_mode(MOUSE_MODE_CLICK, false);
        assert!(!s.is_click());
    }

    #[test]
    fn set_sgr_mode() {
        let mut s = MouseState::new();
        s.set_mode(MOUSE_MODE_SGR | MOUSE_MODE_CLICK, true);
        assert!(s.is_sgr());
        assert!(s.is_click());
    }

    #[test]
    fn encode_x10_left_click() {
        let s = MouseState::new();
        let result = encode_mouse(MouseButton::Left, 10, 5, true, &s);
        assert_eq!(result, vec![0x1b, b'[', b'M', 32, 42, 37]);
    }

    #[test]
    fn encode_x10_release() {
        let s = MouseState::new();
        let result = encode_mouse(MouseButton::Left, 10, 5, false, &s);
        // Release: button code 3 + 32 = 35
        assert_eq!(result[3], 35);
    }

    #[test]
    fn encode_sgr_press() {
        let mut s = MouseState::new();
        s.set_mode(MOUSE_MODE_SGR, true);
        let result = encode_mouse(MouseButton::Left, 10, 5, true, &s);
        assert_eq!(result, b"\x1b[<0;10;5M");
    }

    #[test]
    fn encode_sgr_release() {
        let mut s = MouseState::new();
        s.set_mode(MOUSE_MODE_SGR, true);
        let result = encode_mouse(MouseButton::Left, 10, 5, false, &s);
        assert_eq!(result, b"\x1b[<0;10;5m");
    }

    #[test]
    fn encode_urxvt_press() {
        let mut s = MouseState::new();
        s.set_mode(MOUSE_MODE_URXVT, true);
        let result = encode_mouse(MouseButton::Left, 10, 5, true, &s);
        assert_eq!(result, b"\x1b[<32;11;6M");
    }

    #[test]
    fn button_codes() {
        assert_eq!(MouseButton::Left.code(), 0);
        assert_eq!(MouseButton::Middle.code(), 1);
        assert_eq!(MouseButton::Right.code(), 2);
        assert_eq!(MouseButton::WheelUp.code(), 64);
        assert_eq!(MouseButton::WheelDown.code(), 65);
    }

    #[test]
    fn wheel_does_not_use_release_code() {
        let s = MouseState::new();
        let result = encode_mouse(MouseButton::WheelUp, 10, 5, false, &s);
        // Wheel events use their own code even on "release"
        assert_eq!(result[3], 64 + 32);
    }
}
