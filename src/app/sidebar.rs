//! The sidebar rail — sessions and their windows with agent-state glyphs.
//!
//! Ported from Go TUIOS `internal/app/sidebar_*.go`: a side rail listing the
//! daemon sessions (or, in local mode, the current session's windows), each
//! row carrying its agent state. Navigation is vim-style (j/k), Enter
//! activates a row, Esc leaves.
//!
//! This module is the umbrella for the sidebar's submodules:
//! - `accent` — per-window accent colours and the accent picker model
//! - `agents` — agent section filtering, sorting, and priority
//! - `cache` — render caching with FNV-1a signature invalidation
//! - `marquee` — scrolling long titles that don't fit
//! - `unread` — the done/unread bit on finished panes
//! - `return_state` — return-to-session navigation after leaving the rail
//! - `title_debounce` — title change debouncing to prevent flicker
//! - `strip` — collapsed rail (glyph strip) rendering
//! - `state` — persisted sidebar preferences (order, width, accents)

pub mod accent;
pub mod agents;
pub mod cache;
pub mod marquee;
pub mod return_state;
pub mod strip;
pub mod title_debounce;
pub mod unread;

use std::collections::HashMap;

use crate::config::theme::Rgb;

// ─── Row model ───────────────────────────────────────────────────────────

/// A row's kind: a session header or a window under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RowKind {
    Session,
    Window,
    /// A row in the agents section (targets a window, lives in a different
    /// section).
    Agent,
    /// The all/here filter token in the agents header.
    AgentFilter,
    /// The priority/recency sort token in the agents header.
    AgentSort,
    /// The "+" in the sessions header (new session control).
    NewSession,
    /// The "+" in the terminals header (new window control).
    NewWindow,
    /// The footer's collapse/expand toggle.
    Collapse,
}

impl RowKind {
    /// Whether this row kind targets a specific window.
    pub fn is_window_target(self) -> bool {
        matches!(self, RowKind::Window | RowKind::Agent)
    }

    /// Whether this row kind has a context menu.
    pub fn has_menu(self) -> bool {
        matches!(self, RowKind::Session | RowKind::Window | RowKind::Agent)
    }
}

/// One row of the sidebar rail.
#[derive(Debug, Clone)]
pub struct SidebarRow {
    pub kind: RowKind,
    pub label: String,
    pub detail: String,
    pub session: Option<String>,
    pub window: Option<usize>,
    pub workspace: i32,
    pub agent_state: String,
    /// The window's unique ID (for resolving across reorders).
    pub window_id: Option<String>,
    /// Whether a finished pane has been looked at (unread bit).
    pub done_seen: bool,
    /// When the agent state was last set (epoch nanos, 0 = unknown).
    pub agent_state_at: u64,
    /// Whether this row belongs to a session other than the attached one.
    pub foreign: bool,
}

impl SidebarRow {
    /// Create a session row.
    pub fn session_row(label: impl Into<String>, detail: impl Into<String>, session: impl Into<String>) -> Self {
        Self {
            kind: RowKind::Session,
            label: label.into(),
            detail: detail.into(),
            session: Some(session.into()),
            window: None,
            workspace: 0,
            agent_state: String::new(),
            window_id: None,
            done_seen: false,
            agent_state_at: 0,
            foreign: false,
        }
    }

    /// Create a window row.
    pub fn window_row(
        label: impl Into<String>,
        detail: impl Into<String>,
        window: usize,
        workspace: i32,
        agent_state: impl Into<String>,
        window_id: impl Into<String>,
    ) -> Self {
        Self {
            kind: RowKind::Window,
            label: label.into(),
            detail: detail.into(),
            session: None,
            window: Some(window),
            workspace,
            agent_state: agent_state.into(),
            window_id: Some(window_id.into()),
            done_seen: false,
            agent_state_at: 0,
            foreign: false,
        }
    }
}

// ─── Navigation row ──────────────────────────────────────────────────────

/// One keyboard-navigable rail row: what the cursor can land on and what
/// activating it targets. Mirrors `SidebarRow` without the label/detail, since
/// the keyboard addresses rows by position in the list rather than by pixel.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NavRow {
    pub kind: RowKind,
    pub session_id: String,
    pub window_id: String,
    pub window_index: i32,
}

