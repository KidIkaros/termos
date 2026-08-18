//! Shell detection and terminal environment setup — ported from Go TUIOS
//! `internal/terminal/window_env.go`.
//!
//! Detects the user's preferred shell (config → `$SHELL` → `/etc/passwd` →
//! common paths), builds the `TERM`/`COLORTERM`/`TERM_PROGRAM` environment a
//! guest shell sees, and records which graphics protocols the host terminal
//! can forward so newly spawned shells advertise a matching identity.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::util::guestenv;

// ---------------------------------------------------------------------------
// Graphics capability globals (guarded by atomics)
// ---------------------------------------------------------------------------

/// Whether the host terminal supports the kitty graphics protocol.
static KITTY_GRAPHICS_HOST: AtomicBool = AtomicBool::new(false);

/// Whether the host terminal supports the sixel graphics protocol.
static SIXEL_GRAPHICS_HOST: AtomicBool = AtomicBool::new(false);

/// Record which graphics protocols TermOS can forward to the host terminal.
/// Windows created afterwards advertise a matching terminal identity to their
/// shell (see [`guest_term_program`]).
///
/// Ported from Go `SetGraphicsCapabilities`.
pub fn set_graphics_capabilities(kitty: bool, sixel: bool) {
    KITTY_GRAPHICS_HOST.store(kitty, Ordering::Release);
    SIXEL_GRAPHICS_HOST.store(sixel, Ordering::Release);
}

/// Whether the host supports kitty graphics.
pub fn kitty_graphics_host() -> bool {
    KITTY_GRAPHICS_HOST.load(Ordering::Acquire)
}

/// Whether the host supports sixel graphics.
pub fn sixel_graphics_host() -> bool {
    SIXEL_GRAPHICS_HOST.load(Ordering::Acquire)
}

/// The `TERM_PROGRAM` value for a newly spawned shell, derived from the
/// recorded graphics capabilities.
///
/// Ported from Go `guestTermProgram`.
pub fn guest_term_program() -> &'static str {
    guestenv::term_program(kitty_graphics_host(), sixel_graphics_host())
}

// ---------------------------------------------------------------------------
// Shell detection
// ---------------------------------------------------------------------------

/// Detect the user's preferred shell.
///
/// Resolution order (ported from Go `detectShell`):
/// 1. User config `appearance.preferred_shell` (if set and the path exists).
/// 2. The `SHELL` environment variable.
/// 3. Common Unix shell paths (`/bin/bash`, `/bin/zsh`, `/bin/fish`,
///    `/bin/sh`).
/// 4. Fallback to `/bin/sh`.
pub fn detect_shell() -> String {
    // 1. Check user configuration.
    let cfg = crate::config::userconfig::UserConfig::load();
    if !cfg.appearance.preferred_shell.is_empty() {
        let preferred = &cfg.appearance.preferred_shell;
        if std::path::Path::new(preferred).exists() {
            return preferred.clone();
        }
        eprintln!(
            "Warning: Configured shell '{}' not found. Falling back to defaults.",
            preferred
        );
    }

    // 2. Check the SHELL environment variable.
    if let Ok(shell) = std::env::var("SHELL") {
        if !shell.is_empty() {
            return shell;
        }
    }

    // 3. Check common Unix shell paths.
    for shell in &["/bin/bash", "/bin/zsh", "/bin/fish", "/bin/sh"] {
        if std::path::Path::new(shell).exists() {
            return shell.to_string();
        }
    }

    // 4. Fallback.
    "/bin/sh".to_string()
}

/// Detect the user's preferred shell, consulting `/etc/passwd` when `SHELL` is
/// unset and no config override is present. This is the full Unix fallback
/// chain used when the environment does not carry `SHELL` (e.g. a daemon
/// started by systemd).
pub fn detect_shell_with_passwd() -> String {
    // Config and SHELL first.
    let cfg = crate::config::userconfig::UserConfig::load();
    if !cfg.appearance.preferred_shell.is_empty() {
        let preferred = &cfg.appearance.preferred_shell;
        if std::path::Path::new(preferred).exists() {
            return preferred.clone();
        }
    }
    if let Ok(shell) = std::env::var("SHELL") {
        if !shell.is_empty() {
            return shell;
        }
    }

    // /etc/passwd lookup for the current uid.
    if let Some(shell) = shell_from_passwd() {
        if !shell.is_empty() && std::path::Path::new(&shell).exists() {
            return shell;
        }
    }

    // Common paths.
    for shell in &["/bin/bash", "/bin/zsh", "/bin/fish", "/bin/sh"] {
        if std::path::Path::new(shell).exists() {
            return shell.to_string();
        }
    }

    "/bin/sh".to_string()
}

