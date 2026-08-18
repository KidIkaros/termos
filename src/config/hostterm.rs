//! Host terminal detection — ported from Go TUIOS `internal/config/hostterm.go`.
//!
//! Identifies the terminal emulator tuios is running inside, using environment
//! variables. Used for capability detection (kitty graphics, sixel, etc.) and
//! platform-specific advice (e.g. macOS Option-key settings).

/// The host terminal emulator tuios is running inside.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HostTerminal {
    Unknown,
    AppleTerminal,
    ITerm2,
    Ghostty,
    Kitty,
    WezTerm,
    Alacritty,
    VSCode,
    Rio,
    Warp,
}

impl HostTerminal {
    /// A human-readable name for the terminal.
    pub fn name(self) -> &'static str {
        match self {
            HostTerminal::Unknown => "Unknown",
            HostTerminal::AppleTerminal => "Terminal.app",
            HostTerminal::ITerm2 => "iTerm2",
            HostTerminal::Ghostty => "Ghostty",
            HostTerminal::Kitty => "kitty",
            HostTerminal::WezTerm => "WezTerm",
            HostTerminal::Alacritty => "Alacritty",
            HostTerminal::VSCode => "VS Code",
            HostTerminal::Rio => "Rio",
            HostTerminal::Warp => "Warp",
        }
    }

    /// Whether this terminal supports the kitty graphics protocol.
    pub fn supports_kitty_graphics(self) -> bool {
        matches!(
            self,
            HostTerminal::Kitty | HostTerminal::Ghostty | HostTerminal::WezTerm
        )
    }

    /// Whether this terminal supports sixel graphics.
    pub fn supports_sixel(self) -> bool {
        // WezTerm supports sixel; xterm and mintty also support it but are
        // rarely the host. Foot and Contour are not in the enum yet.
        matches!(self, HostTerminal::WezTerm)
    }

    /// Whether this terminal supports Unicode 9.0+ wide character handling.
    pub fn supports_unicode_9(self) -> bool {
        !matches!(self, HostTerminal::AppleTerminal)
    }

    /// Whether this terminal is on macOS and may need Option-key advice.
    pub fn is_macos_terminal(self) -> bool {
        matches!(
            self,
            HostTerminal::AppleTerminal | HostTerminal::ITerm2 | HostTerminal::Warp
        )
    }
}

/// Detect the host terminal from environment variables.
pub fn detect_host_terminal() -> HostTerminal {
    detect_host_terminal_with_getenv(|k| std::env::var(k).ok())
}

/// Testable version that takes a getenv closure.
pub fn detect_host_terminal_with_getenv<F>(getenv: F) -> HostTerminal
where
    F: Fn(&str) -> Option<String>,
{
    // TERM_PROGRAM is checked first because a multiplexer rewrites TERM to
    // screen/tmux while leaving TERM_PROGRAM alone.
    if let Some(prog) = getenv("TERM_PROGRAM") {
        match prog.as_str() {
            "Apple_Terminal" => return HostTerminal::AppleTerminal,
            "iTerm.app" => return HostTerminal::ITerm2,
            "ghostty" => return HostTerminal::Ghostty,
            "kitty" => return HostTerminal::Kitty,
            "WezTerm" => return HostTerminal::WezTerm,
            "vscode" => return HostTerminal::VSCode,
            "rio" => return HostTerminal::Rio,
            "WarpTerminal" => return HostTerminal::Warp,
            _ => {}
        }
    }

    // Check for terminal-specific env vars that exist even without TERM_PROGRAM.
    if getenv("KITTY_WINDOW_ID").is_some() {
        return HostTerminal::Kitty;
    }
    if getenv("GHOSTTY_RESOURCES_DIR").is_some() {
        return HostTerminal::Ghostty;
    }
    if getenv("ALACRITTY_WINDOW_ID").is_some() {
        return HostTerminal::Alacritty;
    }
    if getenv("WEZTERM_EXECUTABLE").is_some() {
        return HostTerminal::WezTerm;
    }

    // Fall back to TERM substring matching.
    if let Some(term) = getenv("TERM") {
        let term = term.as_str();
        if term.contains("alacritty") {
            return HostTerminal::Alacritty;
        }
        if term.contains("kitty") {
            return HostTerminal::Kitty;
        }
        if term.contains("ghostty") {
            return HostTerminal::Ghostty;
        }
        if term.contains("wezterm") {
            return HostTerminal::WezTerm;
        }
        if term.contains("rio") {
            return HostTerminal::Rio;
        }
    }

    HostTerminal::Unknown
}

/// Whether we're running inside a multiplexer (tmux or screen).
pub fn inside_multiplexer<F>(getenv: F) -> bool
where
    F: Fn(&str) -> Option<String>,
{
    if getenv("TMUX").is_some() {
        return true;
    }
    if let Some(term) = getenv("TERM") {
        if term.starts_with("screen") || term.starts_with("tmux") {
            return true;
        }
    }
    false
}

