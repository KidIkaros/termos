//! Configuration constants — ported from Go TUIOS `internal/config/constants.go`.
//!
//! All default values, style enums, glyph sets, and layout dimensions used
//! across the application. In Go these are mutable globals; in Rust they are
//! constants consumed by the config struct.

// -----------------------------------------------------------------------------
// Window defaults
// -----------------------------------------------------------------------------

pub const DEFAULT_WINDOW_WIDTH: i32 = 20;
pub const DEFAULT_WINDOW_HEIGHT: i32 = 5;
pub const MIN_WINDOW_WIDTH: i32 = 10;
pub const MIN_WINDOW_HEIGHT: i32 = 3;

// -----------------------------------------------------------------------------
// Animation
// -----------------------------------------------------------------------------

pub const DEFAULT_ANIMATION_DURATION_MS: u64 = 300;
pub const FAST_ANIMATION_DURATION_MS: u64 = 200;
pub const ANIMATION_TICK_MS: u64 = 16;

// -----------------------------------------------------------------------------
// Notifications
// -----------------------------------------------------------------------------

pub const NOTIFICATION_DURATION_MS: u64 = 6_000;
pub const NOTIFICATION_WARNING_DURATION_MS: u64 = 8_000;
pub const NOTIFICATION_ERROR_DURATION_MS: u64 = 15_000;
pub const NOTIFICATION_ERROR_STICKY: bool = true;

// -----------------------------------------------------------------------------
// Timeouts and intervals
// -----------------------------------------------------------------------------

pub const PREFIX_COMMAND_TIMEOUT_MS: u64 = 2_000;
pub const CPU_UPDATE_INTERVAL_MS: u64 = 1_000;
pub const WHICH_KEY_DELAY_MS: u64 = 300;
pub const TOOLTIP_DELAY_MS: u64 = 500;
pub const TOOLTIP_DURATION_MS: u64 = 5_000;
pub const MARQUEE_INTERVAL_MS: u64 = 500;
pub const MARQUEE_PAUSE_MS: u64 = 2_000;
pub const TITLE_DEBOUNCE_MS: u64 = 300;

// -----------------------------------------------------------------------------
// FPS
// -----------------------------------------------------------------------------

pub const NORMAL_FPS: u32 = 60;
pub const MAX_FPS_CAP: u32 = 240;
pub const MIN_CONFIGURED_FPS: u32 = 10;
pub const IDLE_FPS: u32 = 10;

// -----------------------------------------------------------------------------
// Layout dimensions
// -----------------------------------------------------------------------------

pub const DOCK_HEIGHT: i32 = 1;
pub const SIDEBAR_MIN_WIDTH: i32 = 20;
pub const SIDEBAR_MAX_WIDTH: i32 = 60;
pub const SIDEBAR_COLLAPSE_WIDTH: i32 = 8;
pub const DOCK_ITEM_MIN_WIDTH: i32 = 5;
pub const DOCK_ITEM_MAX_WIDTH: i32 = 20;
pub const NOTIFICATION_MAX_WIDTH: i32 = 60;
pub const SCROLLBAR_WIDTH: i32 = 1;

// -----------------------------------------------------------------------------
// Border styles
// -----------------------------------------------------------------------------

pub const BORDER_STYLE_ROUNDED: &str = "rounded";
pub const BORDER_STYLE_SINGLE: &str = "single";
pub const BORDER_STYLE_DOUBLE: &str = "double";
pub const BORDER_STYLE_THICK: &str = "thick";
pub const BORDER_STYLE_ASCII: &str = "ascii";
pub const BORDER_STYLE_HIDDEN: &str = "hidden";

pub const BORDER_STYLES: &[&str] = &[
    BORDER_STYLE_ROUNDED,
    BORDER_STYLE_SINGLE,
    BORDER_STYLE_DOUBLE,
    BORDER_STYLE_THICK,
    BORDER_STYLE_ASCII,
    BORDER_STYLE_HIDDEN,
];

// -----------------------------------------------------------------------------
// Scrollbar styles
// -----------------------------------------------------------------------------

pub const SCROLLBAR_STYLE_THIN: &str = "thin";
pub const SCROLLBAR_STYLE_TRACK: &str = "track";

