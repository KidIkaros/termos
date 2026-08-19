//! Web and mobile support — ported from Go TUIOS `cmd/tuios-web/`.
//!
//! Provides:
//! - Touch detection from HTTP headers (Sec-CH-UA-Mobile, User-Agent)
//! - Mobile key bar configuration
//! - Session picker for web
//! - Read-only mode
//! - Max connections limit
//! - Ephemeral sessions

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

// ─── Touch Detection ─────────────────────────────────────────────────────

/// What the operator asked for regarding touch mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TouchMode {
    #[default]
    Auto,
    On,
    Off,
}

impl TouchMode {
    /// Parse a `--touch` value.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "on" | "true" | "yes" | "1" => Some(Self::On),
            "off" | "false" | "no" | "0" => Some(Self::Off),
            _ => None,
        }
    }
}

/// User-Agent substrings that indicate a touch screen, matched
/// case-insensitively. "Mobile" is last because it is the weakest.
pub const TOUCH_TOKENS: &[&str] = &["Android", "iPhone", "iPad", "iPod", "Silk/", "Mobile"];

/// Detect whether a client is a touch device from HTTP headers.
pub fn client_is_touch(sec_ch_ua_mobile: Option<&str>, user_agent: Option<&str>) -> bool {
    // Sec-CH-UA-Mobile is checked first — it's an exact answer.
    if let Some(hint) = sec_ch_ua_mobile {
        match hint.trim() {
            "?1" => return true,
            "?0" => return false,
            _ => {}
        }
    }
    // Fall back to User-Agent pattern matching.
    if let Some(ua) = user_agent {
        let ua_lower = ua.to_lowercase();
        for tok in TOUCH_TOKENS {
            if ua_lower.contains(&tok.to_lowercase()) {
                return true;
            }
        }
    }
    false
}

/// Resolve touch mode given the operator's preference and client headers.
pub fn resolve_touch(
    mode: TouchMode,
    sec_ch_ua_mobile: Option<&str>,
    user_agent: Option<&str>,
) -> bool {
    match mode {
        TouchMode::On => true,
        TouchMode::Off => false,
        TouchMode::Auto => client_is_touch(sec_ch_ua_mobile, user_agent),
    }
}

// ─── Mobile Key Bar ──────────────────────────────────────────────────────

/// A key on the mobile key bar.
#[derive(Debug, Clone)]
pub struct MobileKey {
    pub label: String,
    pub title: String,
    pub prefix: bool,
}

/// A row of keys on the mobile key bar.
#[derive(Debug, Clone)]
pub struct MobileRow {
    pub label: String,
    pub keys: Vec<MobileKey>,
    pub collapsible: bool,
}

/// The mobile key bar: a prefix latch and rows of keys.
#[derive(Debug, Clone, Default)]
pub struct MobileBar {
    pub prefix: Option<MobileKey>,
    pub rows: Vec<MobileRow>,
}

/// The default mobile commands offered in the chord row.
pub const MOBILE_COMMANDS: &[(&str, &str)] = &[
    ("new", "prefix_new_window"),
    ("close", "prefix_close_window"),
    ("next", "prefix_next_window"),
    ("prev", "prefix_prev_window"),
    ("zoom", "prefix_fullscreen"),
    ("cmds", "prefix_command_palette"),
    ("tile", "prefix_toggle_tiling"),
    ("vsplit", "prefix_split_vertical"),
    ("hsplit", "prefix_split_horizontal"),
    ("config", "prefix_settings"),
    ("help", "prefix_help"),
];

/// Default mobile keys for the typing row.
pub fn default_mobile_keys() -> Vec<MobileKey> {
    vec![
        MobileKey {
            label: "Esc".into(),
            title: "Escape".into(),
            prefix: false,
        },
        MobileKey {
            label: "Tab".into(),
            title: "Tab".into(),
            prefix: false,
        },
        MobileKey {
            label: "Ctrl".into(),
            title: "Control".into(),
            prefix: false,
        },
        MobileKey {
            label: "Alt".into(),
            title: "Alt".into(),
            prefix: false,
        },
        MobileKey {
            label: "←".into(),
            title: "Left".into(),
            prefix: false,
        },
        MobileKey {
            label: "↓".into(),
            title: "Down".into(),
            prefix: false,
        },
        MobileKey {
            label: "↑".into(),
            title: "Up".into(),
            prefix: false,
        },
        MobileKey {
            label: "→".into(),
            title: "Right".into(),
            prefix: false,
        },
    ]
}

