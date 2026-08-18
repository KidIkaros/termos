//! SSH server features — ported from Go TUIOS `internal/server/`.
//!
//! Provides:
//! - SSH session picker
//! - SSH capability negotiation (graphics, terminal detection)
//! - Kitty shared memory crash handling

use std::collections::HashMap;

// ─── Client Capabilities ─────────────────────────────────────────────────

/// Graphics and terminal capabilities reported by an SSH client.
#[derive(Debug, Clone, Default)]
pub struct ClientCapabilities {
    pub terminal_name: String,
    pub kitty_graphics: bool,
    pub sixel_graphics: bool,
    pub cell_width: i32,
    pub cell_height: i32,
    pub pixel_width: i32,
    pub pixel_height: i32,
}

/// Terminals known to render the kitty graphics protocol.
pub const KITTY_CAPABLE_TERMINALS: &[&str] = &["kitty", "ghostty", "wezterm"];

/// Terminals known to render sixel graphics.
pub const SIXEL_CAPABLE_TERMINALS: &[&str] =
    &["wezterm", "foot", "contour", "mlterm", "mintty", "xterm"];

/// Parse a "KEY=VALUE" environment slice into a map.
pub fn parse_environ(environ: &[String]) -> HashMap<String, String> {
    let mut m = HashMap::with_capacity(environ.len());
    for kv in environ {
        if let Some(i) = kv.find('=') {
            if i > 0 {
                m.insert(kv[..i].to_string(), kv[i + 1..].to_string());
            }
        }
    }
    m
}

/// Resolve a canonical terminal identity from the client's TERM and
/// forwarded environment.
pub fn terminal_name(term: &str, env: &HashMap<String, String>) -> String {
    let term_program = env
        .get("TERM_PROGRAM")
        .map(|s| s.to_lowercase())
        .unwrap_or_default();

    if term_program.contains("ghostty") {
        return "ghostty".into();
    }
    if term_program.contains("kitty") {
        return "kitty".into();
    }
    if term_program.contains("wezterm") {
        return "wezterm".into();
    }
    if term_program.contains("iterm") {
        return "iterm2".into();
    }
    if term_program.contains("contour") {
        return "contour".into();
    }
    if term_program.contains("foot") {
        return "foot".into();
    }

    if env
        .get("KITTY_WINDOW_ID")
        .map(|s| !s.is_empty())
        .unwrap_or(false)
    {
        return "kitty".into();
    }
    if env
        .get("GHOSTTY_RESOURCES_DIR")
        .map(|s| !s.is_empty())
        .unwrap_or(false)
    {
        return "ghostty".into();
    }
    if env
        .get("WEZTERM_PANE")
        .map(|s| !s.is_empty())
        .unwrap_or(false)
    {
        return "wezterm".into();
    }

    let t = term.to_lowercase();
    for name in &[
        "ghostty", "kitty", "wezterm", "foot", "contour", "mlterm", "xterm",
    ] {
        if t.contains(name) {
            return (*name).into();
        }
    }
    String::new()
}

/// Derive a cell's pixel size from the pty-req window dimensions.
pub fn cell_size_from_window(
    width: i32,
    height: i32,
    width_pixels: i32,
    height_pixels: i32,
) -> (i32, i32) {
    let cw = if width > 0 && width_pixels > 0 {
        width_pixels / width
    } else {
        0
    };
    let ch = if height > 0 && height_pixels > 0 {
        height_pixels / height
    } else {
        0
    };
    (cw, ch)
}

/// Build client capabilities from the SSH session's terminal info.
pub fn build_client_capabilities(
    term: &str,
    environ: &[String],
    width: i32,
    height: i32,
    width_pixels: i32,
    height_pixels: i32,
) -> ClientCapabilities {
    let env = parse_environ(environ);
    let name = terminal_name(term, &env);

    let mut caps = ClientCapabilities {
        terminal_name: name.clone(),
        kitty_graphics: KITTY_CAPABLE_TERMINALS.contains(&name.as_str()),
        sixel_graphics: SIXEL_CAPABLE_TERMINALS.contains(&name.as_str()),
        ..Default::default()
    };

    // Explicit client overrides win over identity-based guesses.
    match env.get("TUIOS_KITTY_GRAPHICS").map(|s| s.as_str()) {
        Some("1") => caps.kitty_graphics = true,
        Some("0") => caps.kitty_graphics = false,
        _ => {}
    }
    match env.get("TUIOS_SIXEL_GRAPHICS").map(|s| s.as_str()) {
        Some("1") => caps.sixel_graphics = true,
        Some("0") => caps.sixel_graphics = false,
        _ => {}
    }

    let (cw, ch) = cell_size_from_window(width, height, width_pixels, height_pixels);
    caps.cell_width = cw;
    caps.cell_height = ch;
    if width_pixels > 0 {
        caps.pixel_width = width_pixels;
    }
    if height_pixels > 0 {
        caps.pixel_height = height_pixels;
    }

    caps
}

