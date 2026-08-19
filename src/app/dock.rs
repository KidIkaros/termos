//! Dock layout helpers — workspace pills, minimized-window items, and the
//! copy-mode help tiers. Ported from Go TUIOS `internal/app/dock_helpers.go`.
//!
//! The dock bar is a single row at the bottom of the screen. It carries three
//! regions left-to-right: the mode pill and workspace strip, the minimized
//! window entries, and the right-hand block (copy-mode help or system meters).
//! This module computes the layout the renderer paints and the mouse handler
//! hit-tests against, so the two never disagree about where a clickable element
//! sits.

use crate::app::sidebar::printable_title;
use crate::app::Os;
use crate::config::constants;
use crate::ui::overlay::{truncate, Hint};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// The bare column between two workspace pills.
const WORKSPACE_PILL_GAP: usize = 1;

/// One overflow gutter: the arrow plus the column separating it from the pills.
const WORKSPACE_ARROW_WIDTH: usize = 2;

/// Caps a workspace name on a pill so a long branch name does not push the mode
/// pill and minimized entries off the bar.
const WORKSPACE_PILL_LABEL_MAX: usize = 12;

/// How much of a window's name a dock pill shows.
const DOCK_ITEM_NAME_CELLS: usize = 12;

/// The narrowest dock that carries the session controls at all.
const DOCK_SESSION_ICON_MIN_WIDTH: usize = 34;

/// The width of the " ..." truncation indicator.
const TRUNCATION_INDICATOR_WIDTH: usize = 4;

// ---------------------------------------------------------------------------
// Structs
// ---------------------------------------------------------------------------

/// One workspace chip in the dock's clickable strip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockWorkspaceTab {
    /// The workspace number (1-9), or 0 for the trailing "+" tab.
    pub workspace: i32,
    /// The (possibly truncated) label printed on the pill.
    pub label: String,
    /// Whether the pill's full name was cut short.
    pub clipped: bool,
    /// Whether this is the current workspace.
    pub active: bool,
    /// Whether this is the trailing "+" tab (opens the next free workspace).
    pub add: bool,
    /// The column span of the pill including caps and padding.
    pub width: usize,
}

/// The strip as this frame will draw it.
#[derive(Debug, Clone, Default)]
pub struct DockWorkspaceStrip {
    /// The pills that fit in the available room.
    pub pills: Vec<DockWorkspaceTab>,
    /// The pinned "+" tab, held out of the scrolling run.
    pub add: Option<DockWorkspaceTab>,
    /// Whether there are pills scrolled off the left end.
    pub more_left: bool,
    /// Whether there are pills scrolled off the right end.
    pub more_right: bool,
    /// Whether the strip is scrolling (arrow gutters are held open).
    pub scrolls: bool,
    /// The fixed span the pills are drawn into when scrolling.
    pub inner: usize,
    /// The total width the strip occupies.
    pub width: usize,
}

/// A hit rectangle for a workspace pill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DockWorkspaceHit {
    /// Inclusive left x.
    pub x0: i32,
    /// Exclusive right x.
    pub x1: i32,
    /// The dock row y.
    pub y: i32,
    /// The workspace number (0 for the "+" tab).
    pub workspace: i32,
}

/// Represents a single minimized-window item in the dock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockItem {
    /// The window index into `Os::windows`.
    pub window_index: i32,
    /// The text inside the pill (e.g. " 1:name ").
    pub label: String,
    /// Total width including circles.
    pub width: usize,
    /// Whether the window is minimized.
    pub minimized: bool,
}

/// The calculated layout for the dock bar.
#[derive(Debug, Clone, Default)]
pub struct DockLayout {
    /// The mode pill's text without caps.
    pub mode_label: String,
    /// The passive badge trailing the mode pill.
    pub trail_text: String,
    /// The width the left region claims.
    pub left_width: usize,
    /// The width the right region claims.
    pub right_width: usize,
    /// Number of items that don't fit.
    pub truncated_count: usize,
    /// Items that fit and should be displayed.
    pub visible_items: Vec<DockItem>,
    /// All dock items (before truncation).
    pub items: Vec<DockItem>,
    /// X positions of visible items.
    pub item_positions: Vec<i32>,
    /// The workspace strip layout.
    pub workspace_strip: DockWorkspaceStrip,
}

