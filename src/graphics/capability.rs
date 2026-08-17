//! Host terminal capability probing — ported from TUIOS
//! `internal/config/hostterm.go` and the Kitty/Sixel detection in
//! `internal/app/kitty_passthrough.go`.
//!
//! The probe is environmental (no DA1/DA2 queries): TUIOS reads
//! `$TERM_PROGRAM`, `$TERM`, and the kitty/ghostty/alacritty-specific
//! environment variables to decide which graphics protocols the host
//! supports. A real DA query would race the PTY reader; the environment is
//! authoritative on Linux and macOS where these terminals set it.

/// The host terminal emulator TUIOS is running inside, as far as the
/// environment gives it away. `Unknown` is not a failure — it only means
/// advice has to stay generic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HostTerminal {
    #[default]
    Unknown,
    AppleTerminal,
    ITerm2,
    Ghostty,
    Kitty,
    WezTerm,
    Alacritty,
    VsCode,
    Rio,
    Warp,
}

impl HostTerminal {
    pub fn as_str(self) -> &'static str {
        match self {
            HostTerminal::Unknown => "unknown",
            HostTerminal::AppleTerminal => "Apple Terminal",
            HostTerminal::ITerm2 => "iTerm2",
            HostTerminal::Ghostty => "Ghostty",
            HostTerminal::Kitty => "kitty",
            HostTerminal::WezTerm => "WezTerm",
            HostTerminal::Alacritty => "Alacritty",
            HostTerminal::VsCode => "VS Code",
            HostTerminal::Rio => "Rio",
            HostTerminal::Warp => "Warp",
        }
    }
}

/// The graphics protocols the host terminal supports.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Capabilities {
    /// Kitty graphics protocol (APC G ... ST).
    pub kitty: bool,
    /// Sixel graphics.
    pub sixel: bool,
    /// The host terminal itself (for advice and placement decisions).
    pub host: HostTerminal,
    /// A multiplexer (tmux/screen) sits between TUIOS and the host.
    pub inside_multiplexer: bool,
}

impl Capabilities {
    /// True if any graphics protocol is available.
    pub fn any_graphics(self) -> bool {
        self.kitty || self.sixel
    }

    /// Probe the current process environment.
    pub fn probe() -> Self {
        Self::probe_with(|k| std::env::var(k).ok())
    }

    /// Probe with a custom env getter (for tests).
    pub fn probe_with<F>(getenv: F) -> Self
    where
        F: Fn(&str) -> Option<String>,
    {
        let host = detect_host_terminal_with(&getenv);
        let inside_mux = inside_multiplexer_with(&getenv);
        // Kitty graphics: kitty, Ghostty, WezTerm, iTerm2 (partial), Konsole.
        // Sixel: xterm + -ti vt340, mlterm, foot, wezterm, alacritty (via
        // config). The environment alone can't always tell sixel, but the
        // terminals that unconditionally support it set recognizable vars.
        let kitty = matches!(
            host,
            HostTerminal::Kitty | HostTerminal::Ghostty | HostTerminal::WezTerm
        );
        let sixel = matches!(host, HostTerminal::WezTerm | HostTerminal::Alacritty)
            || getenv("TERM")
                .map(|t| t.contains("vt340") || t.contains("mlterm") || t.contains("foot"))
                .unwrap_or(false);
        Capabilities {
            kitty,
            sixel,
            host,
            inside_multiplexer: inside_mux,
        }
    }
}

/// Detect the host terminal from the environment.
pub fn detect_host_terminal() -> HostTerminal {
    detect_host_terminal_with(|k| std::env::var(k).ok())
}