/// Build the mobile key bar for a session.
pub fn build_mobile_bar(leader: &str, command_keys: &[(&str, &str)]) -> MobileBar {
    let prefix = MobileKey {
        label: "pfx".into(),
        title: format!("Prefix ({}), then the command key", leader),
        prefix: true,
    };

    let mut keys = vec![prefix.clone()];
    for (label, _action) in command_keys {
        keys.push(MobileKey {
            label: label.to_string(),
            title: label.to_string(),
            prefix: false,
        });
    }

    let rows = vec![
        MobileRow {
            label: "TUIOS commands".into(),
            keys,
            collapsible: true,
        },
        MobileRow {
            label: "Keys".into(),
            keys: default_mobile_keys(),
            collapsible: false,
        },
    ];

    MobileBar {
        prefix: Some(prefix),
        rows,
    }
}

// ─── Session Picker ──────────────────────────────────────────────────────

/// A session entry for the web session picker.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SessionPickerEntry {
    pub name: String,
    pub window_count: usize,
    pub attached: bool,
    pub created_at: u64,
}

/// Build a session picker list from session names and window counts.
pub fn build_session_picker(sessions: &[(String, usize, bool, u64)]) -> Vec<SessionPickerEntry> {
    sessions
        .iter()
        .map(|(name, windows, attached, created_at)| SessionPickerEntry {
            name: name.clone(),
            window_count: *windows,
            attached: *attached,
            created_at: *created_at,
        })
        .collect()
}

// ─── HTML / URL escaping ─────────────────────────────────────────────────

/// Percent-encode a string for use as a single URL path segment (RFC 3986
/// unreserved characters pass through). Session names are already restricted
/// to a safe charset, but this guards against anything that slipped through.
pub fn url_path_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push('%');
                out.push(char::from_digit((b >> 4) as u32, 16).unwrap().to_ascii_uppercase());
                out.push(char::from_digit((b & 0x0f) as u32, 16).unwrap().to_ascii_uppercase());
            }
        }
    }
    out
}

/// Escape a string for safe embedding in HTML text and attribute values.
pub fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

// ─── Read-Only Mode ──────────────────────────────────────────────────────

/// Read-only mode prevents input from being sent to the PTY.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReadOnlyMode {
    pub read_only: bool,
}

impl ReadOnlyMode {
    /// Create a new read-only mode setting.
    pub fn new(read_only: bool) -> Self {
        Self { read_only }
    }

    /// Whether input should be blocked.
    pub fn blocks_input(&self) -> bool {
        self.read_only
    }
}

// ─── Max Connections Limit ───────────────────────────────────────────────

/// A connection limiter for the web server.
pub struct ConnectionLimiter {
    max: u64,
    current: AtomicU64,
}

impl ConnectionLimiter {
    /// Create a new limiter with the given max.
    pub fn new(max: u64) -> Self {
        Self {
            max,
            current: AtomicU64::new(0),
        }
    }

    /// Try to acquire a connection slot. Returns true if allowed.
    pub fn acquire(&self) -> bool {
        if self.max == 0 {
            return true; // 0 = unlimited
        }
        let prev = self.current.fetch_add(1, Ordering::SeqCst);
        if prev >= self.max {
            self.current.fetch_sub(1, Ordering::SeqCst);
            false
        } else {
            true
        }
    }

    /// Release a connection slot.
    pub fn release(&self) {
        self.current.fetch_sub(1, Ordering::SeqCst);
    }

    /// Current number of active connections.
    pub fn current(&self) -> u64 {
        self.current.load(Ordering::SeqCst)
    }

    /// Whether the limiter is at capacity.
    pub fn at_capacity(&self) -> bool {
        self.max > 0 && self.current() >= self.max
    }
}

// ─── Ephemeral Sessions ──────────────────────────────────────────────────

/// An ephemeral session is one that is destroyed when the last client
/// disconnects. Used for web sessions that should not persist.
#[derive(Debug, Default)]
pub struct EphemeralSessions {
    sessions: Mutex<HashMap<String, usize>>, // session_name -> client_count
}

impl EphemeralSessions {
    /// Create a new tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a client attaching to an ephemeral session.
    pub fn attach(&self, session_name: &str) {
        let mut sessions = self.sessions.lock().unwrap();
        *sessions.entry(session_name.to_string()).or_insert(0) += 1;
    }