// ─── SSH Session Picker ──────────────────────────────────────────────────

/// An entry in the SSH session picker.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SshSessionEntry {
    pub name: String,
    pub window_count: usize,
    pub attached: bool,
    pub created_at: u64,
}

/// Build the session picker list for SSH clients.
pub fn build_ssh_session_picker(sessions: &[(String, usize, bool, u64)]) -> Vec<SshSessionEntry> {
    sessions
        .iter()
        .map(|(name, windows, attached, created_at)| SshSessionEntry {
            name: name.clone(),
            window_count: *windows,
            attached: *attached,
            created_at: *created_at,
        })
        .collect()
}

/// Render the session picker as a text menu for SSH clients.
pub fn render_session_picker(sessions: &[SshSessionEntry]) -> String {
    if sessions.is_empty() {
        return "No sessions available. Press Enter to create a new one.\n".into();
    }
    let mut out = String::from("Available sessions:\n\n");
    for (i, s) in sessions.iter().enumerate() {
        let status = if s.attached { " (attached)" } else { "" };
        out.push_str(&format!(
            "  [{}] {} ({} windows){}\n",
            i + 1,
            s.name,
            s.window_count,
            status
        ));
    }
    out.push_str("\n  [0] Create new session\n");
    out.push_str("\nSelect a session: ");
    out
}

/// Parse the user's session picker choice.
/// Returns the session index (0-based) or None for "create new".
pub fn parse_session_choice(input: &str, session_count: usize) -> Option<Option<usize>> {
    let trimmed = input.trim();
    if trimmed == "0" || trimmed.is_empty() {
        return Some(None); // create new
    }
    let n: usize = trimmed.parse().ok()?;
    if n == 0 {
        Some(None)
    } else if n <= session_count {
        Some(Some(n - 1))
    } else {
        None
    }
}

// ─── Kitty SHM Crash Handling ────────────────────────────────────────────

/// Whether to suppress kitty shared memory graphics after a crash.
/// Some SSH clients crash when receiving kitty SHM sequences, so we
/// detect and disable them.
pub fn should_suppress_kitty_shm(terminal_name: &str, crash_count: u32) -> bool {
    crash_count >= 1 && !is_kitty_shm_safe(terminal_name)
}

/// Terminals that are known to handle kitty SHM safely over SSH.
pub fn is_kitty_shm_safe(terminal_name: &str) -> bool {
    matches!(terminal_name, "kitty" | "ghostty")
}

// ─── Host Capabilities ───────────────────────────────────────────────────

/// App-level host capabilities, projected from the client's reported caps.
#[derive(Debug, Clone, Default)]
pub struct HostCapabilities {
    pub kitty_graphics: bool,
    pub kitty_file_transfer: bool,
    pub sixel_graphics: bool,
    pub true_color: bool,
    pub terminal_name: String,
    pub pixel_width: i32,
    pub pixel_height: i32,
    pub cell_width: i32,
    pub cell_height: i32,
}