impl NavRow {
    /// Create a nav row from a sidebar row's identity.
    pub fn from_row(row: &SidebarRow) -> Self {
        Self {
            kind: row.kind,
            session_id: row.session.clone().unwrap_or_default(),
            window_id: row.window_id.clone().unwrap_or_default(),
            window_index: row.window.map(|i| i as i32).unwrap_or(-1),
        }
    }
}

// ─── Hit rectangle ───────────────────────────────────────────────────────

/// The on-screen rectangle of one sidebar row, in absolute screen coordinates,
/// plus what it points at. The mouse handlers hit-test these to route a click.
#[derive(Debug, Clone)]
pub struct RowHit {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
    pub kind: RowKind,
    pub session_id: String,
    pub window_id: String,
    pub window_index: i32,
}

impl RowHit {
    /// Whether the absolute cell (x, y) falls on this row.
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x0 && x < self.x1 && y >= self.y0 && y < self.y1
    }

    /// The nav row this hit rectangle points at.
    pub fn nav_row(&self) -> NavRow {
        NavRow {
            kind: self.kind,
            session_id: self.session_id.clone(),
            window_id: self.window_id.clone(),
            window_index: self.window_index,
        }
    }
}

// ─── Agent entry ─────────────────────────────────────────────────────────

/// One pane running an agent, flattened for the agents section.
#[derive(Debug, Clone)]
pub struct AgentEntry {
    pub session_id: String,
    pub window_id: String,
    pub title: String,
    pub state: String,
    pub done_seen: bool,
    pub state_at: u64,
    pub window_index: i32,
    pub session_label: String,
    pub foreign: bool,
}

// ─── Sidebar state ───────────────────────────────────────────────────────

/// The sidebar state — view preferences and interaction state.
#[derive(Debug, Clone, Default)]
pub struct Sidebar {
    pub open: bool,
    pub selected: usize,
    /// Whether the rail holds keyboard focus.
    pub focused: bool,
    /// Whether the rail is collapsed to its glyph strip.
    pub collapsed: bool,
    /// The user's drag-defined session order.
    pub order: Vec<String>,
    /// Per-window accent colours, keyed by window ID.
    pub accents: HashMap<String, Accent>,
    /// Window IDs of finished panes already looked at (unread bit).
    pub agent_seen: HashMap<String, bool>,
    /// The agents section's filter: "all" or "session".
    pub agents_filter: String,
    /// The agents section's sort: "priority" or "recent".
    pub agents_sort: String,
    /// Per-section scroll offsets: [sessions, terminals, agents].
    pub scroll: [usize; 3],
    /// The cursor position in the nav row list.
    pub cursor: usize,
    /// The nav rows the last frame recorded.
    pub nav: Vec<NavRow>,
    /// The hit rectangles the last frame recorded.
    pub hits: Vec<RowHit>,
    /// The session IDs in rail order.
    pub session_ids: Vec<String>,
    /// The session being previewed (hover/cursor on a session row).
    pub peek: String,
    /// Whether the rail was revealed only to host keyboard focus.
    pub revealed_for_focus: bool,
    /// Hover state.
    pub hover_active: bool,
    pub hover_x: i32,
    pub hover_y: i32,
    /// The marquee key (which row is scrolling), empty when none.
    pub marquee_key: String,
    /// When the marquee started.
    pub marquee_start: Option<std::time::Instant>,
    /// The last row the cursor was on (for restore on re-enter).
    pub last_row: Option<NavRow>,
    /// Return-to-session state.
    pub return_armed: bool,
    pub return_mode: bool,
    pub return_window: String,
    /// Title debounce entries: window ID -> (shown title, when adopted).
    pub titles: HashMap<String, (String, std::time::Instant)>,
    /// Whether any title change is still pending.
    pub title_pending: bool,
    /// The render cache.
    pub cache: cache::RenderCache,
}

impl Sidebar {
    /// Create a closed sidebar.
    pub fn new() -> Self {
        Self::default()
    }