/// Mode display information for styling.
#[derive(Debug, Clone, Default)]
pub struct ModeInfo {
    /// The block character (e.g. "█").
    pub block: String,
    /// Hex color for the block.
    pub color: String,
    /// Cursor position for copy mode.
    pub cursor_pos: String,
    /// Whether tiling mode is active.
    pub is_tiling: bool,
    /// Next split direction when tiling.
    pub next_split: String,
}

// ---------------------------------------------------------------------------
// Workspace pill helpers
// ---------------------------------------------------------------------------

/// The column span of a pill carrying the given label.
pub fn workspace_pill_width(label: &str) -> usize {
    let cap_left = dock_workspace_cap_left();
    let cap_right = dock_workspace_cap_right();
    cap_left.chars().count() + cap_right.chars().count() + label.chars().count() + 2
}

fn dock_workspace_cap_left() -> &'static str {
    if use_ascii() { "" } else { "\u{e0b6}" }
}

fn dock_workspace_cap_right() -> &'static str {
    if use_ascii() { "" } else { "\u{e0b4}" }
}

/// The whole name a pill stands for.
pub fn workspace_pill_name(os: &Os, n: i32) -> String {
    let label = printable_title(&os.workspace(n).name);
    if label.is_empty() { n.to_string() } else { label }
}

fn workspace_pill_label(os: &Os, n: i32) -> String {
    truncate(&workspace_pill_name(os, n), WORKSPACE_PILL_LABEL_MAX)
}

fn workspace_pill_clipped(os: &Os, n: i32) -> bool {
    workspace_pill_name(os, n).chars().count() > WORKSPACE_PILL_LABEL_MAX
}

fn occupied_workspaces(os: &Os) -> Vec<i32> {
    let mut ws = Vec::with_capacity(9);
    for i in 1..=9 {
        if i == os.current_workspace || workspace_window_count(os, i) > 0 {
            ws.push(i);
        }
    }
    ws
}

fn workspace_window_count(os: &Os, ws: i32) -> usize {
    // Tiled windows plus floating panes.
    os.workspace(ws).tree.get_all_window_ids().len() + os.floats_on_workspace(ws).len()
}

fn next_free_workspace(os: &Os) -> i32 {
    for i in 1..=9 {
        if i != os.current_workspace && workspace_window_count(os, i) == 0 {
            return i;
        }
    }
    0
}

/// Build the dock's workspace strip tabs.
pub fn build_dock_workspace_tabs(os: &Os) -> Vec<DockWorkspaceTab> {
    if !os.config.appearance.dock_workspace_tabs {
        return Vec::new();
    }
    let mut tabs = Vec::with_capacity(9);
    for n in occupied_workspaces(os) {
        let label = workspace_pill_label(os, n);
        tabs.push(DockWorkspaceTab {
            workspace: n,
            width: workspace_pill_width(&label),
            label,
            clipped: workspace_pill_clipped(os, n),
            active: n == os.current_workspace,
            add: false,
        });
    }
    if next_free_workspace(os) > 0 {
        tabs.push(DockWorkspaceTab {
            workspace: 0,
            label: "+".into(),
            clipped: false,
            active: false,
            add: true,
            width: workspace_pill_width("+"),
        });
    }
    if tabs.len() < 2 {
        return Vec::new();
    }
    tabs
}

fn dock_workspace_tabs_width(tabs: &[DockWorkspaceTab]) -> usize {
    if tabs.is_empty() { return 0; }
    let mut w = 1;
    for (i, t) in tabs.iter().enumerate() {
        if i > 0 { w += WORKSPACE_PILL_GAP; }
        w += t.width;
    }
    w
}

fn pills_span(tabs: &[DockWorkspaceTab], from: usize, to: usize) -> usize {
    let mut w = 0;
    for (i, t) in tabs.iter().enumerate().take(to).skip(from) {
        if i > from { w += WORKSPACE_PILL_GAP; }
        w += t.width;
    }
    w
}