pub const SCROLLBAR_STYLES: &[&str] = &[SCROLLBAR_STYLE_THIN, SCROLLBAR_STYLE_TRACK];

// -----------------------------------------------------------------------------
// Click-to-type modes
// -----------------------------------------------------------------------------

pub const CLICK_TO_TYPE_SINGLE: &str = "single";
pub const CLICK_TO_TYPE_DOUBLE: &str = "double";
pub const CLICK_TO_TYPE_OFF: &str = "off";

pub const CLICK_TO_TYPE_MODES: &[&str] = &[CLICK_TO_TYPE_SINGLE, CLICK_TO_TYPE_DOUBLE, CLICK_TO_TYPE_OFF];

// -----------------------------------------------------------------------------
// Dock position
// -----------------------------------------------------------------------------

pub const DOCKBAR_POSITION_BOTTOM: &str = "bottom";
pub const DOCKBAR_POSITION_TOP: &str = "top";

// -----------------------------------------------------------------------------
// Which-key position
// -----------------------------------------------------------------------------

pub const WHICH_KEY_POSITION_BOTTOM_RIGHT: &str = "bottom-right";
pub const WHICH_KEY_POSITION_BOTTOM_LEFT: &str = "bottom-left";
pub const WHICH_KEY_POSITION_TOP_RIGHT: &str = "top-right";
pub const WHICH_KEY_POSITION_TOP_LEFT: &str = "top-left";

// -----------------------------------------------------------------------------
// Window title position
// -----------------------------------------------------------------------------

pub const WINDOW_TITLE_POSITION_BOTTOM: &str = "bottom";
pub const WINDOW_TITLE_POSITION_TOP: &str = "top";

// -----------------------------------------------------------------------------
// Nerd Font glyphs (PUA codepoints)
// -----------------------------------------------------------------------------

pub const DOCK_PILL_LEFT_CHAR: char = '\u{e0b6}';
pub const DOCK_PILL_RIGHT_CHAR: char = '\u{e0b4}';
pub const DOCK_MODE_ICON_WINDOW: char = '\u{f2d0}';
pub const DOCK_MODE_ICON_TILING: char = '\u{f24d}';
pub const DOCK_ICON_TERMINAL: char = '\u{f120}';
pub const DOCK_ICON_WORKSPACE: char = '\u{f07b}';
pub const DOCK_SEPARATOR: char = '\u{e0b1}';
pub const SIDEBAR_AGENT_GLYPH_WORKING: char = '\u{f251}';
pub const SIDEBAR_AGENT_GLYPH_NEEDS_INPUT: char = '\u{f059}';
pub const SIDEBAR_AGENT_GLYPH_IDLE: char = '\u{f017}';
pub const SIDEBAR_AGENT_GLYPH_DONE: char = '\u{f00c}';
pub const SIDEBAR_AGENT_GLYPH_ERRORED: char = '\u{f071}';

// -----------------------------------------------------------------------------
// ASCII fallback glyphs
// -----------------------------------------------------------------------------

pub const DOCK_PILL_LEFT_CHAR_ASCII: &str = "[";
pub const DOCK_PILL_RIGHT_CHAR_ASCII: &str = "]";
pub const DOCK_MODE_ICON_WINDOW_ASCII: &str = " W ";
pub const DOCK_MODE_ICON_TILING_ASCII: &str = " T ";
pub const DOCK_ICON_TERMINAL_ASCII: &str = ">";
pub const DOCK_ICON_WORKSPACE_ASCII: &str = "#";
pub const DOCK_SEPARATOR_ASCII: &str = "|";

pub const SIDEBAR_AGENT_GLYPH_WORKING_ASCII: char = '*';
pub const SIDEBAR_AGENT_GLYPH_NEEDS_INPUT_ASCII: char = '?';
pub const SIDEBAR_AGENT_GLYPH_IDLE_ASCII: char = '.';
pub const SIDEBAR_AGENT_GLYPH_DONE_ASCII: char = 'v';
pub const SIDEBAR_AGENT_GLYPH_ERRORED_ASCII: char = '!';

// -----------------------------------------------------------------------------
// Z-index layers
// -----------------------------------------------------------------------------