    /// Register a client detaching from an ephemeral session.
    /// Returns true if the session should be destroyed (last client left).
    pub fn detach(&self, session_name: &str) -> bool {
        let mut sessions = self.sessions.lock().unwrap();
        if let Some(count) = sessions.get_mut(session_name) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                sessions.remove(session_name);
                return true;
            }
        }
        false
    }

    /// Whether a session is ephemeral.
    pub fn is_ephemeral(&self, session_name: &str) -> bool {
        let sessions = self.sessions.lock().unwrap();
        sessions.contains_key(session_name)
    }

    /// Number of clients attached to an ephemeral session.
    pub fn client_count(&self, session_name: &str) -> usize {
        let sessions = self.sessions.lock().unwrap();
        sessions.get(session_name).copied().unwrap_or(0)
    }
}

// ─── Transport Security ──────────────────────────────────────────────────

/// Check whether a bind address is loopback (localhost or 127.x.x.x).
pub fn is_loopback_host(host: &str) -> bool {
    host == "localhost" || host == "127.0.0.1" || host == "::1" || host.starts_with("127.")
}

/// Whether clients must present a token: always when one is configured, and
/// for any non-loopback bind even without one (a remote frontend must be
/// gated, so an empty configured token means "no access"). Loopback binds
/// without a configured token stay open, matching the roadmap rule.
pub fn requires_token(host: &str, token: Option<&str>) -> bool {
    !is_loopback_host(host) || token.is_some()
}

/// Validate a client-supplied token against the configured one. When no
/// token is configured there is nothing to validate against, so any supplied
/// value is rejected rather than accepted.
pub fn token_is_valid(expected: Option<&str>, provided: Option<&str>) -> bool {
    match (expected, provided) {
        (Some(exp), Some(prov)) => {
            // Constant-time comparison to avoid leaking the token via timing.
            if exp.len() != prov.len() {
                return false;
            }
            exp.bytes()
                .zip(prov.bytes())
                .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                == 0
        }
        _ => false,
    }
}