fn pills_fitting(tabs: &[DockWorkspaceTab], first: usize, width: usize) -> usize {
    let mut n = 0;
    for i in first..tabs.len() {
        if pills_span(tabs, first, i + 1) > width {
            break;
        }
        n += 1;
    }
    n
}

/// Decide what the strip draws in the room available.
pub fn plan_dock_workspace_strip(
    room: usize,
    bar_width: usize,
    tabs: &[DockWorkspaceTab],
) -> DockWorkspaceStrip {
    if tabs.is_empty() {
        return DockWorkspaceStrip::default();
    }

    let mut budget = dock_workspace_tabs_width(tabs).min(room);
    if budget > bar_width * 2 / 3 {
        budget = (bar_width / 2).min(room);
    }

    let mut strip = DockWorkspaceStrip {
        pills: tabs.to_vec(),
        ..Default::default()
    };

    if let Some(last) = tabs.last() {
        if last.add {
            strip.add = Some(last.clone());
            strip.pills = tabs[..tabs.len() - 1].to_vec();
        }
    }

    let add_span = if let Some(ref add) = strip.add {
        WORKSPACE_PILL_GAP + add.width
    } else {
        0
    };

    let add_only = |s: &mut DockWorkspaceStrip| {
        s.pills.clear();
        s.scrolls = false;
        s.width = 0;
        if let Some(ref add) = s.add {
            if budget > add.width {
                s.width = 1 + add.width;
            } else {
                s.add = None;
            }
        }
    };

    let avail = budget.saturating_sub(1 + add_span);
    if avail == 0 || strip.pills.is_empty() {
        add_only(&mut strip);
        return strip;
    }

    let natural = pills_span(&strip.pills, 0, strip.pills.len());
    if natural <= avail {
        strip.width = 1 + natural + add_span;
        return strip;
    }

    let inner = avail.saturating_sub(2 * WORKSPACE_ARROW_WIDTH);
    if inner < 1 {
        add_only(&mut strip);
        return strip;
    }
    strip.scrolls = true;
    strip.inner = inner;

    let first = 0;
    let count = pills_fitting(&strip.pills, first, inner);
    if count == 0 {
        add_only(&mut strip);
        return strip;
    }
    let total_pills = if strip.add.is_some() {
        tabs.len().saturating_sub(1)
    } else {
        tabs.len()
    };
    strip.pills = strip.pills[first..first + count].to_vec();
    strip.more_left = first > 0;
    strip.more_right = first + count < total_pills;
    strip.width = 1 + 2 * WORKSPACE_ARROW_WIDTH + inner + add_span;
    strip
}

