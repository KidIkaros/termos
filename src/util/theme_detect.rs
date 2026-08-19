//! Host-terminal light/dark theme detection.
//!
//! TermOS ships paired dark/light themes and can pick between them from the
//! host terminal's preference (`theme = "auto"`). Two signals are used:
//!
//! 1. The `COLORFGBG` environment variable (set by urxvt, konsole, …): a
//!    zero-cost `fg;bg` hint. A bright background index (`>= 8`) means the
//!    terminal defaults to a light background.
//! 2. An OSC 11 query (`ESC ] 11 ; ? BEL`): the terminal replies with its
//!    default background color; relative luminance decides light vs dark.
//!
//! The OSC query must run while the terminal is in raw mode so the reply is
//! delivered immediately (canonical mode buffers input until a newline). The
//! TUI entrypoints call [`query_terminal_background`] right after
//! `enable_raw_mode()`. Contexts without a host terminal (daemon, web, SSH)
//! resolve `auto` from the environment only.

use std::io::{Read, Write};
use std::os::fd::AsFd;
use std::time::Duration;

use nix::poll::{PollFd, PollFlags};
use nix::poll::poll;

use crate::config::theme::Rgb;

/// Which way the host terminal leans.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeMode {
    Light,
    Dark,
}

/// OSC 11 query: "report the default background color".
const OSC11_QUERY: &[u8] = b"\x1b]11;?\x07";

/// How long to wait for the terminal's OSC 11 reply.
const QUERY_TIMEOUT: Duration = Duration::from_millis(250);

/// A complete OSC 11 response, if one has arrived in the buffer.
///
/// - `None` — no complete response yet (keep reading).
/// - `Some(None)` — a complete response arrived but carries no color (the
///   terminal echoed the query back or replied with something unparseable).
/// - `Some(Some(rgb))` — the terminal's default background color.
pub fn parse_osc11(bytes: &[u8]) -> Option<Option<Rgb>> {
    let mut i = 0;
    while i + 5 <= bytes.len() {
        if bytes[i] == 0x1b
            && bytes[i + 1] == b']'
            && bytes[i + 2] == b'1'
            && bytes[i + 3] == b'1'
            && bytes[i + 4] == b';'
        {
            let payload_start = i + 5;
            let mut j = payload_start;
            while j < bytes.len() {
                if bytes[j] == 0x07 {
                    return Some(parse_color_payload(&bytes[payload_start..j]));
                }
                if bytes[j] == 0x1b && j + 1 < bytes.len() && bytes[j + 1] == b'\\' {
                    return Some(parse_color_payload(&bytes[payload_start..j]));
                }
                j += 1;
            }
            // Found the query but no terminator yet — incomplete response.
            return None;
        }
        i += 1;
    }
    None
}

/// Parse a color payload: `rgb:RRRR/GGGG/BBBB` (xterm) or `#RRGGBB` (legacy).
fn parse_color_payload(payload: &[u8]) -> Option<Rgb> {
    let text = std::str::from_utf8(payload).ok()?.trim();
    if text.is_empty() || text == "?" {
        return None;
    }
    if let Some(hex) = text.strip_prefix('#') {
        return Rgb::parse(hex);
    }
    if let Some(rgb) = text.strip_prefix("rgb:") {
        let mut parts = rgb.split('/');
        let r = parts.next()?;
        let g = parts.next()?;
        let b = parts.next()?;
        // xterm pads two-digit colors to four hex digits; use the high byte.
        let hex = |s: &str| u8::from_str_radix(s, 16).ok();
        // 4-digit forms like "1e1e" → take the first two digits.
        let chan = |s: &str| {
            let s = s.trim();
            if s.len() < 2 {
                return None;
            }
            hex(&s[..2])
        };
        return Some(Rgb::new(chan(r)?, chan(g)?, chan(b)?));
    }
    None
}

/// Relative luminance (WCAG 2.x) in `[0, 1]`.
pub fn luminance(rgb: Rgb) -> f64 {
    let lin = |c: u8| {
        let c = f64::from(c) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(rgb.0) + 0.7152 * lin(rgb.1) + 0.0722 * lin(rgb.2)
}

/// A background color above this relative luminance counts as light.
const LIGHT_THRESHOLD: f64 = 0.5;

/// Classify a background color as light or dark.
pub fn mode_from_rgb(rgb: Rgb) -> ThemeMode {
    if luminance(rgb) > LIGHT_THRESHOLD {
        ThemeMode::Light
    } else {
        ThemeMode::Dark
    }
}

/// Classify from the `COLORFGBG` env value (`fg;bg` color indices).
///
/// A background index `>= 8` means the default background is bright (light).
pub fn mode_from_colorfgbg(value: &str) -> Option<ThemeMode> {
    let bg = value.split(';').nth(1)?.trim();
    if bg.is_empty() {
        return None;
    }
    let idx: i32 = bg.parse().ok()?;
    Some(if idx >= 8 { ThemeMode::Light } else { ThemeMode::Dark })
}

/// Resolve the `COLORFGBG` environment variable, if present.
pub fn detect_from_env() -> Option<ThemeMode> {
    mode_from_colorfgbg(&std::env::var("COLORFGBG").ok()?)
}

/// Detect the host terminal's preference: OSC 11 query, then `COLORFGBG`.
///
/// Only call this while the terminal is in raw mode (the reply needs raw
/// input delivery).
pub fn detect_terminal_mode() -> Option<ThemeMode> {
    if let Some(rgb) = query_terminal_background(QUERY_TIMEOUT) {
        return Some(mode_from_rgb(rgb));
    }
    detect_from_env()
}

/// Send an OSC 11 query and read the reply from stdin.
///
/// Returns `None` if the terminal does not answer with a color within
/// `timeout`. Requires raw mode.
pub fn query_terminal_background(timeout: Duration) -> Option<Rgb> {
    let mut stdout = std::io::stdout();
    if stdout.write_all(OSC11_QUERY).is_err() {
        return None;
    }
    if stdout.flush().is_err() {
        return None;
    }

    let mut stdin = std::io::stdin();
    let mut buf = [0u8; 256];
    let mut acc: Vec<u8> = Vec::with_capacity(256);
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        let slice_ms = remaining.as_millis().min(200) as u16;
        let mut fds = [PollFd::new(stdin.as_fd(), PollFlags::POLLIN)];
        match poll(&mut fds, slice_ms) {
            Ok(0) => continue, // poll slice expired; loop until the deadline
            Ok(_) => {}
            Err(_) => return None,
        }
        let readable = fds[0]
            .revents()
            .map(|r| r.contains(PollFlags::POLLIN))
            .unwrap_or(false);
        if !readable {
            continue;
        }
        match stdin.read(&mut buf) {
            Ok(0) => return None, // stdin closed
            Ok(n) => {
                acc.extend_from_slice(&buf[..n]);
                match parse_osc11(&acc) {
                    None => {} // response still coming
                    Some(None) => return None, // answered, but no color
                    Some(Some(rgb)) => return Some(rgb),
                }
            }
            Err(_) => return None,
        }
    }
}