    /// Toggle the sidebar open/closed.
    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.selected = 0;
    }

    /// Open the sidebar.
    pub fn open(&mut self) {
        self.open = true;
        self.selected = 0;
    }

    /// Close the sidebar.
    pub fn close(&mut self) {
        self.open = false;
        self.selected = 0;
    }

    /// Move the selection by `delta` rows (wrapping).
    pub fn move_selection(&mut self, delta: i32, count: usize) {
        if count == 0 {
            return;
        }
        self.selected = (self.selected as i32 + delta).rem_euclid(count as i32) as usize;
    }

    /// Move the cursor by `delta` over the nav rows, clamped to the ends.
    pub fn cursor_move(&mut self, delta: i32) {
        if self.nav.is_empty() {
            return;
        }
        let new = (self.cursor as i32 + delta)
            .clamp(0, self.nav.len() as i32 - 1) as usize;
        self.set_cursor(new);
    }

    /// Jump the cursor to the first row.
    pub fn cursor_first(&mut self) {
        self.set_cursor(0);
    }

    /// Jump the cursor to the last row.
    pub fn cursor_last(&mut self) {
        if !self.nav.is_empty() {
            self.set_cursor(self.nav.len() - 1);
        }
    }

    /// Set the cursor and re-derive the peek from where it landed.
    pub fn set_cursor(&mut self, i: usize) {
        self.cursor = i;
        let Some(row) = self.nav.get(i) else {
            self.peek.clear();
            return;
        };
        if row.kind == RowKind::Session && !row.session_id.is_empty() {
            self.peek = row.session_id.clone();
        } else {
            self.peek.clear();
        }
    }

    /// The nav row the cursor is on, if valid.
    pub fn cursor_row(&self) -> Option<&NavRow> {
        self.nav.get(self.cursor)
    }

    /// Give the keyboard to the rail.
    pub fn enter_focus(&mut self) {
        if self.focused {
            return;
        }
        self.focused = true;
        self.return_armed = true;
    }

    /// Return the keyboard to the panes.
    pub fn exit_focus(&mut self) {
        if !self.focused {
            return;
        }
        self.focused = false;
        // Record the row the cursor was on.
        if let Some(row) = self.cursor_row() {
            self.last_row = Some(row.clone());
        }
        self.peek.clear();
        self.return_armed = false;
    }

    /// The cursor position of the attached session's row, or 0.
    pub fn current_session_nav_index(&self, current_session: &str) -> usize {
        for (i, r) in self.nav.iter().enumerate() {
            if r.kind == RowKind::Session && r.session_id == current_session {
                return i;
            }
        }
        0
    }

    /// Invalidate the render cache, forcing a rebuild on the next frame.
    pub fn invalidate_cache(&mut self) {
        self.cache.invalidate();
    }

    /// Whether a marquee animation is active.
    pub fn marquee_active(&self) -> bool {
        !self.marquee_key.is_empty()
    }

    /// Cycle the agents filter between "all" and "session".
    pub fn cycle_agents_filter(&mut self) {
        if self.agents_filter() == "all" {
            self.agents_filter = "session".into();
        } else {
            self.agents_filter = "all".into();
        }
    }

    /// Cycle the agents sort between "priority" and "recent".
    pub fn cycle_agents_sort(&mut self) {
        if self.agents_sort() == "priority" {
            self.agents_sort = "recent".into();
        } else {
            self.agents_sort = "priority".into();
        }
    }

    /// The effective agents filter, defaulting to "all".
    pub fn agents_filter(&self) -> &str {
        if self.agents_filter == "session" {
            "session"
        } else {
            "all"
        }
    }

    /// The effective agents sort, defaulting to "priority".
    pub fn agents_sort(&self) -> &str {
        if self.agents_sort == "recent" {
            "recent"
        } else {
            "priority"
        }
    }
}

// ─── Accent ──────────────────────────────────────────────────────────────

/// An accent colour: either a named ANSI slot or a literal RGB.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Accent {
    /// A named ANSI slot index (0-14, mapping to the theme's palette).
    Slot(i32),
    /// A literal RGB colour.
    Rgb(Rgb),
}

impl Accent {
    /// Whether this accent is a named slot.
    pub fn is_slot(&self) -> bool {
        matches!(self, Accent::Slot(_))
    }

    /// The slot index, if this is a slot accent.
    pub fn slot(&self) -> Option<i32> {
        match self {
            Accent::Slot(i) => Some(*i),
            _ => None,
        }
    }