pub const Z_INDEX_BASE: i32 = 0;
pub const Z_INDEX_PANE: i32 = 100;
pub const Z_INDEX_BORDER: i32 = 200;
pub const Z_INDEX_SCROLLBAR: i32 = 300;
pub const Z_INDEX_DOCK: i32 = 400;
pub const Z_INDEX_SIDEBAR: i32 = 500;
pub const Z_INDEX_OVERLAY: i32 = 1000;
pub const Z_INDEX_TOOLTIP: i32 = 1500;
pub const Z_INDEX_NOTIFICATION: i32 = 2000;

// -----------------------------------------------------------------------------
// Control characters
// -----------------------------------------------------------------------------

pub const CTRL_B: u8 = 0x02;
pub const DEL: u8 = 0x7f;
pub const ESC: u8 = 0x1b;
pub const NUL: u8 = 0x00;
pub const TAB: u8 = 0x09;
pub const CR: u8 = 0x0d;
pub const LF: u8 = 0x0a;

// -----------------------------------------------------------------------------
// VT attribute flags
// -----------------------------------------------------------------------------

pub const VT_ATTR_BOLD: u16 = 1;
pub const VT_ATTR_FAINT: u16 = 2;
pub const VT_ATTR_ITALIC: u16 = 4;
pub const VT_ATTR_UNDERLINE: u16 = 8;
pub const VT_ATTR_BLINK: u16 = 16;
pub const VT_ATTR_REVERSE: u16 = 32;
pub const VT_ATTR_HIDDEN: u16 = 64;
pub const VT_ATTR_STRIKETHROUGH: u16 = 128;

// -----------------------------------------------------------------------------
// Helper functions
// -----------------------------------------------------------------------------

/// Get the dock pill left character based on ASCII mode.
pub fn dock_pill_left(ascii_only: bool) -> &'static str {
    if ascii_only {
        DOCK_PILL_LEFT_CHAR_ASCII
    } else {
        "\u{e0b6}"
    }
}

/// Get the dock pill right character based on ASCII mode.
pub fn dock_pill_right(ascii_only: bool) -> &'static str {
    if ascii_only {
        DOCK_PILL_RIGHT_CHAR_ASCII
    } else {
        "\u{e0b4}"
    }
}

/// Get the dock mode icon for window mode.
pub fn dock_mode_icon_window(ascii_only: bool) -> &'static str {
    if ascii_only {
        DOCK_MODE_ICON_WINDOW_ASCII
    } else {
        "\u{f2d0}"
    }
}

/// Get the dock mode icon for tiling mode.
pub fn dock_mode_icon_tiling(ascii_only: bool) -> &'static str {
    if ascii_only {
        DOCK_MODE_ICON_TILING_ASCII
    } else {
        "\u{f24d}"
    }
}

/// Get the dock separator character.
pub fn dock_separator(ascii_only: bool) -> &'static str {
    if ascii_only {
        DOCK_SEPARATOR_ASCII
    } else {
        "\u{e0b1}"
    }
}

/// Get the agent glyph for a given state.
pub fn agent_glyph(state: &str, ascii_only: bool) -> char {
    if ascii_only {
        match state {
            "working" => SIDEBAR_AGENT_GLYPH_WORKING_ASCII,
            "needs_input" => SIDEBAR_AGENT_GLYPH_NEEDS_INPUT_ASCII,
            "idle" => SIDEBAR_AGENT_GLYPH_IDLE_ASCII,
            "done" => SIDEBAR_AGENT_GLYPH_DONE_ASCII,
            "errored" => SIDEBAR_AGENT_GLYPH_ERRORED_ASCII,
            _ => ' ',
        }
    } else {
        match state {
            "working" => SIDEBAR_AGENT_GLYPH_WORKING,
            "needs_input" => SIDEBAR_AGENT_GLYPH_NEEDS_INPUT,
            "idle" => SIDEBAR_AGENT_GLYPH_IDLE,
            "done" => SIDEBAR_AGENT_GLYPH_DONE,
            "errored" => SIDEBAR_AGENT_GLYPH_ERRORED,
            _ => ' ',
        }
    }
}

/// Whether a border style uses ASCII-only characters.
pub fn border_is_ascii(style: &str) -> bool {
    style == BORDER_STYLE_ASCII
}