/// Check whether the server should refuse to start without TLS.
///
/// Non-loopback addresses require TLS to prevent credential exposure on the
/// network. Returns an error message if TLS is required but not configured.
pub fn check_transport_security(host: &str, tls_enabled: bool) -> Result<(), String> {
    if !tls_enabled && !is_loopback_host(host) {
        return Err(format!(
            "Refusing to serve without TLS on non-loopback address {host}.\n\
             Use --auto-tls or provide --cert and --key files.\n\
             For local testing, bind to 127.0.0.1 or localhost."
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_path_escape_keeps_unreserved() {
        assert_eq!(url_path_escape("dev"), "dev");
        assert_eq!(url_path_escape("session-0"), "session-0");
        assert_eq!(url_path_escape("Payments-API"), "Payments-API");
    }

    #[test]
    fn url_path_escape_encodes_reserved() {
        assert_eq!(url_path_escape("a b"), "a%20b");
        assert_eq!(url_path_escape("a&b"), "a%26b");
        assert_eq!(url_path_escape("a?b#c"), "a%3Fb%23c");
        assert_eq!(url_path_escape("100%"), "100%25");
    }

    #[test]
    fn html_escape_entities() {
        assert_eq!(html_escape("dev"), "dev");
        assert_eq!(html_escape("<script>&\"'"), "&lt;script&gt;&amp;&quot;&#39;");
    }

    #[test]
    fn requires_token_rules() {
        assert!(!requires_token("127.0.0.1", None));
        assert!(!requires_token("localhost", None));
        assert!(requires_token("127.0.0.1", Some("secret")));
        assert!(requires_token("0.0.0.0", None));
        assert!(requires_token("192.168.1.10", None));
        assert!(requires_token("192.168.1.10", Some("secret")));
    }

    #[test]
    fn token_is_valid_matches_constant_time() {
        assert!(token_is_valid(Some("s3cret"), Some("s3cret")));
        assert!(!token_is_valid(Some("s3cret"), Some("wrong")));
        assert!(!token_is_valid(Some("s3cret"), None));
        assert!(!token_is_valid(None, Some("anything")));
        assert!(!token_is_valid(None, None));
        // Length mismatch short-circuits.
        assert!(!token_is_valid(Some("abc"), Some("abcd")));
    }

    #[test]
    fn touch_mode_parse() {
        assert_eq!(TouchMode::parse("auto"), Some(TouchMode::Auto));
        assert_eq!(TouchMode::parse("on"), Some(TouchMode::On));
        assert_eq!(TouchMode::parse("off"), Some(TouchMode::Off));
        assert_eq!(TouchMode::parse("invalid"), None);
    }

    #[test]
    fn client_is_touch_sec_ch_ua() {
        assert!(client_is_touch(Some("?1"), None));
        assert!(!client_is_touch(Some("?0"), None));
    }

    #[test]
    fn client_is_touch_user_agent() {
        assert!(client_is_touch(
            None,
            Some("Mozilla/5.0 (iPhone; CPU iPhone OS 16_0)")
        ));
        assert!(client_is_touch(
            None,
            Some("Mozilla/5.0 (Linux; Android 13)")
        ));
        assert!(!client_is_touch(
            None,
            Some("Mozilla/5.0 (X11; Linux x86_64)")
        ));
    }

    #[test]
    fn resolve_touch_modes() {
        assert!(resolve_touch(TouchMode::On, None, None));
        assert!(!resolve_touch(TouchMode::Off, Some("?1"), None));
        assert!(resolve_touch(TouchMode::Auto, Some("?1"), None));
        assert!(!resolve_touch(TouchMode::Auto, Some("?0"), None));
    }

    #[test]
    fn mobile_bar_has_prefix() {
        let bar = build_mobile_bar("ctrl+b", MOBILE_COMMANDS);
        assert!(bar.prefix.is_some());
        assert_eq!(bar.prefix.unwrap().label, "pfx");
        assert_eq!(bar.rows.len(), 2);
        assert!(bar.rows[0].collapsible);
        assert!(!bar.rows[1].collapsible);
    }

    #[test]
    fn default_mobile_keys_count() {
        let keys = default_mobile_keys();
        assert_eq!(keys.len(), 8);
    }

    #[test]
    fn session_picker_builds() {
        let sessions = vec![
            ("work".to_string(), 3, true, 1000),
            ("play".to_string(), 1, false, 2000),
        ];
        let picker = build_session_picker(&sessions);
        assert_eq!(picker.len(), 2);
        assert_eq!(picker[0].name, "work");
        assert_eq!(picker[0].window_count, 3);
        assert!(picker[0].attached);
    }

    #[test]
    fn read_only_blocks_input() {
        let ro = ReadOnlyMode::new(true);
        assert!(ro.blocks_input());
        let rw = ReadOnlyMode::new(false);
        assert!(!rw.blocks_input());
    }

    #[test]
    fn connection_limiter_acquire_release() {
        let limiter = ConnectionLimiter::new(2);
        assert!(limiter.acquire());
        assert!(limiter.acquire());
        assert!(!limiter.acquire()); // at capacity
        limiter.release();
        assert!(limiter.acquire()); // slot freed
    }

    #[test]
    fn connection_limiter_unlimited() {
        let limiter = ConnectionLimiter::new(0);
        assert!(limiter.acquire());
        assert!(limiter.acquire());
        assert!(limiter.acquire());
    }

    #[test]
    fn connection_limiter_at_capacity() {
        let limiter = ConnectionLimiter::new(1);
        limiter.acquire();
        assert!(limiter.at_capacity());
        limiter.release();
        assert!(!limiter.at_capacity());
    }

    #[test]
    fn ephemeral_session_lifecycle() {
        let ephem = EphemeralSessions::new();
        ephem.attach("session1");
        assert!(ephem.is_ephemeral("session1"));
        assert_eq!(ephem.client_count("session1"), 1);
        ephem.attach("session1");
        assert_eq!(ephem.client_count("session1"), 2);
        assert!(!ephem.detach("session1")); // still 1 client
        assert!(ephem.detach("session1")); // last client left
        assert!(!ephem.is_ephemeral("session1"));
    }

    #[test]
    fn ephemeral_session_not_found() {
        let ephem = EphemeralSessions::new();
        assert!(!ephem.is_ephemeral("nonexistent"));
        assert_eq!(ephem.client_count("nonexistent"), 0);
        assert!(!ephem.detach("nonexistent"));
    }

    #[test]
    fn is_loopback_checks() {
        assert!(is_loopback_host("localhost"));
        assert!(is_loopback_host("127.0.0.1"));
        assert!(is_loopback_host("127.0.1.1"));
        assert!(is_loopback_host("::1"));
        assert!(!is_loopback_host("0.0.0.0"));
        assert!(!is_loopback_host("192.168.1.1"));
        assert!(!is_loopback_host("example.com"));
    }

    #[test]
    fn transport_security_loopback_ok() {
        assert!(check_transport_security("localhost", false).is_ok());
        assert!(check_transport_security("127.0.0.1", false).is_ok());
    }

    #[test]
    fn transport_security_non_loopback_requires_tls() {
        assert!(check_transport_security("0.0.0.0", false).is_err());
        assert!(check_transport_security("192.168.1.1", false).is_err());
    }

    #[test]
    fn transport_security_non_loopback_with_tls_ok() {
        assert!(check_transport_security("0.0.0.0", true).is_ok());
    }
}