    /// The RGB colour this accent resolves to, given a theme palette.
    pub fn rgb(&self, palette: &[Rgb; 16]) -> Rgb {
        match self {
            Accent::Slot(idx) => {
                let i = (*idx).clamp(0, 14) as usize;
                // First 8 are ANSI 8-15 (bright), rest are ANSI 1-7.
                if i < 8 {
                    palette[8 + i]
                } else {
                    palette[i - 8 + 1]
                }
            }
            Accent::Rgb(c) => *c,
        }
    }

    /// The hex string for this accent.
    pub fn hex(&self, palette: &[Rgb; 16]) -> String {
        let c = self.rgb(palette);
        format!("#{:02x}{:02x}{:02x}", c.0, c.1, c.2)
    }

    /// Fold this accent into a hash for cache signatures.
    pub fn fold(&self, palette: &[Rgb; 16]) -> u64 {
        let c = self.rgb(palette);
        (c.0 as u64) | ((c.1 as u64) << 8) | ((c.2 as u64) << 16)
    }
}

/// Parse an accent from a free-form string: a colour name or #rrggbb.
pub fn parse_accent(s: &str) -> Option<Accent> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    // Normalise: lowercase, strip spaces/dashes/underscores.
    let key: String = s
        .chars()
        .filter_map(|c| match c {
            ' ' | '-' | '_' => None,
            _ => Some(c.to_ascii_lowercase()),
        })
        .collect();
    if let Some(slot) = accent_name_to_slot(&key) {
        return Some(Accent::Slot(slot));
    }
    Rgb::parse(s).map(Accent::Rgb)
}

/// Map a normalised colour name to a legacy accent slot index.
fn accent_name_to_slot(key: &str) -> Option<i32> {
    let map: &[(&str, i32)] = &[
        ("brightblack", 0),
        ("brightred", 1),
        ("brightgreen", 2),
        ("brightyellow", 3),
        ("brightblue", 4),
        ("brightpurple", 5),
        ("brightmagenta", 5),
        ("brightcyan", 6),
        ("brightwhite", 7),
        ("black", 0),
        ("red", 8),
        ("green", 9),
        ("yellow", 10),
        ("blue", 11),
        ("purple", 12),
        ("magenta", 12),
        ("cyan", 13),
        ("white", 14),
    ];
    map.iter().find(|(name, _)| *name == key).map(|(_, slot)| *slot)
}

/// The number of accent swatch slots (8 bright + 7 normal, skipping ANSI black).
pub const ACCENT_SWATCH_COUNT: i32 = 15;

/// The number of bright accent slots.
pub const ACCENT_BRIGHT_COUNT: i32 = 8;

/// The accent slot names, in order.
pub const ACCENT_SLOT_NAMES: &[&str] = &[
    "bright black", "bright red", "bright green", "bright yellow",
    "bright blue", "bright purple", "bright cyan", "bright white",
    "red", "green", "yellow", "blue", "purple", "cyan", "white",
];

/// Resolve a legacy accent index against a theme palette.
pub fn accent_color(idx: i32, palette: &[Rgb; 16]) -> Rgb {
    let idx = idx.clamp(0, ACCENT_SWATCH_COUNT - 1);
    if idx < ACCENT_BRIGHT_COUNT {
        palette[(ACCENT_BRIGHT_COUNT + idx) as usize]
    } else {
        palette[(idx - ACCENT_BRIGHT_COUNT + 1) as usize]
    }
}

/// The accent mark glyph (the one-cell chip an accented row wears).
pub fn accent_mark() -> &'static str {
    "\u{258c}" // ▌
}

/// Find the nearest accent slot to a colour.
pub fn accent_nearest_slot(c: Rgb, palette: &[Rgb; 16]) -> i32 {
    let mut best = 0;
    let mut best_dist = f64::MAX;
    for i in 0..ACCENT_SWATCH_COUNT {
        let s = Accent::Slot(i).rgb(palette);
        let dr = s.0 as f64 - c.0 as f64;
        let dg = s.1 as f64 - c.1 as f64;
        let db = s.2 as f64 - c.2 as f64;
        let d = dr * dr + dg * dg + db * db;
        if d < best_dist {
            best = i;
            best_dist = d;
        }
    }
    best
}

// ─── Printable title ─────────────────────────────────────────────────────