/// Whether we're running inside a multiplexer (convenience wrapper).
pub fn inside_multiplexer_env() -> bool {
    inside_multiplexer(|k| std::env::var(k).ok())
}

/// macOS Option-key advice for the detected terminal, or empty string if none.
pub fn mac_option_advice(term: HostTerminal) -> &'static str {
    match term {
        HostTerminal::AppleTerminal => {
            "Terminal.app: Preferences → Profiles → Keyboard → \"Use Option as Meta key\""
        }
        HostTerminal::ITerm2 => {
            "iTerm2: Preferences → Profiles → Keys → \"Option key sends Esc+\""
        }
        HostTerminal::Ghostty => {
            "Ghostty: set `macos-option-as-alt = true` in your config file"
        }
        HostTerminal::Warp => "Warp: Settings → Keyboard → Option key sends Meta",
        _ => "",
    }
}

/// Ghostty-specific advice for unbinding alt+arrow so they reach the PTY.
pub const GHOSTTY_ALT_ARROW_ADVICE: &str =
    "Ghostty: add `keybind = alt+left=unbind` and `keybind = alt+right=unbind` to your config";

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        let map: HashMap<&str, &str> = pairs.iter().copied().collect();
        move |key: &str| map.get(key).map(|v| v.to_string())
    }

    #[test]
    fn detect_apple_terminal() {
        let env = make_env(&[("TERM_PROGRAM", "Apple_Terminal")]);
        assert_eq!(detect_host_terminal_with_getenv(env), HostTerminal::AppleTerminal);
    }

    #[test]
    fn detect_iterm2() {
        let env = make_env(&[("TERM_PROGRAM", "iTerm.app")]);
        assert_eq!(detect_host_terminal_with_getenv(env), HostTerminal::ITerm2);
    }

    #[test]
    fn detect_kitty_via_term_program() {
        let env = make_env(&[("TERM_PROGRAM", "kitty")]);
        assert_eq!(detect_host_terminal_with_getenv(env), HostTerminal::Kitty);
    }

    #[test]
    fn detect_kitty_via_env_var() {
        let env = make_env(&[("KITTY_WINDOW_ID", "1")]);
        assert_eq!(detect_host_terminal_with_getenv(env), HostTerminal::Kitty);
    }

    #[test]
    fn detect_ghostty_via_env_var() {
        let env = make_env(&[("GHOSTTY_RESOURCES_DIR", "/opt/ghostty")]);
        assert_eq!(detect_host_terminal_with_getenv(env), HostTerminal::Ghostty);
    }

    #[test]
    fn detect_wezterm_via_env_var() {
        let env = make_env(&[("WEZTERM_EXECUTABLE", "/usr/bin/wezterm")]);
        assert_eq!(detect_host_terminal_with_getenv(env), HostTerminal::WezTerm);
    }

    #[test]
    fn detect_alacritty_via_term() {
        let env = make_env(&[("TERM", "alacritty")]);
        assert_eq!(detect_host_terminal_with_getenv(env), HostTerminal::Alacritty);
    }

    #[test]
    fn detect_vscode() {
        let env = make_env(&[("TERM_PROGRAM", "vscode")]);
        assert_eq!(detect_host_terminal_with_getenv(env), HostTerminal::VSCode);
    }

    #[test]
    fn detect_warp() {
        let env = make_env(&[("TERM_PROGRAM", "WarpTerminal")]);
        assert_eq!(detect_host_terminal_with_getenv(env), HostTerminal::Warp);
    }

    #[test]
    fn detect_unknown() {
        let env = make_env(&[("TERM", "xterm-256color")]);
        assert_eq!(detect_host_terminal_with_getenv(env), HostTerminal::Unknown);
    }

    #[test]
    fn kitty_supports_graphics() {
        assert!(HostTerminal::Kitty.supports_kitty_graphics());
        assert!(HostTerminal::Ghostty.supports_kitty_graphics());
        assert!(HostTerminal::WezTerm.supports_kitty_graphics());
        assert!(!HostTerminal::AppleTerminal.supports_kitty_graphics());
    }

    #[test]
    fn mac_option_advice_for_terminals() {
        assert!(!mac_option_advice(HostTerminal::AppleTerminal).is_empty());
        assert!(!mac_option_advice(HostTerminal::ITerm2).is_empty());
        assert!(mac_option_advice(HostTerminal::Kitty).is_empty());
    }

    #[test]
    fn inside_multiplexer_tmux() {
        let env = make_env(&[("TMUX", "/tmp/tmux-1000/default")]);
        assert!(inside_multiplexer(env));
    }

    #[test]
    fn inside_multiplexer_screen() {
        let env = make_env(&[("TERM", "screen-256color")]);
        assert!(inside_multiplexer(env));
    }

    #[test]
    fn not_inside_multiplexer() {
        let env = make_env(&[("TERM", "xterm-256color")]);
        assert!(!inside_multiplexer(env));
    }
}