/// Project client capabilities onto app-level host capabilities.
///
/// `kitty_file_transfer` is always false: a file-medium transmission names a
/// path on the server, which the remote client cannot read, so the passthrough
/// must re-encode as direct.
pub fn client_to_host_capabilities(c: &ClientCapabilities) -> HostCapabilities {
    HostCapabilities {
        kitty_graphics: c.kitty_graphics,
        kitty_file_transfer: false,
        sixel_graphics: c.sixel_graphics,
        true_color: true,
        terminal_name: c.terminal_name.clone(),
        pixel_width: c.pixel_width,
        pixel_height: c.pixel_height,
        cell_width: c.cell_width,
        cell_height: c.cell_height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_environ_basic() {
        let env = vec!["TERM=xterm".into(), "HOME=/home/user".into()];
        let m = parse_environ(&env);
        assert_eq!(m.get("TERM"), Some(&"xterm".to_string()));
        assert_eq!(m.get("HOME"), Some(&"/home/user".to_string()));
    }

    #[test]
    fn parse_environ_no_equals() {
        let env = vec!["INVALID".into()];
        let m = parse_environ(&env);
        assert!(m.is_empty());
    }

    #[test]
    fn terminal_name_from_term_program() {
        let mut env = HashMap::new();
        env.insert("TERM_PROGRAM".into(), "kitty".into());
        assert_eq!(terminal_name("xterm-256color", &env), "kitty");
    }

    #[test]
    fn terminal_name_from_env_var() {
        let mut env = HashMap::new();
        env.insert("KITTY_WINDOW_ID".into(), "1".into());
        assert_eq!(terminal_name("xterm", &env), "kitty");
    }

    #[test]
    fn terminal_name_from_term() {
        let env = HashMap::new();
        assert_eq!(terminal_name("xterm-256color", &env), "xterm");
        assert_eq!(terminal_name("foot", &env), "foot");
    }

    #[test]
    fn terminal_name_unknown() {
        let env = HashMap::new();
        assert_eq!(terminal_name("dumb", &env), "");
    }

    #[test]
    fn cell_size_from_window_dims() {
        assert_eq!(cell_size_from_window(80, 24, 640, 384), (8, 16));
        assert_eq!(cell_size_from_window(0, 0, 640, 384), (0, 0));
    }

    #[test]
    fn build_caps_kitty() {
        let env = vec!["TERM_PROGRAM=kitty".into()];
        let caps = build_client_capabilities("xterm", &env, 80, 24, 640, 384);
        assert_eq!(caps.terminal_name, "kitty");
        assert!(caps.kitty_graphics);
        assert!(!caps.sixel_graphics);
        assert_eq!(caps.cell_width, 8);
        assert_eq!(caps.cell_height, 16);
    }

    #[test]
    fn build_caps_with_override() {
        let env = vec!["TUIOS_KITTY_GRAPHICS=0".into()];
        let caps = build_client_capabilities("kitty", &env, 80, 24, 0, 0);
        assert!(!caps.kitty_graphics); // explicitly disabled
    }

    #[test]
    fn build_caps_wezterm_both() {
        let env = vec![];
        let caps = build_client_capabilities("wezterm", &env, 80, 24, 0, 0);
        assert!(caps.kitty_graphics);
        assert!(caps.sixel_graphics);
    }

    #[test]
    fn ssh_session_picker_renders() {
        let sessions = vec![
            SshSessionEntry {
                name: "work".into(),
                window_count: 3,
                attached: true,
                created_at: 1000,
            },
            SshSessionEntry {
                name: "play".into(),
                window_count: 1,
                attached: false,
                created_at: 2000,
            },
        ];
        let rendered = render_session_picker(&sessions);
        assert!(rendered.contains("work"));
        assert!(rendered.contains("play"));
        assert!(rendered.contains("(attached)"));
        assert!(rendered.contains("Create new session"));
    }

    #[test]
    fn ssh_session_picker_empty() {
        let rendered = render_session_picker(&[]);
        assert!(rendered.contains("No sessions"));
    }

    #[test]
    fn parse_session_choice_valid() {
        assert_eq!(parse_session_choice("1", 3), Some(Some(0)));
        assert_eq!(parse_session_choice("3", 3), Some(Some(2)));
        assert_eq!(parse_session_choice("0", 3), Some(None));
        assert_eq!(parse_session_choice("", 3), Some(None));
    }

    #[test]
    fn parse_session_choice_invalid() {
        assert_eq!(parse_session_choice("4", 3), None);
        assert_eq!(parse_session_choice("abc", 3), None);
    }

    #[test]
    fn kitty_shm_crash_suppression() {
        assert!(should_suppress_kitty_shm("wezterm", 1));
        assert!(!should_suppress_kitty_shm("kitty", 1));
        assert!(!should_suppress_kitty_shm("wezterm", 0));
    }

    #[test]
    fn kitty_shm_safe_terminals() {
        assert!(is_kitty_shm_safe("kitty"));
        assert!(is_kitty_shm_safe("ghostty"));
        assert!(!is_kitty_shm_safe("wezterm"));
        assert!(!is_kitty_shm_safe("xterm"));
    }

    #[test]
    fn client_to_host_projects() {
        let caps = ClientCapabilities {
            terminal_name: "kitty".into(),
            kitty_graphics: true,
            sixel_graphics: false,
            cell_width: 8,
            cell_height: 16,
            pixel_width: 640,
            pixel_height: 384,
        };
        let host = client_to_host_capabilities(&caps);
        assert!(host.kitty_graphics);
        assert!(!host.kitty_file_transfer); // always false over SSH
        assert!(!host.sixel_graphics);
        assert!(host.true_color);
        assert_eq!(host.terminal_name, "kitty");
        assert_eq!(host.cell_width, 8);
        assert_eq!(host.pixel_width, 640);
    }

    #[test]
    fn client_to_host_defaults() {
        let caps = ClientCapabilities::default();
        let host = client_to_host_capabilities(&caps);
        assert!(!host.kitty_graphics);
        assert!(!host.kitty_file_transfer);
        assert!(host.true_color);
    }
}