/// Chrome glyphs that are audited and kept (not stripped by printable_title).
const CHROME_GLYPHS: &[char] = &['\u{25cf}', '\u{25b2}', '\u{25cb}', '\u{25a0}'];

/// Whether a rune is printable as chrome (sidebar rows, dock, title badge).
pub fn printable_rune(r: char, ascii_only: bool) -> bool {
    if (r as u32) < 0x20 || ((r as u32) >= 0x7f && (r as u32) < 0xa0) {
        return false; // C0/C1 controls
    }
    if (r as u32) >= 0xe000 && (r as u32) <= 0xf8ff {
        return false; // BMP private use area
    }
    if (r as u32) >= 0xf0000 {
        return false; // Plane 15/16 private use
    }
    if (r as u32) >= 0x25a0
        && (r as u32) <= 0x2bff
        && !CHROME_GLYPHS.contains(&r)
    {
        return false; // Geometric Shapes through Miscellaneous Symbols
    }
    if (r as u32) >= 0xfe00 && (r as u32) <= 0xfe0f {
        return false; // Variation Selectors
    }
    if (r as u32) >= 0x1f000 && (r as u32) <= 0x1faff {
        return false; // Emoji and pictographic planes
    }
    if ascii_only && (r as u32) > 0x7e {
        return false;
    }
    true
}

/// Strip non-printable characters from a title before showing it as chrome.
pub fn printable_title(s: &str) -> String {
    let filtered: String = s.chars().filter(|&r| printable_rune(r, false)).collect();
    filtered.trim().to_string()
}

/// Like `printable_title` but without the trim (for the rename field).
pub fn printable_runes(s: &str) -> String {
    s.chars().filter(|&r| printable_rune(r, false)).collect()
}

// ─── Row building ────────────────────────────────────────────────────────

/// Build the sidebar rows for the current app state.
///
/// Local mode shows one session node (the current workspace) with its windows;
/// daemon mode shows every session with the attached session's windows
/// expanded underneath.
pub fn build_rows(
    remote_session: Option<&str>,
    remote_sessions: &[crate::session::model::SessionInfo],
    windows: &[crate::terminal::window::Window],
    workspace: i32,
    window_workspace: impl Fn(usize) -> i32,
) -> Vec<SidebarRow> {
    let mut rows = Vec::new();
    if let Some(current) = remote_session {
        // Session rows first.
        let mut sessions: Vec<&crate::session::model::SessionInfo> =
            remote_sessions.iter().collect();
        sessions.sort_by(|a, b| a.name.cmp(&b.name));
        for s in &sessions {
            let detail = if s.attached {
                format!("{} window(s) \u{00b7} attached", s.windows)
            } else {
                format!("{} window(s)", s.windows)
            };
            rows.push(SidebarRow::session_row(
                &s.name,
                &detail,
                &s.name,
            ));
            // Expand the attached session's windows under it.
            if s.name == current {
                for (idx, w) in windows.iter().enumerate() {
                    rows.push(SidebarRow::window_row(
                        &w.title,
                        format!("ws {}", window_workspace(idx)),
                        idx,
                        window_workspace(idx),
                        &w.agent_state,
                        &w.id,
                    ));
                }
            }
        }
    } else {
        // Local mode: one session node plus its windows.
        rows.push(SidebarRow::session_row(
            format!("workspace {workspace}"),
            format!("{} window(s)", windows.len()),
            "local",
        ));
        for (idx, w) in windows.iter().enumerate() {
            rows.push(SidebarRow::window_row(
                &w.title,
                format!("ws {}", window_workspace(idx)),
                idx,
                window_workspace(idx),
                &w.agent_state,
                &w.id,
            ));
        }
    }
    rows
}

// ─── Agent glyphs and colours ────────────────────────────────────────────

/// The agent-state glyph for a window row (mirrors Go's agentStateIndicator).
pub fn agent_glyph(state: &str) -> &'static str {
    match state {
        "working" => "\u{25d0}",     // ◐
        "needs_input" => "\u{270b}", // ✋
        "idle" => "\u{25cb}",        // ○
        "done" => "\u{2713}",        // ✓
        "errored" => "\u{2715}",     // ✕
        _ => " ",
    }
}

/// Whether a state means a human is required (the attention states).
pub fn sidebar_attention(state: &str) -> bool {
    state == "needs_input" || state == "errored"
}