/// Read the login shell from `/etc/passwd` for the current real UID.
/// Returns `None` when the file cannot be read or the entry is not found.
fn shell_from_passwd() -> Option<String> {
    #[cfg(unix)]
    {
        let uid = unsafe { nix::libc::getuid() };
        let content = std::fs::read_to_string("/etc/passwd").ok()?;
        for line in content.lines() {
            let fields: Vec<&str> = line.split(':').collect();
            if fields.len() >= 7 {
                if let Ok(entry_uid) = fields[2].parse::<u32>() {
                    if entry_uid == uid {
                        return Some(fields[6].to_string());
                    }
                }
            }
        }
        None
    }
    #[cfg(not(unix))]
    {
        None
    }
}

// ---------------------------------------------------------------------------
// Terminal environment (TERM / COLORTERM)
// ---------------------------------------------------------------------------

/// A color profile level, mirroring Go's `colorprofile.Profile` but without
/// the charm dependency. TermOS detects the profile from the environment
/// rather than querying the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorProfile {
    /// 24-bit true color (`COLORTERM=truecolor`).
    TrueColor,
    /// 256-color palette.
    Ansi256,
    /// Basic 16-color ANSI.
    Ansi,
    /// No color support or not a TTY.
    Ascii,
}

impl ColorProfile {
    /// Detect the color profile from the environment.
    ///
    /// Ported from Go `getTerminalEnv` / `profileToEnv`. The detection is
    /// environmental: `COLORTERM=truecolor` → TrueColor, `TERM` containing
    /// `256color` → Ansi256, `TERM` not `dumb` → Ansi, else Ascii.
    pub fn detect() -> Self {
        let colorterm = std::env::var("COLORTERM").unwrap_or_default();
        let term = std::env::var("TERM").unwrap_or_default();

        if colorterm == "truecolor" && !term.is_empty() && term != "dumb" {
            return ColorProfile::TrueColor;
        }
        if term.contains("256color") {
            return ColorProfile::Ansi256;
        }
        if !term.is_empty() && term != "dumb" {
            return ColorProfile::Ansi;
        }
        ColorProfile::Ascii
    }
}

/// Convert a color profile to `(TERM, COLORTERM)` environment values.
///
/// Ported from Go `profileToEnv`. Preserves the parent `TERM` when it is
/// suitable, falling back to sensible defaults.
pub fn profile_to_env(profile: ColorProfile) -> (String, String) {
    let parent_term = std::env::var("TERM").unwrap_or_default();
    match profile {
        ColorProfile::TrueColor => {
            let term = if !parent_term.is_empty() {
                parent_term
            } else {
                "xterm-256color".to_string()
            };
            (term, "truecolor".to_string())
        }
        ColorProfile::Ansi256 => {
            let term = if !parent_term.is_empty() && parent_term.contains("256color") {
                parent_term
            } else if parent_term.starts_with("screen") {
                "screen-256color".to_string()
            } else if parent_term.starts_with("tmux") {
                "tmux-256color".to_string()
            } else {
                "xterm-256color".to_string()
            };
            (term, String::new())
        }
        ColorProfile::Ansi => {
            let term = if !parent_term.is_empty() && parent_term != "dumb" {
                parent_term
            } else {
                "xterm".to_string()
            };
            (term, String::new())
        }
        ColorProfile::Ascii => ("dumb".to_string(), String::new()),
    }
}

/// Detect and return the `(TERM, COLORTERM)` pair for the current environment.
/// Convenience wrapper around [`ColorProfile::detect`] + [`profile_to_env`].
pub fn get_terminal_env() -> (String, String) {
    profile_to_env(ColorProfile::detect())
}

// ---------------------------------------------------------------------------
// Environment variable injection for child processes
// ---------------------------------------------------------------------------