/// Pick the concrete theme name for `theme = "auto"`.
///
/// When detection fails the dark theme is used (the safe default for a
/// terminal app).
pub fn resolve_auto_theme_name(mode: Option<ThemeMode>, light_theme: &str, dark_theme: &str) -> String {
    match mode {
        Some(ThemeMode::Light) => light_theme.to_string(),
        Some(ThemeMode::Dark) | None => dark_theme.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_xterm_rgb_reply_with_bel() {
        let reply = b"\x1b]11;rgb:1e1e/2e2e/2e2e\x07";
        assert_eq!(parse_osc11(reply), Some(Some(Rgb::new(0x1e, 0x2e, 0x2e))));
    }

    #[test]
    fn parses_xterm_rgb_reply_with_st() {
        let reply = b"\x1b]11;rgb:efe9/d9d9/d9d9\x1b\\";
        assert_eq!(parse_osc11(reply), Some(Some(Rgb::new(0xef, 0xd9, 0xd9))));
    }

    #[test]
    fn parses_hex_reply() {
        let reply = b"\x1b]11;#282c34\x07";
        assert_eq!(parse_osc11(reply), Some(Some(Rgb::new(0x28, 0x2c, 0x34))));
    }

    #[test]
    fn unsupported_echo_is_none_color() {
        // Some terminals echo the query back verbatim.
        let reply = b"\x1b]11;?\x07";
        assert_eq!(parse_osc11(reply), Some(None));
    }

    #[test]
    fn incomplete_reply_is_none() {
        let partial = b"\x1b]11;rgb:1e1e/";
        assert_eq!(parse_osc11(partial), None);
    }

    #[test]
    fn fragmented_reply_accumulates() {
        let mut acc: Vec<u8> = Vec::new();
        let reply = b"\x1b]11;rgb:2828/2c2c/3434\x07";
        for chunk in reply.chunks(3) {
            acc.extend_from_slice(chunk);
            // Only the final chunk completes the response.
            if parse_osc11(&acc).is_some() {
                break;
            }
        }
        assert_eq!(parse_osc11(&acc), Some(Some(Rgb::new(0x28, 0x2c, 0x34))));
    }

    #[test]
    fn scans_past_preceding_data() {
        let reply = b"user typed\x1b]11;rgb:1e1e/2e2e/2e2e\x07";
        assert_eq!(parse_osc11(reply), Some(Some(Rgb::new(0x1e, 0x2e, 0x2e))));
    }

    #[test]
    fn dark_background_is_dark() {
        assert_eq!(mode_from_rgb(Rgb::new(0x1e, 0x1e, 0x2e)), ThemeMode::Dark);
    }

    #[test]
    fn light_background_is_light() {
        assert_eq!(mode_from_rgb(Rgb::new(0xef, 0xe9, 0xd9)), ThemeMode::Light);
    }

    #[test]
    fn colorfgbg_classifies() {
        assert_eq!(mode_from_colorfgbg("7;0"), Some(ThemeMode::Dark));
        assert_eq!(mode_from_colorfgbg("0;15"), Some(ThemeMode::Light));
        assert_eq!(mode_from_colorfgbg("15;0"), Some(ThemeMode::Dark));
        assert_eq!(mode_from_colorfgbg("11"), None); // missing ';' separator
        assert_eq!(mode_from_colorfgbg("garbage"), None);
    }

    #[test]
    fn auto_resolution_falls_back_to_dark() {
        assert_eq!(
            resolve_auto_theme_name(Some(ThemeMode::Light), "catppuccin-latte", "catppuccin-mocha"),
            "catppuccin-latte"
        );
        assert_eq!(
            resolve_auto_theme_name(Some(ThemeMode::Dark), "catppuccin-latte", "catppuccin-mocha"),
            "catppuccin-mocha"
        );
        assert_eq!(
            resolve_auto_theme_name(None, "catppuccin-latte", "catppuccin-mocha"),
            "catppuccin-mocha"
        );
    }
}