/// Whether a border style is hidden (no borders drawn).
pub fn border_is_hidden(style: &str) -> bool {
    style == BORDER_STYLE_HIDDEN
}

/// Get the animation duration for the given style and fast flag.
pub fn animation_duration_ms(_style: &str, fast: bool) -> u64 {
    if fast {
        FAST_ANIMATION_DURATION_MS
    } else {
        DEFAULT_ANIMATION_DURATION_MS
    }
}

/// Whether the dock needs tick updates (clock, CPU, RAM, animations).
pub fn needs_dock_tick(show_clock: bool, show_cpu: bool, show_ram: bool, animations: bool) -> bool {
    show_clock || show_cpu || show_ram || animations
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn border_styles_complete() {
        assert!(BORDER_STYLES.contains(&"rounded"));
        assert!(BORDER_STYLES.contains(&"hidden"));
        assert_eq!(BORDER_STYLES.len(), 6);
    }

    #[test]
    fn scrollbar_styles_complete() {
        assert!(SCROLLBAR_STYLES.contains(&"thin"));
        assert!(SCROLLBAR_STYLES.contains(&"track"));
    }

    #[test]
    fn dock_pill_ascii_vs_nerd() {
        assert_eq!(dock_pill_left(true), "[");
        assert_ne!(dock_pill_left(false), "[");
    }

    #[test]
    fn dock_mode_icons() {
        assert_eq!(dock_mode_icon_window(true), " W ");
        assert_eq!(dock_mode_icon_tiling(true), " T ");
    }

    #[test]
    fn agent_glyphs_ascii() {
        assert_eq!(agent_glyph("working", true), '*');
        assert_eq!(agent_glyph("needs_input", true), '?');
        assert_eq!(agent_glyph("idle", true), '.');
        assert_eq!(agent_glyph("done", true), 'v');
        assert_eq!(agent_glyph("errored", true), '!');
    }

    #[test]
    fn agent_glyphs_nerd_font() {
        assert_eq!(agent_glyph("working", false), SIDEBAR_AGENT_GLYPH_WORKING);
        assert_eq!(agent_glyph("errored", false), SIDEBAR_AGENT_GLYPH_ERRORED);
    }

    #[test]
    fn border_is_ascii_check() {
        assert!(border_is_ascii("ascii"));
        assert!(!border_is_ascii("rounded"));
    }

    #[test]
    fn border_is_hidden_check() {
        assert!(border_is_hidden("hidden"));
        assert!(!border_is_hidden("rounded"));
    }

    #[test]
    fn animation_duration_fast_vs_normal() {
        assert_eq!(animation_duration_ms("rounded", true), FAST_ANIMATION_DURATION_MS);
        assert_eq!(animation_duration_ms("rounded", false), DEFAULT_ANIMATION_DURATION_MS);
    }

    #[test]
    fn needs_dock_tick_any_flag() {
        assert!(needs_dock_tick(true, false, false, false));
        assert!(needs_dock_tick(false, true, false, false));
        assert!(needs_dock_tick(false, false, true, false));
        assert!(needs_dock_tick(false, false, false, true));
        assert!(!needs_dock_tick(false, false, false, false));
    }

    #[test]
    fn z_index_ordering() {
        const _: () = {
            assert!(Z_INDEX_PANE < Z_INDEX_BORDER);
            assert!(Z_INDEX_BORDER < Z_INDEX_DOCK);
            assert!(Z_INDEX_DOCK < Z_INDEX_OVERLAY);
            assert!(Z_INDEX_OVERLAY < Z_INDEX_NOTIFICATION);
        };
    }

    #[test]
    fn click_to_type_modes() {
        assert!(CLICK_TO_TYPE_MODES.contains(&"single"));
        assert!(CLICK_TO_TYPE_MODES.contains(&"off"));
    }

    #[test]
    fn fps_constants() {
        const _: () = {
            assert!(NORMAL_FPS > 0);
            assert!(MAX_FPS_CAP > NORMAL_FPS);
            assert!(MIN_CONFIGURED_FPS > 0);
            assert!(IDLE_FPS < NORMAL_FPS);
        };
    }
}