fn detect_host_terminal_with<F>(getenv: F) -> HostTerminal
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(prog) = getenv("TERM_PROGRAM") {
        match prog.as_str() {
            "Apple_Terminal" => return HostTerminal::AppleTerminal,
            "iTerm.app" => return HostTerminal::ITerm2,
            "WezTerm" => return HostTerminal::WezTerm,
            "ghostty" => return HostTerminal::Ghostty,
            "vscode" => return HostTerminal::VsCode,
            "WarpTerminal" => return HostTerminal::Warp,
            _ => {}
        }
    }
    if getenv("KITTY_WINDOW_ID").is_some() {
        return HostTerminal::Kitty;
    }
    if getenv("GHOSTTY_RESOURCES_DIR").is_some() || getenv("GHOSTTY_BIN_DIR").is_some() {
        return HostTerminal::Ghostty;
    }
    if getenv("ALACRITTY_WINDOW_ID").is_some() || getenv("ALACRITTY_SOCKET").is_some() {
        return HostTerminal::Alacritty;
    }
    if getenv("WEZTERM_EXECUTABLE").is_some() {
        return HostTerminal::WezTerm;
    }
    if let Some(term) = getenv("TERM") {
        if term.contains("kitty") {
            return HostTerminal::Kitty;
        }
        if term.contains("ghostty") {
            return HostTerminal::Ghostty;
        }
        if term.contains("alacritty") {
            return HostTerminal::Alacritty;
        }
        if term.contains("rio") {
            return HostTerminal::Rio;
        }
    }
    HostTerminal::Unknown
}

/// True if a multiplexer (tmux/screen) sits between TUIOS and the host.
pub fn inside_multiplexer() -> bool {
    inside_multiplexer_with(|k| std::env::var(k).ok())
}

fn inside_multiplexer_with<F>(getenv: F) -> bool
where
    F: Fn(&str) -> Option<String>,
{
    if getenv("TMUX").is_some() {
        return true;
    }
    getenv("TERM")
        .map(|t| t.starts_with("screen") || t.starts_with("tmux"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k: &str| map.get(k).cloned()
    }

    #[test]
    fn detects_kitty_via_term_program() {
        let caps = Capabilities::probe_with(env(&[("TERM_PROGRAM", "WezTerm")]));
        assert_eq!(caps.host, HostTerminal::WezTerm);
        assert!(caps.kitty);
        assert!(caps.sixel);
    }

    #[test]
    fn detects_kitty_via_window_id() {
        let caps =
            Capabilities::probe_with(env(&[("KITTY_WINDOW_ID", "1"), ("TERM", "xterm-kitty")]));
        assert_eq!(caps.host, HostTerminal::Kitty);
        assert!(caps.kitty);
    }

    #[test]
    fn detects_ghostty_via_resources_dir() {
        let caps = Capabilities::probe_with(env(&[("GHOSTTY_RESOURCES_DIR", "/opt/ghostty")]));
        assert_eq!(caps.host, HostTerminal::Ghostty);
        assert!(caps.kitty);
    }

    #[test]
    fn detects_tmux_multiplexer() {
        let caps = Capabilities::probe_with(env(&[("TMUX", "/tmp/tmux")]));
        assert!(caps.inside_multiplexer);
        assert_eq!(caps.host, HostTerminal::Unknown);
    }

    #[test]
    fn detects_screen_via_term() {
        let caps = Capabilities::probe_with(env(&[("TERM", "screen-256color")]));
        assert!(caps.inside_multiplexer);
    }

    #[test]
    fn unknown_terminal_has_no_graphics() {
        let caps = Capabilities::probe_with(env(&[("TERM", "xterm-256color")]));
        assert_eq!(caps.host, HostTerminal::Unknown);
        assert!(!caps.any_graphics());
    }

    #[test]
    fn sixel_via_vt340_term() {
        let caps = Capabilities::probe_with(env(&[("TERM", "vt340")]));
        assert!(caps.sixel);
    }

    #[test]
    fn mac_option_advice_host_names() {
        assert_eq!(HostTerminal::Kitty.as_str(), "kitty");
        assert_eq!(HostTerminal::WezTerm.as_str(), "WezTerm");
        assert_eq!(HostTerminal::Unknown.as_str(), "unknown");
    }
}