/// Build the full environment vector a guest shell should receive.
///
/// This merges:
/// - `TERM` and `COLORTERM` from [`get_terminal_env`].
/// - `TERM_PROGRAM` / `TERM_PROGRAM_VERSION` from the graphics capabilities.
/// - The base guest environment (`TUIOS_SESSION`, `TUIOS_WINDOW_ID`, etc.).
/// - Any caller-supplied extra key-value pairs (e.g. `TERMOS_ENV`).
///
/// The returned vector is suitable for passing as `extra_env` to
/// [`crate::terminal::pty::spawn_pty`].
pub fn build_guest_env(
    session: &str,
    window_id: &str,
    extra: &[(String, String)],
) -> Vec<(String, String)> {
    let (term, colorterm) = get_terminal_env();

    let mut env = Vec::with_capacity(8 + extra.len());
    env.push(("TERM".to_string(), term));
    if !colorterm.is_empty() {
        env.push(("COLORTERM".to_string(), colorterm));
    }
    env.push((
        "TERM_PROGRAM".to_string(),
        guest_term_program().to_string(),
    ));
    env.push((
        "TERM_PROGRAM_VERSION".to_string(),
        std::env::var("TERMOS_TERM_PROGRAM_VERSION").unwrap_or_else(|_| "0.1.0".to_string()),
    ));

    // Base guest env (TUIOS_SESSION, TUIOS_WINDOW_ID, TUIOS_ENV).
    env.extend(guestenv::base_guest_env(
        session,
        window_id,
        kitty_graphics_host(),
        sixel_graphics_host(),
    ));

    // Caller-supplied extras (applied last so they can override).
    env.extend(extra.iter().cloned());

    env
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_shell_returns_a_path() {
        let shell = detect_shell();
        assert!(!shell.is_empty());
        assert!(shell.starts_with('/') || shell.contains('/'));
    }

    #[test]
    fn detect_shell_with_passwd_returns_a_path() {
        let shell = detect_shell_with_passwd();
        assert!(!shell.is_empty());
    }

    #[test]
    fn set_and_read_graphics_capabilities() {
        set_graphics_capabilities(true, false);
        assert!(kitty_graphics_host());
        assert!(!sixel_graphics_host());

        set_graphics_capabilities(false, true);
        assert!(!kitty_graphics_host());
        assert!(sixel_graphics_host());

        set_graphics_capabilities(false, false);
        assert!(!kitty_graphics_host());
        assert!(!sixel_graphics_host());
    }

    #[test]
    fn guest_term_program_reflects_capabilities() {
        set_graphics_capabilities(true, true);
        assert_eq!(guest_term_program(), "ghostty");

        set_graphics_capabilities(false, true);
        assert_eq!(guest_term_program(), "WezTerm");

        set_graphics_capabilities(false, false);
        assert_eq!(guest_term_program(), "TUIOS");
    }

    #[test]
    fn profile_to_env_truecolor() {
        let (term, colorterm) = profile_to_env(ColorProfile::TrueColor);
        assert_eq!(colorterm, "truecolor");
        assert!(!term.is_empty());
    }

    #[test]
    fn profile_to_env_ansi256() {
        let (term, colorterm) = profile_to_env(ColorProfile::Ansi256);
        assert!(term.contains("256color"));
        assert!(colorterm.is_empty());
    }

    #[test]
    fn profile_to_env_ansi() {
        let (term, colorterm) = profile_to_env(ColorProfile::Ansi);
        assert!(!term.is_empty());
        assert!(colorterm.is_empty());
    }

    #[test]
    fn profile_to_env_ascii() {
        let (term, colorterm) = profile_to_env(ColorProfile::Ascii);
        assert_eq!(term, "dumb");
        assert!(colorterm.is_empty());
    }

    #[test]
    fn build_guest_env_includes_term_and_program() {
        let env = build_guest_env("work", "w1", &[]);
        let map: std::collections::HashMap<_, _> = env.into_iter().collect();
        assert!(map.contains_key("TERM"));
        assert!(map.contains_key("TERM_PROGRAM"));
        assert_eq!(map.get("TUIOS_SESSION").unwrap(), "work");
        assert_eq!(map.get("TUIOS_WINDOW_ID").unwrap(), "w1");
    }

    #[test]
    fn build_guest_env_merges_extras() {
        let extras = vec![("CUSTOM_VAR".to_string(), "value".to_string())];
        let env = build_guest_env("s", "w", &extras);
        let map: std::collections::HashMap<_, _> = env.into_iter().collect();
        assert_eq!(map.get("CUSTOM_VAR").unwrap(), "value");
    }
}