/// Hit test for workspace pills.
pub fn dock_workspace_pill_at(
    os: &Os,
    x: i32,
    y: i32,
    hits: &[DockWorkspaceHit],
) -> Option<i32> {
    let _ = os;
    for h in hits {
        if y == h.y && x >= h.x0 && x < h.x1 {
            return Some(h.workspace);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Dock layout
// ---------------------------------------------------------------------------

/// Calculate the full dock layout.
pub fn calculate_dock_layout(os: &Os) -> DockLayout {
    let mut layout = DockLayout::default();

    let (mode_label, trail_text, left_width) = build_dock_left_text(os);
    layout.mode_label = mode_label;
    layout.trail_text = trail_text;
    layout.left_width = left_width;

    let bar_width = (os.width as usize).saturating_sub(dock_session_strip_width());

    let tabs = build_dock_workspace_tabs(os);
    let room = bar_width.saturating_sub(layout.left_width);
    layout.workspace_strip = plan_dock_workspace_strip(room, bar_width, &tabs);
    layout.left_width += layout.workspace_strip.width;

    let all_items = get_dock_items(os);
    layout.items = all_items.clone();

    let room = bar_width.saturating_sub(layout.left_width);
    let want = calculate_dock_right_width(os);
    layout.right_width = want.min(room);

    layout.calculate_item_positions(bar_width, &all_items);
    layout
}

impl DockLayout {
    fn calculate_item_positions(&mut self, screen_width: usize, all_items: &[DockItem]) {
        let total_items_width = dock_items_width(all_items);
        let available = screen_width
            .saturating_sub(self.left_width)
            .saturating_sub(self.right_width)
            .saturating_sub(total_items_width);

        if available > 0 {
            self.visible_items = all_items.to_vec();
            self.truncated_count = 0;
        } else {
            self.truncate_items(screen_width, all_items);
        }

        let visible_width = dock_items_width(&self.visible_items);
        let center_room = screen_width
            .saturating_sub(self.left_width)
            .saturating_sub(self.right_width);
        let start = self.left_width as i32 + (center_room.saturating_sub(visible_width) / 2) as i32;
        self.item_positions.clear();
        let mut x = start;
        for item in &self.visible_items {
            self.item_positions.push(x);
            x += item.width as i32 + 1;
        }
    }

    fn truncate_items(&mut self, screen_width: usize, all_items: &[DockItem]) {
        let max_items_width = screen_width
            .saturating_sub(self.left_width)
            .saturating_sub(self.right_width)
            .saturating_sub(TRUNCATION_INDICATOR_WIDTH)
            .saturating_sub(4);

        let mut current_width = 0;
        let mut visible_count = 0;

        for (i, item) in all_items.iter().enumerate() {
            let item_width = if i > 0 { item.width + 1 } else { item.width };
            if current_width + item_width <= max_items_width {
                current_width += item_width;
                visible_count += 1;
            } else {
                break;
            }
        }

        if visible_count > 0 {
            self.visible_items = all_items[..visible_count].to_vec();
        } else {
            self.visible_items.clear();
        }
        self.truncated_count = all_items.len().saturating_sub(visible_count);
    }
}

/// Build the dock's left region.
pub fn build_dock_left_text(os: &Os) -> (String, String, usize) {
    let mode_label = dock_mode_label(os);
    let trail_text = format!(
        " {}:{} ",
        os.current_workspace,
        workspace_window_count(os, os.current_workspace)
    );

    let cap_left = dock_mode_cap_left();
    let cap_right = dock_mode_cap_right();
    let width = cap_left.chars().count()
        + mode_label.chars().count()
        + cap_right.chars().count()
        + trail_text.chars().count()
        + 4;

    (mode_label, trail_text, width)
}

fn dock_mode_label(os: &Os) -> String {
    let ascii = os.config.appearance.use_ascii_only;
    if os.sidebar.focused {
        return "SIDEBAR".into();
    }
    if os.hold_mode.active() {
        let icon = constants::dock_mode_icon_window(ascii);
        return format!("{icon} HOLD");
    }
    match os.mode {
        crate::app::Mode::Terminal => {
            if os.scrollback_mode {
                format!(" {}:{} ", os.copy_cursor_line, os.copy_cursor_col)
            } else {
                constants::dock_mode_icon_window(ascii).to_string()
            }
        }
        crate::app::Mode::WindowManagement => {
            constants::dock_mode_icon_window(ascii).to_string()
        }
    }
}

fn dock_mode_cap_left() -> &'static str { "" }
fn dock_mode_cap_right() -> &'static str { "" }

fn calculate_dock_right_width(os: &Os) -> usize {
    if let Some(notif) = os.notifications.last() {
        return notif.message.chars().count() + 4;
    }
    if os.scrollback_mode {
        let tiers = copy_mode_help_tiers(copy_mode_state_from_os(os));
        if tiers.is_empty() { return 0; }
        return tiers[0].iter().map(|h| h.width() + 2).sum::<usize>() + 2;
    }
    0
}

fn dock_session_strip_width() -> usize { 0 }

// ---------------------------------------------------------------------------
// Dock items
// ---------------------------------------------------------------------------

/// The room every minimized entry needs laid out at once.
pub fn dock_items_width(items: &[DockItem]) -> usize {
    let mut w = 0;
    for (i, it) in items.iter().enumerate() {
        if i > 0 { w += 1; }
        w += it.width;
    }
    w
}

/// The text inside a dock pill.
pub fn dock_item_label(number: i32, name: &str) -> String {
    let name = printable_title(name);
    if !name.is_empty() {
        format!(" {}:{} ", number, truncate(&name, DOCK_ITEM_NAME_CELLS))
    } else {
        format!(" {} ", number)
    }
}

/// All dock items (minimized windows in the current workspace).
pub fn get_dock_items(os: &Os) -> Vec<DockItem> {
    let _ = os;
    Vec::new()
}

// ---------------------------------------------------------------------------
// Copy mode help tiers
// ---------------------------------------------------------------------------

/// The copy-mode sub-state for help-tier selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyModeState {
    /// Normal navigation.
    Normal,
    /// Search typing.
    Search,
    /// Visual char-wise selection.
    VisualChar,
    /// Visual line-wise selection.
    VisualLine,
}

/// Derive the copy-mode state from the Os.
pub fn copy_mode_state_from_os(os: &Os) -> CopyModeState {
    if os.copy_search_typing {
        return CopyModeState::Search;
    }
    if os.copy_visual {
        if os.copy_visual_line {
            return CopyModeState::VisualLine;
        }
        return CopyModeState::VisualChar;
    }
    CopyModeState::Normal
}

/// Returns the dock's copy-mode help for a sub-state, longest first.
pub fn copy_mode_help_tiers(state: CopyModeState) -> Vec<Vec<Hint>> {
    match state {
        CopyModeState::Normal => vec![
            vec![
                Hint::new("hjkl", "move"),
                Hint::new("w/b/e", "word"),
                Hint::new("f/t", "char"),
                Hint::new("/", "search"),
                Hint::new("n/N", "next"),
                Hint::new("v", "visual"),
                Hint::new("y", "yank"),
                Hint::new("q", "quit"),
            ],
            vec![
                Hint::new("hjkl", "move"),
                Hint::new("/", "search"),
                Hint::new("v", "visual"),
                Hint::new("y", "yank"),
                Hint::new("q", "quit"),
            ],
            vec![
                Hint::new("hjkl", "move"),
                Hint::new("y", "yank"),
                Hint::new("q", "quit"),
            ],
        ],
        CopyModeState::Search => vec![
            vec![
                Hint::new("type", "search"),
                Hint::new("n/N", "next"),
                Hint::new("enter", "done"),
                Hint::new("esc", "cancel"),
            ],
            vec![
                Hint::new("n/N", "next"),
                Hint::new("enter", "done"),
                Hint::new("esc", "cancel"),
            ],
        ],
        CopyModeState::VisualChar => vec![
            vec![
                Hint::new("hjkl", "extend"),
                Hint::new("w/b/e", "word"),
                Hint::new("%", "bracket"),
                Hint::new("y", "yank"),
                Hint::new("esc", "cancel"),
            ],
            vec![
                Hint::new("hjkl", "extend"),
                Hint::new("y", "yank"),
                Hint::new("esc", "cancel"),
            ],
        ],
        CopyModeState::VisualLine => vec![
            vec![
                Hint::new("jk", "extend"),
                Hint::new("y", "yank"),
                Hint::new("esc", "cancel"),
            ],
            vec![
                Hint::new("jk", "extend"),
                Hint::new("y", "yank"),
            ],
        ],
    }
}

// ---------------------------------------------------------------------------
// Session controls
// ---------------------------------------------------------------------------

/// Whether the dock is wide enough to carry the session controls.
pub fn dock_session_controls_fit(render_width: usize) -> bool {
    render_width >= DOCK_SESSION_ICON_MIN_WIDTH
}

fn use_ascii() -> bool { false }

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::userconfig::UserConfig;

    #[test]
    fn pill_width_single_digit() {
        assert_eq!(workspace_pill_width("1"), 5);
    }

    #[test]
    fn pill_width_named() {
        assert_eq!(workspace_pill_width("main"), 8);
    }

    #[test]
    fn pill_width_plus() {
        assert_eq!(workspace_pill_width("+"), 5);
    }

    #[test]
    fn dock_item_label_with_name() {
        assert_eq!(dock_item_label(1, "bash"), " 1:bash ");
    }

    #[test]
    fn dock_item_label_without_name() {
        assert_eq!(dock_item_label(3, ""), " 3 ");
    }

    #[test]
    fn dock_item_label_truncates_long_name() {
        let long = "abcdefghijklmnopqrstuvwxyz";
        let label = dock_item_label(1, long);
        assert!(label.starts_with(" 1:"));
        assert!(label.ends_with(' '));
        assert!(label.len() < long.len() + 6);
    }

    #[test]
    fn dock_items_width_empty() {
        assert_eq!(dock_items_width(&[]), 0);
    }

    #[test]
    fn dock_items_width_single() {
        let items = vec![DockItem {
            window_index: 0, label: " 1 ".into(), width: 5, minimized: true,
        }];
        assert_eq!(dock_items_width(&items), 5);
    }

    #[test]
    fn dock_items_width_multiple() {
        let items = vec![
            DockItem { window_index: 0, label: " 1 ".into(), width: 5, minimized: true },
            DockItem { window_index: 1, label: " 2 ".into(), width: 5, minimized: true },
        ];
        assert_eq!(dock_items_width(&items), 11);
    }

    #[test]
    fn copy_mode_help_normal_has_three_tiers() {
        let tiers = copy_mode_help_tiers(CopyModeState::Normal);
        assert_eq!(tiers.len(), 3);
        assert!(tiers[0].len() >= tiers[1].len());
        assert!(tiers[1].len() >= tiers[2].len());
    }

    #[test]
    fn copy_mode_help_search_has_two_tiers() {
        let tiers = copy_mode_help_tiers(CopyModeState::Search);
        assert_eq!(tiers.len(), 2);
    }

    #[test]
    fn copy_mode_help_visual_char_has_two_tiers() {
        let tiers = copy_mode_help_tiers(CopyModeState::VisualChar);
        assert_eq!(tiers.len(), 2);
    }

    #[test]
    fn copy_mode_help_visual_line_has_two_tiers() {
        let tiers = copy_mode_help_tiers(CopyModeState::VisualLine);
        assert_eq!(tiers.len(), 2);
    }

    #[test]
    fn plan_strip_empty_tabs() {
        let strip = plan_dock_workspace_strip(50, 80, &[]);
        assert_eq!(strip.width, 0);
        assert!(strip.pills.is_empty());
    }

    #[test]
    fn plan_strip_fits_naturally() {
        let tabs = vec![
            DockWorkspaceTab { workspace: 1, label: "1".into(), clipped: false, active: true, add: false, width: 3 },
            DockWorkspaceTab { workspace: 2, label: "2".into(), clipped: false, active: false, add: false, width: 3 },
        ];
        let strip = plan_dock_workspace_strip(50, 80, &tabs);
        assert_eq!(strip.pills.len(), 2);
        assert!(!strip.scrolls);
        assert_eq!(strip.width, 8);
    }

    #[test]
    fn plan_strip_with_add_tab() {
        let tabs = vec![
            DockWorkspaceTab { workspace: 1, label: "1".into(), clipped: false, active: true, add: false, width: 3 },
            DockWorkspaceTab { workspace: 0, label: "+".into(), clipped: false, active: false, add: true, width: 3 },
        ];
        let strip = plan_dock_workspace_strip(50, 80, &tabs);
        assert_eq!(strip.pills.len(), 1);
        assert!(strip.add.is_some());
        assert_eq!(strip.width, 8);
    }

    #[test]
    fn plan_strip_narrow_room_add_only() {
        let tabs = vec![
            DockWorkspaceTab { workspace: 1, label: "1".into(), clipped: false, active: true, add: false, width: 3 },
            DockWorkspaceTab { workspace: 0, label: "+".into(), clipped: false, active: false, add: true, width: 3 },
        ];
        let strip = plan_dock_workspace_strip(4, 80, &tabs);
        assert!(strip.pills.is_empty());
        assert!(strip.add.is_some());
        assert_eq!(strip.width, 4);
    }

    #[test]
    fn session_controls_fit_wide() {
        assert!(dock_session_controls_fit(40));
    }

    #[test]
    fn session_controls_fit_narrow() {
        assert!(!dock_session_controls_fit(20));
    }

    #[test]
    fn session_controls_fit_boundary() {
        assert!(dock_session_controls_fit(DOCK_SESSION_ICON_MIN_WIDTH));
        assert!(!dock_session_controls_fit(DOCK_SESSION_ICON_MIN_WIDTH - 1));
    }

    #[test]
    fn dock_workspace_pill_at_hit() {
        let hits = vec![
            DockWorkspaceHit { x0: 5, x1: 8, y: 23, workspace: 1 },
            DockWorkspaceHit { x0: 9, x1: 12, y: 23, workspace: 2 },
        ];
        let os = Os::new(UserConfig::default());
        assert_eq!(dock_workspace_pill_at(&os, 6, 23, &hits), Some(1));
        assert_eq!(dock_workspace_pill_at(&os, 10, 23, &hits), Some(2));
        assert_eq!(dock_workspace_pill_at(&os, 0, 23, &hits), None);
        assert_eq!(dock_workspace_pill_at(&os, 6, 22, &hits), None);
    }

    #[test]
    fn build_dock_workspace_tabs_disabled() {
        let mut config = UserConfig::default();
        config.appearance.dock_workspace_tabs = false;
        let os = Os::new(config);
        assert!(build_dock_workspace_tabs(&os).is_empty());
    }

    #[test]
    fn build_dock_workspace_tabs_single_workspace_with_add() {
        let os = Os::new(UserConfig::default());
        let tabs = build_dock_workspace_tabs(&os);
        assert_eq!(tabs.len(), 2);
        assert!(!tabs[0].add);
        assert!(tabs[1].add);
    }

    #[test]
    fn calculate_dock_layout_basic() {
        let os = Os::new(UserConfig::default());
        let layout = calculate_dock_layout(&os);
        assert!(layout.left_width > 0);
        assert!(layout.items.is_empty());
        assert!(layout.visible_items.is_empty());
    }

    #[test]
    fn build_dock_left_text_terminal_mode() {
        let os = Os::new(UserConfig::default());
        let (label, trail, width) = build_dock_left_text(&os);
        assert!(!label.is_empty());
        assert!(trail.starts_with(' '));
        assert!(width > 0);
    }

    #[test]
    fn pills_span_full() {
        let tabs = vec![
            DockWorkspaceTab { workspace: 1, label: "1".into(), clipped: false, active: true, add: false, width: 3 },
            DockWorkspaceTab { workspace: 2, label: "2".into(), clipped: false, active: false, add: false, width: 3 },
        ];
        assert_eq!(pills_span(&tabs, 0, 2), 7);
        assert_eq!(pills_span(&tabs, 0, 1), 3);
    }

    #[test]
    fn pills_fitting_count() {
        let tabs = vec![
            DockWorkspaceTab { workspace: 1, label: "1".into(), clipped: false, active: true, add: false, width: 3 },
            DockWorkspaceTab { workspace: 2, label: "2".into(), clipped: false, active: false, add: false, width: 3 },
        ];
        assert_eq!(pills_fitting(&tabs, 0, 7), 2);
        assert_eq!(pills_fitting(&tabs, 0, 5), 1);
        assert_eq!(pills_fitting(&tabs, 0, 2), 0);
    }

    #[test]
    fn copy_mode_state_normal() {
        let os = Os::new(UserConfig::default());
        assert_eq!(copy_mode_state_from_os(&os), CopyModeState::Normal);
    }

    #[test]
    fn copy_mode_state_search() {
        let mut os = Os::new(UserConfig::default());
        os.scrollback_mode = true;
        os.copy_search_typing = true;
        assert_eq!(copy_mode_state_from_os(&os), CopyModeState::Search);
    }

    #[test]
    fn copy_mode_state_visual_char() {
        let mut os = Os::new(UserConfig::default());
        os.scrollback_mode = true;
        os.copy_visual = true;
        os.copy_visual_line = false;
        assert_eq!(copy_mode_state_from_os(&os), CopyModeState::VisualChar);
    }

    #[test]
    fn copy_mode_state_visual_line() {
        let mut os = Os::new(UserConfig::default());
        os.scrollback_mode = true;
        os.copy_visual = true;
        os.copy_visual_line = true;
        assert_eq!(copy_mode_state_from_os(&os), CopyModeState::VisualLine);
    }
}