/// The agent-state severity colour as an ANSI palette index.
pub fn agent_glyph_color_slot(state: &str) -> i32 {
    match state {
        "working" => 4,      // blue (info)
        "needs_input" => 3,  // yellow (warning)
        "idle" => 8,         // bright black (muted)
        "done" => 2,         // green (success)
        "errored" => 1,      // red (warn)
        _ => 8,              // bright black (muted)
    }
}

/// The agent-state colour with the unread bit folded in.
pub fn sidebar_state_color_slot(state: &str, done_seen: bool) -> i32 {
    if state == "done" && done_seen {
        return 8; // muted
    }
    agent_glyph_color_slot(state)
}

/// The severity colour for an attention state, or -1 for none.
pub fn sidebar_severity_color_slot(state: &str) -> i32 {
    match state {
        "needs_input" => 3, // yellow
        "errored" => 1,     // red
        _ => -1,
    }
}

/// Format an elapsed duration (in nanoseconds since state was set) into at
/// most three cells: `Ns`, `Nm`, `Mh`, or `Dd` for days.
pub fn format_elapsed_ns(elapsed_ns: u64) -> String {
    let secs = elapsed_ns / 1_000_000_000;
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86_400)
    }
}

/// How long a pane has been in its state, in at most three cells.
/// `state_at` is when the state was last set; `now` is the current instant.
pub fn agent_elapsed(
    state: &str,
    state_at: Option<std::time::Instant>,
    now: std::time::Instant,
) -> String {
    if state == "idle" || state.is_empty() {
        return String::new();
    }
    let Some(at) = state_at else {
        return String::new();
    };
    if now <= at {
        return String::new();
    }
    let elapsed = now - at;
    format_elapsed_ns(elapsed.as_nanos() as u64)
}

/// Agent priority for the agents section sort.
pub fn agent_priority(state: &str, done_seen: bool) -> i32 {
    match state {
        "errored" => 5,
        "needs_input" => 4,
        "working" => 3,
        "done" => {
            if done_seen {
                1
            } else {
                2
            }
        }
        _ => 0, // idle and unknown
    }
}

/// The transition notice (word and severity) for a state change.
pub fn agent_transition_notice(to: &str) -> Option<(&'static str, &'static str)> {
    match to {
        "needs_input" => Some(("needs input", "warning")),
        "errored" => Some(("errored", "error")),
        "done" => Some(("finished", "success")),
        "working" => Some(("working", "info")),
        "idle" => Some(("idle", "info")),
        _ => None,
    }
}

// ─── Layout variants ─────────────────────────────────────────────────────

/// Sidebar layout variants, chosen from the reserved width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarVariant {
    /// Collapsed to glyph strip (3 columns).
    Glyph,
    /// Narrow (no trailing figures).
    Narrow,
    /// Full width.
    Full,
}

/// The glyph strip width.
pub const SIDEBAR_GLYPH_WIDTH: i32 = 3;

/// The narrow width breakpoint.
pub const SIDEBAR_NARROW_WIDTH: i32 = 20;

/// Determine the layout variant from the reserved width.
pub fn sidebar_variant(width: i32) -> SidebarVariant {
    if width <= SIDEBAR_GLYPH_WIDTH {
        SidebarVariant::Glyph
    } else if width <= SIDEBAR_NARROW_WIDTH {
        SidebarVariant::Narrow
    } else {
        SidebarVariant::Full
    }
}

/// The column every rail row's text starts on: gutter, glyph, one cell of air.
pub const SIDEBAR_NAME_COL: i32 = 3;

/// Reorder items so those whose key appears in `order` come first, in order's
/// sequence; the rest follow in their given (natural) order.
pub fn order_by_key<T: Clone>(
    items: &[T],
    key: impl Fn(&T) -> String,
    order: &[String],
) -> Vec<T> {
    if order.is_empty() || items.len() < 2 {
        return items.to_vec();
    }
    let mut rank: HashMap<String, usize> = HashMap::new();
    for (i, k) in order.iter().enumerate() {
        rank.entry(k.clone()).or_insert(i);
    }
    let mut out = items.to_vec();
    out.sort_by(|a, b| {
        let ka = key(a);
        let kb = key(b);
        let ra = rank.get(&ka);
        let rb = rank.get(&kb);
        match (ra, rb) {
            (Some(ra), Some(rb)) => ra.cmp(rb),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Theme;
    use crate::session::model::SessionInfo;
    use crate::terminal::pty::WinSize;
    use crate::terminal::window::Window;

    fn info(name: &str, attached: bool) -> SessionInfo {
        SessionInfo {
            id: name.into(),
            name: name.into(),
            created_at: 0,
            attached,
            windows: 1,
            restored: false,
        }
    }

    fn window(id: &str, title: &str, agent: &str) -> Window {
        let mut w = Window::without_pty(
            id.to_string(),
            title.to_string(),
            WinSize { cols: 10, rows: 3 },
        );
        w.agent_state = agent.to_string();
        w
    }

    #[test]
    fn local_mode_has_session_and_windows() {
        let windows = vec![window("w0", "alpha", "working"), window("w1", "beta", "")];
        let rows = build_rows(None, &[], &windows, 1, |_| 1);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].kind, RowKind::Session);
        assert_eq!(rows[0].label, "workspace 1");
        assert_eq!(rows[1].window, Some(0));
        assert_eq!(rows[2].window, Some(1));
        assert_eq!(rows[1].agent_state, "working");
    }

    #[test]
    fn remote_mode_lists_sessions_with_expanded_current() {
        let sessions = vec![info("work", true), info("play", false)];
        let windows = vec![window("w0", "alpha", "")];
        let rows = build_rows(Some("work"), &sessions, &windows, 1, |_| 1);
        // Two session rows + one window row under "work".
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].label, "play");
        assert_eq!(rows[1].label, "work");
        assert!(rows[1].detail.contains("attached"));
        assert_eq!(rows[2].kind, RowKind::Window);
    }

    #[test]
    fn selection_wraps() {
        let mut sb = Sidebar::new();
        sb.open();
        sb.move_selection(-1, 3);
        assert_eq!(sb.selected, 2);
        sb.move_selection(1, 3);
        assert_eq!(sb.selected, 0);
    }

    #[test]
    fn toggle_opens_and_closes() {
        let mut sb = Sidebar::new();
        assert!(!sb.open);
        sb.toggle();
        assert!(sb.open);
        sb.toggle();
        assert!(!sb.open);
    }

    #[test]
    fn agent_glyphs() {
        assert_eq!(agent_glyph("working"), "\u{25d0}");
        assert_eq!(agent_glyph("needs_input"), "\u{270b}");
        assert_eq!(agent_glyph("done"), "\u{2713}");
        assert_eq!(agent_glyph("errored"), "\u{2715}");
        assert_eq!(agent_glyph("idle"), "\u{25cb}");
        assert_eq!(agent_glyph(""), " ");
        assert_eq!(agent_glyph("none"), " ");
    }

    #[test]
    fn agent_priority_ranking() {
        assert_eq!(agent_priority("errored", false), 5);
        assert_eq!(agent_priority("needs_input", false), 4);
        assert_eq!(agent_priority("working", false), 3);
        assert_eq!(agent_priority("done", false), 2);
        assert_eq!(agent_priority("done", true), 1);
        assert_eq!(agent_priority("idle", false), 0);
    }

    #[test]
    fn sidebar_attention_states() {
        assert!(sidebar_attention("needs_input"));
        assert!(sidebar_attention("errored"));
        assert!(!sidebar_attention("working"));
        assert!(!sidebar_attention("done"));
        assert!(!sidebar_attention("idle"));
    }

    #[test]
    fn parse_accent_names() {
        assert_eq!(parse_accent("bright blue"), Some(Accent::Slot(4)));
        assert_eq!(parse_accent("Bright-Blue"), Some(Accent::Slot(4)));
        assert_eq!(parse_accent("cyan"), Some(Accent::Slot(13)));
        assert_eq!(parse_accent("magenta"), Some(Accent::Slot(12)));
        assert_eq!(parse_accent(""), None);
    }

    #[test]
    fn parse_accent_hex() {
        assert_eq!(
            parse_accent("#ff0000"),
            Some(Accent::Rgb(Rgb::new(0xff, 0, 0)))
        );
        assert_eq!(
            parse_accent("00ff00"),
            Some(Accent::Rgb(Rgb::new(0, 0xff, 0)))
        );
    }

    #[test]
    fn accent_rgb_resolution() {
        let palette = Theme::default_ansi();
        assert_eq!(Accent::Slot(0).rgb(&palette), palette[8]); // bright black
        assert_eq!(Accent::Slot(1).rgb(&palette), palette[9]); // bright red
        assert_eq!(Accent::Slot(8).rgb(&palette), palette[1]); // red
        assert_eq!(Accent::Slot(14).rgb(&palette), palette[7]); // white
    }

    #[test]
    fn accent_nearest_slot_finds_closest() {
        let palette = Theme::default_ansi();
        // Bright red is slot 1.
        let slot = accent_nearest_slot(palette[9], &palette);
        assert_eq!(slot, 1);
    }

    #[test]
    fn printable_title_strips_controls() {
        assert_eq!(printable_title("hello\x07world"), "helloworld");
        assert_eq!(printable_title("  clean  "), "clean");
    }

    #[test]
    fn printable_title_strips_private_use() {
        // U+E000 is in the BMP private use area.
        assert_eq!(printable_title("test\u{e000}"), "test");
    }

    #[test]
    fn sidebar_variant_classification() {
        assert_eq!(sidebar_variant(0), SidebarVariant::Glyph);
        assert_eq!(sidebar_variant(3), SidebarVariant::Glyph);
        assert_eq!(sidebar_variant(10), SidebarVariant::Narrow);
        assert_eq!(sidebar_variant(30), SidebarVariant::Full);
    }

    #[test]
    fn order_by_key_reorders() {
        let items = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let order = vec!["c".to_string(), "a".to_string()];
        let result = order_by_key(&items, |s| s.clone(), &order);
        assert_eq!(result, vec!["c", "a", "b"]);
    }

    #[test]
    fn order_by_key_empty_order_keeps_natural() {
        let items = vec!["a".to_string(), "b".to_string()];
        let result = order_by_key(&items, |s| s.clone(), &[]);
        assert_eq!(result, items);
    }

    #[test]
    fn nav_row_from_row() {
        let row = SidebarRow::session_row("work", "3 windows", "work");
        let nav = NavRow::from_row(&row);
        assert_eq!(nav.kind, RowKind::Session);
        assert_eq!(nav.session_id, "work");
    }

    #[test]
    fn row_hit_contains() {
        let hit = RowHit {
            x0: 10,
            y0: 5,
            x1: 40,
            y1: 7,
            kind: RowKind::Session,
            session_id: "work".into(),
            window_id: String::new(),
            window_index: -1,
        };
        assert!(hit.contains(20, 5));
        assert!(hit.contains(39, 6));
        assert!(!hit.contains(40, 5));
        assert!(!hit.contains(20, 7));
    }

    #[test]
    fn agents_filter_default() {
        let sb = Sidebar::new();
        assert_eq!(sb.agents_filter(), "all");
    }

    #[test]
    fn agents_sort_default() {
        let sb = Sidebar::new();
        assert_eq!(sb.agents_sort(), "priority");
    }

    #[test]
    fn cycle_agents_filter() {
        let mut sb = Sidebar::new();
        sb.cycle_agents_filter();
        assert_eq!(sb.agents_filter(), "session");
        sb.cycle_agents_filter();
        assert_eq!(sb.agents_filter(), "all");
    }

    #[test]
    fn cursor_move_clamps() {
        let mut sb = Sidebar::new();
        sb.nav = vec![
            NavRow { kind: RowKind::Session, session_id: "a".into(), window_id: String::new(), window_index: -1 },
            NavRow { kind: RowKind::Session, session_id: "b".into(), window_id: String::new(), window_index: -1 },
            NavRow { kind: RowKind::Session, session_id: "c".into(), window_id: String::new(), window_index: -1 },
        ];
        sb.cursor_move(10);
        assert_eq!(sb.cursor, 2);
        sb.cursor_move(-10);
        assert_eq!(sb.cursor, 0);
    }

    #[test]
    fn enter_and_exit_focus() {
        let mut sb = Sidebar::new();
        sb.enter_focus();
        assert!(sb.focused);
        assert!(sb.return_armed);
        sb.exit_focus();
        assert!(!sb.focused);
        assert!(!sb.return_armed);
    }
}
