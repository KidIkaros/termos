//! The window manager — the central application state. Ported from TUIOS
//! `internal/app` (the `OS` struct and its input/render layers).
//!
//! The `Os` struct owns the windows, workspaces, modes, and prefix state. It
//! is a plain state machine: the event loop feeds it input and it produces
//! render state, mirroring the Model-View-Update pattern the Go code gets from
//! Bubble Tea.

pub mod actions;
pub mod agent_alert;
pub mod border_grid;
pub mod clipboard;
pub mod damage;
pub mod copymode_ext;
pub mod dock;
pub mod dock_session_buttons;
pub mod effect;
pub mod float;
pub mod input;
pub mod interaction;
pub mod layout_templates;
pub mod msg;
pub mod overlay_hit;
pub mod overlay_mouse;
pub mod pixel_canvas;
pub mod render;
pub mod sidebar;
pub mod update;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::config::userconfig::UserConfig;
use crate::config::Theme;
use crate::hooks;
use crate::layout::{AutoScheme, BSPTree, PreselectionDir, Rect, SerializedBSPTree, SplitType};
use crate::session::model::WindowInfo;
use crate::session::protocol::Message;
use crate::terminal::pty::{PtySink, WinSize};
use crate::terminal::window::{SpawnOptions, Window};
use crate::vt::Emulator;

/// The interaction mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Window management: keys control panes and workspaces.
    WindowManagement,
    /// Terminal: keys pass through to the focused shell.
    Terminal,
}

/// Which prefix submenu is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prefix {
    None,
    /// The leader key (Ctrl+B) was pressed.
    Leader,
    /// Leader, then `w` — workspace sub-prefix.
    Workspace,
    /// Leader, then `t` — window sub-prefix.
    Window,
    /// Leader, then `m` — minimize sub-prefix.
    Minimize,
    /// Leader, then `T` — tape prefix (record/manager).
    Tape,
    /// Leader, then `D` — debug prefix (logs, stats, animations, showkeys).
    Debug,
    /// Leader, then `F` — floating-pane prefix (float, spawn, move, resize).
    Float,
}

/// A command the command palette can run. Ported from the TUIOS command list,
/// adapted to the local single-process architecture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    NewWindow,
    CloseWindow,
    SplitHorizontal,
    SplitVertical,
    NextWindow,
    PrevWindow,
    ToggleTiling,
    EqualizeSplits,
    Scrollback,
    SwitchWorkspace(i32),
    Quit,
    Theme,
    ThemeDetect,
    CommandPane,
    Settings,
    // Extended commands for category-aware palette.
    FocusLeft,
    FocusRight,
    FocusUp,
    FocusDown,
    SwapLeft,
    SwapRight,
    SwapUp,
    SwapDown,
    ZoomToggle,
    ToggleFloat,
    FloatNew,
    RenameWindow,
    CopyMode,
    ToggleSidebar,
    OpenBrowser,
    OpenAggregate,
    CommandPalette,
    SessionSwitcher,
    WorkspaceSwitcher,
    LayoutSwitcher,
    CycleLayoutMode,
    TapeManager,
    AccentPicker,
    Fullscreen,
    Detach,
    Help,
    StackPane,
    CycleStack,
    MultiSelect,
    BulkClose,
    BulkStack,
    BulkBreak,
    /// A user-defined custom action (shell command from config).
    CustomAction(String),
}

impl Command {
    /// The full list of commands shown in the palette.
    pub fn all() -> Vec<Command> {
        let mut cmds = vec![
            Command::NewWindow,
            Command::CloseWindow,
            Command::SplitHorizontal,
            Command::SplitVertical,
            Command::NextWindow,
            Command::PrevWindow,
            Command::ToggleTiling,
            Command::EqualizeSplits,
            Command::Scrollback,
            Command::Settings,
            Command::FocusLeft,
            Command::FocusRight,
            Command::FocusUp,
            Command::FocusDown,
            Command::SwapLeft,
            Command::SwapRight,
            Command::SwapUp,
            Command::SwapDown,
            Command::ZoomToggle,
            Command::ToggleFloat,
            Command::FloatNew,
            Command::RenameWindow,
            Command::CopyMode,
            Command::ToggleSidebar,
            Command::OpenBrowser,
            Command::OpenAggregate,
            Command::CommandPalette,
            Command::SessionSwitcher,
            Command::WorkspaceSwitcher,
            Command::LayoutSwitcher,
            Command::CycleLayoutMode,
            Command::TapeManager,
            Command::AccentPicker,
            Command::Fullscreen,
            Command::Detach,
            Command::Help,
        ];
        for i in 1..=9 {
            cmds.push(Command::SwitchWorkspace(i));
        }
        cmds.push(Command::Quit);
        cmds.push(Command::Theme);
        cmds.push(Command::ThemeDetect);
        cmds.push(Command::CommandPane);
        cmds.push(Command::StackPane);
        cmds.push(Command::CycleStack);
        cmds.push(Command::MultiSelect);
        cmds.push(Command::BulkClose);
        cmds.push(Command::BulkStack);
        cmds.push(Command::BulkBreak);
        cmds
    }

    /// The human-readable label for the palette.
    pub fn label(&self) -> String {
        match self {
            Command::NewWindow => "New window".into(),
            Command::CloseWindow => "Close window".into(),
            Command::SplitHorizontal => "Split horizontal".into(),
            Command::SplitVertical => "Split vertical".into(),
            Command::NextWindow => "Next window".into(),
            Command::PrevWindow => "Previous window".into(),
            Command::ToggleTiling => "Toggle tiling".into(),
            Command::EqualizeSplits => "Equalize splits".into(),
            Command::Scrollback => "Scrollback mode".into(),
            Command::SwitchWorkspace(i) => format!("Switch to workspace {i}"),
            Command::Quit => "Quit".into(),
            Command::Theme => "Theme picker".into(),
            Command::ThemeDetect => "Re-detect light/dark theme".into(),
            Command::CommandPane => "New command pane…".into(),
            Command::Settings => "Settings".into(),
            Command::FocusLeft => "Focus left".into(),
            Command::FocusRight => "Focus right".into(),
            Command::FocusUp => "Focus up".into(),
            Command::FocusDown => "Focus down".into(),
            Command::SwapLeft => "Swap left".into(),
            Command::SwapRight => "Swap right".into(),
            Command::SwapUp => "Swap up".into(),
            Command::SwapDown => "Swap down".into(),
            Command::ZoomToggle => "Toggle zoom".into(),
            Command::ToggleFloat => "Float / tile focused window".into(),
            Command::FloatNew => "New floating window".into(),
            Command::RenameWindow => "Rename window".into(),
            Command::CopyMode => "Copy mode".into(),
            Command::ToggleSidebar => "Toggle sidebar".into(),
            Command::OpenBrowser => "Open scrollback browser".into(),
            Command::OpenAggregate => "Open aggregate view".into(),
            Command::CommandPalette => "Command palette".into(),
            Command::SessionSwitcher => "Session switcher".into(),
            Command::WorkspaceSwitcher => "Workspace switcher".into(),
            Command::LayoutSwitcher => "Layout switcher".into(),
            Command::CycleLayoutMode => "Cycle layout mode".into(),
            Command::TapeManager => "Tape manager".into(),
            Command::AccentPicker => "Accent picker".into(),
            Command::Fullscreen => "Toggle fullscreen".into(),
            Command::Detach => "Detach session".into(),
            Command::Help => "Help".into(),
            Command::StackPane => "Stack / unstack focused pane".into(),
            Command::CycleStack => "Cycle focus in stack".into(),
            Command::MultiSelect => "Toggle multi-select mode".into(),
            Command::BulkClose => "Close selected panes".into(),
            Command::BulkStack => "Stack selected panes".into(),
            Command::BulkBreak => "Break selected from stack".into(),
            Command::CustomAction(name) => name.clone(),
        }
    }

    /// The category for grouping in the palette.
    pub fn category(&self) -> &'static str {
        match self {
            Command::NewWindow | Command::CloseWindow | Command::SplitHorizontal
            | Command::SplitVertical | Command::ZoomToggle | Command::Fullscreen
            | Command::RenameWindow | Command::ToggleFloat | Command::FloatNew
            | Command::CommandPane => "Window",
            Command::NextWindow | Command::PrevWindow | Command::FocusLeft
            | Command::FocusRight | Command::FocusUp | Command::FocusDown
            | Command::SwapLeft | Command::SwapRight | Command::SwapUp
            | Command::SwapDown => "Navigation",
            Command::ToggleTiling | Command::EqualizeSplits | Command::CycleLayoutMode => "Layout",
            Command::Scrollback | Command::CopyMode | Command::OpenBrowser
            | Command::OpenAggregate => "View",
            Command::SwitchWorkspace(_) | Command::WorkspaceSwitcher => "Workspace",
            Command::Settings | Command::Theme | Command::ThemeDetect
            | Command::AccentPicker | Command::ToggleSidebar => "Settings",
            Command::StackPane | Command::CycleStack | Command::MultiSelect
            | Command::BulkClose | Command::BulkStack | Command::BulkBreak
            | Command::CustomAction(_) => "Custom",
            Command::CommandPalette | Command::SessionSwitcher | Command::LayoutSwitcher
            | Command::TapeManager | Command::Help => "Open",
            Command::Quit | Command::Detach => "Session",
        }
    }

    /// A short description shown in the palette as a dimmed hint.
    pub fn description(&self) -> &'static str {
        match self {
            Command::NewWindow => "Open a new terminal pane",
            Command::CloseWindow => "Close the focused pane",
            Command::SplitHorizontal => "Split horizontally (top/bottom)",
            Command::SplitVertical => "Split vertically (left/right)",
            Command::NextWindow => "Move focus to the next pane",
            Command::PrevWindow => "Move focus to the previous pane",
            Command::ToggleTiling => "Toggle between tiled and floating",
            Command::EqualizeSplits => "Reset all splits to equal size",
            Command::Scrollback => "Enter scrollback/copy mode",
            Command::SwitchWorkspace(_) => "Switch to a numbered workspace",
            Command::Quit => "Quit TermOS",
            Command::Theme => "Browse and apply themes",
            Command::ThemeDetect => "Auto-detect light/dark from terminal",
            Command::CommandPane => "Run a custom shell command",
            Command::Settings => "Open the settings overlay",
            Command::FocusLeft => "Move focus to the left pane",
            Command::FocusRight => "Move focus to the right pane",
            Command::FocusUp => "Move focus to the pane above",
            Command::FocusDown => "Move focus to the pane below",
            Command::SwapLeft => "Swap focused pane left",
            Command::SwapRight => "Swap focused pane right",
            Command::SwapUp => "Swap focused pane up",
            Command::SwapDown => "Swap focused pane down",
            Command::ZoomToggle => "Toggle zoom on focused pane",
            Command::ToggleFloat => "Toggle float/tile on focused pane",
            Command::FloatNew => "Create a new floating pane",
            Command::RenameWindow => "Rename the focused pane",
            Command::CopyMode => "Enter vim-style copy mode",
            Command::ToggleSidebar => "Toggle the sidebar panel",
            Command::OpenBrowser => "Open scrollback in a browser",
            Command::OpenAggregate => "View all panes combined",
            Command::CommandPalette => "Open the command palette",
            Command::SessionSwitcher => "Switch between sessions",
            Command::WorkspaceSwitcher => "Switch workspaces",
            Command::LayoutSwitcher => "Switch tiling layout",
            Command::CycleLayoutMode => "Cycle BSP → Master-Stack → Scrolling",
            Command::TapeManager => "Manage recorded tapes",
            Command::AccentPicker => "Pick an accent color",
            Command::Fullscreen => "Toggle fullscreen mode",
            Command::Detach => "Detach from the current session",
            Command::Help => "Show the help overlay",
            Command::StackPane => "Stack or unstack the focused pane",
            Command::CycleStack => "Cycle focus within a stack",
            Command::MultiSelect => "Toggle multi-select mode",
            Command::BulkClose => "Close all selected panes",
            Command::BulkStack => "Stack all selected panes",
            Command::BulkBreak => "Break selected panes from stack",
            Command::CustomAction(_) => "Run a custom shell command",
        }
    }

    /// Default keybinding for this command, if any.
    pub fn keybinding(&self) -> Option<&'static str> {
        match self {
            Command::NewWindow => Some("Ctrl+B c"),
            Command::CloseWindow => Some("Ctrl+B x"),
            Command::SplitHorizontal => Some("Ctrl+B -"),
            Command::SplitVertical => Some("Ctrl+B \\"),
            Command::NextWindow => Some("Ctrl+B l"),
            Command::PrevWindow => Some("Ctrl+B h"),
            Command::ToggleTiling => Some("Ctrl+B t"),
            Command::CycleLayoutMode => None,
            Command::ZoomToggle => Some("Ctrl+B z"),
            Command::Scrollback => Some("Ctrl+B ["),
            Command::Quit => Some("Ctrl+B q"),
            Command::Settings => Some("Ctrl+B ,"),
            Command::Help => Some("Ctrl+B ?"),
            Command::CommandPalette => Some("Ctrl+B P"),
            Command::CopyMode => Some("Ctrl+B ["),
            Command::ToggleSidebar => Some("Ctrl+B s"),
            _ => None,
        }
    }
}

/// Which switcher overlay is open. In local mode `S` lists windows; in daemon
/// mode `S` lists sessions (the real session switcher).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwitcherKind {
    /// List workspaces 1-9 (prefix `W`).
    Workspace,
    /// List every window across workspaces (prefix `S` in local mode).
    Window,
    /// List daemon sessions (prefix `S` in daemon mode).
    Session,
    /// List saved layout templates (prefix `L`).
    Layout,
}

/// A rectangular text selection anchored to a window's content lines and
/// columns (content coordinates, stable under scrolling).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub window: usize,
    pub anchor_line: usize,
    pub anchor_col: i32,
    pub cursor_line: usize,
    pub cursor_col: i32,
}

impl Selection {
    pub fn line_range(&self) -> (usize, usize) {
        if self.anchor_line <= self.cursor_line {
            (self.anchor_line, self.cursor_line)
        } else {
            (self.cursor_line, self.anchor_line)
        }
    }

    pub fn col_range(&self) -> (i32, i32) {
        if self.anchor_col <= self.cursor_col {
            (self.anchor_col, self.cursor_col)
        } else {
            (self.cursor_col, self.anchor_col)
        }
    }

    pub fn is_empty(&self) -> bool {
        self.anchor_line == self.cursor_line && self.anchor_col == self.cursor_col
    }
}

/// Pending mark operation in copy mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkOp {
    /// `m{letter}` — set a mark at the cursor position.
    Set,
    /// `'{letter}` — jump to the mark's line, first non-blank column.
    JumpLine,
    /// `` `{letter} `` — jump to the mark's exact line and column.
    JumpCol,
}

/// One row in a switcher overlay.
#[derive(Debug, Clone)]
pub struct SwitcherEntry {
    /// The primary label.
    pub label: String,
    /// A secondary detail line (e.g. focused window title).
    pub detail: String,
    /// The workspace this entry switches to.
    pub workspace: i32,
    /// The window to focus after switching, if any.
    pub window: Option<usize>,
    /// The session this entry switches to (session switcher only).
    pub session: Option<String>,
}

/// Case-insensitive fuzzy subsequence match: every character of `query` must
/// appear in `text` in order. An empty query matches everything.
pub fn matches_query(text: &str, query: &str) -> bool {
    fuzzy_match(text, query).is_some()
}

// ---------------------------------------------------------------------------
// Fuzzy scoring with match-position tracking
// ---------------------------------------------------------------------------

/// Result of a fuzzy match: a score (lower is better) and the set of character
/// indices in `text` that matched the query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuzzyMatch {
    /// Composite score — lower is better.  0 = perfect match.
    pub score: i64,
    /// Character indices in the (lowercased) text that matched.
    pub positions: Vec<usize>,
}

/// Score a single fuzzy-matching character pair.
///
/// Rewards (lower = better):
/// - Exact-case match:      -2
/// - Word-boundary match:   -4  (after separator or at start)
/// - Consecutive match:     -2  (adjacent to previous match)
/// - Start-of-string:       -2
///
/// Penalties (higher = worse):
/// - Base cost per char:     +1
fn fuzzy_char_score(
    text_chars: &[char],
    text_idx: usize,
    query_char: char,
    prev_text_idx: Option<usize>,
) -> i64 {
    let ch = text_chars.get(text_idx).copied().unwrap_or_default();
    let mut score: i64 = 1; // base cost

    // Exact case match bonus
    if ch == query_char {
        score -= 2;
    }

    // Word-boundary bonus: start of string or preceded by a non-alphanumeric
    if text_idx == 0 {
        score -= 2;
    } else if let Some(prev_idx) = prev_text_idx {
        // Adjacent to previous match — consecutive bonus
        if prev_idx + 1 == text_idx {
            score -= 2;
        }
    }
    // Check word boundary (previous char is a separator)
    if text_idx > 0 {
        let prev_ch = text_chars.get(text_idx - 1).copied().unwrap_or_default();
        if !prev_ch.is_alphanumeric() {
            score -= 4;
        }
    }

    score
}

/// Find the best fuzzy match of `query` in `text`, returning the score and
/// matched character positions.  Both are compared case-insensitively.
///
/// An empty query matches everything with score 0 and no positions.
pub fn fuzzy_match(text: &str, query: &str) -> Option<FuzzyMatch> {
    if query.is_empty() {
        return Some(FuzzyMatch {
            score: 0,
            positions: Vec::new(),
        });
    }

    let text_l: Vec<char> = text.to_lowercase().chars().collect();
    let query_l: Vec<char> = query.to_lowercase().chars().collect();
    let n = text_l.len();
    let m = query_l.len();

    if m > n {
        return None;
    }

    // Greedy rightmost match: find the last possible position for each query
    // character, then score from the left to prefer early matches.
    let mut right_positions = Vec::with_capacity(m);
    let mut ti = n;
    for qi in (0..m).rev() {
        ti = text_l[..ti].iter().rposition(|&c| c == query_l[qi])?;
        right_positions.push(ti);
    }
    right_positions.reverse();

    // Left-to-right greedy match within the rightmost bounds.
    // Prefer word-boundary positions over earlier non-boundary positions:
    // when a query char matches at multiple positions, pick the first one
    // that sits at a word boundary (start of string or after a separator)
    // rather than blindly taking the earliest.
    let mut positions = Vec::with_capacity(m);
    let mut ti = 0;
    for qi in 0..m {
        let bound = right_positions[qi];
        let slice = &text_l[ti..=bound];
        // Find all candidate positions for this query char.
        let mut earliest = None;
        let mut earliest_boundary = None;
        for (offset, &ch) in slice.iter().enumerate() {
            if ch == query_l[qi] {
                let abs = ti + offset;
                if earliest.is_none() {
                    earliest = Some(offset);
                }
                if earliest_boundary.is_none() {
                    let at_boundary = abs == 0
                        || !text_l.get(abs - 1).map(|c| c.is_alphanumeric()).unwrap_or(false);
                    if at_boundary {
                        earliest_boundary = Some(offset);
                    }
                }
                if earliest.is_some() && earliest_boundary.is_some() {
                    break;
                }
            }
        }
        // Prefer word-boundary match, fall back to earliest.
        let offset = earliest_boundary.or(earliest)?;
        positions.push(ti + offset);
        ti = ti + offset + 1;
    }

    // Score
    let mut score: i64 = 0;
    let mut prev: Option<usize> = None;
    for (qi, &ti) in positions.iter().enumerate() {
        score += fuzzy_char_score(&text_l, ti, query_l[qi], prev);
        prev = Some(ti);
    }

    Some(FuzzyMatch { score, positions })
}

/// Multi-token fuzzy match: split `query` on whitespace and require every
/// token to match independently.  Returns the combined score (sum of per-
/// token scores) and the merged set of matched positions.
///
/// A bonus is applied when every token matches a complete word (the last
/// matched character is at a word boundary), so `new win` prefers
/// `New window` over `Next window`.
pub fn fuzzy_match_tokens(text: &str, query: &str) -> Option<FuzzyMatch> {
    let tokens: Vec<&str> = query.split_whitespace().collect();
    if tokens.is_empty() {
        return Some(FuzzyMatch {
            score: 0,
            positions: Vec::new(),
        });
    }

    let text_chars: Vec<char> = text.chars().collect();
    let mut total_score: i64 = 0;
    let mut all_positions: Vec<usize> = Vec::new();
    let mut all_word_boundary = true;

    for token in &tokens {
        let m = fuzzy_match(text, token)?;
        total_score += m.score;

        // Check if the last matched character is at a word boundary
        // (end of string or followed by a non-alphanumeric char).
        if let Some(&last_pos) = m.positions.last() {
            let at_end = last_pos + 1 >= text_chars.len();
            let next_is_sep = text_chars
                .get(last_pos + 1)
                .map(|c| !c.is_alphanumeric())
                .unwrap_or(true);
            if !(at_end || next_is_sep) {
                all_word_boundary = false;
            }
        }
        all_positions.extend(m.positions);
    }

    // Bonus when every token matches a complete word.
    if all_word_boundary && tokens.len() > 1 {
        total_score -= (tokens.len() as i64) * 6;
    }

    all_positions.sort_unstable();
    all_positions.dedup();

    Some(FuzzyMatch {
        score: total_score,
        positions: all_positions,
    })
}

/// Rank how well `text` matches `query`, lower is better. `None` means no
/// match.  Delegates to `fuzzy_match_tokens` for scoring.
pub fn fuzzy_rank(text: &str, query: &str) -> Option<usize> {
    fuzzy_match_tokens(text, query).map(|m| m.score as usize)
}

/// One workspace (1-9), holding its own BSP tree.
#[derive(Debug)]
pub struct Workspace {
    pub number: i32,
    pub tree: BSPTree,
    /// Focused window index within this workspace.
    pub focused: Option<usize>,
    pub name: String,
}

impl Workspace {
    pub fn new(number: i32) -> Self {
        Self {
            number,
            tree: BSPTree::new(),
            focused: None,
            name: format!("Workspace {number}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct LayoutCacheKey {
    workspace: i32,
    bounds: Rect,
    gap: i32,
    tree: SerializedBSPTree,
}

/// The central window manager state.
pub struct Os {
    /// All windows, global across workspaces.
    pub windows: Vec<Window>,
    /// The focused window index (into `windows`), if any.
    pub focused_window: Option<usize>,
    /// The current mode.
    pub mode: Mode,
    /// The active prefix state.
    pub prefix: Prefix,
    /// Workspaces 1-9.
    pub workspaces: HashMap<i32, Workspace>,
    /// The current workspace (1-9).
    pub current_workspace: i32,
    /// The user config.
    pub config: UserConfig,
    /// The active theme.
    pub theme: Option<Theme>,
    /// Whether `theme = "auto"` is active (host-terminal light/dark detection).
    pub auto_theme: bool,
    /// Terminal dimensions in cells.
    pub width: i32,
    pub height: i32,
    /// Whether shared borders (tmux-style separators) are on.
    pub shared_borders: bool,
    /// Gap in cells between panes (0 when not shared).
    pub gap: i32,
    /// The auto-split scheme.
    pub auto_scheme: AutoScheme,
    /// The active layout mode (BSP, master-stack, or scrolling).
    pub layout_mode: crate::layout::LayoutMode,
    /// Master-stack master pane width ratio (0.3-0.7).
    pub master_ratio: f64,
    /// Scrolling layout state (niri-style columns).
    pub scrolling: crate::layout::ScrollingLayout,
    /// Pending preselection direction.
    pub preselection: PreselectionDir,
    /// Notifications to show in the dock.
    pub notifications: Vec<Notification>,
    /// Whether the app is quitting.
    pub quitting: bool,
    /// Whether a quit confirmation is being shown.
    pub show_quit_confirmation: bool,
    /// The open quit menu, if any.
    pub quit_menu: Option<QuitMenu>,
    /// Kill-and-quit: after the pending session kill, the client quits even
    /// when other sessions exist.
    pub quit_after_kill: bool,
    /// The open session-close confirmation: (session name, selected row).
    /// Cancel (0) is the default; Close (1) is the destructive row.
    pub session_close: Option<(String, usize)>,
    /// The visible tooltip: (text, x, y).
    pub tooltip: Option<(String, i32, i32)>,
    /// Whether the aggregate view (all windows across workspaces) is open.
    pub aggregate_open: bool,
    pub aggregate_selected: usize,
    /// The sidebar rail state.
    pub sidebar: sidebar::Sidebar,
    /// The structured scrollback browser overlay.
    pub browser_open: bool,
    pub browser_blocks: Vec<crate::scrollback::CommandBlock>,
    pub browser_selected: usize,
    pub browser_mode: crate::scrollback::BrowseMode,
    pub browser_scroll: usize,
    /// A hover awaiting the delay window: (text, position, since).
    pub tooltip_pending: Option<(String, (i32, i32), std::time::Instant)>,
    /// Command palette state.
    pub palette_open: bool,
    pub palette_query: String,
    pub palette_selected: usize,
    /// Recently-used palette commands (most recent last). Capped at 8.
    pub palette_recent: Vec<Command>,
    /// Switcher (workspace/window) state.
    pub switcher_open: bool,
    pub switcher_kind: SwitcherKind,
    pub switcher_query: String,
    pub switcher_selected: usize,
    /// Whether vim-like scrollback navigation is active.
    pub scrollback_mode: bool,
    /// Overlay hit geometry recorded each frame for mouse routing.
    pub overlay_hits: Vec<overlay_hit::OverlayPanelHit>,
    /// Overlay z-order stack (frontmost is last).
    pub overlay_z_order: Vec<String>,
    /// Overlay drag offsets (kind → (dx, dy)).
    pub overlay_offsets: overlay_hit::OverlayOffsets,
    /// In-progress overlay drag state.
    pub overlay_drag: overlay_hit::OverlayDragState,
    /// Copy-mode cursor position (content coordinates), used while
    /// `scrollback_mode` is active.
    pub copy_cursor_line: usize,
    pub copy_cursor_col: i32,
    /// Whether vim visual selection is active in copy mode.
    pub copy_visual: bool,
    /// Whether visual selection is line-wise (`V`) vs char-wise (`v`).
    pub copy_visual_line: bool,
    /// Pending char-search state: (char, forward, till, pending).
    /// When `pending` is true, the next key is the target char.
    pub copy_char_search: Option<(char, bool, bool)>,
    /// The last completed char search for `;`/`,` repeat.
    pub copy_last_char_search: Option<(char, bool, bool)>,
    /// The current regex search query (empty = no search).
    pub copy_search_query: String,
    /// Whether the search is forward (`true`) or backward (`false`).
    pub copy_search_forward: bool,
    /// Whether we're typing a search query (`/` or `?` was pressed).
    pub copy_search_typing: bool,
    /// Consolidated search state with match list for highlighting.
    pub copy_search_state: copymode_ext::SearchState,
    /// Count prefix state for vim-style {count}motion.
    pub copy_count: copymode_ext::CountState,
    /// Whether the previous copy-mode command was `g`, awaiting a second `g`.
    pub copy_pending_g: bool,
    /// Mark store for vim-style marks (m{letter} / '{letter}).
    pub copy_marks: copymode_ext::MarkStore,
    /// Register store for vim-style named registers ("{letter}y).
    pub copy_registers: copymode_ext::RegisterStore,
    /// Pending register prefix: when `Some(letter)`, the next yank goes to
    /// that register instead of the unnamed one.
    pub copy_pending_register: Option<char>,
    /// Pending mark operation: when `Some`, the next key sets a mark (`m` was
    /// pressed) or jumps to one (`'` or `` ` `` was pressed).
    pub copy_pending_mark: Option<MarkOp>,
    /// The active selection (keyboard visual or mouse drag), if any.
    pub selection: Option<Selection>,
    /// Whether a mouse drag selection is in progress.
    pub mouse_selecting: bool,
    /// Multi-pane select mode: Alt+click toggles panes, Alt+drag draws a
    /// selection rectangle.  Bulk ops act on this set.
    pub multi_select_mode: bool,
    pub selected_panes: std::collections::HashSet<usize>,
    /// The open right-click context menu, if any.
    pub context_menu: Option<ContextMenu>,
    /// The open rename dialog: (window index, current text).
    pub rename_dialog: Option<(usize, String)>,
    /// Text-input dialog for the "New command pane" palette command.
    /// The bool tracks the `start_suspended` toggle (toggled with `s`).
    pub command_pane_dialog: Option<(String, bool)>,
    /// The last yanked text (internal clipboard).
    pub clipboard: String,
    /// Mouse border-drag resize state: (window_id, edge, start_pos).
    pub drag_resize: Option<(i32, crate::layout::ResizeEdge, i32)>,
    /// Multi-click tracking: (last click time, last position, click count).
    pub last_click: Option<(std::time::Instant, (u16, u16), u8)>,
    /// Click-to-type: a clean left press in window-management mode arms this
    /// with the press cell; a release without a drag enters terminal mode
    /// (Go's ClickToTypePending). A drag cancels it and starts a selection.
    pub click_to_type: Option<(i32, i32)>,
    /// Whether the help modal overlay is open.
    pub help_open: bool,
    /// Whether the debug stats overlay is open (leader D, then `c`).
    pub debug_overlay_open: bool,
    /// Whether the debug log viewer is open (leader D, then `l`).
    pub log_viewer_open: bool,
    /// A ring of recent app events (actions + notifications) for the log
    /// viewer.
    pub event_log: Vec<String>,
    /// The last key chord pressed, for the showkeys overlay.
    pub last_key_chord: String,
    /// Whether the theme picker overlay is open.
    pub theme_picker_open: bool,
    /// The selected index in the theme picker.
    pub theme_picker_selected: usize,
    /// Whether the accent picker overlay is open.
    pub accent_picker_open: bool,
    /// The selected index in the accent picker.
    pub accent_picker_selected: usize,
    /// Whether the settings overlay is open and the selected row.
    pub settings_open: bool,
    pub settings_selected: usize,
    /// Whether to show the welcome overlay on first launch.
    pub show_welcome: bool,
    /// Whether the persistent key-hints bar is visible.
    pub hints_visible: bool,

    /// Cached list of available theme names.
    pub theme_list: Vec<String>,
    /// Available accent colors for the accent picker.
    pub accent_list: Vec<String>,
    /// The current daemon session name (Some = daemon/attach mode).
    pub remote_session: Option<String>,
    /// Cached session list for the session switcher.
    pub remote_sessions: Vec<crate::session::model::SessionInfo>,
    /// A session the event loop should switch to (set by the switcher).
    pub pending_switch: Option<String>,
    /// A session the event loop should kill (set by the switcher).
    pub pending_kill: Option<String>,
    /// The channel to the daemon's socket writer (Some = remote mode), used
    /// for window-lifecycle requests (`NewWindow`/`CloseWindow`).
    pub remote_commands: Option<crossbeam_channel::Sender<Message>>,
    /// A pending split direction to apply when the next remote window is
    /// announced (set by split keybindings in remote mode).
    pub pending_split: Option<SplitType>,
    /// Floating panes: terminal windows rendered above the tiled layout.
    pub floats: Vec<float::FloatPane>,
    /// An in-progress mouse drag on a floating pane (move or resize).
    pub float_drag: Option<float::FloatDragState>,
    /// Whether the next remote window announced should float instead of tile
    /// (set by the new-floating-shell key in remote mode).
    pub pending_float: bool,
    /// Saved layout templates: name → serialized BSP tree.
    pub layouts: HashMap<String, SerializedBSPTree>,
    /// Cached current-workspace layout keyed by tree and geometry state.
    layout_cache: Mutex<Option<(LayoutCacheKey, HashMap<i32, Rect>)>>,
    /// Lifecycle hooks, loaded from the `[hooks]` config section.
    pub hook_manager: hooks::Manager,
    /// Agent alerts parked in their settle window, keyed by window id.
    pending_agent_alerts: HashMap<String, agent_alert::PendingAgentAlert>,
    /// Anti-flicker holds for locally-detected OSC agent states, keyed by
    /// window id: (held state, when it was recorded).
    agent_state_holds:
        HashMap<String, (crate::session::agent_state::AgentState, std::time::Instant)>,
    /// Global audible-cue cooldown across every pane.
    sound_cue: agent_alert::SoundCue,
    /// Host-terminal sequences queued by alerts (OSC 9 / BEL), flushed by the
    /// event loop after each draw so they never interleave a frame.
    host_output: Vec<u8>,
    /// The last OSC 22 pointer shape sent to the host, so a hover over the
    /// same border region does not re-emit it every mouse event.
    pointer_shape: interaction::PointerShape,
    /// The last mouse position, for pointer-shape computation.
    last_mouse_pos: (i32, i32),
    /// Hold-mode state: a held key suppresses repeat spam until release.
    pub hold_mode: interaction::HoldMode,
    /// Frame timing statistics for the trace overlay.
    pub tick_stats: interaction::TickStats,
    /// Whether a state/input change requires a new frame. PTY output is also
    /// detected through each window's dirty flag.
    render_requested: bool,
    /// Damage rectangles for incremental compositor rendering.
    pub damage: crate::app::damage::DamageSet,
    /// Active window animations (minimize/restore/snap), keyed by window id.
    animations: HashMap<i32, crate::ui::animation::Animation>,
    /// Whether the client's pointer is a finger (per-session, set by the web
    /// server's touch detection). Drives mobile affordances.
    pub touch_client: bool,
    /// Read-only mode: input is not forwarded to PTYs.
    pub read_only: bool,
    /// Whether tape playback is active (`tape play`).
    pub script_mode: bool,
    /// Whether tape playback is paused (Ctrl+P).
    pub script_paused: bool,
    /// The active tape player, if any.
    pub script_player: Option<crate::tape::player::Player>,
    /// Sleep deadline armed by a Sleep/Wait command (None = not waiting).
    script_sleep_until: Option<std::time::Instant>,
    /// A pending WaitUntilRegex: (compiled pattern, deadline).
    script_wait_regex: Option<(regex::Regex, std::time::Instant)>,
    /// Expected window count after a NewWindow/Split in daemon mode; playback
    /// holds until the pane arrives or the deadline passes.
    script_await_windows: usize,
    script_await_deadline: Option<std::time::Instant>,
    /// Active tape recorder, if recording.
    pub recorder: Option<crate::tape::recorder::Recorder>,
    /// Tape manager overlay state.
    pub tape_manager_open: bool,
    pub tape_manager_query: String,
    pub tape_manager_selected: usize,
    /// Tape manager scroll offset (first visible row).
    pub tape_manager_scroll: usize,
    /// Tape manager mode (list, confirm-delete, naming).
    pub tape_manager_mode: TapeManagerMode,
    /// Buffer for naming a new tape.
    pub tape_manager_name_buffer: String,
    /// Pending delete confirmation path.
    pub tape_manager_delete_path: Option<std::path::PathBuf>,
    /// Cached tape file list + the query it was filtered for, to avoid
    /// re-scanning the filesystem on every render frame.
    pub tape_manager_cache: Option<(String, Vec<std::path::PathBuf>)>,
    /// Remote `tape exec` progress (current index, total), if one is running.
    pub remote_tape: Option<(usize, usize)>,
    /// A discovered project tape awaiting a trust decision (the review
    /// dialog). `content` is the exact hashed bytes to execute on approval.
    pub project_tape_pending: Option<ProjectTapePending>,
    /// Graphics passthrough: Kitty APC forwarding.
    pub kitty_passthrough: Option<crate::graphics::kitty::KittyPassthrough>,
    /// Graphics passthrough: Sixel forwarding.
    pub sixel_passthrough: Option<crate::graphics::sixel::SixelPassthrough>,
    /// Host terminal capabilities (probed at startup).
    pub graphics_caps: crate::graphics::capability::Capabilities,
    /// Reusable RGB background canvas; resized only when the terminal changes.
    pub pixel_canvas: Mutex<crate::app::pixel_canvas::PixelCanvas>,
    /// Terminal color capability tier (detected once at startup).
    pub color_capability: crate::app::pixel_canvas::ColorCapability,
    /// Optional asciline-backed compositor for palette-character rendering.
    #[cfg(feature = "asciline-compositor")]
    pub asciline_compositor: Mutex<crate::app::pixel_canvas::AscilineCompositor>,
    /// The last time an alert sound was played (for cooldown).
    pub last_sound_played: Option<std::time::Instant>,
    /// Cached audio player command (None = not probed yet, Some(None) = none found).
    pub sound_player: Option<Option<&'static str>>,
    /// Cached status widget output: widget name → rendered text.
    /// Shared with the background refresh thread.
    pub widget_cache: Arc<Mutex<HashMap<String, String>>>,
    /// When each widget was last refreshed: widget name → Instant.
    pub widget_last_run: HashMap<String, std::time::Instant>,
    /// Background thread handles for widget refresh, paired with widget names.
    widget_threads: Vec<(String, std::thread::JoinHandle<()>)>,
    /// Widgets currently being refreshed; prevents overlapping jobs when the
    /// configured interval is shorter than the command runtime.
    widget_inflight: std::collections::HashSet<String>,
}

/// A discovered `.tuios.tape` waiting on the trust review.
#[derive(Debug, Clone)]
pub struct ProjectTapePending {
    pub path: String,
    pub hash: String,
    pub content: Vec<u8>,
}

/// An action a context-menu row can run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextAction {
    NewWindow,
    SplitHorizontal,
    SplitVertical,
    CloseWindow,
    Rename,
    Zoom,
    Copy,
    Paste,
    Cancel,
}

impl ContextAction {
    /// The row label.
    pub fn label(&self) -> &'static str {
        match self {
            Self::NewWindow => "New window",
            Self::SplitHorizontal => "Split horizontal",
            Self::SplitVertical => "Split vertical",
            Self::CloseWindow => "Close window",
            Self::Rename => "Rename window",
            Self::Zoom => "Toggle zoom",
            Self::Copy => "Copy selection",
            Self::Paste => "Paste clipboard",
            Self::Cancel => "Cancel",
        }
    }
}

/// The open right-click context menu, anchored to the cell it was opened on.
#[derive(Debug, Clone)]
pub struct ContextMenu {
    pub x: i32,
    pub y: i32,
    pub selected: usize,
    pub items: Vec<ContextAction>,
}

impl ContextMenu {
    /// The standard item set, newest first.
    pub fn standard() -> Vec<ContextAction> {
        vec![
            ContextAction::NewWindow,
            ContextAction::SplitHorizontal,
            ContextAction::SplitVertical,
            ContextAction::CloseWindow,
            ContextAction::Rename,
            ContextAction::Zoom,
            ContextAction::Copy,
            ContextAction::Paste,
            ContextAction::Cancel,
        ]
    }
}

/// What a quit-menu row does when run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuitMenuKind {
    /// Quit this client; the session keeps running (daemon mode).
    Detach,
    /// Close the menu and open the session switcher.
    SwitchSession,
    /// Kill the current session and quit this client.
    KillAndQuit,
    /// Quit a standalone client (no session to keep).
    Standalone,
    /// Close the menu and do nothing.
    Cancel,
}

/// Tape manager overlay mode (mirrors the Go `TapeManagerMode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TapeManagerMode {
    /// List of tape files (default).
    #[default]
    List,
    /// Recording a new tape.
    Recording,
    /// Playing back a tape.
    Playing,
    /// Confirm deletion of the selected tape.
    ConfirmDelete,
    /// Enter a name for a new tape recording.
    Naming,
}

/// One row of the quit menu.
#[derive(Debug, Clone)]
pub struct QuitMenuItem {
    pub label: String,
    pub key: char,
    pub kind: QuitMenuKind,
    /// Destructive rows (kill) draw in the warn color when a pane is busy.
    pub warn: bool,
}

/// The open quit menu.
#[derive(Debug, Clone)]
pub struct QuitMenu {
    pub selected: usize,
    pub items: Vec<QuitMenuItem>,
}

/// Whether two x-ranges overlap.
fn cols_overlap(x1: i32, w1: i32, x2: i32, w2: i32) -> bool {
    x1 < x2 + w2 && x2 < x1 + w1
}

/// Whether two y-ranges overlap.
fn rows_overlap(y1: i32, h1: i32, y2: i32, h2: i32) -> bool {
    y1 < y2 + h2 && y2 < y1 + h1
}

/// Expand a rect by `margin` cells on all sides, clamped to non-negative.
fn expand_rect(r: Rect, margin: i32) -> Rect {
    Rect {
        x: (r.x - margin).max(0),
        y: (r.y - margin).max(0),
        w: (r.w + margin * 2).max(1),
        h: (r.h + margin * 2).max(1),
    }
}

/// A dock notification.
#[derive(Debug, Clone)]
pub struct Notification {
    pub message: String,
    pub kind: String,
}

impl Os {
    pub fn new(config: UserConfig) -> Self {
        let mut workspaces = HashMap::new();
        for i in 1..=9 {
            workspaces.insert(i, Workspace::new(i));
        }
        let auto_theme = config.appearance.theme == "auto";
        let theme = if config.appearance.theme.is_empty() {
            None
        } else if auto_theme {
            // Env-only resolution: no terminal I/O here (daemon/web/ssh
            // contexts have no host terminal). The TUI re-detects live via
            // `redetect_theme()` once raw mode is active.
            let mode = crate::util::theme_detect::detect_from_env();
            let name = crate::util::theme_detect::resolve_auto_theme_name(
                mode,
                &config.appearance.theme_auto_light,
                &config.appearance.theme_auto_dark,
            );
            Theme::built_in(&name)
        } else {
            Theme::built_in(&config.appearance.theme)
        };
        let shared_borders = config.appearance.shared_borders;
        let layout_mode = crate::layout::LayoutMode::from_config(&config.appearance.layout_mode);
        let master_ratio = config.appearance.master_ratio.clamp(0.3, 0.7);
        let hook_manager = hooks::Manager::new();
        hook_manager.load_from_config(&config.hooks);
        // `[notifications.agent] command` is shorthand for registering one
        // command under the after-agent-state hook (Go's factory.go).
        let agent_command = config.notifications.agent.command.trim().to_string();
        if !agent_command.is_empty() {
            hook_manager.register(hooks::Event::AfterAgentState, agent_command);
        }
        Self {
            windows: Vec::new(),
            focused_window: None,
            mode: if config.startup.start_in_terminal_mode {
                Mode::Terminal
            } else {
                Mode::WindowManagement
            },
            prefix: Prefix::None,
            workspaces,
            current_workspace: 1,
            config,
            theme,
            auto_theme,
            width: 80,
            height: 24,
            shared_borders,
            gap: if shared_borders { 1 } else { 0 },
            auto_scheme: AutoScheme::Spiral,
            layout_mode,
            master_ratio,
            scrolling: crate::layout::ScrollingLayout::new(),
            preselection: PreselectionDir::None,
            notifications: Vec::new(),
            quitting: false,
            show_quit_confirmation: false,
            quit_menu: None,
            quit_after_kill: false,
            session_close: None,
            tooltip: None,
            aggregate_open: false,
            aggregate_selected: 0,
            sidebar: sidebar::Sidebar::new(),
            browser_open: false,
            browser_blocks: Vec::new(),
            browser_selected: 0,
            browser_mode: crate::scrollback::BrowseMode::Commands,
            browser_scroll: 0,
            tooltip_pending: None,
            palette_open: false,
            palette_query: String::new(),
            palette_selected: 0,
            palette_recent: Vec::new(),
            switcher_open: false,
            switcher_kind: SwitcherKind::Workspace,
            switcher_query: String::new(),
            switcher_selected: 0,
            scrollback_mode: false,
            overlay_hits: Vec::new(),
            overlay_z_order: Vec::new(),
            overlay_offsets: Vec::new(),
            overlay_drag: overlay_hit::OverlayDragState::default(),
            copy_cursor_line: 0,
            copy_cursor_col: 0,
            copy_visual: false,
            copy_visual_line: false,
            copy_char_search: None,
            copy_last_char_search: None,
            copy_search_query: String::new(),
            copy_search_forward: true,
            copy_search_typing: false,
            copy_search_state: copymode_ext::SearchState::new(),
            copy_count: copymode_ext::CountState::new(),
            copy_pending_g: false,
            copy_marks: copymode_ext::MarkStore::new(),
            copy_registers: copymode_ext::RegisterStore::new(),
            copy_pending_register: None,
            copy_pending_mark: None,
            selection: None,
            mouse_selecting: false,
            multi_select_mode: false,
            selected_panes: std::collections::HashSet::new(),
            context_menu: None,
            rename_dialog: None,
            command_pane_dialog: None,
            clipboard: String::new(),
            drag_resize: None,
            last_click: None,
            click_to_type: None,
            help_open: false,
            last_key_chord: String::new(),
            debug_overlay_open: false,
            log_viewer_open: false,
            event_log: Vec::new(),
            theme_picker_open: false,
            theme_picker_selected: 0,
            accent_picker_open: false,
            accent_picker_selected: 0,
            settings_open: false,
            settings_selected: 0,
            show_welcome: false,
            hints_visible: true,
            theme_list: Vec::new(),
            accent_list: vec![
                "blue".into(),
                "cyan".into(),
                "green".into(),
                "magenta".into(),
                "orange".into(),
                "purple".into(),
                "red".into(),
                "yellow".into(),
            ],
            remote_session: None,
            remote_sessions: Vec::new(),
            pending_switch: None,
            pending_kill: None,
            remote_commands: None,
            pending_split: None,
            floats: Vec::new(),
            float_drag: None,
            pending_float: false,
            layouts: HashMap::new(),
            layout_cache: Mutex::new(None),
            hook_manager,
            pending_agent_alerts: HashMap::new(),
            agent_state_holds: HashMap::new(),
            sound_cue: agent_alert::SoundCue::new(),
            host_output: Vec::new(),
            pointer_shape: interaction::PointerShape::Default,
            last_mouse_pos: (0, 0),
            hold_mode: interaction::HoldMode::new(),
            tick_stats: interaction::TickStats::new(),
            render_requested: true,
            damage: crate::app::damage::DamageSet::new(Rect { x: 0, y: 0, w: 0, h: 0 }),
            animations: HashMap::new(),
            touch_client: false,
            read_only: false,
            script_mode: false,
            script_paused: false,
            script_player: None,
            script_sleep_until: None,
            script_wait_regex: None,
            script_await_windows: 0,
            script_await_deadline: None,
            recorder: None,
            tape_manager_open: false,
            tape_manager_query: String::new(),
            tape_manager_selected: 0,
            tape_manager_scroll: 0,
            tape_manager_mode: TapeManagerMode::List,
            tape_manager_name_buffer: String::new(),
            tape_manager_delete_path: None,
            tape_manager_cache: None,
            remote_tape: None,
            project_tape_pending: None,
            kitty_passthrough: None,
            sixel_passthrough: None,
            graphics_caps: crate::graphics::capability::Capabilities::default(),
            pixel_canvas: Mutex::new(crate::app::pixel_canvas::PixelCanvas::new(1, 1)),
            color_capability: crate::app::pixel_canvas::ColorCapability::detect(),
            #[cfg(feature = "asciline-compositor")]
            asciline_compositor: Mutex::new(crate::app::pixel_canvas::AscilineCompositor::new(1, 1)),
            last_sound_played: None,
            sound_player: None,
            widget_cache: Arc::new(Mutex::new(HashMap::new())),
            widget_last_run: HashMap::new(),
            widget_threads: Vec::new(),
            widget_inflight: std::collections::HashSet::new(),
        }
    }

    /// Request a frame after an input, state, or configuration change.
    pub fn request_render(&mut self) {
        self.render_requested = true;
    }

    /// Mark the compositor's full bounds as dirty (theme, resize, workspace).
    pub fn damage_full(&mut self, reason: crate::app::damage::DamageReason) {
        self.damage.mark_full(reason);
        self.request_render();
    }

    /// Mark a specific rectangle as dirty (pane output, float movement, overlay).
    pub fn damage_rect(&mut self, rect: Rect, reason: crate::app::damage::DamageReason) {
        self.damage.mark(rect, reason);
        self.request_render();
    }

    /// Update the damage set's bounds (called on resize).
    pub fn damage_resize(&mut self, width: i32, height: i32) {
        self.damage = crate::app::damage::DamageSet::new(Rect { x: 0, y: 0, w: width, h: height });
        self.damage_full(crate::app::damage::DamageReason::Resize);
    }

    /// Drain pending damage for a frame.
    pub fn damage_take(&mut self) -> Vec<crate::app::damage::DamageRect> {
        self.damage.take()
    }

    /// Walk dirty windows and mark their pane rects as output damage.
    /// Call this right before `render()` so the compositor knows which
    /// regions have new content.
    pub fn collect_pane_damage(&mut self) {
        let dirty_indices: Vec<usize> = self
            .windows
            .iter()
            .enumerate()
            .filter(|(_, w)| w.is_dirty())
            .map(|(i, _)| i)
            .collect();
        let layout = self.current_layout();
        let mut rects: Vec<Rect> = Vec::with_capacity(dirty_indices.len());
        for idx in &dirty_indices {
            if let Some(rect) = layout.get(&(*idx as i32)) {
                rects.push(*rect);
            } else if let Some(fi) = self.float_for_window(*idx) {
                rects.push(self.floats[fi].rect());
            }
        }
        drop(layout);
        for rect in rects {
            self.damage_rect(rect, crate::app::damage::DamageReason::Output);
        }
    }

    /// Whether a frame is currently needed. PTY output and active animations
    /// bypass the explicit request flag.
    pub fn needs_render(&self) -> bool {
        self.render_requested
            || !self.animations.is_empty()
            || self.windows.iter().any(|w| w.is_dirty())
    }

    /// Mark the current state as composited into a terminal buffer.
    pub fn mark_rendered(&mut self) {
        self.render_requested = false;
    }

    // -----------------------------------------------------------------------
    // Hooks
    // -----------------------------------------------------------------------

    /// Build a hook context for a window index, filled with its id/title and
    /// the current workspace/session (the Go `FireHook` helper behavior).
    fn window_hook_ctx(&self, index: usize) -> hooks::Context {
        let mut ctx = hooks::Context::default();
        if let Some(w) = self.windows.get(index) {
            ctx.window_id = w.id.clone();
            ctx.window_name = w.title.clone();
        }
        ctx.workspace = self.current_workspace;
        ctx.session_id = self.remote_session.clone().unwrap_or_default();
        ctx
    }

    /// Fire a hook, auto-filling workspace and session when the context left
    /// them unset (Go's `FireHookContext` behavior, os_notify.go).
    pub fn fire_hook(&self, event: hooks::Event, mut ctx: hooks::Context) {
        if ctx.workspace == 0 {
            ctx.workspace = self.current_workspace;
        }
        if ctx.session_id.is_empty() {
            ctx.session_id = self.remote_session.clone().unwrap_or_default();
        }
        self.hook_manager.fire(event, ctx);
    }

    /// Fire the after-attach hook (client attach path).
    pub fn fire_attached(&self) {
        self.fire_hook(hooks::Event::AfterAttach, hooks::Context::default());
    }

    /// Fire the after-detach hook and drain in-flight hooks for up to 2s so
    /// they land before the client exits (Go's `FireDetached`).
    pub fn fire_detached(&self) {
        self.fire_hook(hooks::Event::AfterDetach, hooks::Context::default());
        self.hook_manager.wait_timeout(Duration::from_secs(2));
    }

    /// Fire the after-layout-change hook (once per mutation). The port
    /// currently only runs BSP tiling, so this fires with `bsp`; layout
    /// switches (master-stack/scrolling) will call it when they land.
    pub fn fire_layout_changed(&self) {
        let label = self.layout_mode.label().to_lowercase();
        self.fire_hook(
            hooks::Event::AfterLayoutChange,
            hooks::Context {
                layout: label,
                ..hooks::Context::default()
            },
        );
    }

    /// Cycle the layout mode: BSP → Master-Stack → Scrolling → BSP.
    pub fn cycle_layout_mode(&mut self) {
        self.layout_mode = self.layout_mode.next();
        self.damage_full(crate::app::damage::DamageReason::Resize);
        // Invalidate the layout cache so the new mode takes effect immediately.
        if let Ok(mut cache) = self.layout_cache.lock() {
            *cache = None;
        }
        // For scrolling mode, sync columns from the current workspace's BSP tree.
        if self.layout_mode == crate::layout::LayoutMode::Scrolling {
            self.sync_scrolling_from_workspace();
        }
        let label = self.layout_mode.label();
        self.notify(
            format!("Layout: {label}"),
            "info",
        );
        self.fire_layout_changed();
    }

    /// Populate the scrolling layout from the current workspace's window list.
    fn sync_scrolling_from_workspace(&mut self) {
        let ws = self.current_workspace;
        let ids = self.workspace_window_ids(ws);
        self.scrolling = crate::layout::ScrollingLayout::new();
        for &id in &ids {
            self.scrolling.add_column(id);
        }
    }

    // -----------------------------------------------------------------------
    // Agent state + alerts
    // -----------------------------------------------------------------------

    /// The index of the window with `window_id`, if any.
    pub fn window_index_by_id(&self, window_id: &str) -> Option<usize> {
        self.windows.iter().position(|w| w.id == window_id)
    }

    /// Rename a window by index (used by tape's RenameWindow).
    pub fn rename_window(&mut self, index: usize, name: &str) {
        if let Some(w) = self.windows.get_mut(index) {
            w.title = name.to_string();
        }
    }

    /// Move a window by index to another workspace, following it there (used
    /// by tape's MoveAndFollowWorkspace).
    pub fn move_window_to_workspace(&mut self, index: usize, number: i32) {
        if !(1..=9).contains(&number) {
            return;
        }
        // A floating window just moves its float entry (and re-clamps to the
        // target workspace's bounds).
        if let Some(fi) = self.float_for_window(index) {
            let from = self.floats[fi].workspace;
            if number == from {
                return;
            }
            self.floats[fi].workspace = number;
            let bounds = self.workspace_bounds(number);
            let r = float::clamp_rect(self.floats[fi].rect(), bounds);
            self.floats[fi].x = r.x;
            self.floats[fi].y = r.y;
            self.floats[fi].w = r.w;
            self.floats[fi].h = r.h;
            // Refocus the source workspace.
            let remaining = self.workspace(from).tree.get_all_window_ids();
            self.workspace_mut(from).focused = remaining.first().map(|&i| i as usize);
            if from == self.current_workspace {
                self.focused_window = remaining.first().map(|&i| i as usize);
            }
            self.current_workspace = number;
            self.workspace_mut(number).focused = Some(index);
            self.focused_window = Some(index);
            self.prefix = Prefix::None;
            return;
        }
        // Find the source workspace (the one whose tree owns the window).
        let mut from = self.current_workspace;
        for ws_num in 1..=9 {
            if self.workspace(ws_num).tree.has_window(index as i32) {
                from = ws_num;
                break;
            }
        }
        if number == from {
            return;
        }
        self.workspace_mut(from).tree.remove_window(index as i32);

        let bounds = self.workspace_bounds(number);
        let target_focused = self.workspace(number).focused;
        let gap = self.gap;
        let tree = &mut self.workspace_mut(number).tree;
        tree.insert_window(
            index as i32,
            target_focused.map(|f| f as i32).unwrap_or(-1),
            SplitType::None,
            0.5,
            bounds,
            gap,
        );

        // Refocus the source workspace.
        let remaining = self.workspace(from).tree.get_all_window_ids();
        self.workspace_mut(from).focused = remaining.first().map(|&i| i as usize);
        if from == self.current_workspace {
            self.focused_window = remaining.first().map(|&i| i as usize);
        }

        // Switch to the target.
        self.current_workspace = number;
        self.workspace_mut(number).focused = Some(index);
        self.focused_window = Some(index);
        self.prefix = Prefix::None;
    }

    /// Handle a daemon `AgentStateChanged` broadcast: update the window and
    /// run the alert policy on the transition.
    pub fn handle_agent_state_changed(
        &mut self,
        window_id: &str,
        state: &str,
        message: &str,
        harness: &str,
    ) {
        let Some(index) = self.window_index_by_id(window_id) else {
            return;
        };
        let from = self.windows[index].agent_state.clone();
        self.windows[index].agent_state = state.to_string();
        self.windows[index].agent_message = message.to_string();
        self.windows[index].agent_harness = harness.to_string();
        self.consider_agent_alert(window_id.to_string(), from, state.to_string());
    }

    /// Resolve the current `[notifications.agent]` policy. Resolved per call
    /// rather than cached: transitions are rare, the resolve is a few field
    /// reads, and a config reload is picked up with no extra wiring.
    fn agent_alert_policy(&self) -> agent_alert::AgentAlertPolicy {
        agent_alert::resolve_agent_alerts(&self.config.notifications.agent)
    }

    /// Decide what one transition earns. Any further transition retires
    /// whatever was parked for this pane: the state it was going to announce
    /// is no longer the state the pane is in (the whole anti-flicker rule).
    pub fn consider_agent_alert(&mut self, window_id: String, from: String, to: String) {
        let policy = self.agent_alert_policy();
        self.pending_agent_alerts.remove(&window_id);

        if !policy.alerts(&to) {
            return;
        }
        if policy.suppress_focused {
            if let Some(focused) = self.focused_window {
                if self
                    .windows
                    .get(focused)
                    .map(|w| w.id == window_id)
                    .unwrap_or(false)
                {
                    return;
                }
            }
        }
        if policy.quiet(local_minutes_since_midnight()) {
            return;
        }
        if policy.settle <= std::time::Duration::ZERO {
            self.fire_agent_alert(&window_id, &from, &to, &policy);
            return;
        }
        self.pending_agent_alerts.insert(
            window_id.clone(),
            agent_alert::PendingAgentAlert {
                window_id,
                from,
                to,
                due: std::time::Instant::now() + policy.settle,
            },
        );
    }

    /// Raise the parked alerts whose settle window has expired and whose pane
    /// is still in the state they were parked for. Called from the event-loop
    /// tick; cheap no-op when nothing is parked.
    /// Drain OSC 9;4 progress reports from each window's emulator and apply
    /// them to the window's agent state with the anti-flicker hold (Go's
    /// `agent_hold.go`). Fires the `AfterAgentState` hook on change.
    pub fn tick_agent_progress(&mut self) {
        const HOLD: std::time::Duration = std::time::Duration::from_millis(700);
        let now = std::time::Instant::now();
        let mut changed: Vec<(usize, String, String)> = Vec::new();
        for (i, w) in self.windows.iter_mut().enumerate() {
            let report = {
                let Ok(mut emu) = w.emulator.lock() else {
                    continue;
                };
                emu.take_pending_progress()
            };
            let Some((state, _percent)) = report else {
                continue;
            };
            let Some(next) = crate::session::osc_scan::agent_state_for_progress(state) else {
                continue;
            };
            let current = crate::session::agent_state::AgentState::parse(&w.agent_state)
                .unwrap_or(crate::session::agent_state::AgentState::None);
            if next == current {
                self.agent_state_holds.remove(&w.id);
                continue;
            }
            // Publish louder-or-equal transitions at once; hold quieter ones
            // (Go's `agentLoudness`: NeedsInput/Errored > Working > Idle/Done > None).
            let loudness = |s: &crate::session::agent_state::AgentState| match s {
                crate::session::agent_state::AgentState::NeedsInput
                | crate::session::agent_state::AgentState::Errored => 3,
                crate::session::agent_state::AgentState::Working => 2,
                crate::session::agent_state::AgentState::Idle
                | crate::session::agent_state::AgentState::Done => 1,
                crate::session::agent_state::AgentState::None => 0,
            };
            let publish = if loudness(&next) >= loudness(&current) {
                self.agent_state_holds.remove(&w.id);
                true
            } else if let Some((held, since)) = self.agent_state_holds.get(&w.id) {
                if *held == next && now.duration_since(*since) >= HOLD {
                    self.agent_state_holds.remove(&w.id);
                    true
                } else {
                    false
                }
            } else {
                self.agent_state_holds.insert(w.id.clone(), (next, now));
                false
            };
            if publish {
                let from = w.agent_state.clone();
                w.agent_state = next.name().to_string();
                w.agent_message.clear();
                w.agent_harness = "osc".to_string();
                changed.push((i, from, next.name().to_string()));
            }
        }
        for (index, from, to) in changed {
            self.fire_hook(
                hooks::Event::AfterAgentState,
                hooks::Context {
                    window_id: self.windows[index].id.clone(),
                    agent_state: to.clone(),
                    prev_agent_state: from.clone(),
                    ..Default::default()
                },
            );
        }
    }

    /// Drain pending desktop notifications from all window emulators and fire
    /// system notifications (OSC 9/777/99).
    pub fn tick_notifications(&mut self) {
        let mut pending = Vec::new();
        for w in self.windows.iter_mut() {
            let notif = {
                let Ok(mut emu) = w.emulator.lock() else {
                    continue;
                };
                emu.take_pending_notification()
            };
            if let Some((title, body)) = notif {
                pending.push((title, body));
            }
        }
        for (title, body) in pending {
            fire_desktop_notification(&title, &body);
            self.notify(
                if title.is_empty() {
                    body
                } else {
                    format!("{title}: {body}")
                },
                "info",
            );
        }
    }

    pub fn tick_agent_alerts(&mut self) {
        if self.pending_agent_alerts.is_empty() {
            return;
        }
        let now = std::time::Instant::now();
        let policy = self.agent_alert_policy();
        let due: Vec<agent_alert::PendingAgentAlert> = self
            .pending_agent_alerts
            .values()
            .filter(|p| now >= p.due)
            .cloned()
            .collect();
        for p in due {
            self.pending_agent_alerts.remove(&p.window_id);
            // Re-validate rather than trust the parked state: the pane may
            // have closed, moved on, or been focused while it waited.
            let Some(index) = self.window_index_by_id(&p.window_id) else {
                continue;
            };
            if self.windows[index].agent_state != p.to {
                continue;
            }
            if policy.suppress_focused {
                if let Some(focused) = self.focused_window {
                    if self
                        .windows
                        .get(focused)
                        .map(|w| w.id == p.window_id)
                        .unwrap_or(false)
                    {
                        continue;
                    }
                }
            }
            self.fire_agent_alert(&p.window_id, &p.from, &p.to, &policy);
        }
    }

    /// Write the alert to every sink the policy leaves on: dock notification,
    /// host sequence (OSC 9 + optional BEL), audible cue, and the
    /// after-agent-state hook.
    fn fire_agent_alert(
        &mut self,
        window_id: &str,
        from: &str,
        to: &str,
        policy: &agent_alert::AgentAlertPolicy,
    ) {
        let Some(index) = self.window_index_by_id(window_id) else {
            return;
        };
        let name = if self.windows[index].title.is_empty() {
            "pane".to_string()
        } else {
            self.windows[index].title.clone()
        };
        let text = format!("{} {}", name, agent_transition_notice(to));

        if policy.dock {
            self.notify(&text, "agent");
        }
        let mut seq = Vec::new();
        if policy.notify {
            seq.extend_from_slice(format!("\x1b]9;{text}\x07").as_bytes());
        }
        if policy.plays_bell() {
            seq.push(0x07);
        }
        self.queue_host_sequence(seq);

        if policy.plays_audio() {
            let file = policy.cue_file(to);
            self.sound_cue.play(file, policy.sound_cooldown);
        }

        // Built-in alert sound cue (independent of user-supplied cue files).
        if policy.sound {
            let cue = if policy.attention_cue(to) {
                "needs-input"
            } else {
                "done"
            };
            self.play_alert_sound(cue);
        }

        self.fire_hook(
            hooks::Event::AfterAgentState,
            hooks::Context {
                window_id: window_id.to_string(),
                window_name: name,
                workspace: self.current_workspace,
                session_id: self.remote_session.clone().unwrap_or_default(),
                agent_state: to.to_string(),
                prev_agent_state: from.to_string(),
                agent_harness: self.windows[index].agent_harness.clone(),
                agent_message: self.windows[index].agent_message.clone(),
                ..hooks::Context::default()
            },
        );
    }

    /// Queue bytes to write to the host terminal (alert notifications, BEL).
    /// The event loop flushes them after each draw so they never interleave a
    /// frame.
    pub fn queue_host_sequence(&mut self, bytes: Vec<u8>) {
        if !bytes.is_empty() {
            self.host_output.extend_from_slice(&bytes);
        }
    }

    /// Begin a minimize animation for a window (from its current layout rect
    /// to the dock strip). No-op when animations are disabled or the window
    /// has no rect.
    pub fn begin_minimize(&mut self, window_id: i32) {
        if !self.config.appearance.animations_enabled {
            return;
        }
        let Some(rect) = self.current_layout().get(&window_id).copied() else {
            return;
        };
        let duration = crate::ui::animation::animation_duration();
        if let Some(anim) = crate::ui::animation::Animation::new_minimize(
            rect.x, rect.y, rect.w, rect.h, rect.x, rect.y, duration,
        ) {
            self.animations.insert(window_id, anim);
        }
    }

    /// Begin a restore animation for a window (from the dock strip to its
    /// layout rect).
    pub fn begin_restore(&mut self, window_id: i32) {
        if !self.config.appearance.animations_enabled {
            return;
        }
        let Some(rect) = self.current_layout().get(&window_id).copied() else {
            return;
        };
        let duration = crate::ui::animation::animation_duration();
        if let Some(anim) = crate::ui::animation::Animation::new_restore(
            rect.x, rect.y, rect.x, rect.y, rect.w, rect.h, duration,
        ) {
            self.animations.insert(window_id, anim);
        }
    }

    /// Begin a snap animation from the window's current layout rect to
    /// `target`.
    pub fn begin_snap(&mut self, window_id: i32, target: crate::layout::Rect) {
        if !self.config.appearance.animations_enabled {
            return;
        }
        let Some(rect) = self.current_layout().get(&window_id).copied() else {
            return;
        };
        let duration = crate::ui::animation::animation_duration();
        if let Some(anim) = crate::ui::animation::Animation::new_snap(
            rect.x, rect.y, rect.w, rect.h, target.x, target.y, target.w, target.h, duration,
        ) {
            self.animations.insert(window_id, anim);
        }
    }

    /// Resolve the hover target for a mouse position: the title bar of the
    /// Return the hover target text for the cursor position (title + agent
    /// state), or a dock pill tooltip if hovering the dock bar.
    pub fn hover_target_at(&self, x: i32, y: i32) -> Option<String> {
        // Dock bar hover: show window title for the pill under cursor.
        let dock_row = if self.config.appearance.dockbar_position == "top" {
            0
        } else {
            self.height - 1
        };
        if self.config.appearance.mouse_friendly
            && self.config.appearance.dockbar_position != "hidden"
            && y == dock_row
        {
            if let Some(idx) = self.dock_item_at(x, y) {
                let window = self.windows.get(idx)?;
                let mut text = window.title.clone();
                if !window.agent_state.is_empty() && window.agent_state != "none" {
                    text.push_str(&format!(" — {}", window.agent_state));
                }
                return Some(text);
            }
            return None;
        }
        // Pane title bar hover.
        let idx = self.window_at(x, y)?;
        let layout = self.current_layout();
        let rect = layout.get(&(idx as i32))?;
        if y != rect.y || x >= rect.x + rect.w {
            return None;
        }
        let window = self.windows.get(idx)?;
        let mut text = window.title.clone();
        if !window.agent_state.is_empty() && window.agent_state != "none" {
            text.push_str(&format!(" — {}", window.agent_state));
            if !window.agent_message.is_empty() {
                text.push_str(&format!(": {}", window.agent_message));
            }
        }
        Some(text)
    }

    /// Record a hover for the tooltip delay window. Called from mouse motion.
    pub fn arm_tooltip(&mut self, x: i32, y: i32) {
        if let Some(text) = self.hover_target_at(x, y) {
            let changed = self
                .tooltip_pending
                .as_ref()
                .map(|(t, pos, _)| *t != text || *pos != (x, y))
                .unwrap_or(true);
            if changed {
                self.tooltip_pending = Some((text, (x, y), std::time::Instant::now()));
            }
        } else {
            self.tooltip_pending = None;
            self.tooltip = None;
        }
    }

    /// Promote an armed hover to a visible tooltip once the delay elapses.
    /// Called from the maintenance tick.
    pub fn tick_tooltip(&mut self) {
        const DELAY: std::time::Duration = std::time::Duration::from_millis(350);
        if let Some((text, pos, since)) = self.tooltip_pending.take() {
            if since.elapsed() >= DELAY {
                self.tooltip = Some((text, pos.0 + 1, pos.1 + 1));
            } else {
                self.tooltip_pending = Some((text, pos, since));
            }
        }
    }

    /// Clear any tooltip state (mouse left the surface).
    pub fn clear_tooltip(&mut self) {
        self.tooltip = None;
        self.tooltip_pending = None;
    }

    /// Advance every active animation; finished ones are removed. Called from
    /// the maintenance tick.
    pub fn tick_animations(&mut self) {
        if self.animations.is_empty() {
            return;
        }
        self.animations.retain(|_, anim| anim.update().is_some());
    }

    /// The current interpolated position of a window's animation, if one is
    /// running. The returned rect is in content coordinates (clamped by the
    /// caller to the pane area).
    pub fn animation_position(&self, window_id: i32) -> Option<(i32, i32, i32, i32)> {
        let anim = self.animations.get(&window_id)?;
        let elapsed = anim.start_time.elapsed();
        let mut progress = elapsed.as_secs_f64() / anim.duration.as_secs_f64();
        if progress >= 1.0 {
            progress = 1.0;
        }
        let t = crate::ui::animation::ease_in_out_cubic(progress);
        Some((
            crate::ui::animation::interpolate(anim.start_x, anim.end_x, t),
            crate::ui::animation::interpolate(anim.start_y, anim.end_y, t),
            crate::ui::animation::interpolate(anim.start_width, anim.end_width, t),
            crate::ui::animation::interpolate(anim.start_height, anim.end_height, t),
        ))
    }

    /// Drain the queued host-terminal sequences.
    pub fn take_host_sequence(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.host_output)
    }

    /// Recompute the pointer shape for a mouse position and queue an OSC 22
    /// host sequence when it changes (hover over a border/corner).
    pub fn update_pointer_shape(&mut self, x: i32, y: i32) {
        self.last_mouse_pos = (x, y);
        let border_off = if self.config.appearance.border_style == "none" {
            0
        } else {
            1
        };
        let layout: Vec<(i32, crate::layout::Rect)> = self.current_layout().into_iter().collect();
        let shape = interaction::pointer_shape_at(x, y, &layout, border_off);
        if shape != self.pointer_shape {
            self.pointer_shape = shape;
            let mut buf = Vec::new();
            let mut current = interaction::PointerShape::Default;
            // Write the shape sequence into a scratch writer; track as sent.
            let _ = interaction::set_pointer_shape(&mut buf, shape, &mut current);
            self.queue_host_sequence(buf);
        }
    }

    /// Queue a one-time tip when the host terminal is a macOS terminal and
    /// the Option key behaves as Meta (Go's `mac_option_advice`).
    pub fn queue_mac_option_advice(&mut self) {
        let term_program = std::env::var("TERM_PROGRAM").unwrap_or_default();
        if let Some(advice) = interaction::mac_option_advice(&term_program) {
            self.notify(advice, "info");
        }
    }

    /// Emit the kitty keyboard-enhancement flags once, at startup, so the
    /// host reports key releases/repeats (needed by hold mode).
    pub fn queue_keyboard_enhancements(&mut self) {
        let flags = interaction::KeyboardEnhancements {
            disambiguate_escape: true,
            report_event_types: true,
            report_alternate_keys: true,
            report_all_keys_as_escapes: false,
            report_associated_text: false,
        };
        self.queue_host_sequence(flags.set_flags_sequence().into_bytes());
    }

    /// The focused window's agent state wire value ("" = none), for the dock
    /// indicator.
    pub fn focused_agent_state(&self) -> &str {
        self.focused_window
            .and_then(|i| self.windows.get(i))
            .map(|w| w.agent_state.as_str())
            .unwrap_or("")
    }

    /// The focused window's agent message, if any.
    pub fn focused_agent_message(&self) -> &str {
        self.focused_window
            .and_then(|i| self.windows.get(i))
            .map(|w| w.agent_message.as_str())
            .unwrap_or("")
    }

    // -----------------------------------------------------------------------
    // Tape playback
    // -----------------------------------------------------------------------

    /// The playback tick, called by the event loop each iteration (Go's
    /// script block in `update.go`). Blocks on pane readiness, WaitUntilRegex,
    /// and Sleep deadlines; executes the next command otherwise.
    pub fn tick_script(&mut self) {
        if !self.script_mode || self.script_paused {
            return;
        }
        // Pane readiness gate: a pane an earlier command asked for must have
        // turned up (or timed out) before the next command runs.
        if !self.script_pane_ready() {
            return;
        }
        // WaitUntilRegex blocking.
        if self.script_wait_regex.is_some() && !self.check_script_wait_regex() {
            return;
        }
        // Sleep blocking.
        if let Some(until) = self.script_sleep_until {
            if std::time::Instant::now() < until {
                return;
            }
            self.script_sleep_until = None;
        }

        // Decide what the current command does without holding the player
        // borrow across execution.
        let mut action: Option<crate::tape::command::Command> = None;
        let mut wait_regex: Option<crate::tape::command::Command> = None;
        {
            let Some(player) = self.script_player.as_mut() else {
                return;
            };
            if player.is_finished() {
                return;
            }
            let Some(next) = player.next_command().cloned() else {
                return;
            };
            match next.type_ {
                // Sleep and its Wait alias both just delay playback.
                crate::tape::command::CommandType::Sleep
                | crate::tape::command::CommandType::Wait
                    if next.delay > std::time::Duration::ZERO =>
                {
                    self.script_sleep_until = Some(std::time::Instant::now() + next.delay);
                    player.advance();
                }
                // Arm the wait; playback blocks above until it resolves.
                crate::tape::command::CommandType::WaitUntilRegex => {
                    wait_regex = Some(next);
                    player.advance();
                }
                _ => {
                    player.advance();
                    action = Some(next);
                }
            }
        }
        if let Some(cmd) = wait_regex {
            self.start_script_wait_regex(&cmd);
        }
        if let Some(cmd) = action {
            let mut ce = crate::tape::executor::CommandExecutor::new(self);
            if let Err(e) = ce.execute(&cmd) {
                self.notify(format!("tape: {e}"), "error");
            }
        }
    }

    /// Arm the pane-readiness gate after a NewWindow/Split so playback holds
    /// until the pane actually exists (matters in daemon mode, where the pane
    /// arrives on a later state push).
    fn await_new_window(&mut self) {
        self.script_await_windows = self.windows.len();
        self.script_await_deadline =
            Some(std::time::Instant::now() + std::time::Duration::from_secs(5));
    }

    /// Whether playback may dispatch its next command: false only while a pane
    /// an earlier command asked for has not turned up yet. The timeout is
    /// reported, not swallowed.
    fn script_pane_ready(&mut self) -> bool {
        if self.script_await_windows == 0 {
            return true;
        }
        if self.windows.len() >= self.script_await_windows {
            self.script_await_windows = 0;
            self.script_await_deadline = None;
            return true;
        }
        if let Some(deadline) = self.script_await_deadline {
            if std::time::Instant::now() < deadline {
                return false;
            }
        }
        self.script_await_windows = 0;
        self.script_await_deadline = None;
        self.notify(
            "Tape: the new pane never appeared; the rest of the tape will run in the current pane",
            "error",
        );
        true
    }

    /// Arm a WaitUntilRegex condition: Args[0] is the pattern, Args[1] the
    /// optional timeout in milliseconds (default 5000). A bad or missing
    /// pattern is reported and the wait is skipped.
    fn start_script_wait_regex(&mut self, cmd: &crate::tape::command::Command) {
        let Some(pattern) = cmd.args.first() else {
            self.notify("WaitUntilRegex: missing pattern", "error");
            return;
        };
        let re = match regex::Regex::new(pattern) {
            Ok(re) => re,
            Err(e) => {
                self.notify(format!("WaitUntilRegex: invalid pattern: {e}"), "error");
                return;
            }
        };
        let timeout_ms = cmd
            .args
            .get(1)
            .and_then(|s| s.parse::<u64>().ok())
            .filter(|&ms| ms > 0)
            .unwrap_or(5000);
        self.script_wait_regex = Some((
            re,
            std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms),
        ));
    }

    /// Whether a pending WaitUntilRegex condition is satisfied (match against
    /// the focused window's screen, or deadline passed with a warning).
    fn check_script_wait_regex(&mut self) -> bool {
        let Some((re, deadline)) = self.script_wait_regex.clone() else {
            return true;
        };
        let matched = self
            .focused_window
            .and_then(|i| self.windows.get(i))
            .and_then(|w| w.emulator.lock().ok())
            .map(|emu| re.is_match(&emu.render_text()))
            .unwrap_or(false);
        if matched {
            self.script_wait_regex = None;
            return true;
        }
        if std::time::Instant::now() >= deadline {
            self.notify("WaitUntilRegex: timed out", "warning");
            self.script_wait_regex = None;
            return true;
        }
        false
    }

    /// True while a tape is playing (for the dock indicator).
    pub fn script_active(&self) -> bool {
        self.script_mode
            && (self.remote_tape.is_some()
                || self
                    .script_player
                    .as_ref()
                    .map(|p| !p.is_finished())
                    .unwrap_or(false))
    }

    /// The current tape progress percentage, if playing (local player or
    /// remote `tape exec`).
    pub fn script_progress(&self) -> Option<usize> {
        if let Some((index, total)) = self.remote_tape {
            return Some(if total == 0 {
                100
            } else {
                index.saturating_mul(100).checked_div(total).unwrap_or(100)
            });
        }
        self.script_player.as_ref().map(|p| p.progress())
    }

    /// Handle one command from a remote `tape exec`.
    pub fn handle_remote_tape_command(
        &mut self,
        index: usize,
        total: usize,
        command: &crate::tape::command::Command,
    ) {
        self.script_mode = true;
        self.remote_tape = Some((index, total));
        let mut ce = crate::tape::executor::CommandExecutor::new(self);
        if let Err(e) = ce.execute(command) {
            self.notify(format!("tape: {e}"), "error");
        }
    }

    /// The remote tape finished.
    pub fn remote_tape_finished(&mut self) {
        self.remote_tape = None;
    }

    // -----------------------------------------------------------------------
    // Graphics passthrough
    // -----------------------------------------------------------------------

    /// Probe the host terminal and initialize graphics passthrough. The host
    /// output is stdout (the terminal TermOS is running inside).
    pub fn init_graphics(&mut self) {
        let caps = crate::graphics::capability::Capabilities::probe();
        self.graphics_caps = caps;
        // Export TERM_PROGRAM for guest processes based on graphics capabilities.
        let term_program = match caps.host {
            crate::graphics::capability::HostTerminal::Kitty
            | crate::graphics::capability::HostTerminal::Ghostty => "ghostty",
            crate::graphics::capability::HostTerminal::WezTerm => "WezTerm",
            _ => "TermOS",
        };
        std::env::set_var("TERMOS_TERM_PROGRAM", term_program);
        if caps.kitty {
            self.kitty_passthrough = Some(crate::graphics::kitty::KittyPassthrough::new(
                caps,
                Box::new(std::io::stdout()),
            ));
        }
        if caps.sixel {
            self.sixel_passthrough = Some(crate::graphics::sixel::SixelPassthrough::new(
                caps,
                Box::new(std::io::stdout()),
            ));
        }
    }

    /// Refresh status widgets whose interval has elapsed.
    ///
    /// Refresh commands run outside the UI thread. Completed jobs are joined
    /// opportunistically; unfinished jobs remain in the registry and never
    /// block a maintenance tick. At most one job per widget may be in flight.
    pub fn update_status_widgets(&mut self) {
        const MAX_WIDGET_WORKERS: usize = 4;
        if self.reap_widget_threads() {
            self.request_render();
        }
        let now = std::time::Instant::now();
        for widget in &self.config.status_widgets {
            if self.widget_inflight.len() >= MAX_WIDGET_WORKERS {
                break;
            }
            if widget.command.is_empty() || self.widget_inflight.contains(&widget.name) {
                continue;
            }
            let last = self.widget_last_run.get(&widget.name).copied()
                .unwrap_or(now - std::time::Duration::from_secs(86400));
            let interval = std::time::Duration::from_millis(widget.refresh_ms.max(1));
            if now.duration_since(last) < interval {
                continue;
            }
            let name = widget.name.clone();
            let cmd = widget.command.clone();
            let cache = Arc::clone(&self.widget_cache);
            let thread_name = name.clone();
            let handle = std::thread::spawn(move || {
                let text = Self::run_widget_command(&cmd);
                cache.lock().unwrap().insert(thread_name, text);
            });
            self.widget_threads.push((name.clone(), handle));
            self.widget_inflight.insert(name.clone());
            self.widget_last_run.insert(name, now);
        }
    }

    /// Run a widget command with a bounded lifetime so a broken external
    /// command cannot leave a worker alive forever.
    fn run_widget_command(command: &str) -> String {
        const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
        let mut child = match std::process::Command::new("sh")
            .arg("-c")
            .arg(command)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(_) => return "err".into(),
        };
        let deadline = std::time::Instant::now() + TIMEOUT;
        loop {
            match child.try_wait() {
                Ok(Some(_)) => {
                    return child
                        .wait_with_output()
                        .ok()
                        .map(|out| {
                            String::from_utf8_lossy(&out.stdout)
                                .lines()
                                .next()
                                .unwrap_or("")
                                .trim()
                                .to_string()
                        })
                        .unwrap_or_else(|| "err".into());
                }
                Ok(None) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return "timeout".into();
                }
                Err(_) => return "err".into(),
            }
        }
    }

    /// Join only workers that have already finished, never blocking the UI.
    fn reap_widget_threads(&mut self) -> bool {
        let threads = std::mem::take(&mut self.widget_threads);
        let mut pending = Vec::with_capacity(threads.len());
        let mut completed = false;
        for (name, handle) in threads {
            if handle.is_finished() {
                let _ = handle.join();
                self.widget_inflight.remove(&name);
                completed = true;
            } else {
                pending.push((name, handle));
            }
        }
        self.widget_threads = pending;
        completed
    }

    /// Explicitly wait for all pending widget refreshes. This is intended for
    /// shutdown and tests, not for the UI maintenance tick.
    pub fn flush_widget_threads(&mut self) {
        for (name, handle) in self.widget_threads.drain(..) {
            let _ = handle.join();
            self.widget_inflight.remove(&name);
        }
    }

    /// Execute a custom action asynchronously so a long-running command
    /// cannot freeze input or rendering. The child inherits the host stdio,
    /// matching the previous command behavior while returning immediately.
    pub fn run_custom_action(&mut self, name: &str) {
        let Some(action) = self
            .config
            .custom_actions
            .iter()
            .find(|a| a.name == name)
            .cloned()
        else {
            return;
        };
        let ctx = self.window_hook_ctx(self.focused_window.unwrap_or(0));
        let mut cmd = std::process::Command::new("sh");
        cmd.arg("-c").arg(&action.command);
        for (key, value) in ctx.env_pairs() {
            cmd.env(key, value);
        }
        match cmd.spawn() {
            Ok(_) => self.notify(format!("started custom action: {}", action.name), "info"),
            Err(e) => self.notify(format!("custom action failed to start: {e}"), "error"),
        }
    }

    /// them to the host terminal. Called once per render tick, before
    /// drawing, so images appear in the right pane.
    pub fn flush_graphics(&mut self) {
        if self.kitty_passthrough.is_none() && self.sixel_passthrough.is_none() {
            return;
        }
        // Precompute pane origins for the current workspace layout so we
        // don't borrow self while iterating windows.
        let origins = self.compute_pane_origins();

        let mut apc_jobs: Vec<(u32, u32, u32, Vec<u8>)> = Vec::new();
        let mut sixel_jobs: Vec<(u32, u32, Vec<u8>)> = Vec::new();
        for (i, w) in self.windows.iter_mut().enumerate() {
            let mut emu = w.emulator.lock().unwrap_or_else(|e| e.into_inner());
            let apcs = emu.drain_pending_apc();
            if !apcs.is_empty() {
                let (px, py) = origins.get(i).copied().unwrap_or((0, 0));
                for apc in apcs {
                    apc_jobs.push((i as u32, px, py, apc));
                }
            }
            let sixels = emu.drain_pending_sixel();
            if !sixels.is_empty() {
                let (px, py) = origins.get(i).copied().unwrap_or((0, 0));
                for s in sixels {
                    sixel_jobs.push((px, py, s));
                }
            }
        }
        for (wid, px, py, apc) in apc_jobs {
            if let Some(kp) = &self.kitty_passthrough {
                let payload = if apc.first() == Some(&b'G') {
                    String::from_utf8_lossy(&apc[1..]).into_owned()
                } else {
                    String::from_utf8_lossy(&apc).into_owned()
                };
                let _ = kp.forward(wid, px, py, &payload);
            }
        }
        for (px, py, s) in sixel_jobs {
            if let Some(sp) = &self.sixel_passthrough {
                let _ = sp.forward(px, py, &s);
            }
        }
    }

    /// Compute the (x, y) cell origin of each window's inner content area
    /// on the current workspace.
    fn compute_pane_origins(&self) -> Vec<(u32, u32)> {
        let ws = self.current_workspace;
        if !self.workspaces.contains_key(&ws) {
            return Vec::new();
        }
        let rects = self.current_layout();
        self.windows
            .iter()
            .enumerate()
            .map(|(i, _)| {
                rects
                    .get(&(i as i32))
                    .map(|r| ((r.x + 1) as u32, (r.y + 1) as u32))
                    .unwrap_or((0, 0))
            })
            .collect()
    }

    /// Clear all graphics for a window (on close or workspace switch).
    pub fn clear_window_graphics(&self, window_id: u32) {
        if let Some(kp) = &self.kitty_passthrough {
            kp.clear_window(window_id);
        }
    }

    /// Re-emit placement commands for all windows at their current pane
    /// positions. Called after a layout change (resize, move, workspace
    /// switch) so images follow their panes.
    ///
    /// This builds `WindowPositionInfo` for every window in the current
    /// workspace and delegates to the kitty and sixel passthrough refresh
    /// logic, which handles occlusion detection, clipping, alt-screen
    /// mismatch, and change detection.
    pub fn refresh_all_placements(&self) {
        if self.kitty_passthrough.is_none() && self.sixel_passthrough.is_none() {
            return;
        }
        let all_windows = self.compute_all_window_positions();
        if let Some(kp) = &self.kitty_passthrough {
            let _ = kp.refresh_all_placements(&all_windows);
        }
        if let Some(sp) = &self.sixel_passthrough {
            let cell_height = 20;
            let host_height = self.height;
            sp.refresh_placements(
                &|wid| all_windows.get(&wid).cloned(),
                cell_height,
                host_height,
            );
        }
    }

    /// Compute `WindowPositionInfo` for every window on the current workspace,
    /// keyed by window index (as u32). This is the geometry snapshot the
    /// placement refresh logic needs: absolute screen position, content
    /// offsets, scroll state, z-index, and visibility.
    fn compute_all_window_positions(
        &self,
    ) -> HashMap<u32, crate::graphics::placement::WindowPositionInfo> {
        let ws = self.current_workspace;
        let rects = self.current_layout();
        let mut result = HashMap::new();
        for (i, w) in self.windows.iter().enumerate() {
            let Some(rect) = rects.get(&(i as i32)) else {
                continue;
            };
            let border_offset = if w.tiled { 0 } else { 1 };
            let (scrollback_len, scroll_offset, is_alt) = {
                match w.emulator.try_lock() {
                    Ok(emu) => (
                        emu.scrollback_len() as i32,
                        w.scrollback_offset() as i32,
                        emu.is_alt_screen(),
                    ),
                    Err(_) => (0, w.scrollback_offset() as i32, false),
                }
            };
            result.insert(
                i as u32,
                crate::graphics::placement::WindowPositionInfo {
                    window_x: rect.x,
                    window_y: rect.y,
                    content_offset_x: border_offset,
                    content_offset_y: border_offset,
                    width: rect.w,
                    height: rect.h,
                    visible: true,
                    scrollback_len,
                    scroll_offset,
                    is_being_manipulated: false,
                    screen_width: self.width,
                    screen_height: self.height,
                    window_z: 1,
                    is_alt_screen: is_alt,
                },
            );
        }
        // Floating panes composite above the tiles: report their rects with a
        // higher z so in-pane images stay on top.
        for f in self.floats.iter().filter(|f| f.workspace == ws) {
            let Some(w) = self.windows.get(f.window) else {
                continue;
            };
            let (scrollback_len, scroll_offset, is_alt) = match w.emulator.try_lock() {
                Ok(emu) => (
                    emu.scrollback_len() as i32,
                    w.scrollback_offset() as i32,
                    emu.is_alt_screen(),
                ),
                Err(_) => (0, w.scrollback_offset() as i32, false),
            };
            result.insert(
                f.window as u32,
                crate::graphics::placement::WindowPositionInfo {
                    window_x: f.x,
                    window_y: f.y,
                    content_offset_x: 1,
                    content_offset_y: 1,
                    width: f.w,
                    height: f.h,
                    visible: true,
                    scrollback_len,
                    scroll_offset,
                    is_being_manipulated: false,
                    screen_width: self.width,
                    screen_height: self.height,
                    window_z: 10,
                    is_alt_screen: is_alt,
                },
            );
        }
        result
    }

    // -----------------------------------------------------------------------
    // Tape recording
    // -----------------------------------------------------------------------

    /// Start recording user interactions, capturing the initial state.
    pub fn start_recording(&mut self) {
        let mode = if self.mode == Mode::Terminal {
            "terminal"
        } else {
            "window"
        };
        let mut recorder = crate::tape::recorder::Recorder::new();
        recorder.start_with_state(mode, self.current_workspace, true);
        self.recorder = Some(recorder);
        self.notify("recording… (Ctrl+B T s to stop)", "info");
    }

    /// Stop recording, save the tape, and return its path.
    pub fn stop_recording(&mut self) -> Option<std::path::PathBuf> {
        let recorder = self.recorder.as_mut()?;
        recorder.stop();
        let count = recorder.command_count();
        let content = recorder.string("Recorded in TermOS");
        let name = format!("recording-{}", crate::tape::tapes::timestamp_stamp());
        match crate::tape::tapes::save_tape(&name, &content) {
            Ok(path) => {
                self.notify(
                    format!("saved {count} commands to {}", path.display()),
                    "info",
                );
                self.recorder = None;
                Some(path)
            }
            Err(e) => {
                self.notify(format!("failed to save tape: {e}"), "error");
                self.recorder = None;
                None
            }
        }
    }

    /// Record a terminal-mode key press (if recording).
    pub fn record_terminal_key(&mut self, key: &crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyModifiers};
        let Some(recorder) = self.recorder.as_mut() else {
            return;
        };
        if !recorder.is_recording() {
            return;
        }
        match key.code {
            KeyCode::Char(c)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                recorder.record_type(&c.to_string());
            }
            KeyCode::Enter => recorder.record_key("enter"),
            KeyCode::Backspace => recorder.record_key("backspace"),
            KeyCode::Tab => recorder.record_key("tab"),
            KeyCode::Esc => recorder.record_key("esc"),
            KeyCode::Delete => recorder.record_key("delete"),
            KeyCode::Up => recorder.record_key("up"),
            KeyCode::Down => recorder.record_key("down"),
            KeyCode::Left => recorder.record_key("left"),
            KeyCode::Right => recorder.record_key("right"),
            KeyCode::Home => recorder.record_key("home"),
            KeyCode::End => recorder.record_key("end"),
            KeyCode::PageUp => recorder.record_key("pageup"),
            KeyCode::PageDown => recorder.record_key("pagedown"),
            KeyCode::Char(c) => {
                let mut combo = String::new();
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    combo.push_str("ctrl+");
                }
                if key.modifiers.contains(KeyModifiers::ALT) {
                    combo.push_str("alt+");
                }
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    combo.push_str("shift+");
                }
                combo.push(c);
                recorder.record_key(&combo);
            }
            _ => {}
        }
    }

    /// Record a window-management action (if recording). Hooks in the Os
    /// lifecycle methods feed this.
    pub fn record_action(&mut self, action: &str, args: &[&str]) {
        if let Some(recorder) = self.recorder.as_mut() {
            recorder.record_action(action, args);
        }
    }

    /// Record a workspace switch (if recording).
    pub fn record_workspace_switch(&mut self, workspace: i32) {
        if let Some(recorder) = self.recorder.as_mut() {
            recorder.record_workspace_switch(workspace);
        }
    }

    /// True while a recording is active (for the dock indicator).
    pub fn recording_active(&self) -> bool {
        self.recorder
            .as_ref()
            .map(|r| r.is_recording())
            .unwrap_or(false)
    }

    /// Open the tape manager overlay.
    pub fn open_tape_manager(&mut self) {
        self.tape_manager_open = true;
        self.tape_manager_query.clear();
        self.tape_manager_selected = 0;
        self.tape_manager_scroll = 0;
        self.tape_manager_mode = TapeManagerMode::List;
        self.tape_manager_name_buffer.clear();
        self.tape_manager_delete_path = None;
        self.tape_manager_cache = None;
        self.update_tape_manager_cache();
        self.prefix = Prefix::None;
    }

    /// Number of tape files visible at once in the manager list.
    pub const TAPE_MANAGER_VISIBLE_ROWS: usize = 10;

    /// Clamp the scroll offset so the selected row stays visible and the
    /// offset never runs past the end of the list.
    pub fn clamp_tape_scroll(&mut self) {
        let items_len = self.tape_manager_items().len();
        let visible = Self::TAPE_MANAGER_VISIBLE_ROWS;
        if self.tape_manager_selected < self.tape_manager_scroll {
            self.tape_manager_scroll = self.tape_manager_selected;
        } else if self.tape_manager_selected >= self.tape_manager_scroll + visible {
            self.tape_manager_scroll = self.tape_manager_selected - visible + 1;
        }
        let max_offset = items_len.saturating_sub(visible);
        if self.tape_manager_scroll > max_offset {
            self.tape_manager_scroll = max_offset;
        }
    }

    /// Initiate deletion of the selected tape (enters confirm mode).
    pub fn tape_manager_delete(&mut self) {
        let items = self.tape_manager_items();
        if let Some(path) = items.get(self.tape_manager_selected).cloned() {
            self.tape_manager_delete_path = Some(path);
            self.tape_manager_mode = TapeManagerMode::ConfirmDelete;
        }
    }

    /// Confirm and execute deletion of the pending tape.
    pub fn tape_manager_confirm_delete(&mut self) {
        if let Some(path) = self.tape_manager_delete_path.take() {
            match std::fs::remove_file(&path) {
                Ok(_) => {
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    self.notify(format!("deleted '{name}'"), "info");
                }
                Err(e) => {
                    self.notify(format!("failed to delete: {e}"), "error");
                }
            }
        }
        self.tape_manager_mode = TapeManagerMode::List;
        // Repopulate the cache so the list reflects the deletion.
        self.update_tape_manager_cache();
        let items_len = self.tape_manager_items().len();
        if self.tape_manager_selected >= items_len && items_len > 0 {
            self.tape_manager_selected = items_len - 1;
        }
        self.clamp_tape_scroll();
    }

    /// Cancel deletion (return to list mode).
    pub fn tape_manager_cancel_delete(&mut self) {
        self.tape_manager_mode = TapeManagerMode::List;
        self.tape_manager_delete_path = None;
    }

    /// Enter naming mode for a new tape recording.
    pub fn tape_manager_start_naming(&mut self) {
        self.tape_manager_mode = TapeManagerMode::Naming;
        self.tape_manager_name_buffer.clear();
    }

    /// Confirm the name, start recording, and close the manager.
    pub fn tape_manager_confirm_name(&mut self) {
        let name = if self.tape_manager_name_buffer.trim().is_empty() {
            format!("recording-{}", crate::tape::tapes::timestamp_stamp())
        } else {
            self.tape_manager_name_buffer.trim().to_string()
        };
        self.start_recording_with_name(&name);
        self.tape_manager_open = false;
        self.tape_manager_mode = TapeManagerMode::List;
        self.tape_manager_name_buffer.clear();
    }

    /// Cancel naming mode (return to list mode).
    pub fn tape_manager_cancel_naming(&mut self) {
        self.tape_manager_mode = TapeManagerMode::List;
        self.tape_manager_name_buffer.clear();
    }

    /// Start recording with a specific tape name (used by naming mode).
    fn start_recording_with_name(&mut self, _name: &str) {
        self.start_recording();
    }

    /// The tape files for the manager overlay, filtered by the query.
    ///
    /// Uses an internal cache keyed by the current query string to avoid
    /// re-scanning the filesystem on every render frame. The cache is
    /// invalidated when the query changes or `refresh_tape_manager_cache`
    /// is called.
    pub fn tape_manager_items(&self) -> Vec<std::path::PathBuf> {
        let query = self.tape_manager_query.to_lowercase();
        if let Some((cached_query, ref cached_items)) = &self.tape_manager_cache {
            if cached_query == &query {
                return cached_items.clone();
            }
        }
        self.scan_tape_files(&query)
    }

    /// Invalidate the tape manager cache so the next `tape_manager_items`
    /// call re-scans the filesystem.
    pub fn refresh_tape_manager_cache(&mut self) {
        self.tape_manager_cache = None;
    }

    /// Update the tape manager cache after a query or file change.
    pub fn update_tape_manager_cache(&mut self) {
        let query = self.tape_manager_query.to_lowercase();
        let filtered = self.scan_tape_files(&query);
        self.tape_manager_cache = Some((query, filtered));
    }

    /// Scan the tape directory and filter by query. Shared by
    /// `tape_manager_items` (cache miss) and `update_tape_manager_cache`.
    fn scan_tape_files(&self, query: &str) -> Vec<std::path::PathBuf> {
        let Ok(files) = crate::tape::tapes::list_tapes() else {
            return Vec::new();
        };
        files
            .into_iter()
            .filter(|p| {
                query.is_empty()
                    || p.file_name()
                        .map(|n| n.to_string_lossy().to_lowercase().contains(query))
                        .unwrap_or(false)
            })
            .collect()
    }

    /// Play the selected tape from the manager (loads it as the script).
    pub fn play_selected_tape(&mut self) {
        let files = self.tape_manager_items();
        let Some(path) = files.get(self.tape_manager_selected) else {
            self.notify("no tape selected", "info");
            return;
        };
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                self.notify(format!("failed to read tape: {e}"), "error");
                return;
            }
        };
        self.start_script_from_content(&content);
        self.tape_manager_open = false;
    }

    /// Load parsed tape content as the active script (with error reporting).
    fn start_script_from_content(&mut self, content: &str) {
        let (commands, errors) = crate::tape::parser::parse_file(content);
        if !errors.is_empty() || commands.is_empty() {
            self.notify("tape failed to parse", "error");
            return;
        }
        self.script_mode = true;
        self.script_paused = false;
        self.script_player = Some(crate::tape::player::Player::new(commands));
        self.notify("tape started", "info");
    }

    /// Discover `.tuios.tape` in the current directory and start the trust
    /// review (`Ctrl+B T t`). Trusted tapes play immediately.
    pub fn review_project_tape(&mut self) {
        use crate::tape::trust::Status;
        let path = std::env::current_dir()
            .ok()
            .map(|d| d.join(crate::tape::trust::TAPE_FILE_NAME));
        let Some(path) = path else {
            self.notify("no project tape found", "info");
            return;
        };
        if !path.exists() {
            self.notify("no .tuios.tape in this directory", "info");
            return;
        }
        let path_str = path.to_string_lossy().into_owned();
        let Ok(store) = crate::tape::trust::Store::load() else {
            self.notify("cannot open the trust store", "error");
            return;
        };
        let Ok(result) = store.check(&path_str) else {
            self.notify("cannot read the project tape", "error");
            return;
        };
        match result.status {
            Status::Trusted => {
                let content = String::from_utf8_lossy(&result.content).into_owned();
                self.start_script_from_content(&content);
            }
            Status::Untrusted => {
                self.project_tape_pending = Some(ProjectTapePending {
                    path: result.path.clone(),
                    hash: result.hash.clone(),
                    content: result.content.clone(),
                });
            }
            Status::Denied => {
                self.notify("project tape is denied", "warning");
            }
            Status::Ineligible => {
                self.notify(
                    format!("project tape is ineligible: {}", result.reason),
                    "error",
                );
            }
        }
    }

    /// Resolve the pending trust review: `trust_it` trusts and plays the tape,
    /// `false` leaves it untrusted and clears the dialog.
    pub fn resolve_project_tape(&mut self, trust_it: bool) {
        let Some(pending) = self.project_tape_pending.take() else {
            return;
        };
        if !trust_it {
            self.notify("project tape not trusted", "info");
            return;
        }
        let mut store = match crate::tape::trust::Store::load() {
            Ok(s) => s,
            Err(e) => {
                self.notify(format!("cannot open the trust store: {e}"), "error");
                return;
            }
        };
        if let Err(e) = store.trust(&pending.path, &pending.hash) {
            self.notify(format!("cannot record trust: {e}"), "error");
            return;
        }
        let content = String::from_utf8_lossy(&pending.content).into_owned();
        self.start_script_from_content(&content);
    }

    // -----------------------------------------------------------------------
    // Workspace helpers
    // -----------------------------------------------------------------------

    pub fn workspace_mut(&mut self, number: i32) -> &mut Workspace {
        self.workspaces
            .entry(number)
            .or_insert_with(|| Workspace::new(number))
    }

    pub fn workspace(&self, number: i32) -> &Workspace {
        self.workspaces.get(&number).expect("workspace exists")
    }

    /// The windows on the current workspace, in layout order (BSP IDs).
    pub fn current_workspace_windows(&self) -> Vec<i32> {
        let ws = self.workspace(self.current_workspace);
        ws.tree.get_all_window_ids()
    }

    /// Window IDs on the given workspace, in BSP order.
    fn workspace_window_ids(&self, ws: i32) -> Vec<i32> {
        self.workspace(ws).tree.get_all_window_ids()
    }

    /// True if the given window index is on the current workspace (tiled or
    /// floating).
    pub fn window_on_current_workspace(&self, index: usize) -> bool {
        let ws = self.current_workspace;
        if self
            .workspace(ws)
            .tree
            .get_all_window_ids()
            .contains(&(index as i32))
        {
            return true;
        }
        self.float_for_window(index)
            .map(|fi| self.floats[fi].workspace == ws)
            .unwrap_or(false)
    }

    // -----------------------------------------------------------------------
    // Window creation
    // -----------------------------------------------------------------------

    /// Spawn a new shell window on the current workspace. If there is a
    /// focused window, it splits (BSP); otherwise the window becomes the root.
    pub fn spawn_window(
        &mut self,
        shell: &str,
        wake: Box<dyn Fn() + Send + 'static>,
    ) -> Result<usize, String> {
        let index = self.windows.len();
        let id = format!("win-{index}");
        let size = WinSize { cols: 80, rows: 24 };
        let env = crate::util::guestenv::base_guest_env("local", &id, false, false);
        let window = Window::spawn(
            id,
            "Terminal",
            size,
            shell,
            SpawnOptions::shell(),
            wake,
            &env,
        )
        .map_err(|e| e.to_string())?;
        self.push_window(window, index);
        Ok(index)
    }

    /// Insert a freshly spawned window into the current workspace's BSP tree
    /// and focus it.
    fn push_window(&mut self, window: Window, index: usize) {
        self.windows.push(window);
        let ws = self.current_workspace;
        let bounds = self.workspace_bounds(ws);
        let focused = self.workspace(ws).focused;
        let gap = self.gap;
        let tree = &mut self.workspace_mut(ws).tree;
        match focused {
            Some(f) => {
                tree.insert_window(index as i32, f as i32, SplitType::None, 0.5, bounds, gap);
            }
            None => {
                tree.insert_window(index as i32, -1, SplitType::None, 0.5, bounds, gap);
            }
        }
        self.workspace_mut(ws).focused = Some(index);
        self.focused_window = Some(index);
        // In scrolling mode, add the window as a new column.
        if self.layout_mode == crate::layout::LayoutMode::Scrolling {
            self.scrolling.add_column(index as i32);
        }
        self.invalidate_layout_cache();
        self.record_action("new_window", &[]);
        let ctx = self.window_hook_ctx(index);
        self.fire_hook(hooks::Event::AfterNewWindow, ctx);
    }

    pub(crate) fn invalidate_layout_cache(&self) {
        if let Ok(mut cache) = self.layout_cache.lock() {
            *cache = None;
        }
    }

    /// Spawn a command pane: a window that runs `sh -c <command>` instead of
    /// an interactive shell, shows its exit status, re-runs on Enter, and can
    /// start suspended (`start_suspended` semantics) until manually triggered.
    pub fn spawn_command_window(
        &mut self,
        command: &str,
        suspended: bool,
    ) -> Result<usize, String> {
        let command = command.trim();
        if command.is_empty() {
            return Err("empty command".into());
        }
        let index = self.windows.len();
        let id = format!("win-{index}");
        let size = WinSize { cols: 80, rows: 24 };
        let env = crate::util::guestenv::base_guest_env("local", &id, false, false);
        let title = command.split_whitespace().next().unwrap_or(command).to_string();
        let wake = Box::new(|| {}) as Box<dyn Fn() + Send + 'static>;
        let window = Window::spawn(
            id,
            format!("cmd: {title}"),
            size,
            "/bin/sh",
            SpawnOptions { command: Some(command), suspended },
            wake,
            &env,
        )
        .map_err(|e| e.to_string())?;
        self.push_window(window, index);
        self.notify(
            if suspended {
                format!("command pane '{title}' (suspended — Enter to run)")
            } else {
                format!("command pane '{title}'")
            },
            "info",
        );
        Ok(index)
    }

    /// Swap the focused window with its neighbor in `dir` (left/right/up/
    /// down), computed from the current layout geometry. No-op when there is
    /// no neighbor in that direction.
    pub fn swap_focused_with(&mut self, dir: crate::layout::PreselectionDir) {
        let Some(focused) = self.focused_window else {
            return;
        };
        let layout = self.current_layout();
        let Some(rect) = layout.get(&(focused as i32)).copied() else {
            return;
        };
        // Find the neighbor: the window adjacent across the rect's edge.
        let neighbor = layout.iter().find(|(id, r)| {
            **id != focused as i32 && {
                match dir {
                    crate::layout::PreselectionDir::Left => {
                        r.x + r.w == rect.x && rows_overlap(r.y, r.h, rect.y, rect.h)
                    }
                    crate::layout::PreselectionDir::Right => {
                        rect.x + rect.w == r.x && rows_overlap(r.y, r.h, rect.y, rect.h)
                    }
                    crate::layout::PreselectionDir::Up => {
                        r.y + r.h == rect.y && cols_overlap(r.x, r.w, rect.x, rect.w)
                    }
                    crate::layout::PreselectionDir::Down => {
                        rect.y + rect.h == r.y && cols_overlap(r.x, r.w, rect.x, rect.w)
                    }
                    crate::layout::PreselectionDir::None => false,
                }
            }
        });
        if let Some((other, _)) = neighbor {
            let ws = self.current_workspace;
            self.workspace_mut(ws)
                .tree
                .swap_windows(focused as i32, *other);
            self.sync_window_sizes();
            let dir_name = match dir {
                crate::layout::PreselectionDir::Left => "left",
                crate::layout::PreselectionDir::Right => "right",
                crate::layout::PreselectionDir::Up => "up",
                crate::layout::PreselectionDir::Down => "down",
                crate::layout::PreselectionDir::None => "none",
            };
            self.log_action(&format!("swap_window_{dir_name}"));
        }
    }

    /// Snap the focused window to the left or right half of the workspace.
    pub fn snap_half(&mut self, left: bool) {
        let Some(focused) = self.focused_window else {
            return;
        };
        let bounds = self.workspace_bounds(self.current_workspace);
        let gap = self.gap;
        let ws = self.current_workspace;
        let tree = &mut self.workspace_mut(ws).tree;
        // Re-parent the focused window into a fresh split at 50%: remove it,
        // then insert it against the first remaining window.
        tree.remove_window(focused as i32);
        let ids = tree.get_all_window_ids();
        let anchor = ids.first().copied().unwrap_or(-1);
        let dir = crate::layout::SplitType::Vertical;
        tree.insert_window(focused as i32, anchor, dir, 0.5, bounds, gap);
        let _ = tree;
        self.sync_window_sizes();
        self.log_action(if left { "snap_left" } else { "snap_right" });
    }

    /// Move the focused window to `workspace` and switch to it (move-and-follow).
    pub fn move_window_and_follow(&mut self, workspace: i32) {
        if let Some(focused) = self.focused_window {
            self.move_window_to_workspace(focused, workspace);
        }
        self.switch_workspace(workspace);
        self.log_action(&format!("move_and_follow_{workspace}"));
    }

    /// The usable bounds of a workspace, minus the dock bar.
    pub fn workspace_bounds(&self, _ws: i32) -> Rect {
        let dock_height = self.dock_height() as i32;
        // y is always 0 because rect_to_tui adds content_area.y which
        // already accounts for the dock position.
        Rect {
            x: 0,
            y: 0,
            w: self.width,
            h: (self.height - dock_height).max(1),
        }
    }

    /// Height of the dock area in rows (0 when hidden).
    pub(crate) fn dock_height(&self) -> u16 {
        match self.config.appearance.dockbar_position.as_str() {
            "hidden" => 0,
            _ => {
                let hints: u16 = if self.hints_visible && !self.show_welcome {
                    1
                } else {
                    0
                };
                2 + hints
            }
        }
    }

    /// Close the focused window, collapsing the BSP tree. Local close path:
    /// remote closes are initiated daemon-side, which fires its own
    /// after-close-window hook, so this does not double-fire.
    pub fn close_focused_window(&mut self) {
        if let Some(focused) = self.focused_window {
            let ctx = self.window_hook_ctx(focused);
            self.remove_window(focused);
            self.record_action("close_window", &[]);
            self.clear_window_graphics(focused as u32);
            self.fire_hook(hooks::Event::AfterCloseWindow, ctx);
        }
    }

    /// Remove the window at `index`, collapsing the BSP trees and shifting
    /// every later window's index down by one. Also used by the remote TUI
    /// when a daemon window is closed.
    pub fn remove_window(&mut self, index: usize) {
        if index >= self.windows.len() {
            return;
        }
        // The workspace that owns this window (current if unknown). A
        // floating window lives on its float's workspace.
        let mut target_ws = self.current_workspace;
        for ws_num in 1..=9 {
            if self.workspace(ws_num).tree.has_window(index as i32) {
                target_ws = ws_num;
                break;
            }
        }
        if let Some(fi) = self.float_for_window(index) {
            target_ws = self.floats[fi].workspace;
        }

        self.windows.remove(index);

        // Drop floats on the removed window and shift later windows' float
        // entries down with their window indexes.
        let mut i = 0;
        while i < self.floats.len() {
            if self.floats[i].window == index {
                self.floats.remove(i);
            } else if self.floats[i].window > index {
                self.floats[i].window -= 1;
                i += 1;
            } else {
                i += 1;
            }
        }

        // Rebuild every workspace tree with shifted IDs: windows after the
        // removed one move down by one index, and the removed window drops out.
        for ws_num in 1..=9 {
            let bounds = self.workspace_bounds(ws_num);
            let old_ids = self.workspace(ws_num).tree.get_all_window_ids();
            let old_focused = self.workspace(ws_num).focused;
            let mut new_tree = BSPTree::new();
            new_tree.set_auto_scheme(self.auto_scheme);
            for old_id in old_ids {
                if old_id == index as i32 {
                    continue;
                }
                let new_id = if old_id > index as i32 {
                    old_id - 1
                } else {
                    old_id
                };
                new_tree.insert_window(new_id, -1, SplitType::None, 0.5, bounds, self.gap);
            }
            self.workspace_mut(ws_num).tree = new_tree;

            // Remap the workspace's focus: drop it if it was the removed
            // window, shift it down if it was after it.
            let new_focused = match old_focused {
                None => None,
                Some(f) if f == index => None,
                Some(f) if f > index => Some(f - 1),
                Some(f) => Some(f),
            };
            self.workspace_mut(ws_num).focused = new_focused;
        }

        // Focus a neighbor on the window's own workspace.
        let remaining = self.workspace(target_ws).tree.get_all_window_ids();
        self.workspace_mut(target_ws).focused = remaining.first().map(|&i| i as usize);
        // In scrolling mode, remove the window from the scrolling layout.
        if self.layout_mode == crate::layout::LayoutMode::Scrolling {
            self.scrolling.remove_window(index as i32);
        }
        self.invalidate_layout_cache();
        if target_ws == self.current_workspace {
            self.focused_window = remaining.first().map(|&i| i as usize);
            // If the tree emptied but floats remain, focus the frontmost float.
            if self.focused_window.is_none() {
                if let Some(&fi) = self.floats_on_workspace(target_ws).last() {
                    self.focused_window = Some(self.floats[fi].window);
                    self.workspace_mut(target_ws).focused = self.focused_window;
                } else {
                    self.mode = Mode::WindowManagement;
                }
            }
        }
    }

    /// Add a daemon-backed window to the window's workspace BSP tree,
    /// optionally splitting the focused window in `direction`.
    pub fn add_remote_window(
        &mut self,
        info: WindowInfo,
        sink: Box<dyn PtySink>,
        output: crossbeam_channel::Receiver<Vec<u8>>,
        direction: Option<SplitType>,
    ) -> usize {
        let size = WinSize {
            cols: info.cols,
            rows: info.rows,
        };
        let index = self.windows.len();
        let window = Window::remote(info.id, info.title, size, sink, output);
        self.windows.push(window);

        let ws = info.workspace.clamp(1, 9);
        // A pending float (new-floating-shell in remote mode) skips the BSP
        // tree entirely and becomes a floating pane.
        if self.pending_float {
            self.pending_float = false;
            let bounds = self.workspace_bounds(ws);
            let rect = float::default_float_rect(bounds);
            let z = self.floats.iter().map(|f| f.z).max().unwrap_or(0) + 1;
            self.floats.push(float::FloatPane {
                window: index,
                workspace: ws,
                x: rect.x,
                y: rect.y,
                w: rect.w,
                h: rect.h,
                z,
                pinned: false,
                modal: false,
            });
            if let Some(window) = self.windows.get_mut(index) {
                window.resize(WinSize {
                    cols: rect.w.max(1) as u16,
                    rows: rect.h.max(1) as u16,
                });
            }
            self.workspace_mut(ws).focused = Some(index);
            self.focused_window = Some(index);
            return index;
        }

        let bounds = self.workspace_bounds(ws);
        let focused = self.workspace(ws).focused;
        let gap = self.gap;
        let tree = &mut self.workspace_mut(ws).tree;
        let dir = direction.unwrap_or(SplitType::None);
        match focused {
            Some(f) => tree.insert_window(index as i32, f as i32, dir, 0.5, bounds, gap),
            None => tree.insert_window(index as i32, -1, dir, 0.5, bounds, gap),
        }
        self.workspace_mut(ws).focused = Some(index);
        self.focused_window = Some(index);
        index
    }

    /// Request a new window from the daemon (remote mode). The window is not
    /// created locally until the daemon announces it via `WindowAdded`.
    pub fn request_new_window(&self, workspace: i32, shell: &str) {
        if let Some(tx) = &self.remote_commands {
            let _ = tx.send(Message::NewWindow {
                shell: shell.to_string(),
                workspace,
            });
        }
    }

    /// Request the daemon close a window (remote mode).
    pub fn request_close_window(&self, window: &str) {
        if let Some(tx) = &self.remote_commands {
            let _ = tx.send(Message::CloseWindow {
                window: window.to_string(),
            });
        }
    }

    /// Drop every window and reset all workspaces (used when re-attaching to a
    /// different daemon session).
    pub fn clear_all_windows(&mut self) {
        self.windows.clear();
        self.floats.clear();
        self.float_drag = None;
        self.focused_window = None;
        for i in 1..=9 {
            self.workspace_mut(i).tree = BSPTree::new();
            self.workspace_mut(i).focused = None;
        }
        self.mode = Mode::WindowManagement;
        self.prefix = Prefix::None;
        self.selection = None;
        self.mouse_selecting = false;
        self.scrollback_mode = false;
        self.copy_visual = false;
    }

    /// Focus the first window on the current workspace (used after a remote
    /// attach rebuilds the window set).
    pub fn focus_first_window(&mut self) {
        let ws = self.current_workspace;
        let ids = self.workspace(ws).tree.get_all_window_ids();
        let first = ids.first().map(|&i| i as usize);
        self.focused_window = first;
        self.workspace_mut(ws).focused = first;
    }

    // -----------------------------------------------------------------------
    // Focus
    // -----------------------------------------------------------------------

    pub fn focus_next(&mut self) {
        self.cycle_focus(true, "next_window");
    }

    pub fn focus_prev(&mut self) {
        self.cycle_focus(false, "prev_window");
    }

    /// Cycle focus through the current workspace's windows: tiled windows in
    /// BSP order first, then floating panes in z-order.
    fn cycle_focus(&mut self, forward: bool, action: &str) {
        // A modal float blocks focus movement to every other pane.
        if self.focused_is_modal() {
            return;
        }
        let ws = self.current_workspace;
        let mut order: Vec<usize> = self
            .workspace(ws)
            .tree
            .get_all_window_ids()
            .iter()
            .map(|&i| i as usize)
            .collect();
        for fi in self.floats_on_workspace(ws) {
            order.push(self.floats[fi].window);
        }
        if order.is_empty() {
            self.focused_window = None;
            return;
        }
        let current = self.focused_window;
        let next = match current {
            Some(c) => {
                let pos = order.iter().position(|&id| id == c).unwrap_or(0);
                if forward {
                    (pos + 1) % order.len()
                } else {
                    (pos + order.len() - 1) % order.len()
                }
            }
            None => 0,
        };
        let n = order[next];
        if self.focused_window != Some(n) {
            self.focused_window = Some(n);
            self.workspace_mut(ws).focused = Some(n);
            self.record_action(action, &[]);
            let ctx = self.window_hook_ctx(n);
            self.fire_hook(hooks::Event::AfterFocusChange, ctx);
        }
    }

    /// Focus the window in the given direction (left/right/up/down).
    /// Returns an error if directional focus is not available.
    pub fn focus_direction(&mut self, direction: &str) -> Result<(), String> {
        // Delegate to the tape executor's focus_direction for parity.
        // Currently a stub that returns an error.
        let _ = direction;
        Err("directional focus is not implemented in this port".into())
    }

    /// Focus the window at the given index (if on the current workspace,
    /// tiled or floating).
    pub fn focus_window(&mut self, index: usize) {
        let ws = self.current_workspace;
        let on_ws = self.workspace(ws).tree.has_window(index as i32)
            || self
                .float_for_window(index)
                .map(|fi| self.floats[fi].workspace == ws)
                .unwrap_or(false);
        if on_ws && self.focused_window != Some(index) {
            self.focused_window = Some(index);
            self.workspace_mut(ws).focused = Some(index);
            let ctx = self.window_hook_ctx(index);
            self.fire_hook(hooks::Event::AfterFocusChange, ctx);
        }
    }

    // -----------------------------------------------------------------------
    // Workspace switching
    // -----------------------------------------------------------------------

    pub fn switch_workspace(&mut self, number: i32) {
        if !(1..=9).contains(&number) {
            return;
        }
        let previous = self.current_workspace;
        if number == previous {
            return;
        }
        self.damage_full(crate::app::damage::DamageReason::Full);
        self.current_workspace = number;
        self.focused_window = self.workspace(number).focused;
        self.prefix = Prefix::None;
        self.record_workspace_switch(number);
        // Go does not fire when switching to the already-visible workspace.
        let ctx = self.window_hook_ctx(self.focused_window.unwrap_or(0));
        self.fire_hook(
            hooks::Event::AfterWorkspaceSwitch,
            hooks::Context {
                previous_workspace: previous,
                ..ctx
            },
        );
        // Re-place images on the new workspace's panes.
        self.refresh_all_placements();
    }

    /// Move the focused window to another workspace and follow it.
    pub fn move_focused_to_workspace(&mut self, number: i32) {
        if !(1..=9).contains(&number) || number == self.current_workspace {
            return;
        }
        let Some(focused) = self.focused_window else {
            return;
        };
        // A floating window moves with its float entry; the source workspace
        // keeps its other windows (tiled or floating).
        if let Some(fi) = self.float_for_window(focused) {
            let from = self.floats[fi].workspace;
            self.floats[fi].workspace = number;
            let bounds = self.workspace_bounds(number);
            let r = float::clamp_rect(self.floats[fi].rect(), bounds);
            self.floats[fi].x = r.x;
            self.floats[fi].y = r.y;
            self.floats[fi].w = r.w;
            self.floats[fi].h = r.h;
            let remaining = self.workspace(from).tree.get_all_window_ids();
            self.workspace_mut(from).focused = remaining.first().map(|&i| i as usize);
            if from == self.current_workspace {
                self.focused_window = remaining.first().map(|&i| i as usize);
            }
            self.current_workspace = number;
            self.workspace_mut(number).focused = Some(focused);
            self.focused_window = Some(focused);
            self.prefix = Prefix::None;
            return;
        }
        let from = self.current_workspace;

        // Remove from the source tree.
        self.workspace_mut(from).tree.remove_window(focused as i32);

        // Insert into the target tree.
        let bounds = self.workspace_bounds(number);
        let target_focused = self.workspace(number).focused;
        let gap = self.gap;
        let tree = &mut self.workspace_mut(number).tree;
        tree.insert_window(
            focused as i32,
            target_focused.map(|f| f as i32).unwrap_or(-1),
            SplitType::None,
            0.5,
            bounds,
            gap,
        );

        // Refocus source.
        let remaining = self.workspace(from).tree.get_all_window_ids();
        self.workspace_mut(from).focused = remaining.first().map(|&i| i as usize);
        self.focused_window = remaining.first().map(|&i| i as usize);

        // Switch to the target.
        self.current_workspace = number;
        self.workspace_mut(number).focused = Some(focused);
        self.focused_window = Some(focused);
        self.prefix = Prefix::None;
    }

    // -----------------------------------------------------------------------
    // Splitting
    // -----------------------------------------------------------------------

    /// Split the focused window in a given direction (creating a new shell).
    pub fn split(
        &mut self,
        direction: SplitType,
        shell: &str,
        wake: Box<dyn Fn() + Send + 'static>,
    ) -> Result<usize, String> {
        let index = self.windows.len();
        let id = format!("win-{index}");
        let size = WinSize { cols: 40, rows: 12 };
        let env = vec![("TERMOS_ENV".to_string(), "1".to_string())];
        let window = Window::spawn(
            id,
            "Terminal",
            size,
            shell,
            SpawnOptions::shell(),
            wake,
            &env,
        )
        .map_err(|e| e.to_string())?;
        self.windows.push(window);

        let ws = self.current_workspace;
        let bounds = self.workspace_bounds(ws);
        let focused = self.workspace(ws).focused.map(|f| f as i32).unwrap_or(-1);
        let gap = self.gap;
        let tree = &mut self.workspace_mut(ws).tree;
        tree.insert_window(index as i32, focused, direction, 0.5, bounds, gap);
        self.workspace_mut(ws).focused = Some(index);
        self.focused_window = Some(index);
        self.record_action("new_window", &[]);
        let ctx = self.window_hook_ctx(index);
        self.fire_hook(hooks::Event::AfterNewWindow, ctx);
        Ok(index)
    }

    // -----------------------------------------------------------------------
    // Stacked panes
    // -----------------------------------------------------------------------

    /// Toggle stacked mode on the focused window: if it is already stacked,
    /// pop it out; otherwise, stack it with the previously-focused window.
    pub fn stack_focused(&mut self) {
        let Some(focused) = self.focused_window else {
            return;
        };
        let ws = self.current_workspace;
        let tree = &mut self.workspace_mut(ws).tree;
        if tree.find_stack_root(focused as i32).is_some() {
            // Already stacked — pop out.
            tree.pop_from_stack(focused as i32);
            self.record_action("stack_toggle", &[&format!("{focused}")]);
            return;
        }
        // Find another tiled window on this workspace to stack with.
        let ids = self.workspace(ws).tree.get_all_window_ids();
        let prev_id = ids.iter()
            .find(|&&id| id != focused as i32)
            .copied()
            .unwrap_or(-1);
        if prev_id < 0 {
            return;
        }
        let tree = &mut self.workspace_mut(ws).tree;
        tree.push_to_stack(prev_id, focused as i32);
        self.record_action("stack_toggle", &[
            &format!("{focused}"),
            &format!("{prev_id}"),
        ]);
    }

    /// Navigate focus within a stack: arrow left/right cycles through the
    /// stacked panes.  Returns the newly-focused window index.
    pub fn cycle_stack_focus(&mut self, forward: bool) {
        let Some(focused) = self.focused_window else {
            return;
        };
        let ws = self.current_workspace;
        let tree = &mut self.workspace_mut(ws).tree;
        let new_id = tree.cycle_stack_focus(focused as i32, forward);
        if new_id != focused as i32 {
            let new_idx = new_id as usize;
            self.focused_window = Some(new_idx);
            self.workspace_mut(ws).focused = Some(new_idx);
            self.record_action("stack_cycle", &[&format!("{new_idx}")]);
            let ctx = self.window_hook_ctx(new_idx);
            self.fire_hook(hooks::Event::AfterFocusChange, ctx);
        }
    }

    // -----------------------------------------------------------------------
    // Multi-pane select & bulk ops
    // -----------------------------------------------------------------------

    /// Toggle multi-select mode on/off.
    pub fn toggle_multi_select_mode(&mut self) {
        self.multi_select_mode = !self.multi_select_mode;
        if !self.multi_select_mode {
            self.selected_panes.clear();
        }
        self.record_action(
            if self.multi_select_mode { "multi_select_on" } else { "multi_select_off" },
            &[],
        );
    }

    /// Toggle a window in the selection set.
    pub fn select_pane(&mut self, window: usize) {
        if self.selected_panes.contains(&window) {
            self.selected_panes.remove(&window);
        } else {
            self.selected_panes.insert(window);
        }
        self.record_action("select_pane", &[&format!("{window}")]);
    }

    /// Select all tiled windows on the current workspace.
    pub fn select_all_panes(&mut self) {
        let ws = self.current_workspace;
        let ids = self.workspace(ws).tree.get_all_window_ids();
        self.selected_panes = ids.iter().map(|&i| i as usize).collect();
        self.multi_select_mode = true;
        self.record_action("select_all_panes", &[]);
    }

    /// Close all selected panes.  Clears the selection afterward.
    pub fn bulk_close_selected(&mut self) {
        let mut to_close: Vec<usize> = self.selected_panes.iter().copied().collect();
        // Remove in reverse index order to avoid index shifting.
        to_close.sort_unstable_by(|a, b| b.cmp(a));
        for idx in to_close {
            self.remove_window(idx);
        }
        self.selected_panes.clear();
        self.multi_select_mode = false;
        self.record_action("bulk_close", &[]);
    }

    /// Stack all selected panes into one stack (the first selected pane
    /// becomes the active one).
    pub fn bulk_stack_selected(&mut self) {
        let mut ids: Vec<usize> = self.selected_panes.iter().copied().collect();
        if ids.len() < 2 {
            return;
        }
        ids.sort();
        let active = ids[0];
        let ws = self.current_workspace;
        let tree = &mut self.workspace_mut(ws).tree;
        for &other in &ids[1..] {
            tree.push_to_stack(active as i32, other as i32);
        }
        self.focused_window = Some(active);
        self.workspace_mut(ws).focused = Some(active);
        self.selected_panes.clear();
        self.multi_select_mode = false;
        self.record_action("bulk_stack", &[]);
    }

    /// Break all selected panes into their own new windows (unstack them).
    pub fn bulk_break_selected(&mut self) {
        let to_break: Vec<usize> = self.selected_panes.iter().copied().collect();
        for idx in to_break {
            let ws = self.current_workspace;
            let tree = &mut self.workspace_mut(ws).tree;
            tree.pop_from_stack(idx as i32);
        }
        self.selected_panes.clear();
        self.multi_select_mode = false;
        self.record_action("bulk_break", &[]);
    }

    // -----------------------------------------------------------------------
    // Input / mode
    // -----------------------------------------------------------------------

    /// Enter terminal mode (keys pass through to the shell).
    pub fn enter_terminal_mode(&mut self) {
        self.mode = Mode::Terminal;
        self.prefix = Prefix::None;
        if let Some(recorder) = self.recorder.as_mut() {
            recorder.record_mode_switch(crate::tape::command::CommandType::TerminalMode);
        }
    }

    /// Leave terminal mode back to window management.
    pub fn leave_terminal_mode(&mut self) {
        self.mode = Mode::WindowManagement;
        self.prefix = Prefix::None;
        if let Some(recorder) = self.recorder.as_mut() {
            recorder.record_mode_switch(crate::tape::command::CommandType::WindowManagementMode);
        }
    }

    /// Write bytes to the focused window's PTY.
    pub fn write_to_focused(&self, data: &[u8]) {
        if let Some(index) = self.focused_window {
            if let Some(window) = self.windows.get(index) {
                window.write(data);
            }
        }
    }

    /// Toggle zoom on the focused window: a zoomed window fills the
    /// workspace (with a snap animation); zooming out restores its layout
    /// rect. Used by the `z` key and the tape `ToggleZoom` command.
    pub fn toggle_zoom_internal(&mut self) -> Result<(), String> {
        let Some(index) = self.focused_window else {
            return Err("no focused window".into());
        };
        // Floating panes zoom to the workspace and back, remembering their
        // float rect in the window's pre-zoom fields.
        if let Some(fi) = self.float_for_window(index) {
            let ws = self.floats[fi].workspace;
            let bounds = self.workspace_bounds(ws);
            let window = self
                .windows
                .get_mut(index)
                .ok_or_else(|| "window not found".to_string())?;
            if window.zoomed {
                window.zoomed = false;
                let r = float::clamp_rect(
                    Rect {
                        x: window.pre_zoom_x,
                        y: window.pre_zoom_y,
                        w: window.pre_zoom_width,
                        h: window.pre_zoom_height,
                    },
                    bounds,
                );
                window.pre_zoom_x = 0;
                window.pre_zoom_y = 0;
                window.pre_zoom_width = 0;
                window.pre_zoom_height = 0;
                self.floats[fi].x = r.x;
                self.floats[fi].y = r.y;
                self.floats[fi].w = r.w;
                self.floats[fi].h = r.h;
            } else {
                window.zoomed = true;
                window.pre_zoom_x = self.floats[fi].x;
                window.pre_zoom_y = self.floats[fi].y;
                window.pre_zoom_width = self.floats[fi].w;
                window.pre_zoom_height = self.floats[fi].h;
                self.floats[fi].x = bounds.x;
                self.floats[fi].y = bounds.y;
                self.floats[fi].w = bounds.w;
                self.floats[fi].h = bounds.h;
            }
            self.sync_float_sizes();
            return Ok(());
        }
        let Some(rect) = self.current_layout().get(&(index as i32)).copied() else {
            return Err("window has no layout rect".into());
        };
        let bounds = self.workspace_bounds(self.current_workspace);
        let window = self
            .windows
            .get_mut(index)
            .ok_or_else(|| "window not found".to_string())?;
        if window.zoomed {
            window.zoomed = false;
            window.pre_zoom_x = 0;
            window.pre_zoom_y = 0;
            window.pre_zoom_width = 0;
            window.pre_zoom_height = 0;
            let _ = window;
            self.sync_window_sizes();
            return Ok(());
        }
        window.zoomed = true;
        window.pre_zoom_x = rect.x;
        window.pre_zoom_y = rect.y;
        window.pre_zoom_width = rect.w;
        window.pre_zoom_height = rect.h;
        let _ = window;
        self.begin_snap(index as i32, bounds);
        self.sync_window_sizes();
        Ok(())
    }

    /// Resize all windows to their BSP layout rects (tiled) and float rects
    /// (floating). Windows whose size actually changed fire the after-resize
    /// hook.
    pub fn sync_window_sizes(&mut self) {
        let layout = self.current_layout();
        let mut resized = Vec::new();
        for (window_id, rect) in layout {
            if let Some(window) = self.windows.get_mut(window_id as usize) {
                let changed = window.resize(WinSize {
                    cols: rect.w.max(1) as u16,
                    rows: rect.h.max(1) as u16,
                });
                if changed {
                    resized.push((window_id as usize, rect));
                }
            }
        }
        self.sync_float_sizes();
        for (index, rect) in resized {
            let mut ctx = self.window_hook_ctx(index);
            ctx.width = rect.w;
            ctx.height = rect.h;
            self.fire_hook(hooks::Event::AfterResize, ctx);
        }
    }

    /// The layout rects for the current workspace.
    pub fn current_layout(&self) -> HashMap<i32, Rect> {
        match self.layout_mode {
            crate::layout::LayoutMode::BSP => self.current_layout_bsp(),
            crate::layout::LayoutMode::MasterStack => self.current_layout_master_stack(),
            crate::layout::LayoutMode::Scrolling => self.current_layout_scrolling(),
        }
    }

    fn current_layout_bsp(&self) -> HashMap<i32, Rect> {
        let ws = self.current_workspace;
        let bounds = self.workspace_bounds(ws);
        let key = LayoutCacheKey {
            workspace: ws,
            bounds,
            gap: self.gap,
            tree: self.workspace(ws).tree.serialize(),
        };
        if let Ok(cache) = self.layout_cache.lock() {
            if let Some((cached_key, layout)) = cache.as_ref() {
                if cached_key == &key {
                    return layout.clone();
                }
            }
        }
        let layout = self.workspace(ws).tree.apply_layout(bounds, self.gap);
        if let Ok(mut cache) = self.layout_cache.lock() {
            *cache = Some((key, layout.clone()));
        }
        layout
    }

    fn current_layout_master_stack(&self) -> HashMap<i32, Rect> {
        let ws = self.current_workspace;
        let ids = self.workspace_window_ids(ws);
        let n = ids.len() as i32;
        if n == 0 {
            return HashMap::new();
        }
        let bounds = self.workspace_bounds(ws);
        let tiles = crate::layout::tiling::calculate_tiling_layout(
            n,
            bounds.w,
            bounds.h,
            bounds.y,
            self.master_ratio,
            self.gap,
        );
        let mut layout = HashMap::new();
        for (i, &id) in ids.iter().enumerate() {
            if let Some(tile) = tiles.get(i) {
                layout.insert(id, Rect { x: tile.x, y: tile.y, w: tile.width, h: tile.height });
            }
        }
        layout
    }

    fn current_layout_scrolling(&self) -> HashMap<i32, Rect> {
        let bounds = self.workspace_bounds(self.current_workspace);
        let raw = self.scrolling.compute_positions(bounds.w, bounds.h, bounds.y);
        // Clamp positions to the visible area so the renderer doesn't
        // write outside the buffer bounds.
        let mut result = HashMap::new();
        for (id, rect) in raw {
            let x = rect.x.max(0);
            let w = (rect.w).min(bounds.w - x);
            if w > 0 && rect.h > 0 {
                result.insert(id, Rect { x, y: rect.y, w, h: rect.h });
            }
        }
        result
    }

    // -----------------------------------------------------------------------
    // Floating panes
    // -----------------------------------------------------------------------

    /// The indices (into `floats`) of the floats on a workspace, sorted
    /// back-to-front: unpinned by z-order, then pinned by z-order (so pinned
    /// floats always composite above and win hit-testing).
    pub fn floats_on_workspace(&self, ws: i32) -> Vec<usize> {
        let mut v: Vec<usize> = self
            .floats
            .iter()
            .enumerate()
            .filter(|(_, f)| f.workspace == ws)
            .map(|(i, _)| i)
            .collect();
        v.sort_by_key(|&i| (self.floats[i].pinned, self.floats[i].z));
        v
    }

    /// The index (into `floats`) for a window, if it is floating.
    pub fn float_for_window(&self, index: usize) -> Option<usize> {
        self.floats.iter().position(|f| f.window == index)
    }

    /// Whether a window is currently floating.
    pub fn is_float(&self, index: usize) -> bool {
        self.float_for_window(index).is_some()
    }

    /// Whether the focused window is floating.
    pub fn focused_is_float(&self) -> bool {
        self.focused_window.map(|i| self.is_float(i)).unwrap_or(false)
    }

    /// The screen rect of a floating window, if any.
    pub fn float_rect(&self, index: usize) -> Option<Rect> {
        self.float_for_window(index).map(|fi| self.floats[fi].rect())
    }

    /// The topmost floating window containing a screen cell (current
    /// workspace only). Pinned floats win over unpinned ones at the same
    /// cell; within a pin group the highest z wins. Returns `None` while a
    /// tiled window is zoomed (floats are hidden then).
    pub fn float_at(&self, column: i32, row: i32) -> Option<usize> {
        if self.floats_hidden_by_zoom() {
            return None;
        }
        let ws = self.current_workspace;
        let mut best: Option<(usize, bool, i32)> = None;
        for f in &self.floats {
            if f.workspace != ws || !f.contains(column, row) {
                continue;
            }
            let better = match best {
                None => true,
                Some((_, bp, bz)) => (f.pinned, f.z) > (bp, bz),
            };
            if better {
                best = Some((f.window, f.pinned, f.z));
            }
        }
        best.map(|(w, _, _)| w)
    }

    /// Whether floating panes are hidden because a tiled window on the
    /// current workspace is zoomed (tmux parity: zoom shows only the zoomed
    /// pane, so floats disappear until unzoom).
    pub fn floats_hidden_by_zoom(&self) -> bool {
        let ws = self.current_workspace;
        self.workspace(ws)
            .tree
            .get_all_window_ids()
            .iter()
            .any(|&wid| {
                self.windows
                    .get(wid as usize)
                    .map(|w| w.zoomed)
                    .unwrap_or(false)
            })
    }

    /// Whether the focused window is a modal floating pane.
    pub fn focused_is_modal(&self) -> bool {
        self.focused_window
            .and_then(|i| self.float_for_window(i))
            .map(|fi| self.floats[fi].modal)
            .unwrap_or(false)
    }

    /// Toggle always-on-top on the focused floating pane. Pinning raises the
    /// pane above every unpinned float; unpinning returns it to normal
    /// z-order.
    pub fn toggle_float_pin(&mut self) {
        let Some(focused) = self.focused_window else {
            return;
        };
        let Some(fi) = self.float_for_window(focused) else {
            self.notify("no floating pane is focused", "info");
            return;
        };
        let pinned = {
            let top = self.floats.iter().map(|f| f.z).max().unwrap_or(0);
            let f = &mut self.floats[fi];
            f.pinned = !f.pinned;
            if f.pinned {
                f.z = top + 1;
            }
            f.pinned
        };
        self.record_action(if pinned { "float_pin" } else { "float_unpin" }, &[]);
        self.notify(if pinned { "float pinned" } else { "float unpinned" }, "info");
    }

    /// Toggle modal mode on the focused floating pane. While modal, the pane
    /// blocks focus movement and clicks on every other pane until modal is
    /// toggled off or the pane is closed.
    pub fn toggle_float_modal(&mut self) {
        let Some(focused) = self.focused_window else {
            return;
        };
        let Some(fi) = self.float_for_window(focused) else {
            self.notify("no floating pane is focused", "info");
            return;
        };
        let modal = {
            let f = &mut self.floats[fi];
            f.modal = !f.modal;
            f.modal
        };
        self.raise_float(focused);
        self.record_action(if modal { "float_modal" } else { "float_unmodal" }, &[]);
        self.notify(
            if modal {
                "modal pane active — other panes are blocked until released"
            } else {
                "modal pane released"
            },
            "info",
        );
    }

    /// Raise a floating window to the front of its workspace's z-order.
    pub fn raise_float(&mut self, index: usize) {
        let Some(fi) = self.float_for_window(index) else {
            return;
        };
        let top = self.floats.iter().map(|f| f.z).max().unwrap_or(0);
        if self.floats[fi].z < top {
            self.floats[fi].z = top + 1;
        }
    }

    /// Float the window at `index`: remove it from its workspace's BSP tree
    /// and give it a centered float rect above the tiles. The window keeps
    /// running; only its placement changes.
    pub fn float_window(&mut self, index: usize) {
        if index >= self.windows.len() || self.is_float(index) {
            return;
        }
        let mut ws = self.current_workspace;
        for n in 1..=9 {
            if self.workspace(n).tree.has_window(index as i32) {
                ws = n;
                break;
            }
        }
        if !self.workspace(ws).tree.has_window(index as i32) {
            return;
        }
        let bounds = self.workspace_bounds(ws);
        let rect = float::default_float_rect(bounds);
        let z = self.floats.iter().map(|f| f.z).max().unwrap_or(0) + 1;
        self.workspace_mut(ws).tree.remove_window(index as i32);
        let remaining = self.workspace(ws).tree.get_all_window_ids();
        self.workspace_mut(ws).focused = remaining.first().map(|&i| i as usize);
        self.floats.push(float::FloatPane {
            window: index,
            workspace: ws,
            x: rect.x,
            y: rect.y,
            w: rect.w,
            h: rect.h,
            z,
            pinned: false,
            modal: false,
        });
        if let Some(window) = self.windows.get_mut(index) {
            window.resize(WinSize {
                cols: rect.w.max(1) as u16,
                rows: rect.h.max(1) as u16,
            });
        }
        self.focused_window = Some(index);
        self.workspace_mut(ws).focused = Some(index);
        self.record_action("float_window", &[]);
        self.log_action("float_window");
    }

    /// Tile a floating window back into its workspace's BSP tree.
    pub fn unfloat_window(&mut self, index: usize) {
        let Some(fi) = self.float_for_window(index) else {
            return;
        };
        let ws = self.floats[fi].workspace;
        self.floats.remove(fi);
        let bounds = self.workspace_bounds(ws);
        let focused = self.workspace(ws).focused.filter(|&f| f != index);
        let gap = self.gap;
        let tree = &mut self.workspace_mut(ws).tree;
        tree.insert_window(
            index as i32,
            focused.map(|f| f as i32).unwrap_or(-1),
            SplitType::None,
            0.5,
            bounds,
            gap,
        );
        self.workspace_mut(ws).focused = Some(index);
        self.focused_window = Some(index);
        self.sync_window_sizes();
        self.record_action("tile_window", &[]);
        self.log_action("tile_window");
    }

    /// Float the focused window if it is tiled; tile it if it is floating.
    pub fn toggle_float(&mut self) {
        let Some(focused) = self.focused_window else {
            return;
        };
        if self.is_float(focused) {
            self.unfloat_window(focused);
        } else {
            self.float_window(focused);
        }
    }

    /// Spawn a new shell window directly into a floating pane (no BSP insert).
    pub fn spawn_floating_window(
        &mut self,
        shell: &str,
        wake: Box<dyn Fn() + Send + 'static>,
    ) -> Result<usize, String> {
        let index = self.windows.len();
        let id = format!("win-{index}");
        let size = WinSize { cols: 40, rows: 12 };
        let env = crate::util::guestenv::base_guest_env("local", &id, false, false);
        let window = Window::spawn(
            id,
            "Terminal",
            size,
            shell,
            SpawnOptions::shell(),
            wake,
            &env,
        )
        .map_err(|e| e.to_string())?;
        self.windows.push(window);

        let ws = self.current_workspace;
        let bounds = self.workspace_bounds(ws);
        let rect = float::default_float_rect(bounds);
        let z = self.floats.iter().map(|f| f.z).max().unwrap_or(0) + 1;
        self.floats.push(float::FloatPane {
            window: index,
            workspace: ws,
            x: rect.x,
            y: rect.y,
            w: rect.w,
            h: rect.h,
            z,
            pinned: false,
            modal: false,
        });
        if let Some(window) = self.windows.get_mut(index) {
            window.resize(WinSize {
                cols: rect.w.max(1) as u16,
                rows: rect.h.max(1) as u16,
            });
        }
        self.focused_window = Some(index);
        self.workspace_mut(ws).focused = Some(index);
        self.record_action("new_floating_window", &[]);
        let ctx = self.window_hook_ctx(index);
        self.fire_hook(hooks::Event::AfterNewWindow, ctx);
        Ok(index)
    }

    /// Move the focused floating pane by a cell delta, clamped to its
    /// workspace.
    pub fn float_move(&mut self, dx: i32, dy: i32) {
        let Some(focused) = self.focused_window else {
            return;
        };
        let Some(fi) = self.float_for_window(focused) else {
            return;
        };
        let ws = self.floats[fi].workspace;
        let bounds = self.workspace_bounds(ws);
        let f = &mut self.floats[fi];
        f.x = (f.x + dx).clamp(bounds.x, (bounds.x + bounds.w - f.w).max(bounds.x));
        f.y = (f.y + dy).clamp(bounds.y, (bounds.y + bounds.h - f.h).max(bounds.y));
        self.log_action("float_move");
    }

    /// Resize the focused floating pane by dragging `edge` by `delta` cells
    /// (positive = outward), clamped to the workspace.
    pub fn float_resize(&mut self, edge: crate::layout::ResizeEdge, delta: i32) {
        let Some(focused) = self.focused_window else {
            return;
        };
        let Some(fi) = self.float_for_window(focused) else {
            return;
        };
        let ws = self.floats[fi].workspace;
        let bounds = self.workspace_bounds(ws);
        let f = &mut self.floats[fi];
        match edge {
            crate::layout::ResizeEdge::Right => {
                f.w = (f.w + delta)
                    .clamp(float::FLOAT_MIN_W, (bounds.x + bounds.w - f.x).max(float::FLOAT_MIN_W));
            }
            crate::layout::ResizeEdge::Left => {
                let new_w = (f.w - delta)
                    .clamp(float::FLOAT_MIN_W, (f.x + f.w - bounds.x).max(float::FLOAT_MIN_W));
                f.x = f.x + f.w - new_w;
                f.w = new_w;
            }
            crate::layout::ResizeEdge::Bottom => {
                f.h = (f.h + delta)
                    .clamp(float::FLOAT_MIN_H, (bounds.y + bounds.h - f.y).max(float::FLOAT_MIN_H));
            }
            crate::layout::ResizeEdge::Top => {
                let new_h = (f.h - delta)
                    .clamp(float::FLOAT_MIN_H, (f.y + f.h - bounds.y).max(float::FLOAT_MIN_H));
                f.y = f.y + f.h - new_h;
                f.h = new_h;
            }
        }
        self.sync_float_sizes();
        self.log_action("float_resize");
    }

    /// Re-center the focused floating pane in its workspace.
    pub fn float_center(&mut self) {
        let Some(focused) = self.focused_window else {
            return;
        };
        let Some(fi) = self.float_for_window(focused) else {
            return;
        };
        let ws = self.floats[fi].workspace;
        let bounds = self.workspace_bounds(ws);
        let f = &mut self.floats[fi];
        f.x = bounds.x + (bounds.w - f.w) / 2;
        f.y = bounds.y + (bounds.h - f.h) / 2;
        self.log_action("float_center");
    }

    /// Cycle focus through the floating panes on the current workspace
    /// (wrapping), raising each pane as it is focused.
    pub fn float_cycle_focus(&mut self, forward: bool) {
        // A modal float blocks cycling; floats hidden by a tiled zoom are
        // unreachable.
        if self.focused_is_modal() || self.floats_hidden_by_zoom() {
            return;
        }
        let ws = self.current_workspace;
        let floats = self.floats_on_workspace(ws);
        if floats.is_empty() {
            return;
        }
        let current = self.focused_window.and_then(|i| self.float_for_window(i));
        let pos = current
            .and_then(|ci| floats.iter().position(|&f| f == ci))
            .unwrap_or(0);
        let next = if forward {
            (pos + 1) % floats.len()
        } else {
            (pos + floats.len() - 1) % floats.len()
        };
        let idx = self.floats[floats[next]].window;
        if self.focused_window != Some(idx) {
            self.focused_window = Some(idx);
            self.workspace_mut(ws).focused = Some(idx);
            self.raise_float(idx);
            self.record_action(if forward { "next_float" } else { "prev_float" }, &[]);
            let ctx = self.window_hook_ctx(idx);
            self.fire_hook(hooks::Event::AfterFocusChange, ctx);
        }
    }

    /// Clamp every float rect to its workspace bounds and resize the backing
    /// windows to match. Idempotent, so it is safe to call every frame.
    pub fn sync_float_sizes(&mut self) {
        let mut to_resize: Vec<(usize, u16, u16)> = Vec::new();
        for i in 0..self.floats.len() {
            let bounds = self.workspace_bounds(self.floats[i].workspace);
            let r = float::clamp_rect(self.floats[i].rect(), bounds);
            self.floats[i].x = r.x;
            self.floats[i].y = r.y;
            self.floats[i].w = r.w;
            self.floats[i].h = r.h;
            to_resize.push((self.floats[i].window, r.w.max(1) as u16, r.h.max(1) as u16));
        }
        for (window, cols, rows) in to_resize {
            if let Some(win) = self.windows.get_mut(window) {
                win.resize(WinSize { cols, rows });
            }
        }
    }

    /// Begin a mouse drag (move or resize) on a floating pane.
    pub fn start_float_drag(&mut self, window: usize, kind: float::FloatDragKind, x: i32, y: i32) {
        let Some(rect) = self.float_rect(window) else {
            return;
        };
        self.float_drag = Some(float::FloatDragState {
            window,
            kind,
            start_x: x,
            start_y: y,
            start_rect: rect,
        });
    }

    /// Apply an in-progress float drag to the current cursor position. The
    /// drag state is passed in (it was taken out of `float_drag`) and stored
    /// back so the original start rect is preserved across motion events.
    pub fn apply_float_drag(&mut self, drag: float::FloatDragState, x: i32, y: i32) {
        let Some(fi) = self.float_for_window(drag.window) else {
            return;
        };
        let ws = self.floats[fi].workspace;
        let bounds = self.workspace_bounds(ws);
        let dx = x - drag.start_x;
        let dy = y - drag.start_y;
        let mut r = drag.start_rect;
        match drag.kind {
            float::FloatDragKind::Move => {
                r.x += dx;
                r.y += dy;
            }
            float::FloatDragKind::Resize(edge) => match edge {
                crate::layout::ResizeEdge::Right => {
                    r.w = (r.w + dx).clamp(float::FLOAT_MIN_W, i32::MAX);
                }
                crate::layout::ResizeEdge::Left => {
                    let new_w = (r.w - dx).clamp(float::FLOAT_MIN_W, i32::MAX);
                    r.x = r.x + r.w - new_w;
                    r.w = new_w;
                }
                crate::layout::ResizeEdge::Bottom => {
                    r.h = (r.h + dy).clamp(float::FLOAT_MIN_H, i32::MAX);
                }
                crate::layout::ResizeEdge::Top => {
                    let new_h = (r.h - dy).clamp(float::FLOAT_MIN_H, i32::MAX);
                    r.y = r.y + r.h - new_h;
                    r.h = new_h;
                }
            },
        }
        r = float::clamp_rect(r, bounds);
        let f = &mut self.floats[fi];
        f.x = r.x;
        f.y = r.y;
        f.w = r.w;
        f.h = r.h;
        if let Some(win) = self.windows.get_mut(drag.window) {
            win.resize(WinSize {
                cols: r.w.max(1) as u16,
                rows: r.h.max(1) as u16,
            });
        }
        // Damage old position + new position, including shadow margin.
        let margin = 3;
        let old_rect = expand_rect(drag.start_rect, margin);
        let new_rect = expand_rect(r, margin);
        self.damage_rect(old_rect, crate::app::damage::DamageReason::Geometry);
        self.damage_rect(new_rect, crate::app::damage::DamageReason::Geometry);
        self.float_drag = Some(drag);
    }

    /// The float border interaction a screen cell starts (topmost float
    /// wins), if any.
    pub fn float_edge_at(&self, column: i32, row: i32) -> Option<(usize, float::FloatDragKind)> {
        if self.floats_hidden_by_zoom() {
            return None;
        }
        let ws = self.current_workspace;
        let mut best: Option<(usize, bool, i32, float::FloatDragKind)> = None;
        for f in &self.floats {
            if f.workspace != ws {
                continue;
            }
            if let Some(kind) = float::float_edge_at(f, column, row) {
                let better = match best {
                    None => true,
                    Some((_, bp, bz, _)) => (f.pinned, f.z) > (bp, bz),
                };
                if better {
                    best = Some((f.window, f.pinned, f.z, kind));
                }
            }
        }
        best.map(|(w, _, _, k)| (w, k))
    }

    /// The focused window's emulator, if any.
    pub fn focused_emulator(&self) -> Option<Arc<Mutex<Emulator>>> {
        self.focused_window
            .and_then(|index| self.windows.get(index))
            .map(|w| Arc::clone(&w.emulator))
    }

    pub fn notify(&mut self, message: impl Into<String>, kind: impl Into<String>) {
        let message = message.into();
        let kind = kind.into();
        // Every notification also lands in the debug log viewer's ring.
        self.event_log.push(format!("[{kind}] {message}"));
        if self.event_log.len() > 200 {
            let overflow = self.event_log.len() - 200;
            self.event_log.drain(..overflow);
        }
        self.notifications.push(Notification { message, kind });
        if self.notifications.len() > 5 {
            self.notifications.remove(0);
        }
    }

    /// Append an action to the debug log viewer's ring (actions already
    /// recorded for tape are logged here too).
    pub fn log_action(&mut self, action: &str) {
        self.event_log.push(format!("[action] {action}"));
        if self.event_log.len() > 200 {
            let overflow = self.event_log.len() - 200;
            self.event_log.drain(..overflow);
        }
    }

    /// Play an agent alert sound cue if enabled and not on cooldown.
    fn play_alert_sound(&mut self, cue: &str) {
        if !self.config.notifications.agent.sound.unwrap_or(false) {
            return;
        }
        // Delegate to the sound subsystem: atomic cooldown, single worker,
        // one cue at a time, permanent silence after repeated failures.
        let cooldown = std::time::Duration::from_secs(
            self.config
                .notifications
                .agent
                .sound_cooldown_seconds
                .unwrap_or(5) as u64,
        );
        let req = match cue {
            "done" => crate::sound::Request {
                cue: crate::sound::Cue::Done,
                cooldown,
            },
            "needs-input" => crate::sound::Request {
                cue: crate::sound::Cue::Attention,
                cooldown,
            },
            _ => return,
        };
        crate::sound::play(req);
    }

    /// The leader key from config.
    pub fn leader_key(&self) -> &str {
        &self.config.keybindings.leader_key
    }

    /// Whether the app has any windows.
    pub fn has_windows(&self) -> bool {
        !self.windows.is_empty()
    }

    // -----------------------------------------------------------------------
    // Command palette
    // -----------------------------------------------------------------------

    pub fn open_palette(&mut self) {
        self.palette_open = true;
        self.palette_query.clear();
        self.palette_selected = 0;
    }

    pub fn close_palette(&mut self) {
        self.palette_open = false;
        self.palette_query.clear();
        self.palette_selected = 0;
    }

    /// The commands matching the current query, best match first.
    /// Each entry carries the command and the character positions that matched
    /// the query (for rendering with highlights).
    pub fn palette_items(&self) -> Vec<(Command, Vec<usize>)> {
        let mut items: Vec<(i64, Command, Vec<usize>)> = Command::all()
            .into_iter()
            .filter_map(|c| {
                fuzzy_match_tokens(&c.label(), &self.palette_query)
                    .map(|m| (m.score, c, m.positions))
            })
            .collect();
        // Append custom actions from config.
        for action in &self.config.custom_actions {
            if let Some(m) = fuzzy_match_tokens(&action.name, &self.palette_query) {
                items.push((m.score, Command::CustomAction(action.name.clone()), m.positions));
            }
        }
        // Boost recently-used commands: each recency slot gives a -10 bonus
        // so the most-recent command beats any fuzzy-score difference.
        for (score, cmd, _) in &mut items {
            if let Some(pos) = self.palette_recent.iter().rev().position(|r| r == cmd) {
                *score -= ((self.palette_recent.len() - pos) as i64) * 10;
            }
        }
        items.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.label().cmp(&b.1.label())));
        items.into_iter().map(|(_, c, p)| (c, p)).collect()
    }

    pub fn palette_move(&mut self, delta: i32) {
        let len = self.palette_items().len();
        if len == 0 {
            return;
        }
        let sel = self.palette_selected as i32 + delta;
        self.palette_selected = sel.rem_euclid(len as i32) as usize;
    }

    /// Compute the palette overlay geometry for mouse hit-testing.
    /// Returns (panel_x, panel_y, panel_w, panel_h, row_y_starts).
    pub fn palette_geometry(&self) -> Option<(i32, i32, i32, i32, Vec<i32>)> {
        if !self.palette_open {
            return None;
        }
        let items = self.palette_items();
        let rows: Vec<(String, String)> = items
            .iter()
            .map(|(c, _)| (c.label(), c.category().to_string()))
            .collect();
        let max_row = rows
            .iter()
            .map(|(l, d)| {
                l.chars().count()
                    + if d.is_empty() { 0 } else { 2 + d.chars().count() }
            })
            .max()
            .unwrap_or(0);
        let query_w = self.palette_query.chars().count() + 2;
        let content_w = max_row.max(query_w).max("Commands".len()) + 4;
        let w = (content_w as i32).clamp(20, self.width - 2);
        let h = ((rows.len() + 4) as i32).clamp(3, self.height - 2);
        let px = (self.width - w) / 2;
        let py = (self.height - h) / 2;
        let visible = (h as usize).saturating_sub(3).max(1);
        let start = if rows.len() > visible {
            self.palette_selected.saturating_sub(visible - 1)
        } else {
            0
        };
        let mut row_ys = Vec::new();
        for i in 0..visible {
            if start + i >= rows.len() {
                break;
            }
            let ry = py + 2 + i as i32;
            if ry >= py + h - 1 {
                break;
            }
            row_ys.push(ry);
        }
        Some((px, py, w, h, row_ys))
    }

    /// Run the selected command and close the palette.
    pub fn activate_palette(&mut self) {
        let items = self.palette_items();
        let cmd = items.get(self.palette_selected).map(|(c, _)| c.clone());
        self.close_palette();
        if let Some(cmd) = cmd {
            // Track recency: remove if already present, push to end, cap at 8.
            self.palette_recent.retain(|c| c != &cmd);
            self.palette_recent.push(cmd.clone());
            if self.palette_recent.len() > 8 {
                self.palette_recent.remove(0);
            }
            self.run_command(cmd);
        }
    }

    fn run_command(&mut self, cmd: Command) {
        match cmd {
            Command::NewWindow => {
                let shell = self.default_shell();
                let _ = self.spawn_window(&shell, Box::new(|| {}));
            }
            Command::CloseWindow => self.close_focused_window(),
            Command::SplitHorizontal => {
                let shell = self.default_shell();
                let _ = self.split(SplitType::Horizontal, &shell, Box::new(|| {}));
            }
            Command::SplitVertical => {
                let shell = self.default_shell();
                let _ = self.split(SplitType::Vertical, &shell, Box::new(|| {}));
            }
            Command::NextWindow => self.focus_next(),
            Command::PrevWindow => self.focus_prev(),
            Command::ToggleTiling => {
                // BSP tiling is always on in this port; the palette entry
                // exists for parity with the Go command list.
            }
            Command::CycleLayoutMode => self.cycle_layout_mode(),
            Command::ToggleFloat => self.toggle_float(),
            Command::FloatNew => {
                let shell = self.default_shell();
                let _ = self.spawn_floating_window(&shell, Box::new(|| {}));
            }
            Command::EqualizeSplits => {
                let ws = self.current_workspace;
                self.workspace_mut(ws).tree.equalize_ratios();
            }
            Command::Scrollback => self.enter_scrollback_mode(),
            Command::SwitchWorkspace(i) => self.switch_workspace(i),
            Command::Quit => {
                self.show_quit_confirmation = true;
            }
            Command::Theme => self.open_theme_picker(),
            Command::ThemeDetect => self.redetect_theme(),
            Command::CommandPane => self.open_command_pane_dialog(),
            Command::Settings => self.open_settings(),
            Command::FocusLeft => {
                let _ = self.focus_direction("left");
            }
            Command::FocusRight => {
                let _ = self.focus_direction("right");
            }
            Command::FocusUp => {
                let _ = self.focus_direction("up");
            }
            Command::FocusDown => {
                let _ = self.focus_direction("down");
            }
            Command::SwapLeft => {
                self.swap_focused_with(crate::layout::PreselectionDir::Left);
            }
            Command::SwapRight => {
                self.swap_focused_with(crate::layout::PreselectionDir::Right);
            }
            Command::SwapUp => {
                self.swap_focused_with(crate::layout::PreselectionDir::Up);
            }
            Command::SwapDown => {
                self.swap_focused_with(crate::layout::PreselectionDir::Down);
            }
            Command::ZoomToggle | Command::Fullscreen => {
                if let Err(e) = self.toggle_zoom_internal() {
                    self.notify(e, "error");
                }
            }
            Command::RenameWindow => self.open_rename_dialog(),
            Command::CopyMode => self.enter_scrollback_mode(),
            Command::ToggleSidebar => self.sidebar.toggle(),
            Command::OpenBrowser => self.open_scrollback_browser(),
            Command::OpenAggregate => self.open_aggregate_view(),
            Command::CommandPalette => self.open_palette(),
            Command::SessionSwitcher => {
                self.open_switcher(SwitcherKind::Session);
            }
            Command::WorkspaceSwitcher => {
                self.open_switcher(SwitcherKind::Workspace);
            }
            Command::LayoutSwitcher => self.open_switcher(SwitcherKind::Layout),
            Command::TapeManager => self.open_tape_manager(),
            Command::AccentPicker => self.open_accent_picker(),
            Command::Detach => self.leave_terminal_mode(),
            Command::Help => self.toggle_help(),
            Command::StackPane => self.stack_focused(),
            Command::CycleStack => self.cycle_stack_focus(true),
            Command::MultiSelect => self.toggle_multi_select_mode(),
            Command::BulkClose => self.bulk_close_selected(),
            Command::BulkStack => self.bulk_stack_selected(),
            Command::BulkBreak => self.bulk_break_selected(),
            Command::CustomAction(name) => self.run_custom_action(&name),
        }
    }

    pub fn default_shell(&self) -> String {
        if !self.config.appearance.preferred_shell.is_empty() {
            return self.config.appearance.preferred_shell.clone();
        }
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
    }

    // -----------------------------------------------------------------------
    // Switcher (workspace / window)
    // -----------------------------------------------------------------------

    pub fn open_switcher(&mut self, kind: SwitcherKind) {
        self.switcher_kind = kind;
        self.switcher_query.clear();
        self.switcher_selected = 0;
        self.switcher_open = true;
    }

    pub fn close_switcher(&mut self) {
        self.switcher_open = false;
        self.switcher_query.clear();
        self.switcher_selected = 0;
    }

    /// The switcher rows matching the current query.
    pub fn switcher_items(&self) -> Vec<SwitcherEntry> {
        let items = match self.switcher_kind {
            SwitcherKind::Workspace => {
                let mut items = Vec::new();
                for i in 1..=9 {
                    let ws = self.workspace(i);
                    let ids = ws.tree.get_all_window_ids();
                    let focused_title = ws
                        .focused
                        .and_then(|f| self.windows.get(f))
                        .map(|w| w.title.clone())
                        .unwrap_or_default();
                    let detail = if ids.is_empty() {
                        "(empty)".to_string()
                    } else {
                        format!("{} window(s) — {}", ids.len(), focused_title)
                    };
                    items.push(SwitcherEntry {
                        label: format!("{i}: {}", ws.name),
                        detail,
                        workspace: i,
                        window: ws.focused,
                        session: None,
                    });
                }
                items
            }
            SwitcherKind::Window => {
                let mut items = Vec::new();
                for (idx, window) in self.windows.iter().enumerate() {
                    let mut ws_num = 0;
                    for i in 1..=9 {
                        if self.workspace(i).tree.has_window(idx as i32) {
                            ws_num = i;
                            break;
                        }
                    }
                    items.push(SwitcherEntry {
                        label: window.title.clone(),
                        detail: format!("workspace {ws_num}"),
                        workspace: ws_num,
                        window: Some(idx),
                        session: None,
                    });
                }
                items
            }
            SwitcherKind::Layout => {
                let mut items: Vec<SwitcherEntry> = self
                    .layouts
                    .keys()
                    .map(|name| SwitcherEntry {
                        label: name.clone(),
                        detail: "saved layout — Enter to apply, x to delete".into(),
                        workspace: 0,
                        window: None,
                        session: None,
                    })
                    .collect();
                items.sort_by(|a, b| a.label.cmp(&b.label));
                items
            }
            SwitcherKind::Session => {
                let mut items = Vec::new();
                for s in &self.remote_sessions {
                    let mut suffix = String::new();
                    if Some(&s.name) == self.remote_session.as_ref() {
                        suffix.push_str(" (current)");
                    } else if s.attached {
                        suffix.push_str(" (attached)");
                    }
                    items.push(SwitcherEntry {
                        label: format!("{}{}", s.name, suffix),
                        detail: format!("{} window(s)", s.windows),
                        workspace: 0,
                        window: None,
                        session: Some(s.name.clone()),
                    });
                }
                items
            }
        };
        let mut items: Vec<(usize, SwitcherEntry)> = items
            .into_iter()
            .filter_map(|e| fuzzy_rank(&e.label, &self.switcher_query).map(|r| (r, e)))
            .collect();
        items.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.label.cmp(&b.1.label)));
        items.into_iter().map(|(_, e)| e).collect()
    }

    /// Apply a saved layout to the current workspace (`layouts` map).
    pub fn apply_saved_layout(&mut self, name: &str) -> bool {
        if let Some(serialized) = self.layouts.get(name) {
            let tree = BSPTree::deserialize(serialized);
            if let Some(ws) = self.workspaces.get_mut(&self.current_workspace) {
                ws.tree = tree;
                self.notify(format!("applied layout '{name}'"), "info");
                self.log_action(&format!("load_layout {name}"));
                self.sync_window_sizes();
                return true;
            }
        }
        false
    }

    /// Delete a saved layout by name.
    pub fn delete_saved_layout(&mut self, name: &str) {
        self.layouts.remove(name);
        self.notify(format!("deleted layout '{name}'"), "info");
        self.log_action(&format!("delete_layout {name}"));
    }

    pub fn switcher_move(&mut self, delta: i32) {
        let len = self.switcher_items().len();
        if len == 0 {
            return;
        }
        let sel = self.switcher_selected as i32 + delta;
        self.switcher_selected = sel.rem_euclid(len as i32) as usize;
    }

    /// Activate the selected switcher row: switch workspace and focus window,
    /// or (for the session switcher) request a session switch.
    pub fn activate_switcher(&mut self) {
        let items = self.switcher_items();
        let entry = items.get(self.switcher_selected).cloned();
        self.close_switcher();
        if let Some(entry) = entry {
            if self.switcher_kind == SwitcherKind::Layout {
                if !entry.label.is_empty() {
                    self.apply_saved_layout(&entry.label);
                }
                return;
            }
            if let Some(session) = entry.session {
                self.pending_switch = Some(session);
                return;
            }
            if entry.workspace >= 1 {
                self.switch_workspace(entry.workspace);
            }
            if let Some(w) = entry.window {
                self.focus_window(w);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Scrollback / copy mode
    // -----------------------------------------------------------------------

    pub fn enter_scrollback_mode(&mut self) {
        let Some(i) = self.focused_window else {
            return;
        };
        self.scrollback_mode = true;
        self.mode = Mode::WindowManagement;
        self.prefix = Prefix::None;
        self.palette_open = false;
        self.switcher_open = false;
        self.copy_visual = false;
        self.selection = None;
        // Start the cursor at the live bottom line.
        if let Some(w) = self.windows.get(i) {
            if let Ok(emu) = w.emulator.lock() {
                let count = emu.content_line_count();
                self.copy_cursor_line = count.saturating_sub(1);
                self.copy_cursor_col = 0;
            }
        }
    }

    pub fn exit_scrollback_mode(&mut self) {
        self.scrollback_mode = false;
        self.copy_visual = false;
        self.copy_visual_line = false;
        self.copy_char_search = None;
        self.copy_search_typing = false;
        self.copy_search_query.clear();
        self.copy_search_state.clear();
        self.copy_count.reset();
        self.copy_pending_g = false;
        self.copy_pending_register = None;
        self.copy_pending_mark = None;
        self.selection = None;
        self.mouse_selecting = false;
        if let Some(i) = self.focused_window {
            if let Some(w) = self.windows.get(i) {
                if let Ok(mut emu) = w.emulator.lock() {
                    emu.reset_viewport();
                }
            }
        }
    }

    /// Whether the focused pane is currently scrolled back into history.
    pub fn focused_in_scrollback(&self) -> bool {
        self.focused_window
            .and_then(|i| self.windows.get(i))
            .and_then(|w| w.emulator.lock().ok())
            .map(|emu| emu.in_scrollback())
            .unwrap_or(false)
    }

    /// Scroll the focused pane's viewport (positive = back in history).
    pub fn scroll_focused_viewport(&mut self, delta: i32) {
        if let Some(i) = self.focused_window {
            if let Some(w) = self.windows.get(i) {
                if let Ok(mut emu) = w.emulator.lock() {
                    emu.scroll_viewport(delta);
                }
            }
        }
    }

    /// Scroll an arbitrary window's viewport (mouse wheel over any pane).
    pub fn scroll_window_viewport(&self, index: usize, delta: i32) {
        if let Some(w) = self.windows.get(index) {
            if let Ok(mut emu) = w.emulator.lock() {
                emu.scroll_viewport(delta);
            }
        }
    }

    // -- Copy-mode cursor & selection --------------------------------------

    /// Move the copy-mode cursor vertically by `delta` content lines,
    /// auto-scrolling the viewport to keep it visible.
    pub fn copy_move_line(&mut self, delta: i32) {
        let Some(i) = self.focused_window else {
            return;
        };
        let count = {
            let Some(w) = self.windows.get(i) else {
                return;
            };
            let Ok(emu) = w.emulator.lock() else {
                return;
            };
            emu.content_line_count()
        };
        if count == 0 {
            return;
        }
        let new = (self.copy_cursor_line as i64 + delta as i64).clamp(0, count as i64 - 1) as usize;
        self.copy_cursor_line = new;
        self.scroll_to_cursor(new);
        self.sync_selection_cursor();
    }

    /// Move the copy-mode cursor horizontally by `delta` columns.
    pub fn copy_move_col(&mut self, delta: i32) {
        self.copy_cursor_col = (self.copy_cursor_col + delta).max(0);
        self.sync_selection_cursor();
    }

    /// Move a copy-mode line motion by a vim count.
    pub fn copy_move_lines(&mut self, delta: i32, count: usize) {
        let distance = delta.saturating_mul(count.min(i32::MAX as usize) as i32);
        self.copy_move_line(distance);
    }

    /// Jump the copy-mode cursor to the oldest line.
    pub fn copy_top(&mut self) {
        self.copy_cursor_line = 0;
        self.scroll_to_cursor(0);
        self.sync_selection_cursor();
    }

    /// Jump the copy-mode cursor to the live bottom line.
    pub fn copy_bottom(&mut self) {
        let Some(i) = self.focused_window else {
            return;
        };
        if let Some(w) = self.windows.get(i) {
            if let Ok(mut emu) = w.emulator.lock() {
                let count = emu.content_line_count();
                self.copy_cursor_line = count.saturating_sub(1);
                self.copy_cursor_col = 0;
                emu.reset_viewport();
            }
        }
        self.sync_selection_cursor();
    }

    /// Adjust the viewport so `line` is visible, keeping it as close to the
    /// current view as possible.
    fn scroll_to_cursor(&mut self, line: usize) {
        let Some(i) = self.focused_window else {
            return;
        };
        let Some(w) = self.windows.get(i) else {
            return;
        };
        let Ok(mut emu) = w.emulator.lock() else {
            return;
        };
        let sb = emu.scrollback_len();
        let h = emu.height() as usize;
        if h == 0 {
            return;
        }
        let viewport = emu.viewport();
        let view_start = sb.saturating_sub(viewport);
        let new_viewport = if line < view_start {
            sb.saturating_sub(line)
        } else if line >= view_start + h {
            sb.saturating_sub(line) + h - 1
        } else {
            viewport
        };
        emu.set_viewport(new_viewport);
    }

    fn sync_selection_cursor(&mut self) {
        if self.copy_visual {
            if let Some(sel) = &mut self.selection {
                sel.cursor_line = self.copy_cursor_line;
                sel.cursor_col = self.copy_cursor_col;
            }
        }
    }

    /// Toggle vim visual selection anchored at the copy-mode cursor.
    /// `line_wise` true for `V` (line-wise), false for `v` (char-wise).
    pub fn toggle_visual(&mut self, line_wise: bool) {
        let Some(i) = self.focused_window else {
            return;
        };
        if self.copy_visual {
            self.copy_visual = false;
            self.copy_visual_line = false;
            self.selection = None;
        } else {
            self.copy_visual = true;
            self.copy_visual_line = line_wise;
            self.selection = Some(Selection {
                window: i,
                anchor_line: self.copy_cursor_line,
                anchor_col: self.copy_cursor_col,
                cursor_line: self.copy_cursor_line,
                cursor_col: self.copy_cursor_col,
            });
        }
    }

    /// Open the context menu at a cell (right-click), focusing the window
    /// under the cursor first.
    pub fn open_context_menu_at(&mut self, x: i32, y: i32) {
        if let Some(idx) = self.window_at(x, y) {
            self.focus_window(idx);
        }
        self.context_menu = Some(ContextMenu {
            x,
            y,
            selected: 0,
            items: ContextMenu::standard(),
        });
    }

    /// Open the rename dialog for the focused window.
    pub fn open_rename_dialog(&mut self) {
        let Some(index) = self.focused_window else {
            return;
        };
        let title = self
            .windows
            .get(index)
            .map(|w| w.title.clone())
            .unwrap_or_default();
        self.rename_dialog = Some((index, title));
    }

    /// Commit the rename dialog text to the window title.
    pub fn commit_rename_dialog(&mut self) {
        if let Some((index, text)) = self.rename_dialog.take() {
            let text = text.trim().to_string();
            if !text.is_empty() {
                self.rename_window(index, &text);
                self.log_action(&format!("rename_window {text}"));
            }
        }
    }

    /// Cancel the rename dialog without applying.
    pub fn cancel_rename_dialog(&mut self) {
        self.rename_dialog = None;
    }

    /// Open the "New command pane" text-input dialog.
    pub fn open_command_pane_dialog(&mut self) {
        self.command_pane_dialog = Some((String::new(), false));
    }

    /// Commit the command-pane dialog text: spawn the command as a command
    /// pane. The suspended flag comes from the dialog's current toggle state.
    pub fn commit_command_pane_dialog_inner(&mut self) {
        if let Some((text, suspended)) = self.command_pane_dialog.take() {
            let text = text.trim().to_string();
            if text.is_empty() {
                return;
            }
            match self.spawn_command_window(&text, suspended) {
                Ok(_) => self.log_action(&format!("command_pane {text}")),
                Err(e) => self.notify(format!("command pane: {e}"), "error"),
            }
        }
    }

    /// Toggle the suspended flag in the command-pane dialog.
    pub fn toggle_command_pane_suspended(&mut self) {
        if let Some((_, ref mut suspended)) = self.command_pane_dialog {
            *suspended = !*suspended;
        }
    }

    /// Cancel the command-pane dialog without spawning.
    pub fn cancel_command_pane_dialog(&mut self) {
        self.command_pane_dialog = None;
    }

    /// Re-run the focused window if it is a finished command pane.
    /// Returns `true` when a re-run was triggered.
    pub fn rerun_focused_command_pane(&mut self) -> bool {
        let Some(i) = self.focused_window else {
            return false;
        };
        if !self.windows[i].can_rerun() {
            return false;
        }
        let size = self.windows[i]
            .last_geometry()
            .map(|g| WinSize {
                cols: g.width.max(1) as u16,
                rows: g.height.max(1) as u16,
            })
            .unwrap_or(WinSize { cols: 80, rows: 24 });
        let env = crate::util::guestenv::base_guest_env("local", &self.windows[i].id, false, false);
        let wake = Box::new(|| {}) as Box<dyn Fn() + Send + 'static>;
        match self.windows[i].restart(size, wake, &env) {
            Ok(()) => {
                let title = self.windows[i].title.clone();
                self.notify(format!("re-ran {title}"), "info");
                self.log_action("command_pane_rerun");
                true
            }
            Err(e) => {
                self.notify(format!("command pane: {e}"), "error");
                true
            }
        }
    }

    /// Resume the focused window if it is a suspended command pane.
    /// Returns `true` when a pane was resumed.
    pub fn resume_focused_suspended_pane(&mut self) -> bool {
        let Some(i) = self.focused_window else {
            return false;
        };
        self.windows[i].resume_if_suspended()
    }

    /// Poll every command pane for a finished child and record its exit
    /// status. Called from the render path each frame.
    pub fn poll_window_exits(&mut self) {
        let mut finished: Vec<(String, i32)> = Vec::new();
        for w in &mut self.windows {
            if w.poll_exit() {
                finished.push((w.title.clone(), w.exit_code.unwrap_or(-1)));
            }
        }
        for (title, code) in finished {
            let level = if code == 0 { "success" } else { "error" };
            self.notify(format!("command pane '{title}' finished (exit {code})"), level);
        }
    }

    /// Open the scrollback browser for the focused window: parse its
    /// semantic markers into command blocks (prompt fallback when no markers).
    pub fn open_scrollback_browser(&mut self) {
        let Some(i) = self.focused_window else { return };
        let Some(window) = self.windows.get(i) else {
            return;
        };
        let (markers, count) = {
            let Ok(emu) = window.emulator.lock() else {
                return;
            };
            let markers = emu.semantic_markers().markers();
            let count = emu.content_line_count();
            (markers, count)
        };
        let text = |line: usize| {
            self.windows
                .get(i)
                .and_then(|w| w.emulator.lock().ok())
                .map(|emu| emu.content_line_text(line))
                .unwrap_or_default()
        };
        self.browser_blocks = crate::scrollback::parse_blocks(&markers, count, text);
        self.browser_selected = 0;
        self.browser_scroll = 0;
        self.browser_open = true;
    }

    /// Close the scrollback browser.
    pub fn close_scrollback_browser(&mut self) {
        self.browser_open = false;
        self.browser_blocks.clear();
        self.browser_selected = 0;
        self.browser_scroll = 0;
    }

    /// Cycle the browser display mode (Commands/Output/JSON/Paths).
    pub fn cycle_browser_mode(&mut self) {
        use crate::scrollback::BrowseMode;
        self.browser_mode = match self.browser_mode {
            BrowseMode::Commands => BrowseMode::Output,
            BrowseMode::Output => BrowseMode::Json,
            BrowseMode::Json => BrowseMode::Paths,
            BrowseMode::Paths => BrowseMode::Commands,
        };
        self.browser_scroll = 0;
    }

    /// The text rows for the selected block in the current mode.
    pub fn browser_rows(&self) -> Vec<String> {
        use crate::scrollback::BrowseMode;
        match self.browser_mode {
            BrowseMode::Commands => self
                .browser_blocks
                .iter()
                .enumerate()
                .map(|(i, b)| {
                    let marker = if i == self.browser_selected {
                        "› "
                    } else {
                        "  "
                    };
                    format!("{marker}{}", b.command)
                })
                .collect(),
            BrowseMode::Output => {
                let mut rows: Vec<String> = Vec::new();
                for (i, b) in self.browser_blocks.iter().enumerate() {
                    if i == self.browser_selected {
                        rows.push(format!("── {} ──", b.command));
                        rows.extend(b.output.lines().map(|l| l.to_string()));
                    }
                }
                if rows.is_empty() {
                    rows.push("(no output)".into());
                }
                rows
            }
            BrowseMode::Json => {
                let mut rows: Vec<String> = Vec::new();
                for (i, b) in self.browser_blocks.iter().enumerate() {
                    if i == self.browser_selected {
                        rows.push(format!("── {} ──", b.command));
                        for frag in crate::scrollback::extract_json(&b.output) {
                            rows.push(frag);
                        }
                    }
                }
                if rows.is_empty() {
                    rows.push("(no JSON found)".into());
                }
                rows
            }
            BrowseMode::Paths => {
                let mut rows: Vec<String> = Vec::new();
                for (i, b) in self.browser_blocks.iter().enumerate() {
                    if i == self.browser_selected {
                        rows.push(format!("── {} ──", b.command));
                        for p in crate::scrollback::extract_paths(&b.output) {
                            rows.push(p);
                        }
                    }
                }
                if rows.is_empty() {
                    rows.push("(no paths found)".into());
                }
                rows
            }
        }
    }

    /// The sidebar rail rows for the current state.
    pub fn sidebar_rows(&self) -> Vec<sidebar::SidebarRow> {
        let ws_of = |idx: usize| {
            for i in 1..=9 {
                if self.workspace(i).tree.has_window(idx as i32) {
                    return i;
                }
            }
            0
        };
        sidebar::build_rows(
            self.remote_session.as_deref(),
            &self.remote_sessions,
            &self.windows,
            self.current_workspace,
            ws_of,
        )
    }

    /// Activate the selected sidebar row: switch to the session (daemon mode)
    /// or focus the window.
    pub fn activate_sidebar_selection(&mut self) {
        let rows = self.sidebar_rows();
        let Some(row) = rows.get(self.sidebar.selected) else {
            self.sidebar.close();
            return;
        };
        self.sidebar.close();
        if let Some(session) = &row.session {
            self.pending_switch = Some(session.clone());
            return;
        }
        if let Some(idx) = row.window {
            if row.workspace >= 1 {
                self.switch_workspace(row.workspace);
            }
            self.focus_window(idx);
        }
    }

    /// Open the aggregate view.
    pub fn open_aggregate_view(&mut self) {
        self.aggregate_open = true;
        self.aggregate_selected = 0;
    }

    /// Close the aggregate view.
    pub fn close_aggregate_view(&mut self) {
        self.aggregate_open = false;
        self.aggregate_selected = 0;
    }

    /// Every window across every workspace, grouped by workspace: returns
    /// (workspace, window index, title, first content line).
    pub fn aggregate_items(&self) -> Vec<(i32, usize, String, String)> {
        let mut items = Vec::new();
        for ws in 1..=9 {
            let ids = self.workspace(ws).tree.get_all_window_ids();
            if ids.is_empty() && self.floats_on_workspace(ws).is_empty() {
                continue;
            }
            let mut windows: Vec<usize> = ids.iter().map(|&i| i as usize).collect();
            for fi in self.floats_on_workspace(ws) {
                windows.push(self.floats[fi].window);
            }
            for idx in windows {
                let Some(window) = self.windows.get(idx) else {
                    continue;
                };
                let preview = window
                    .emulator
                    .lock()
                    .ok()
                    .and_then(|emu| {
                        (0..emu.content_line_count())
                            .map(|i| emu.content_line_text(i))
                            .find(|l| !l.trim().is_empty())
                    })
                    .unwrap_or_default();
                let cwd = window.cwd();
                let detail = if cwd.is_empty() {
                    preview
                } else {
                    format!("[{}] {}", cwd, preview)
                };
                items.push((ws, idx, window.title.clone(), detail));
            }
        }
        items
    }

    /// Activate the selected aggregate row: switch to its workspace and focus
    /// the window.
    pub fn activate_aggregate_selection(&mut self) {
        let items = self.aggregate_items();
        let Some((ws, idx, _, _)) = items.get(self.aggregate_selected) else {
            self.close_aggregate_view();
            return;
        };
        self.close_aggregate_view();
        self.switch_workspace(*ws);
        self.focus_window(*idx);
    }

    /// Open the quit menu, building rows from the session state (daemon vs
    /// standalone). The first row is always the safe default.
    pub fn open_quit_menu(&mut self) {
        let busy = self.windows.iter().any(|w| !w.exited);
        let items = if self.remote_session.is_some() {
            let has_others = self
                .remote_sessions
                .iter()
                .any(|s| Some(&s.name) != self.remote_session.as_ref());
            let mut items = vec![QuitMenuItem {
                label: "Detach — leave session running".into(),
                key: 'D',
                kind: QuitMenuKind::Detach,
                warn: false,
            }];
            if has_others {
                items.push(QuitMenuItem {
                    label: "Switch session".into(),
                    key: 'S',
                    kind: QuitMenuKind::SwitchSession,
                    warn: false,
                });
                items.push(QuitMenuItem {
                    label: "Kill this session and quit".into(),
                    key: 'K',
                    kind: QuitMenuKind::KillAndQuit,
                    warn: busy,
                });
            } else {
                items.push(QuitMenuItem {
                    label: "Kill this session and quit".into(),
                    key: 'K',
                    kind: QuitMenuKind::KillAndQuit,
                    warn: busy,
                });
            }
            items.push(QuitMenuItem {
                label: "Cancel".into(),
                key: 'C',
                kind: QuitMenuKind::Cancel,
                warn: false,
            });
            items
        } else {
            vec![
                QuitMenuItem {
                    label: "Quit".into(),
                    key: 'Q',
                    kind: QuitMenuKind::Standalone,
                    warn: false,
                },
                QuitMenuItem {
                    label: "Cancel".into(),
                    key: 'C',
                    kind: QuitMenuKind::Cancel,
                    warn: false,
                },
            ]
        };
        self.quit_menu = Some(QuitMenu { selected: 0, items });
    }

    /// Dismiss the quit menu without running anything.
    pub fn close_quit_menu(&mut self) {
        self.quit_menu = None;
    }

    /// Run the selected quit-menu row. Returns `true` when the client should
    /// quit.
    pub fn run_quit_menu_selection(&mut self) -> bool {
        let Some(menu) = self.quit_menu.take() else {
            return false;
        };
        let Some(item) = menu.items.get(menu.selected) else {
            return false;
        };
        match item.kind {
            QuitMenuKind::Detach | QuitMenuKind::Standalone => {
                self.quitting = true;
                true
            }
            QuitMenuKind::SwitchSession => {
                self.open_switcher(SwitcherKind::Session);
                false
            }
            QuitMenuKind::KillAndQuit => {
                if let Some(current) = self.remote_session.clone() {
                    self.pending_kill = Some(current);
                    self.quit_after_kill = true;
                } else {
                    self.quitting = true;
                    return true;
                }
                false
            }
            QuitMenuKind::Cancel => false,
        }
    }

    /// Open the session-close confirmation for a session. Raised every time,
    /// whatever the session holds; the toll line counts what would be lost.
    pub fn open_session_close(&mut self, session: &str) {
        self.session_close = Some((session.to_string(), 0));
    }

    /// Cancel the session-close confirmation.
    pub fn cancel_session_close(&mut self) {
        self.session_close = None;
    }

    /// The toll closing `session` would take: panes and agent-marked windows.
    pub fn session_toll(&self, session: &str) -> (usize, usize) {
        let windows = self
            .remote_sessions
            .iter()
            .find(|s| s.name == session)
            .map(|s| s.windows)
            .unwrap_or(0);
        // Agent-marked windows are only known locally for the attached
        // session; remote sessions report their count from the listing.
        let agents = if self.remote_session.as_deref() == Some(session) {
            self.windows
                .iter()
                .filter(|w| !w.agent_state.is_empty())
                .count()
        } else {
            0
        };
        (windows, agents)
    }

    /// Confirm the close: request the kill.
    pub fn confirm_session_close(&mut self) {
        if let Some((session, _)) = self.session_close.take() {
            self.pending_kill = Some(session);
        }
    }

    /// Dismiss the context menu (also called by the next click anywhere).
    pub fn dismiss_context_menu(&mut self) {
        self.context_menu = None;
    }

    /// Run the selected context-menu action.
    pub fn run_context_action(&mut self, action: ContextAction) {
        let shell = self.default_shell();
        match action {
            ContextAction::NewWindow => {
                let wake = Box::new(|| {});
                let _ = self.split(crate::layout::SplitType::None, &shell, wake);
            }
            ContextAction::SplitHorizontal => {
                let wake = Box::new(|| {});
                let _ = self.split(crate::layout::SplitType::Horizontal, &shell, wake);
            }
            ContextAction::SplitVertical => {
                let wake = Box::new(|| {});
                let _ = self.split(crate::layout::SplitType::Vertical, &shell, wake);
            }
            ContextAction::CloseWindow => self.close_focused_window(),
            ContextAction::Rename => self.open_rename_dialog(),
            ContextAction::Zoom => {
                let _ = self.toggle_zoom_internal();
            }
            ContextAction::Copy => self.yank_selection(),
            ContextAction::Paste => {
                let text = self.clipboard.clone();
                if !text.is_empty() {
                    self.write_to_focused(text.as_bytes());
                }
            }
            ContextAction::Cancel => {}
        }
    }

    /// Copy the current selection to the clipboard and the host terminal via
    /// OSC 52.
    pub fn yank_selection(&mut self) {
        let Some(sel) = self.selection.clone() else {
            return;
        };
        let Some(w) = self.windows.get(sel.window) else {
            return;
        };
        let text = {
            let Ok(emu) = w.emulator.lock() else {
                return;
            };
            emu.selection_text(
                sel.anchor_line,
                sel.anchor_col,
                sel.cursor_line,
                sel.cursor_col,
            )
        };
        self.selection = None;
        self.copy_visual = false;
        self.mouse_selecting = false;
        self.clipboard = text.clone();
        if !text.is_empty() {
            Self::emit_osc52(&text);
        }
        self.notify(format!("yanked {} char(s)", text.chars().count()), "info");
    }

    /// Emit an OSC 52 clipboard-write to the host terminal.
    fn emit_osc52(text: &str) {
        use base64::Engine;
        use std::io::Write;
        let b64 = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
        let seq = format!("\x1b]52;c;{b64}\x07");
        let mut out = std::io::stdout();
        let _ = out.write_all(seq.as_bytes());
        let _ = out.flush();
    }

    // -- Copy-mode word/char/line motions -----------------------------------

    /// Get the text of the content line at `line` (or empty if out of range).
    fn copy_line_text(&self, line: usize) -> String {
        let Some(i) = self.focused_window else {
            return String::new();
        };
        let Some(w) = self.windows.get(i) else {
            return String::new();
        };
        let Ok(emu) = w.emulator.lock() else {
            return String::new();
        };
        emu.content_line_text(line)
    }

    /// Move cursor to the first non-blank column of the current line.
    pub fn copy_first_non_blank(&mut self) {
        let text = self.copy_line_text(self.copy_cursor_line);
        let col = text
            .char_indices()
            .skip_while(|(_, c)| c.is_whitespace())
            .map(|(i, _)| i as i32)
            .next()
            .unwrap_or(0);
        self.copy_cursor_col = col;
        self.sync_selection_cursor();
    }

    /// Move cursor to the last non-blank column of the current line.
    pub fn copy_last_non_blank(&mut self) {
        let text = self.copy_line_text(self.copy_cursor_line);
        let trimmed = text.trim_end();
        let col = trimmed.chars().count() as i32;
        self.copy_cursor_col = col;
        self.sync_selection_cursor();
    }

    /// Move cursor to column 0.
    pub fn copy_col_zero(&mut self) {
        self.copy_cursor_col = 0;
        self.sync_selection_cursor();
    }

    /// Move to the next word start (`w` motion).
    /// `big` true uses whitespace-only word boundaries (`W`).
    pub fn copy_word_forward(&mut self, big: bool) {
        let text = self.copy_line_text(self.copy_cursor_line);
        if self.copy_cursor_col as usize >= text.chars().count() {
            // At end of line — move to next line.
            self.copy_move_line(1);
            self.copy_cursor_col = 0;
            self.sync_selection_cursor();
            return;
        }
        let motion = if big {
            copymode_ext::WordMotion::WordForwardBig
        } else {
            copymode_ext::WordMotion::WordForward
        };
        self.copy_cursor_col =
            copymode_ext::word_motion(&text, self.copy_cursor_col as usize, motion) as i32;
        self.sync_selection_cursor();
    }

    /// Move to the previous word start (`b` motion).
    pub fn copy_word_backward(&mut self, big: bool) {
        let text = self.copy_line_text(self.copy_cursor_line);
        if text.chars().count() == 0 {
            self.copy_move_line(-1);
            self.copy_last_non_blank();
            return;
        }
        let motion = if big {
            copymode_ext::WordMotion::WordBackwardBig
        } else {
            copymode_ext::WordMotion::WordBackward
        };
        self.copy_cursor_col =
            copymode_ext::word_motion(&text, self.copy_cursor_col as usize, motion) as i32;
        self.sync_selection_cursor();
    }

    /// Move to the next word end (`e` motion).
    pub fn copy_word_end(&mut self, big: bool) {
        let text = self.copy_line_text(self.copy_cursor_line);
        let motion = if big {
            copymode_ext::WordMotion::WordEndBig
        } else {
            copymode_ext::WordMotion::WordEnd
        };
        self.copy_cursor_col =
            copymode_ext::word_motion(&text, self.copy_cursor_col as usize, motion) as i32;
        self.sync_selection_cursor();
    }

    /// Move to the next/previous occurrence of `target` on the current line.
    /// `forward` true = `f`/`t`, false = `F`/`T`.
    /// `till` true = `t`/`T` (stop before), false = `f`/`F` (land on).
    /// Delegates to `copymode_ext::find_char_on_line` (wide-character aware).
    pub fn copy_char_search(&mut self, target: char, forward: bool, till: bool) {
        let text = self.copy_line_text(self.copy_cursor_line);
        let search = if forward {
            if till {
                copymode_ext::CharSearch::forward_till(target)
            } else {
                copymode_ext::CharSearch::forward_find(target)
            }
        } else if till {
            copymode_ext::CharSearch::backward_till(target)
        } else {
            copymode_ext::CharSearch::backward_find(target)
        };
        if let Some(col) =
            copymode_ext::find_char_on_line(&text, self.copy_cursor_col as usize, &search)
        {
            self.copy_cursor_col = col as i32;
            self.sync_selection_cursor();
            self.copy_last_char_search = Some((target, forward, till));
        }
    }

    /// Repeat the last char search (`;`), optionally reversed (`,`).
    pub fn copy_char_search_repeat(&mut self, reverse: bool) {
        if let Some((target, forward, till)) = self.copy_last_char_search {
            let fwd = if reverse { !forward } else { forward };
            self.copy_char_search(target, fwd, till);
        }
    }

    /// Jump to the matching bracket (`%` motion), if the cursor is on one.
    pub fn copy_bracket_match(&mut self) {
        let text = self.copy_line_text(self.copy_cursor_line);
        if let Some(col) = copymode_ext::find_matching_bracket(&text, self.copy_cursor_col as usize)
        {
            self.copy_cursor_col = col as i32;
            self.sync_selection_cursor();
        }
    }

    /// Execute the pending regex search.
    pub fn copy_execute_search(&mut self) {
        if self.copy_search_query.is_empty() {
            self.copy_search_typing = false;
            return;
        }
        let query = self.copy_search_query.clone();
        let forward = self.copy_search_forward;
        self.copy_search_typing = false;
        self.copy_search_next_match(&query, forward, false);
    }

    /// Find the next/previous match of `query` from the current cursor.
    pub fn copy_search_next_match(&mut self, query: &str, forward: bool, reverse: bool) {
        let fwd = if reverse { !forward } else { forward };
        let Ok(re) = regex::Regex::new(query) else {
            return;
        };
        let count = {
            let Some(i) = self.focused_window else {
                return;
            };
            let Some(w) = self.windows.get(i) else {
                return;
            };
            let Ok(emu) = w.emulator.lock() else {
                return;
            };
            emu.content_line_count()
        };
        let start_line = self.copy_cursor_line;
        let start_col = self.copy_cursor_col as usize;
        if fwd {
            // Search forward from current position.
            for line in start_line..count {
                let text = self.copy_line_text(line);
                let start = if line == start_line { start_col + 1 } else { 0 };
                if let Some(m) = re.find_at(&text, start) {
                    self.copy_cursor_line = line;
                    self.copy_cursor_col = m.start() as i32;
                    self.scroll_to_cursor(line);
                    self.sync_selection_cursor();
                    return;
                }
            }
        } else {
            // Search backward from current position.
            for line in (0..=start_line).rev() {
                let text = self.copy_line_text(line);
                let end = if line == start_line {
                    start_col
                } else {
                    text.len()
                };
                if let Some(pos) = re.find_iter(&text[..end]).last() {
                    self.copy_cursor_line = line;
                    self.copy_cursor_col = pos.start() as i32;
                    self.scroll_to_cursor(line);
                    self.sync_selection_cursor();
                    return;
                }
            }
        }
    }

    /// Move cursor to top of viewport (`H`).
    pub fn copy_viewport_top(&mut self) {
        if let Some(i) = self.focused_window {
            if let Some(w) = self.windows.get(i) {
                if let Ok(emu) = w.emulator.lock() {
                    let sb = emu.scrollback_len();
                    let viewport = emu.viewport();
                    self.copy_cursor_line = sb.saturating_sub(viewport);
                }
            }
        }
        self.sync_selection_cursor();
    }

    /// Move cursor to middle of viewport (`M`).
    pub fn copy_viewport_middle(&mut self) {
        if let Some(i) = self.focused_window {
            if let Some(w) = self.windows.get(i) {
                if let Ok(emu) = w.emulator.lock() {
                    let sb = emu.scrollback_len();
                    let viewport = emu.viewport();
                    let h = emu.height() as usize;
                    self.copy_cursor_line = sb.saturating_sub(viewport) + h / 2;
                }
            }
        }
        self.sync_selection_cursor();
    }

    /// Move cursor to bottom of viewport (`L`).
    pub fn copy_viewport_bottom(&mut self) {
        if let Some(i) = self.focused_window {
            if let Some(w) = self.windows.get(i) {
                if let Ok(emu) = w.emulator.lock() {
                    let sb = emu.scrollback_len();
                    let viewport = emu.viewport();
                    let h = emu.height() as usize;
                    self.copy_cursor_line = sb.saturating_sub(viewport) + h.saturating_sub(1);
                }
            }
        }
        self.sync_selection_cursor();
    }

    /// Move to the next/previous blank line (`}`/`{`).
    pub fn copy_blank_line(&mut self, forward: bool) {
        let count = {
            let Some(i) = self.focused_window else {
                return;
            };
            let Some(w) = self.windows.get(i) else {
                return;
            };
            let Ok(emu) = w.emulator.lock() else {
                return;
            };
            emu.content_line_count()
        };
        if count == 0 {
            return;
        }
        let delta = if forward { 1 } else { -1 };
        let mut line = self.copy_cursor_line as i64 + delta;
        while line >= 0 && (line as usize) < count {
            let text = self.copy_line_text(line as usize);
            if text.trim().is_empty() {
                self.copy_cursor_line = line as usize;
                self.copy_cursor_col = 0;
                self.scroll_to_cursor(line as usize);
                self.sync_selection_cursor();
                return;
            }
            line += delta;
        }
    }

    /// Move to the start of the next sentence (`)` motion).
    pub fn copy_sentence_next(&mut self) {
        let count = {
            let Some(i) = self.focused_window else {
                return;
            };
            let Some(w) = self.windows.get(i) else {
                return;
            };
            let Ok(emu) = w.emulator.lock() else {
                return;
            };
            emu.content_line_count()
        };
        let start_line = self.copy_cursor_line;
        let start_col = self.copy_cursor_col as usize;
        let text = |line: usize| self.copy_line_text(line);
        let (line, col) =
            copymode_ext::next_sentence(count, start_line, start_col, text);
        if line != start_line || col != start_col {
            self.copy_cursor_line = line;
            self.copy_cursor_col = col as i32;
            self.scroll_to_cursor(line);
            self.sync_selection_cursor();
        }
    }

    /// Move to the start of the previous sentence (`(` motion).
    pub fn copy_sentence_prev(&mut self) {
        let start_line = self.copy_cursor_line;
        let start_col = self.copy_cursor_col as usize;
        let text = |line: usize| self.copy_line_text(line);
        let (line, col) =
            copymode_ext::prev_sentence(start_line, start_col, text);
        if line != start_line || col != start_col {
            self.copy_cursor_line = line;
            self.copy_cursor_col = col as i32;
            self.scroll_to_cursor(line);
            self.sync_selection_cursor();
        }
    }

    /// Move to the start of the next paragraph (`}` motion, paragraph-aware).
    /// Unlike `copy_blank_line` which stops on the blank line, this skips
    /// past blank lines to the first non-blank line of the next paragraph.
    pub fn copy_paragraph_next(&mut self) {
        let count = {
            let Some(i) = self.focused_window else {
                return;
            };
            let Some(w) = self.windows.get(i) else {
                return;
            };
            let Ok(emu) = w.emulator.lock() else {
                return;
            };
            emu.content_line_count()
        };
        let start = self.copy_cursor_line;
        let text = |line: usize| self.copy_line_text(line);
        let target = copymode_ext::paragraph_forward(count, start, text);
        if target != start {
            self.copy_cursor_line = target;
            self.copy_cursor_col = 0;
            self.scroll_to_cursor(target);
            self.sync_selection_cursor();
        }
    }

    /// Move to the start of the previous paragraph (`{` motion, paragraph-aware).
    pub fn copy_paragraph_prev(&mut self) {
        let start = self.copy_cursor_line;
        let text = |line: usize| self.copy_line_text(line);
        let target = copymode_ext::paragraph_backward(start, text);
        if target != start {
            self.copy_cursor_line = target;
            self.copy_cursor_col = 0;
            self.scroll_to_cursor(target);
            self.sync_selection_cursor();
        }
    }

    /// Set a vim-style mark at the current cursor position (`m{letter}`).
    pub fn copy_set_mark(&mut self, letter: char) {
        self.copy_marks
            .set(letter, self.copy_cursor_line, self.copy_cursor_col as usize);
    }

    /// Jump the cursor to a vim-style mark (`'{letter}` or `` `{letter} ``).
    /// `exact_col` true for backtick (exact column), false for apostrophe
    /// (first non-blank column of the mark's line).
    pub fn copy_goto_mark(&mut self, letter: char, exact_col: bool) {
        if let Some(mark) = self.copy_marks.get(letter) {
            self.copy_cursor_line = mark.line;
            if exact_col {
                self.copy_cursor_col = mark.col as i32;
            } else {
                let text = self.copy_line_text(mark.line);
                let col = text
                    .char_indices()
                    .skip_while(|(_, c)| c.is_whitespace())
                    .map(|(i, _)| i as i32)
                    .next()
                    .unwrap_or(0);
                self.copy_cursor_col = col;
            }
            self.scroll_to_cursor(mark.line);
            self.sync_selection_cursor();
        }
    }

    /// Yank the current selection into a named register (or the unnamed
    /// register if `register` is `None`). Also copies to the clipboard and
    /// emits OSC 52.
    pub fn yank_selection_to_register(&mut self, register: Option<char>) {
        let Some(sel) = self.selection.clone() else {
            return;
        };
        let Some(w) = self.windows.get(sel.window) else {
            return;
        };
        let text = {
            let Ok(emu) = w.emulator.lock() else {
                return;
            };
            emu.selection_text(
                sel.anchor_line,
                sel.anchor_col,
                sel.cursor_line,
                sel.cursor_col,
            )
        };
        let cleaned = copymode_ext::clean_selection_text(&text);
        let kind = if self.copy_visual_line {
            copymode_ext::RegisterKind::Line
        } else {
            copymode_ext::RegisterKind::Char
        };
        self.copy_registers.yank(register, &cleaned, kind);
        self.selection = None;
        self.copy_visual = false;
        self.mouse_selecting = false;
        self.clipboard = cleaned.clone();
        if !cleaned.is_empty() {
            Self::emit_osc52(&cleaned);
        }
        let count = cleaned.chars().count();
        self.notify(format!("yanked {count} char(s)"), "info");
    }

    /// Execute the search using the consolidated search state, storing all
    /// matches for highlighting and navigation.
    pub fn copy_execute_search_state(&mut self) {
        if self.copy_search_query.is_empty() {
            self.copy_search_typing = false;
            self.copy_search_state.clear();
            return;
        }
        let count = {
            let Some(i) = self.focused_window else {
                return;
            };
            let Some(w) = self.windows.get(i) else {
                return;
            };
            let Ok(emu) = w.emulator.lock() else {
                return;
            };
            emu.content_line_count()
        };
        self.copy_search_state.query = self.copy_search_query.clone();
        self.copy_search_state.forward = self.copy_search_forward;
        let start_line = self.copy_cursor_line;
        let start_col = self.copy_cursor_col as usize;
        let lines: Vec<String> = {
            let mut lines = Vec::with_capacity(count);
            for i in 0..count {
                lines.push(self.copy_line_text(i));
            }
            lines
        };
        self.copy_search_state
            .execute(count, |line: usize| lines.get(line).cloned().unwrap_or_default());
        self.copy_search_typing = false;
        if let Some(m) = self.copy_search_state.jump_initial(start_line, start_col) {
            self.copy_cursor_line = m.line;
            self.copy_cursor_col = m.start as i32;
            self.scroll_to_cursor(m.line);
            self.sync_selection_cursor();
        }
    }

    /// Jump to the next match in the consolidated search state.
    pub fn copy_search_state_next(&mut self) {
        let m = self.copy_search_state.next();
        if let Some(m) = m {
            self.copy_cursor_line = m.line;
            self.copy_cursor_col = m.start as i32;
            self.scroll_to_cursor(m.line);
            self.sync_selection_cursor();
        }
    }

    /// Jump to the previous match in the consolidated search state.
    pub fn copy_search_state_prev(&mut self) {
        let m = self.copy_search_state.prev();
        if let Some(m) = m {
            self.copy_cursor_line = m.line;
            self.copy_cursor_col = m.start as i32;
            self.scroll_to_cursor(m.line);
            self.sync_selection_cursor();
        }
    }

    /// Clear search highlighting (vim's `:noh` / Ctrl+L).
    pub fn copy_clear_search(&mut self) {
        self.copy_search_state.clear();
        self.copy_search_query.clear();
    }

    /// The content position (line, column) under a screen cell coordinate for
    /// a window.
    pub fn content_position_at(
        &self,
        window: usize,
        column: i32,
        row: i32,
    ) -> Option<(usize, i32)> {
        let rect = if self.is_float(window) {
            self.float_rect(window)
        } else {
            self.current_layout().get(&(window as i32)).copied()
        }?;
        // The pane border consumes the outer ring; content starts one cell in.
        let rel_row = (row - rect.y - 1).max(0);
        let rel_col = (column - rect.x - 1).max(0);
        let w = self.windows.get(window)?;
        let emu = w.emulator.lock().ok()?;
        let line = emu.content_index_for_view_row(rel_row);
        // Terminal columns span wide glyphs, but selection coordinates are
        // emulator columns (leads only). A click on a wide char's second
        // column snaps to the glyph's lead so the char is selected whole;
        // clicks beyond the line's content keep their raw column.
        let mut snapped = rel_col;
        let mut col = 0i32;
        for cell in &emu.content_line(line) {
            let cell_w = cell.width.max(1) as i32;
            if rel_col < col + cell_w {
                snapped = col;
                break;
            }
            col += cell_w;
        }
        Some((line, snapped))
    }

    // -- Mouse selection ----------------------------------------------------

    pub fn begin_mouse_selection(&mut self, window: usize, column: i32, row: i32) {
        let Some((line, col)) = self.content_position_at(window, column, row) else {
            return;
        };
        self.mouse_selecting = true;
        self.copy_visual = false;
        self.selection = Some(Selection {
            window,
            anchor_line: line,
            anchor_col: col,
            cursor_line: line,
            cursor_col: col,
        });
    }

    pub fn extend_mouse_selection(&mut self, window: usize, column: i32, row: i32) {
        if !self.mouse_selecting {
            return;
        }
        let Some((line, col)) = self.content_position_at(window, column, row) else {
            return;
        };
        if let Some(sel) = &mut self.selection {
            if sel.window == window {
                sel.cursor_line = line;
                sel.cursor_col = col;
            }
        }
    }

    pub fn end_mouse_selection(&mut self) {
        self.mouse_selecting = false;
        let empty = self
            .selection
            .as_ref()
            .map(|s| s.is_empty())
            .unwrap_or(true);
        if empty {
            self.selection = None;
        } else {
            self.yank_selection();
        }
    }

    /// The window index under a cell coordinate (column, row), if any.
    pub fn window_at(&self, column: i32, row: i32) -> Option<usize> {
        let layout = self.current_layout();
        self.window_at_with_layout(column, row, &layout)
    }

    /// Like `window_at` but accepts a precomputed layout to avoid recomputing
    /// the BSP tree on every call.
    pub fn window_at_with_layout(
        &self,
        column: i32,
        row: i32,
        layout: &HashMap<i32, Rect>,
    ) -> Option<usize> {
        // Floating panes sit above the tiled layout: the topmost float wins.
        if let Some(idx) = self.float_at(column, row) {
            return Some(idx);
        }
        let mut best: Option<(usize, i32)> = None;
        for (&window_id, &rect) in layout {
            if column >= rect.x
                && column < rect.x + rect.w
                && row >= rect.y
                && row < rect.y + rect.h
            {
                // Topmost is approximated by largest area here (single layer).
                let area = rect.w * rect.h;
                if best.map(|(_, a)| area > a).unwrap_or(true) {
                    best = Some((window_id as usize, area));
                }
            }
        }
        best.map(|(idx, _)| idx)
    }

    /// Hit-test the dock bar: returns the window index if the click lands
    /// on a dock pill, or `None` otherwise.  Computes layout on demand.
    pub fn dock_item_at(&self, column: i32, row: i32) -> Option<usize> {
        let dock_position = self.config.appearance.dockbar_position.as_str();
        if dock_position == "hidden" {
            return None;
        }
        let dock_row = if dock_position == "top" {
            0
        } else {
            self.height - 1
        };
        if row != dock_row {
            return None;
        }
        let layout = crate::app::dock::calculate_dock_layout(self);
        for (i, item) in layout.visible_items.iter().enumerate() {
            let pill_x = layout.item_positions.get(i).copied().unwrap_or(0);
            let pill_w = item.width as i32;
            if column >= pill_x && column < pill_x + pill_w {
                return Some(item.window_index as usize);
            }
        }
        None
    }

    /// Detect if a screen coordinate is on a pane border (within 1 cell slop).
    /// Returns (window_id, edge) if the click is on a border between panes.
    /// Precomputes edge coordinate sets so neighbor checks are O(1) instead
    /// of O(n), making the overall function O(n) instead of O(n²).
    pub fn border_at_with_layout(
        &self,
        column: i32,
        row: i32,
        layout: &HashMap<i32, Rect>,
    ) -> Option<(i32, crate::layout::ResizeEdge)> {
        let slop = 1;
        // Precompute sets of all x and y coordinates that appear as rect
        // left edges or right edges, so neighbor checks are O(1).
        let mut left_edges: std::collections::HashSet<i32> = std::collections::HashSet::new();
        let mut right_edges: std::collections::HashSet<i32> = std::collections::HashSet::new();
        let mut top_edges: std::collections::HashSet<i32> = std::collections::HashSet::new();
        let mut bottom_edges: std::collections::HashSet<i32> = std::collections::HashSet::new();
        for rect in layout.values() {
            left_edges.insert(rect.x);
            right_edges.insert(rect.x + rect.w);
            top_edges.insert(rect.y);
            bottom_edges.insert(rect.y + rect.h);
        }
        for (&wid, &rect) in layout {
            // Right border: column is at rect.x + rect.w or rect.x + rect.w - 1
            if (column == rect.x + rect.w || column == rect.x + rect.w - 1)
                && row >= rect.y.saturating_sub(slop)
                && row < rect.y + rect.h + slop
                && left_edges.contains(&(rect.x + rect.w))
            {
                return Some((wid, crate::layout::ResizeEdge::Right));
            }
            // Bottom border
            if (row == rect.y + rect.h || row == rect.y + rect.h - 1)
                && column >= rect.x.saturating_sub(slop)
                && column < rect.x + rect.w + slop
                && top_edges.contains(&(rect.y + rect.h))
            {
                return Some((wid, crate::layout::ResizeEdge::Bottom));
            }
            // Left border
            if (column == rect.x || column == rect.x + 1)
                && row >= rect.y.saturating_sub(slop)
                && row < rect.y + rect.h + slop
                && right_edges.contains(&rect.x)
            {
                return Some((wid, crate::layout::ResizeEdge::Left));
            }
            // Top border
            if (row == rect.y || row == rect.y + 1)
                && column >= rect.x.saturating_sub(slop)
                && column < rect.x + rect.w + slop
                && bottom_edges.contains(&rect.y)
            {
                return Some((wid, crate::layout::ResizeEdge::Top));
            }
        }
        None
    }

    /// Begin a border-drag resize operation.
    pub fn begin_border_drag(&mut self, window_id: i32, edge: crate::layout::ResizeEdge, pos: i32) {
        self.drag_resize = Some((window_id, edge, pos));
    }

    /// Apply a border-drag resize to the current position.
    pub fn apply_border_drag(&mut self, pos: i32) {
        let Some((wid, edge, _start)) = self.drag_resize else {
            return;
        };
        let ws = self.current_workspace;
        let bounds = self.workspace_bounds(ws);
        let gap = self.gap;
        let tree = &mut self.workspace_mut(ws).tree;
        tree.resize_split(wid, edge, pos, bounds, gap);
    }

    /// End the border-drag resize.
    pub fn end_border_drag(&mut self) {
        self.drag_resize = None;
    }

    /// Select a word at the given screen position and return the selection text.
    pub fn select_word_at(&mut self, window: usize, column: i32, row: i32) {
        let Some((line, col)) = self.content_position_at(window, column, row) else {
            return;
        };
        let (text, cells) = {
            let Some(w) = self.windows.get(window) else {
                return;
            };
            let Ok(emu) = w.emulator.lock() else {
                return;
            };
            (emu.content_line_text(line), emu.content_line(line))
        };
        let chars: Vec<char> = text.chars().collect();
        if chars.is_empty() {
            return;
        }
        let pos = (col as usize).min(chars.len().saturating_sub(1));
        let is_word = |c: char| c.is_alphanumeric() || c == '_';
        if !is_word(chars[pos]) {
            // If not on a word char, select the single char.
            self.selection = Some(Selection {
                window,
                anchor_line: line,
                anchor_col: col,
                cursor_line: line,
                cursor_col: col + 1,
            });
            return;
        }
        // Find word boundaries in text-char space.
        let mut start = pos;
        while start > 0 && is_word(chars[start - 1]) {
            start -= 1;
        }
        let mut end = pos;
        while end + 1 < chars.len() && is_word(chars[end + 1]) {
            end += 1;
        }
        let end_excl = end + 1;
        // Convert the char range to column space: wide runes occupy two
        // columns per char, so the selection must span their full width.
        let mut char_idx = 0usize;
        let mut col_idx = 0i32;
        let mut start_col = 0i32;
        let mut end_col = col;
        for cell in &cells {
            if cell.width == 0 {
                continue;
            }
            if char_idx == start {
                start_col = col_idx;
            }
            if char_idx == end_excl {
                end_col = col_idx;
                break;
            }
            char_idx += 1;
            col_idx += cell.width.max(1) as i32;
        }
        if char_idx == end_excl {
            end_col = col_idx;
        }
        // end_col is exclusive; selection_text treats the end column as
        // inclusive, so back off by one (saturating for the empty case).
        let end_col_incl = end_col.saturating_sub(1);
        self.selection = Some(Selection {
            window,
            anchor_line: line,
            anchor_col: start_col,
            cursor_line: line,
            cursor_col: end_col_incl,
        });
    }

    /// Select the entire line at the given screen position.
    pub fn select_line_at(&mut self, window: usize, column: i32, row: i32) {
        let Some((line, _)) = self.content_position_at(window, column, row) else {
            return;
        };
        let width = {
            let Some(w) = self.windows.get(window) else {
                return;
            };
            let Ok(emu) = w.emulator.lock() else {
                return;
            };
            emu.width()
        };
        self.selection = Some(Selection {
            window,
            anchor_line: line,
            anchor_col: 0,
            cursor_line: line,
            // Inclusive end column: width - 1 covers the entire line.
            cursor_col: (width - 1).max(0),
        });
    }

    /// Open the settings overlay.
    pub fn open_settings(&mut self) {
        self.settings_open = true;
        self.settings_selected = 0;
    }

    /// Close the settings overlay.
    pub fn close_settings(&mut self) {
        self.settings_open = false;
        self.settings_selected = 0;
    }

    /// The settings rows with their current values.
    pub fn settings_rows(&self) -> Vec<(String, String)> {
        let theme = self
            .theme
            .as_ref()
            .map(|t| t.name.clone())
            .unwrap_or_else(|| self.config.appearance.theme.clone());
        let mut rows = vec![
            ("Theme".to_string(), theme),
            (
                "Animations".to_string(),
                if self.config.appearance.animations_enabled {
                    "on".into()
                } else {
                    "off".into()
                },
            ),
            (
                "Which-key overlay".to_string(),
                if self.config.appearance.which_key_enabled {
                    "on".into()
                } else {
                    "off".into()
                },
            ),
            (
                "Pane gap".to_string(),
                if self.gap > 0 {
                    self.gap.to_string()
                } else {
                    "off".into()
                },
            ),
        ];
        rows.push((
            "Scroll lines".to_string(),
            self.config.appearance.scroll_lines.to_string(),
        ));
        rows
    }

    /// Adjust the selected settings row: `delta` -1/0/+1 (left/enter/right).
    pub fn adjust_settings_row(&mut self, delta: i32) {
        let theme_names = crate::config::theme::Theme::built_in_names();
        let row = self.settings_selected;
        match row {
            0 => {
                // Cycle the theme.
                let current = self
                    .theme
                    .as_ref()
                    .map(|t| t.name.clone())
                    .unwrap_or_else(|| self.config.appearance.theme.clone());
                let idx = theme_names.iter().position(|n| *n == current).unwrap_or(0);
                let next = (idx as i32 + delta).rem_euclid(theme_names.len() as i32) as usize;
                let name = theme_names[next].to_string();
                self.theme = crate::config::Theme::built_in(&name);
                self.config.appearance.theme = name.clone();
                self.notify(format!("theme: {name}"), "info");
                self.log_action(&format!("set_theme {name}"));
            }
            1 => {
                if delta != 0 {
                    self.config.appearance.animations_enabled =
                        !self.config.appearance.animations_enabled;
                }
            }
            2 => {
                if delta != 0 {
                    self.config.appearance.which_key_enabled =
                        !self.config.appearance.which_key_enabled;
                }
            }
            3 => {
                self.gap = (self.gap + delta).max(0);
            }
            4 => {
                self.config.appearance.scroll_lines =
                    (self.config.appearance.scroll_lines + delta).max(1);
            }
            _ => {}
        }
    }

    pub fn open_theme_picker(&mut self) {
        self.theme_list = crate::config::theme::all_themes()
            .into_iter()
            .map(|t| t.name)
            .collect();
        self.theme_picker_open = true;
        self.theme_picker_selected = 0;
    }

    pub fn close_theme_picker(&mut self) {
        self.theme_picker_open = false;
    }

    /// Re-run host-terminal light/dark detection and swap the theme in place.
    ///
    /// Only acts when `theme = "auto"` is configured, and should be called
    /// with the terminal in raw mode so the OSC 11 reply is delivered. Leaves
    /// `config.appearance.theme` as `"auto"` so the next launch re-detects.
    pub fn redetect_theme(&mut self) {
        if !self.auto_theme {
            return;
        }
        let mode = crate::util::theme_detect::detect_terminal_mode();
        let name = crate::util::theme_detect::resolve_auto_theme_name(
            mode,
            &self.config.appearance.theme_auto_light,
            &self.config.appearance.theme_auto_dark,
        );
        let applied = Theme::built_in(&name);
        let unchanged = matches!(
            (&self.theme, &applied),
            (Some(cur), Some(next)) if cur.name == next.name
        );
        if unchanged {
            return;
        }
        let mode_name = match mode {
            Some(crate::util::theme_detect::ThemeMode::Light) => "light",
            _ => "dark",
        };
        self.theme = applied;
        self.damage_full(crate::app::damage::DamageReason::Theme);
        self.notify(format!("theme: {name} ({mode_name} detected)"), "info");
        self.log_action(&format!("theme_detect {name}"));
    }

    pub fn theme_picker_move(&mut self, delta: i32) {
        if self.theme_list.is_empty() {
            return;
        }
        let len = self.theme_list.len() as i32;
        let new = (self.theme_picker_selected as i32 + delta).rem_euclid(len) as usize;
        self.theme_picker_selected = new;
    }

    pub fn apply_selected_theme(&mut self) {
        if let Some(name) = self.theme_list.get(self.theme_picker_selected).cloned() {
            self.config.appearance.theme = name;
            self.close_theme_picker();
        }
    }

    // --- Accent picker ---

    pub fn open_accent_picker(&mut self) {
        self.accent_picker_open = true;
        self.accent_picker_selected = 0;
    }

    pub fn close_accent_picker(&mut self) {
        self.accent_picker_open = false;
    }

    pub fn accent_picker_move(&mut self, delta: i32) {
        if self.accent_list.is_empty() {
            return;
        }
        let len = self.accent_list.len() as i32;
        let new = (self.accent_picker_selected as i32 + delta).rem_euclid(len) as usize;
        self.accent_picker_selected = new;
    }

    pub fn apply_selected_accent(&mut self) {
        if let Some(name) = self.accent_list.get(self.accent_picker_selected).cloned() {
            // Store the accent color as the border_focused_color.
            self.config.appearance.border_focused_color = Some(name);
            self.close_accent_picker();
        }
    }

    /// Toggle the help modal overlay.
    pub fn toggle_help(&mut self) {
        self.help_open = !self.help_open;
    }

    /// Toggle the persistent key-hints bar.
    pub fn toggle_hints(&mut self) {
        self.hints_visible = !self.hints_visible;
    }

    /// Dismiss the welcome overlay.
    pub fn dismiss_welcome(&mut self) {
        self.show_welcome = false;
    }
}

/// Tape playback drives the app through this executor (ported from the Go
/// `os_tape_executor.go` interface implementation). Operations the port's
/// `Os` does not implement (snap, zoom, layout save/load, config paths)
/// report a clear error rather than silently doing nothing.
impl crate::tape::executor::TapeExecutor for Os {
    fn focused_window_id(&self) -> Option<String> {
        self.focused_window
            .and_then(|i| self.windows.get(i))
            .map(|w| w.id.clone())
    }

    fn send_to_window(&mut self, window_id: &str, data: &[u8]) -> Result<(), String> {
        match self.window_index_by_id(window_id) {
            Some(i) => {
                if let Some(w) = self.windows.get(i) {
                    w.write(data);
                    Ok(())
                } else {
                    Err(format!("window not found: {window_id}"))
                }
            }
            None => Err(format!("window not found: {window_id}")),
        }
    }

    fn set_mode(&mut self, mode: &str) -> Result<(), String> {
        match mode {
            "terminal" => {
                self.enter_terminal_mode();
                Ok(())
            }
            "window" => {
                self.leave_terminal_mode();
                Ok(())
            }
            other => Err(format!("unknown mode {other:?} (use terminal or window)")),
        }
    }

    fn create_new_window(&mut self) -> Result<(), String> {
        let shell = self.default_shell();
        let idx = self
            .spawn_window(&shell, Box::new(|| {}))
            .map_err(|e| e.to_string())?;
        self.await_new_window();
        let _ = idx;
        Ok(())
    }

    fn create_new_window_with_name(&mut self, name: &str) -> Result<(), String> {
        let shell = self.default_shell();
        let idx = self
            .spawn_window(&shell, Box::new(|| {}))
            .map_err(|e| e.to_string())?;
        self.rename_window(idx, name);
        self.await_new_window();
        Ok(())
    }

    fn close_window(&mut self, window_id: &str) -> Result<(), String> {
        let idx = self
            .window_index_by_id(window_id)
            .ok_or_else(|| format!("window not found: {window_id}"))?;
        self.remove_window(idx);
        Ok(())
    }

    fn close_window_by_name(&mut self, name: &str) -> Result<(), String> {
        let idx = self
            .windows
            .iter()
            .position(|w| w.title == name)
            .ok_or_else(|| format!("no window named {name:?}"))?;
        self.remove_window(idx);
        Ok(())
    }

    fn next_window(&mut self) -> Result<(), String> {
        self.focus_next();
        Ok(())
    }

    fn prev_window(&mut self) -> Result<(), String> {
        self.focus_prev();
        Ok(())
    }

    fn focus_window_by_id(&mut self, window_id: &str) -> Result<(), String> {
        let idx = self
            .window_index_by_id(window_id)
            .ok_or_else(|| format!("window not found: {window_id}"))?;
        self.focus_window(idx);
        Ok(())
    }

    fn focus_window_by_name(&mut self, name: &str) -> Result<(), String> {
        let matches: Vec<usize> = self
            .windows
            .iter()
            .enumerate()
            .filter(|(_, w)| w.title == name)
            .map(|(i, _)| i)
            .collect();
        match matches.len() {
            1 => {
                self.focus_window(matches[0]);
                Ok(())
            }
            0 => Err(format!("no window named {name:?}")),
            _ => Err(format!("multiple windows named {name:?}")),
        }
    }

    fn rename_window_by_id(&mut self, window_id: &str, name: &str) -> Result<(), String> {
        let idx = self
            .window_index_by_id(window_id)
            .ok_or_else(|| format!("window not found: {window_id}"))?;
        self.rename_window(idx, name);
        Ok(())
    }

    fn rename_window_by_name(&mut self, old_name: &str, new_name: &str) -> Result<(), String> {
        let idx = self
            .windows
            .iter()
            .position(|w| w.title == old_name)
            .ok_or_else(|| format!("no window named {old_name:?}"))?;
        self.rename_window(idx, new_name);
        Ok(())
    }

    fn minimize_window_by_id(&mut self, _window_id: &str) -> Result<(), String> {
        Err("minimize is not implemented in this port".into())
    }

    fn minimize_window_by_name(&mut self, _name: &str) -> Result<(), String> {
        Err("minimize is not implemented in this port".into())
    }

    fn restore_window_by_id(&mut self, _window_id: &str) -> Result<(), String> {
        Err("restore-minimized is not implemented in this port".into())
    }

    fn restore_window_by_name(&mut self, _name: &str) -> Result<(), String> {
        Err("restore-minimized is not implemented in this port".into())
    }

    // BSP tiling is always on in this port; the toggles are accepted no-ops.
    fn toggle_tiling(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn enable_tiling(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn disable_tiling(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn snap_by_direction(&mut self, _direction: &str) -> Result<(), String> {
        Err("snap is not implemented in this port".into())
    }

    fn split_horizontal(&mut self) -> Result<(), String> {
        let shell = self.default_shell();
        self.split(SplitType::Horizontal, &shell, Box::new(|| {}))
            .map(|_| {
                self.await_new_window();
            })
    }

    fn split_vertical(&mut self) -> Result<(), String> {
        let shell = self.default_shell();
        self.split(SplitType::Vertical, &shell, Box::new(|| {}))
            .map(|_| {
                self.await_new_window();
            })
    }

    fn rotate_split(&mut self) -> Result<(), String> {
        if let Some(focused) = self.focused_window {
            let ws = self.current_workspace;
            self.workspace_mut(ws).tree.rotate_split(focused as i32);
        }
        Ok(())
    }

    fn equalize_splits(&mut self) -> Result<(), String> {
        let ws = self.current_workspace;
        self.workspace_mut(ws).tree.equalize_ratios();
        Ok(())
    }

    fn preselect(&mut self, direction: &str) -> Result<(), String> {
        use crate::layout::PreselectionDir;
        self.preselection = match direction {
            "left" => PreselectionDir::Left,
            "right" => PreselectionDir::Right,
            "up" => PreselectionDir::Up,
            "down" => PreselectionDir::Down,
            other => return Err(format!("unknown preselect direction {other:?}")),
        };
        Ok(())
    }

    fn switch_workspace(&mut self, workspace: i32) -> Result<(), String> {
        self.switch_workspace(workspace);
        Ok(())
    }

    fn move_window_to_workspace_by_id(
        &mut self,
        window_id: &str,
        workspace: i32,
    ) -> Result<(), String> {
        let idx = self
            .window_index_by_id(window_id)
            .ok_or_else(|| format!("window not found: {window_id}"))?;
        if workspace == self.current_workspace {
            return Ok(()); // already there
        }
        // Move without following: save the workspace we were on.
        let from = self.current_workspace;
        self.move_window_to_workspace(idx, workspace);
        // Return to the source workspace (move-without-follow).
        self.current_workspace = from;
        self.focused_window = self.workspace(from).focused;
        Ok(())
    }

    fn move_and_follow_workspace_by_id(
        &mut self,
        window_id: &str,
        workspace: i32,
    ) -> Result<(), String> {
        let idx = self
            .window_index_by_id(window_id)
            .ok_or_else(|| format!("window not found: {window_id}"))?;
        self.move_window_to_workspace(idx, workspace);
        Ok(())
    }

    fn enable_animations(&mut self) -> Result<(), String> {
        self.config.appearance.animations_enabled = true;
        Ok(())
    }

    fn disable_animations(&mut self) -> Result<(), String> {
        self.config.appearance.animations_enabled = false;
        Ok(())
    }

    fn toggle_animations(&mut self) -> Result<(), String> {
        self.config.appearance.animations_enabled = !self.config.appearance.animations_enabled;
        Ok(())
    }

    fn toggle_zoom(&mut self) -> Result<(), String> {
        self.toggle_zoom_internal()
    }

    fn smart_split_focused(&mut self) -> Result<(), String> {
        let shell = self.default_shell();
        self.split(SplitType::None, &shell, Box::new(|| {}))
            .map(|_| {
                self.await_new_window();
            })
    }

    fn show_command_palette(&mut self) -> Result<(), String> {
        self.open_palette();
        Ok(())
    }

    fn save_layout(&mut self, name: &str) -> Result<(), String> {
        let tree = self.workspace(self.current_workspace).tree.serialize();
        self.layouts.insert(name.to_string(), tree);
        Ok(())
    }

    fn load_layout(&mut self, name: &str) -> Result<(), String> {
        if let Some(serialized) = self.layouts.get(name) {
            let tree = BSPTree::deserialize(serialized);
            if let Some(ws) = self.workspaces.get_mut(&self.current_workspace) {
                ws.tree = tree;
            }
            Ok(())
        } else {
            Err(format!("Layout '{name}' not found"))
        }
    }

    fn set_config(&mut self, _path: &str, _value: &str) -> Result<(), String> {
        Err("Set is not implemented in this port".into())
    }

    fn set_theme(&mut self, theme_name: &str) -> Result<(), String> {
        self.config.appearance.theme = theme_name.to_string();
        self.theme = Theme::built_in(theme_name);
        Ok(())
    }

    fn set_dockbar_position(&mut self, position: &str) -> Result<(), String> {
        self.config.appearance.dockbar_position = position.to_string();
        Ok(())
    }

    fn set_border_style(&mut self, style: &str) -> Result<(), String> {
        self.config.appearance.border_style = style.to_string();
        Ok(())
    }

    fn show_notification(&mut self, message: &str, notification_type: &str) -> Result<(), String> {
        self.notify(message, notification_type);
        Ok(())
    }

    fn focus_direction(&mut self, direction: &str) -> Result<(), String> {
        self.focus_direction(direction)
    }
}

/// The human word for a transition into `state`, for the alert text. Empty
/// means the state is not one that gets announced.
fn agent_transition_notice(state: &str) -> String {
    match state {
        "needs_input" => "needs your input".into(),
        "errored" => "errored".into(),
        "done" => "finished".into(),
        _ => String::new(),
    }
}

/// Minutes since local midnight (libc localtime, no extra deps).
fn local_minutes_since_midnight() -> i32 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let mut tm: nix::libc::tm = unsafe { std::mem::zeroed() };
    unsafe {
        nix::libc::localtime_r(&now, &mut tm);
    }
    tm.tm_hour as i32 * 60 + tm.tm_min as i32
}

/// Fire a desktop notification using the platform's notification tool.
///
/// On Linux: `notify-send`. On macOS: `osascript`. On Windows: `msg` (if
/// available). Falls back to no-op if no tool is found.
fn fire_desktop_notification(title: &str, body: &str) {
    let title = if title.is_empty() { "TermOS" } else { title };
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("notify-send")
            .arg(title)
            .arg(body)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let script = format!(
            "display notification \"{}\" with title \"{}\"",
            body.replace('\\', "\\\\").replace('"', "\\\""),
            title.replace('\\', "\\\\").replace('"', "\\\"")
        );
        let _ = std::process::Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (title, body); // no-op on unsupported platforms
    }
}

#[cfg(test)]
impl Os {
    /// Insert a fake window into the OS (no PTY spawned) for unit tests.
    pub fn push_fake_window(&mut self, id: &str, title: &str, direction: SplitType) {
        use crate::terminal::pty::WinSize;
        use crate::terminal::Window;
        let index = self.windows.len();
        let ws = self.current_workspace;
        let bounds = self.workspace_bounds(ws);
        let focused = self.workspace(ws).focused.map(|f| f as i32).unwrap_or(-1);
        let gap = self.gap;
        let tree = &mut self.workspace_mut(ws).tree;
        tree.insert_window(index as i32, focused, direction, 0.5, bounds, gap);
        let win = Window::without_pty(id, title, WinSize { cols: 40, rows: 12 });
        self.windows.push(win);
        self.workspace_mut(ws).focused = Some(index);
        self.focused_window = Some(index);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_os() -> Os {
        let mut os = Os::new(UserConfig::default_config());
        os.width = 80;
        os.height = 25;
        os
    }

    #[test]
    fn start_mode_follows_startup_config() {
        // Default: window-management mode (keystrokes are commands).
        let os = Os::new(UserConfig::default_config());
        assert_eq!(os.mode, Mode::WindowManagement);

        // `[startup] start_in_terminal_mode = true`: keystrokes reach the
        // shell immediately after launch.
        let mut cfg = UserConfig::default_config();
        cfg.startup.start_in_terminal_mode = true;
        let os = Os::new(cfg);
        assert_eq!(os.mode, Mode::Terminal);
    }

    #[test]
    fn fuzzy_match_is_subsequence_case_insensitive() {
        assert!(matches_query("Switch to workspace 3", "sw3"));
        assert!(matches_query("New window", ""));
        assert!(matches_query("New window", "nw"));
        assert!(!matches_query("New window", "zq"));
    }

    #[test]
    fn fuzzy_match_returns_positions() {
        let m = fuzzy_match("Close window", "cw").unwrap();
        // 'C' at 0, 'w' at 6
        assert!(m.positions.contains(&0));
        assert!(m.positions.contains(&6));
    }

    #[test]
    fn fuzzy_match_scores_prefix_higher_than_subsequence() {
        let prefix = fuzzy_match("Close window", "close").unwrap();
        let subseq = fuzzy_match("Close window", "cow").unwrap();
        assert!(prefix.score < subseq.score);
    }

    #[test]
    fn fuzzy_match_scores_word_boundary_higher() {
        let word = fuzzy_match("Close window", "window").unwrap();
        let mid = fuzzy_match("Close window", "lose").unwrap();
        assert!(word.score < mid.score);
    }

    #[test]
    fn fuzzy_match_tokens_multi_word() {
        let m = fuzzy_match_tokens("Close window", "cl wi");
        assert!(m.is_some());
        let m = fuzzy_match_tokens("Close window", "cl xyz");
        assert!(m.is_none());
    }

    #[test]
    fn palette_multi_token_search() {
        let mut os = test_os();
        os.open_palette();
        // "win close" should match "Close window" (multi-token fuzzy)
        os.palette_query = "win close".into();
        let items = os.palette_items();
        let cmds: Vec<Command> = items.into_iter().map(|(c, _)| c).collect();
        assert!(cmds.contains(&Command::CloseWindow));
    }

    #[test]
    fn palette_highlight_positions_non_empty() {
        let mut os = test_os();
        os.open_palette();
        os.palette_query = "quit".into();
        let items = os.palette_items();
        // Quit should be first and have non-empty highlight positions
        let (cmd, positions) = items.first().unwrap();
        assert_eq!(*cmd, Command::Quit);
        assert!(!positions.is_empty());
    }

    #[test]
    fn palette_multi_token_prefers_full_word_matches() {
        let mut os = test_os();
        os.open_palette();
        // "new win" should rank "New window" above "Next window"
        // because both tokens match complete words in "New window".
        os.palette_query = "new win".into();
        let items = os.palette_items();
        let cmds: Vec<Command> = items.into_iter().map(|(c, _)| c).collect();
        let new_win_idx = cmds.iter().position(|c| *c == Command::NewWindow);
        let next_win_idx = cmds.iter().position(|c| *c == Command::NextWindow);
        if let (Some(nw), Some(nx)) = (new_win_idx, next_win_idx) {
            assert!(nw < nx, "New window should rank above Next window");
        }
    }

    #[test]
    fn palette_empty_query_shows_all() {
        let mut os = test_os();
        os.open_palette();
        os.palette_query.clear();
        let items = os.palette_items();
        assert!(!items.is_empty());
    }

    #[test]
    fn palette_recent_commands_sort_first() {
        let mut os = test_os();
        // Simulate using NewWindow and CloseWindow recently.
        os.palette_recent = vec![Command::NewWindow, Command::CloseWindow];
        os.open_palette();
        os.palette_query.clear(); // show all
        let items = os.palette_items();
        let cmds: Vec<Command> = items.into_iter().map(|(c, _)| c).collect();
        // CloseWindow (most recent) should be first, NewWindow second.
        assert_eq!(cmds[0], Command::CloseWindow);
        assert_eq!(cmds[1], Command::NewWindow);
    }

    #[test]
    fn palette_filters_commands() {
        let mut os = test_os();
        os.open_palette();
        os.palette_query = "close".into();
        let items = os.palette_items();
        assert!(items.iter().any(|(c, _)| c == &Command::CloseWindow));
    }

    #[test]
    fn palette_ranks_best_match_first() {
        let mut os = test_os();
        os.open_palette();
        // "quit" also subsequence-matches "equalize splits"; the prefix match
        // on "Quit" must rank first.
        os.palette_query = "quit".into();
        let first = os.palette_items().first().map(|(c, _)| c.clone());
        assert_eq!(first, Some(Command::Quit));
    }

    #[test]
    fn palette_move_wraps_and_activate_runs_command() {
        let mut os = test_os();
        os.open_palette();
        os.palette_query = "workspace 3".into();
        let items = os.palette_items();
        let cmds: Vec<Command> = items.into_iter().map(|(c, _)| c).collect();
        assert_eq!(cmds, vec![Command::SwitchWorkspace(3)]);
        os.activate_palette();
        assert!(!os.palette_open);
        assert_eq!(os.current_workspace, 3);
    }

    #[test]
    fn workspace_switcher_lists_nine_workspaces() {
        let mut os = test_os();
        os.open_switcher(SwitcherKind::Workspace);
        let items = os.switcher_items();
        assert_eq!(items.len(), 9);
        assert!(items[0].label.starts_with("1:"));
    }

    #[test]
    fn switcher_activate_switches_workspace() {
        let mut os = test_os();
        os.open_switcher(SwitcherKind::Workspace);
        os.switcher_selected = 4; // workspace 5
        os.activate_switcher();
        assert!(!os.switcher_open);
        assert_eq!(os.current_workspace, 5);
    }

    #[test]
    fn window_at_hit_tests_layout() {
        let mut os = test_os();
        let bounds = os.workspace_bounds(1);
        os.workspace_mut(1)
            .tree
            .insert_window(0, -1, SplitType::None, 0.5, bounds, 0);
        assert_eq!(os.window_at(10, 5), Some(0));
        assert_eq!(os.window_at(10, 10_000), None);
    }

    #[test]
    fn hooks_fire_on_window_lifecycle_events() {
        let mut os = test_os();
        let seen: Arc<Mutex<Vec<(hooks::Event, hooks::Context)>>> =
            Arc::new(Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        os.hook_manager.set_runner(move |_, ctx| {
            if let Some(ev) = ctx.event {
                seen2.lock().unwrap().push((ev, ctx.clone()));
            }
        });
        // `fire` only runs registered commands; the runner just replaces their
        // execution, so register a placeholder for each event under test.
        for ev in [
            hooks::Event::AfterNewWindow,
            hooks::Event::AfterFocusChange,
            hooks::Event::AfterWorkspaceSwitch,
            hooks::Event::AfterCloseWindow,
        ] {
            os.hook_manager.register(ev, "dummy");
        }

        // Local window creation fires after-new-window with the window id.
        let idx = os.spawn_window("/bin/sh", Box::new(|| {})).unwrap();
        os.hook_manager.wait();
        assert!(seen.lock().unwrap().iter().any(|(e, c)| {
            *e == hooks::Event::AfterNewWindow && c.window_id == format!("win-{idx}")
        }));

        // focus_next on a single window does not fire (focus unchanged).
        let before = seen.lock().unwrap().len();
        os.focus_next();
        os.hook_manager.wait();
        assert_eq!(seen.lock().unwrap().len(), before);

        // With two windows, focus_next fires after-focus-change.
        os.spawn_window("/bin/sh", Box::new(|| {})).unwrap();
        os.hook_manager.wait();
        os.focus_next();
        os.hook_manager.wait();
        assert!(seen
            .lock()
            .unwrap()
            .iter()
            .any(|(e, _)| *e == hooks::Event::AfterFocusChange));

        // Closing the focused window fires after-close-window.
        os.close_focused_window();
        os.hook_manager.wait();
        assert!(seen
            .lock()
            .unwrap()
            .iter()
            .any(|(e, _)| *e == hooks::Event::AfterCloseWindow));

        // Workspace switch fires after-workspace-switch with the previous
        // workspace; switching to the same workspace does not fire.
        os.switch_workspace(3);
        os.hook_manager.wait();
        assert!(seen.lock().unwrap().iter().any(|(e, c)| {
            *e == hooks::Event::AfterWorkspaceSwitch && c.previous_workspace == 1
        }));
        let before = seen.lock().unwrap().len();
        os.switch_workspace(3);
        os.hook_manager.wait();
        assert_eq!(seen.lock().unwrap().len(), before);
    }

    /// Build an Os with one PTY-less window so selection/yank can be tested
    /// without spawning a shell.
    fn os_with_window() -> Os {
        use crate::terminal::pty::WinSize;
        use crate::terminal::window::Window;
        let mut os = test_os();
        let win = Window::without_pty(
            "w0".to_string(),
            "w0".to_string(),
            WinSize { cols: 20, rows: 4 },
        );
        os.windows.push(win);
        {
            let w = &os.windows[0];
            let mut emu = w.emulator.lock().unwrap();
            emu.write(b"hello world");
        }
        let bounds = os.workspace_bounds(1);
        os.workspace_mut(1)
            .tree
            .insert_window(0, -1, SplitType::None, 0.5, bounds, 0);
        os.workspace_mut(1).focused = Some(0);
        os.focused_window = Some(0);
        os
    }

    #[test]
    fn yank_selection_copies_text() {
        let mut os = os_with_window();
        os.selection = Some(Selection {
            window: 0,
            anchor_line: 0,
            anchor_col: 0,
            cursor_line: 0,
            cursor_col: 4, // "hello"
        });
        os.yank_selection();
        assert_eq!(os.clipboard, "hello");
        assert!(os.selection.is_none());
        assert!(!os.copy_visual);
    }

    #[test]
    fn toggle_visual_anchors_at_cursor() {
        let mut os = os_with_window();
        os.enter_scrollback_mode();
        assert!(os.scrollback_mode);
        os.toggle_visual(false);
        assert!(os.copy_visual);
        let sel = os.selection.as_ref().unwrap();
        assert_eq!(sel.window, 0);
        assert_eq!(sel.anchor_line, sel.cursor_line);
        // Esc clears visual selection.
        os.toggle_visual(false);
        assert!(!os.copy_visual);
        assert!(os.selection.is_none());
    }

    #[test]
    fn copy_move_line_clamps_to_content() {
        let mut os = os_with_window();
        os.enter_scrollback_mode();
        // content_line_count is 4 (one live screen, no scrollback); the cursor
        // starts at line 3.
        assert_eq!(os.copy_cursor_line, 3);
        os.copy_move_line(10);
        assert_eq!(os.copy_cursor_line, 3);
        os.copy_move_line(-10);
        assert_eq!(os.copy_cursor_line, 0);
    }

    #[test]
    fn word_select_on_wide_run_selects_full_word() {
        let mut os = os_with_window();
        {
            let w = &os.windows[0];
            let mut emu = w.emulator.lock().unwrap();
            emu.write(b"\x1b[2J\x1b[H");
            emu.write("\u{4f60}\u{4f60}XX end".as_bytes());
        }
        // Double-click on the second 你 (screen col 3 = content col 2): the
        // word 你你XX must be selected whole, including both X's.
        os.select_word_at(0, 3, 1);
        let sel = os.selection.as_ref().unwrap();
        assert_eq!(sel.anchor_col, 0);
        // 你你XX spans 6 columns (2+2+1+1); the range is in column space
        // and the end column is inclusive.
        assert_eq!(sel.cursor_col, 5, "word must cover 你你XX");
        let text = {
            let w = &os.windows[0];
            let emu = w.emulator.lock().unwrap();
            emu.selection_text(sel.anchor_line, sel.anchor_col, sel.cursor_line, sel.cursor_col)
        };
        assert_eq!(text, "\u{4f60}\u{4f60}XX");
    }

    #[test]
    fn mouse_click_on_wide_continuation_snaps_to_lead() {
        let mut os = os_with_window();
        // Replace content with a wide char + trailing text: 你 occupies
        // content cols 0-1, X is at col 2.
        {
            let w = &os.windows[0];
            let mut emu = w.emulator.lock().unwrap();
            emu.write(b"\x1b[2J\x1b[H");
            emu.write("\u{4f60}X".as_bytes());
        }
        // Click on the wide char's continuation column (screen col 2 =
        // content col 1): must snap to the lead col 0.
        os.begin_mouse_selection(0, 2, 1);
        let sel = os.selection.as_ref().unwrap();
        assert_eq!(sel.anchor_col, 0, "continuation click must snap to lead");
        assert_eq!(sel.cursor_col, 0);
        // Click exactly on the lead column also lands on the lead.
        os.begin_mouse_selection(0, 1, 1);
        assert_eq!(os.selection.as_ref().unwrap().anchor_col, 0);
        // Click past the content end keeps its raw column (no clamping).
        os.begin_mouse_selection(0, 15, 1);
        assert_eq!(os.selection.as_ref().unwrap().anchor_col, 14);
    }

    #[test]
    fn mouse_selection_yanks_on_release() {
        let mut os = os_with_window();
        // Click at content (line 0, col 0) then release over (0, 4). The pane
        // rect is (0,0,80,24-dock) with a 1-cell border ring, so screen
        // (1,1)..(5,1) maps to content cols 0..4 ("hello").
        os.begin_mouse_selection(0, 1, 1);
        os.extend_mouse_selection(0, 5, 1);
        assert!(os.mouse_selecting);
        assert!(!os.selection.as_ref().unwrap().is_empty());
        os.end_mouse_selection();
        assert!(!os.mouse_selecting);
        assert_eq!(os.clipboard, "hello");
    }

    #[test]
    fn mouse_drag_yanks_clean_wide_text() {
        let mut os = os_with_window();
        {
            let w = &os.windows[0];
            let mut emu = w.emulator.lock().unwrap();
            emu.write(b"\x1b[2J\x1b[H");
            emu.write("\u{4f60}\u{4f60}XX end".as_bytes());
        }
        // Drag from the first 你's lead (content col 0) to 'd' (content col 9).
        // The pane has a 1-cell border ring, so screen (1,1)..(10,1) maps to
        // content cols 0..9.
        os.begin_mouse_selection(0, 1, 1);
        os.extend_mouse_selection(0, 10, 1);
        os.end_mouse_selection();
        assert_eq!(os.clipboard, "\u{4f60}\u{4f60}XX end");
    }

    #[test]
    fn mouse_drag_starting_on_wide_continuation_yanks_full_word() {
        let mut os = os_with_window();
        {
            let w = &os.windows[0];
            let mut emu = w.emulator.lock().unwrap();
            emu.write(b"\x1b[2J\x1b[H");
            emu.write("\u{4f60}\u{4f60}XX end".as_bytes());
        }
        // Press lands on the second 你's continuation (content col 3) and
        // drags to 'd' (content col 9). The snap anchors the selection at the
        // second 你's lead (col 2), so cols 2..=9 are copied cleanly — the
        // first 你 is legitimately excluded, and there must be no phantom
        // space where the continuation cell sits.
        os.begin_mouse_selection(0, 4, 1);
        os.extend_mouse_selection(0, 10, 1);
        os.end_mouse_selection();
        assert_eq!(os.clipboard, "\u{4f60}XX end");
    }

    #[test]
    fn select_line_at_wide_chars_yanks_full_line() {
        let mut os = os_with_window();
        {
            let w = &os.windows[0];
            let mut emu = w.emulator.lock().unwrap();
            emu.write(b"\x1b[2J\x1b[H");
            emu.write("\u{4f60}\u{4f60}XX end".as_bytes());
        }
        // Triple-click (line select) on the content: screen (2,1) = content col 0.
        os.select_line_at(0, 2, 1);
        let sel = os.selection.as_ref().unwrap();
        assert_eq!(sel.anchor_col, 0);
        // cursor_col is inclusive: covers the entire line (cols 0..=width-1).
        let w = &os.windows[0];
        let emu = w.emulator.lock().unwrap();
        let width = emu.width();
        drop(emu);
        assert_eq!(sel.cursor_col, width - 1, "line select must cover full line");
        let text = {
            let w = &os.windows[0];
            let emu = w.emulator.lock().unwrap();
            emu.selection_text(sel.anchor_line, sel.anchor_col, sel.cursor_line, sel.cursor_col)
        };
        assert_eq!(text, "\u{4f60}\u{4f60}XX end");
    }

    struct NullSink;
    impl PtySink for NullSink {
        fn write(&self, _data: &[u8]) {}
        fn resize(&self, _size: WinSize) {}
    }

    #[test]
    fn tape_script_tick_drives_commands_in_order() {
        use crate::tape::command::{Command, CommandType};
        use crate::tape::player::Player;

        let mut os = os_with_window();
        let cmd = |type_: CommandType, args: &[&str]| Command {
            type_,
            args: args.iter().map(|s| s.to_string()).collect(),
            delay: std::time::Duration::ZERO,
            line: 1,
            column: 1,
            raw: String::new(),
        };
        os.script_mode = true;
        let mut sleep = cmd(CommandType::Sleep, &["100ms"]);
        sleep.delay = std::time::Duration::from_millis(100);
        os.script_player = Some(Player::new(vec![
            cmd(CommandType::Type, &["hello"]),
            sleep,
            cmd(CommandType::Enter, &[]),
        ]));

        // First tick executes Type (sent to the focused window) and advances.
        os.tick_script();
        assert_eq!(os.script_player.as_ref().unwrap().current_index(), 1);

        // The tick that reaches Sleep arms the deadline and advances past it.
        os.tick_script();
        assert_eq!(os.script_player.as_ref().unwrap().current_index(), 2);
        assert!(os.script_sleep_until.is_some());

        // The next tick blocks while the deadline is in the future.
        os.tick_script();
        assert_eq!(os.script_player.as_ref().unwrap().current_index(), 2);

        // Force the sleep deadline into the past; the next tick clears it and
        // executes the Enter command.
        os.script_sleep_until = Some(std::time::Instant::now() - std::time::Duration::from_secs(1));
        os.tick_script();
        assert!(os.script_player.as_ref().unwrap().is_finished());
    }

    #[test]
    fn tape_script_wait_until_regex_matches_screen() {
        use crate::tape::command::{Command, CommandType};
        use crate::tape::player::Player;

        let mut os = os_with_window(); // emulator contains "hello world"
        let cmd = Command {
            type_: CommandType::WaitUntilRegex,
            args: vec!["hello".to_string()],
            delay: std::time::Duration::ZERO,
            line: 1,
            column: 1,
            raw: String::new(),
        };
        os.script_mode = true;
        os.script_player = Some(Player::new(vec![cmd]));

        // The first tick arms the wait without advancing past it.
        os.tick_script();
        assert!(os.script_wait_regex.is_some());
        assert_eq!(os.script_player.as_ref().unwrap().current_index(), 1);

        // The next tick matches the screen content and finishes.
        os.tick_script();
        assert!(os.script_wait_regex.is_none());
        assert!(os.script_player.as_ref().unwrap().is_finished());
    }

    #[test]
    fn tape_script_invalid_regex_notifies_and_skips() {
        use crate::tape::command::{Command, CommandType};
        use crate::tape::player::Player;

        let mut os = os_with_window();
        let cmd = Command {
            type_: CommandType::WaitUntilRegex,
            args: vec!["[".to_string()], // invalid regex
            delay: std::time::Duration::ZERO,
            line: 1,
            column: 1,
            raw: String::new(),
        };
        os.script_mode = true;
        os.script_player = Some(Player::new(vec![cmd]));
        os.tick_script();
        assert!(os.script_wait_regex.is_none());
        assert!(os.script_player.as_ref().unwrap().is_finished());
        assert!(
            os.notifications
                .iter()
                .any(|n| n.message.contains("invalid pattern")),
            "expected an invalid-pattern notification"
        );
    }

    #[test]
    fn tape_script_paused_blocks_tick() {
        use crate::tape::command::{Command, CommandType};
        use crate::tape::player::Player;

        let mut os = os_with_window();
        let cmd = Command {
            type_: CommandType::Enter,
            args: vec![],
            delay: std::time::Duration::ZERO,
            line: 1,
            column: 1,
            raw: String::new(),
        };
        os.script_mode = true;
        os.script_paused = true;
        os.script_player = Some(Player::new(vec![cmd]));
        os.tick_script();
        assert_eq!(os.script_player.as_ref().unwrap().current_index(), 0);
    }

    #[test]
    fn apc_sequences_are_collected_and_forwarded() {
        // Feed a Kitty APC into the emulator; flush_graphics should drain it.
        // We use a sink-backed passthrough so the test doesn't write to stdout.
        use std::sync::Arc;
        use std::sync::Mutex as StdMutex;
        struct Sink;
        impl std::io::Write for Sink {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                Ok(b.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut os = os_with_window();
        os.graphics_caps.kitty = true;
        os.kitty_passthrough = Some(crate::graphics::kitty::KittyPassthrough::new(
            os.graphics_caps,
            Box::new(Sink),
        ));
        // Feed a Kitty APC: ESC _ G a=T,f=100,i=1;AAAA ESC \
        let apc: &[u8] = b"\x1b_Ga=T,f=100,i=1;AAAA\x1b\\";
        {
            let w = &mut os.windows[0];
            let mut emu = w.emulator.lock().unwrap();
            emu.write(apc);
            // Drain immediately from the emulator to verify it was collected.
            let apcs = emu.drain_pending_apc();
            assert_eq!(apcs.len(), 1, "APC not collected by emulator");
            assert_eq!(apcs[0].first(), Some(&b'G'), "not a Kitty APC");
        }
        let _ = Arc::new(StdMutex::new(())); // suppress unused import warning
    }

    #[test]
    fn render_overlay_does_not_panic_on_offset_rects() {
        // Regression: overlays narrower than the screen (which-key, switcher,
        // tape manager) used absolute indexing into an offset block buffer.
        use crate::app::render::render;
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let mut os = test_os();
        os.prefix = Prefix::Tape; // narrow which-key popup
        let mut terminal = Terminal::new(TestBackend::new(80, 25)).unwrap();
        terminal.draw(|f| render(&os, f.buffer_mut(), &[])).unwrap();
        os.prefix = Prefix::None;
        os.tape_manager_open = true; // tape manager overlay
        terminal.draw(|f| render(&os, f.buffer_mut(), &[])).unwrap();
        os.tape_manager_open = false;
        os.switcher_open = true; // switcher overlay
        terminal.draw(|f| render(&os, f.buffer_mut(), &[])).unwrap();
    }

    #[test]
    fn render_shows_pane_content_inside_the_border() {
        // Regression: the pane border ring must not wipe the content drawn
        // under it, and content must be inset by one cell.
        use crate::app::render::render;
        use crate::terminal::pty::WinSize;
        use crate::terminal::window::Window;
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let mut os = test_os();
        let win = Window::without_pty(
            "w0".to_string(),
            "w0".to_string(),
            WinSize { cols: 20, rows: 4 },
        );
        os.windows.push(win);
        {
            let w = &os.windows[0];
            let mut emu = w.emulator.lock().unwrap();
            emu.write(b"hello");
        }
        let bounds = os.workspace_bounds(1);
        os.workspace_mut(1)
            .tree
            .insert_window(0, -1, SplitType::None, 0.5, bounds, 0);
        os.workspace_mut(1).focused = Some(0);
        os.focused_window = Some(0);
        os.sync_window_sizes();

        let mut terminal = Terminal::new(TestBackend::new(80, 25)).unwrap();
        terminal.draw(|f| render(&os, f.buffer_mut(), &[])).unwrap();
        assert!(os.windows[0]
            .render_cache
            .lock()
            .unwrap()
            .as_ref()
            .is_some());
        terminal.draw(|f| render(&os, f.buffer_mut(), &[])).unwrap();
        let buf = terminal.backend().buffer();
        // The border ring: row 0 is the top edge, column 0/79 are the sides.
        assert_eq!(buf[(0, 0)].symbol(), "╭");
        // Content starts one cell in from the border.
        let row: String = (0..8)
            .map(|col| {
                let sym = buf[(col, 1)].symbol();
                if sym == " " {
                    ' '
                } else {
                    sym.chars().next().unwrap()
                }
            })
            .collect();
        assert_eq!(row, "│hello  ");
    }

    #[test]
    fn recording_captures_lifecycle_and_typing() {
        let mut os = os_with_window();
        os.start_recording();
        // Terminal input accumulates into a Type command.
        {
            use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
            let k = KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE);
            os.record_terminal_key(&k);
            os.record_terminal_key(&k);
            let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
            os.record_terminal_key(&enter);
        }
        // A mode switch flushes and records.
        os.enter_terminal_mode();
        // A new window records an action.
        os.spawn_window("/bin/sh", Box::new(|| {})).unwrap();

        let recorder = os.recorder.as_ref().unwrap();
        let types: Vec<_> = recorder.commands().iter().map(|c| c.type_).collect();
        assert!(types.contains(&crate::tape::command::CommandType::Type));
        assert!(types.contains(&crate::tape::command::CommandType::Enter));
        assert!(types.contains(&crate::tape::command::CommandType::TerminalMode));
        assert!(types.contains(&crate::tape::command::CommandType::NewWindow));

        // Stop saves a real .tape file and clears the recorder.
        let path = os.stop_recording().expect("saved tape");
        assert!(path.exists());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("NewWindow"));
        assert!(content.contains("DisableAnimations"));
        assert!(os.recorder.is_none());
        // Clean up the artifact.
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn tape_executor_drives_the_app() {
        use crate::tape::command::{Command, CommandType};
        use crate::tape::executor::{CommandExecutor, TapeExecutor};

        let mut os = os_with_window();
        assert_eq!(os.windows.len(), 1);

        {
            let mut ce = CommandExecutor::new(&mut os);
            // Type into the focused window (a PTY-less window: write is a no-op
            // through the missing writer, but the executor path must succeed).
            let type_cmd = Command {
                type_: CommandType::Type,
                args: vec!["echo hi".into()],
                delay: std::time::Duration::ZERO,
                line: 1,
                column: 1,
                raw: "Type".into(),
            };
            ce.execute(&type_cmd).unwrap();
            // NewWindow spawns a second shell window.
            let new_cmd = Command {
                type_: CommandType::NewWindow,
                args: vec!["editor".into()],
                delay: std::time::Duration::ZERO,
                line: 1,
                column: 1,
                raw: "NewWindow".into(),
            };
            ce.execute(&new_cmd).unwrap();
            // Rename the focused window.
            let rename_cmd = Command {
                type_: CommandType::RenameWindow,
                args: vec!["renamed".into()],
                delay: std::time::Duration::ZERO,
                line: 1,
                column: 1,
                raw: "RenameWindow".into(),
            };
            ce.execute(&rename_cmd).unwrap();
            // Zoom is implemented: toggles the focused window.
            let zoom_cmd = Command {
                type_: CommandType::ToggleZoom,
                args: vec![],
                delay: std::time::Duration::ZERO,
                line: 1,
                column: 1,
                raw: "ToggleZoom".into(),
            };
            ce.execute(&zoom_cmd).unwrap();
        }
        // `ce` is dropped: the app-level zoom state is observable now.
        let zoomed = os.windows[os.focused_window.unwrap()].zoomed;
        assert!(zoomed);

        assert_eq!(os.windows.len(), 2);
        assert_eq!(os.focused_window_id(), os.windows[1].id.clone().into());
        // The focused (newest) window was renamed.
        let focused = os.focused_window.unwrap();
        assert_eq!(os.windows[focused].title, "renamed");
    }

    #[test]
    fn agent_alert_fires_dock_hook_and_host_sequence() {
        let mut os = os_with_window();
        os.config.notifications.agent.suppress_focused = Some(false);
        os.config.notifications.agent.settle_seconds = Some(0);
        os.hook_manager
            .register(hooks::Event::AfterAgentState, "dummy");
        let seen: Arc<Mutex<Vec<hooks::Context>>> = Arc::new(Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        os.hook_manager.set_runner(move |_, ctx| {
            seen2.lock().unwrap().push(ctx.clone());
        });

        os.handle_agent_state_changed("w0", "working", "", "");
        os.handle_agent_state_changed("w0", "needs_input", "awaiting approval", "claude-code");
        os.hook_manager.wait();

        assert!(!os.notifications.is_empty());
        let host = os.take_host_sequence();
        assert!(
            host.starts_with(b"\x1b]9;"),
            "expected an OSC 9 notification in {:?}",
            String::from_utf8_lossy(&host)
        );
        let ctxs = seen.lock().unwrap();
        let ctx = ctxs.last().expect("hook fired");
        assert_eq!(ctx.agent_state, "needs_input");
        assert_eq!(ctx.prev_agent_state, "working");
        assert_eq!(ctx.agent_message, "awaiting approval");
        assert_eq!(ctx.agent_harness, "claude-code");
        assert_eq!(ctx.window_id, "w0");
    }

    #[test]
    fn agent_alert_suppresses_focused_and_non_alerting_states() {
        let mut os = os_with_window();
        // Default policy: suppress_focused (w0 is focused) and working is not
        // an alerting state.
        os.handle_agent_state_changed("w0", "working", "", "");
        os.tick_agent_alerts();
        assert!(os.notifications.is_empty());
        assert!(os.take_host_sequence().is_empty());

        os.handle_agent_state_changed("w0", "needs_input", "", "");
        os.tick_agent_alerts();
        assert!(
            os.notifications.is_empty(),
            "focused pane must be suppressed"
        );
        assert!(os.take_host_sequence().is_empty());
    }

    #[test]
    fn agent_alert_settle_window_parks_then_fires() {
        let mut os = os_with_window();
        os.focused_window = None; // nothing focused → nothing suppressed
        os.hook_manager
            .register(hooks::Event::AfterAgentState, "dummy");
        let fired = Arc::new(Mutex::new(0usize));
        let fired2 = fired.clone();
        os.hook_manager.set_runner(move |_, _| {
            *fired2.lock().unwrap() += 1;
        });

        // needs_input alerts; default settle (2s) parks it.
        os.handle_agent_state_changed("w0", "needs_input", "", "");
        assert!(os.notifications.is_empty());
        assert!(!os.pending_agent_alerts.is_empty());

        // A further transition retires the parked alert (anti-flicker).
        os.handle_agent_state_changed("w0", "working", "", "");
        os.tick_agent_alerts();
        assert!(os.notifications.is_empty());
        assert!(os.pending_agent_alerts.is_empty());

        // Park an already-due alert and flush it.
        os.handle_agent_state_changed("w0", "done", "all done", "claude");
        os.pending_agent_alerts.insert(
            "w0".to_string(),
            agent_alert::PendingAgentAlert {
                window_id: "w0".into(),
                from: String::new(),
                to: "done".into(),
                due: std::time::Instant::now() - std::time::Duration::from_secs(1),
            },
        );
        os.tick_agent_alerts();
        assert!(!os.notifications.is_empty());
        assert!(!os.take_host_sequence().is_empty());
        os.hook_manager.wait();
        assert_eq!(*fired.lock().unwrap(), 1);
    }

    #[test]
    fn add_and_remove_remote_window() {
        let mut os = test_os();
        let (_out_tx, out_rx) = crossbeam_channel::unbounded::<Vec<u8>>();
        let info = WindowInfo {
            id: "w0".into(),
            title: "Terminal".into(),
            workspace: 1,
            cols: 20,
            rows: 10,
            agent_state: String::new(),
            agent_message: String::new(),
            agent_harness: String::new(),
        };
        let idx = os.add_remote_window(info, Box::new(NullSink), out_rx, None);
        assert_eq!(idx, 0);
        assert_eq!(os.windows.len(), 1);
        assert_eq!(os.focused_window, Some(0));
        assert!(os.workspace(1).tree.has_window(0));

        // Removing the window collapses the tree and clears focus.
        os.remove_window(0);
        assert!(os.windows.is_empty());
        assert_eq!(os.focused_window, None);
    }

    #[test]
    fn clear_all_windows_resets_workspaces() {
        let mut os = test_os();
        let (_out_tx, out_rx) = crossbeam_channel::unbounded::<Vec<u8>>();
        let info = WindowInfo {
            id: "w0".into(),
            title: "Terminal".into(),
            workspace: 2,
            cols: 20,
            rows: 10,
            agent_state: String::new(),
            agent_message: String::new(),
            agent_harness: String::new(),
        };
        os.add_remote_window(info, Box::new(NullSink), out_rx, None);
        os.clear_all_windows();
        assert!(os.windows.is_empty());
        for i in 1..=9 {
            assert!(os.workspace(i).tree.get_all_window_ids().is_empty());
        }
    }

    // --- Floating panes ---

    fn float_test_os() -> Os {
        let mut os = test_os();
        for i in 0..2 {
            let w = Window::without_pty(
                format!("w{i}"),
                format!("win{i}"),
                WinSize { cols: 10, rows: 3 },
            );
            os.windows.push(w);
        }
        let bounds = os.workspace_bounds(1);
        os.workspace_mut(1)
            .tree
            .insert_window(0, -1, SplitType::None, 0.5, bounds, 0);
        os.workspace_mut(1)
            .tree
            .insert_window(1, 0, SplitType::Horizontal, 0.5, bounds, 0);
        os.workspace_mut(1).focused = Some(0);
        os.focused_window = Some(0);
        os
    }

    #[test]
    fn float_window_removes_from_tree_and_keeps_running() {
        let mut os = float_test_os();
        os.float_window(0);
        assert!(os.is_float(0));
        assert!(!os.workspace(1).tree.has_window(0));
        // The window is still alive, just not tiled.
        assert_eq!(os.windows.len(), 2);
        assert_eq!(os.focused_window, Some(0));
        // Float rect is centered and inside the workspace.
        let r = os.float_rect(0).unwrap();
        assert!(r.w > 0 && r.h > 0);
        assert!(r.x >= 0 && r.x + r.w <= 80);
        assert!(r.y >= 0 && r.y + r.h <= 24);
    }

    #[test]
    fn unfloat_window_reinserts_into_tree() {
        let mut os = float_test_os();
        os.float_window(0);
        os.unfloat_window(0);
        assert!(!os.is_float(0));
        assert!(os.workspace(1).tree.has_window(0));
    }

    #[test]
    fn toggle_float_floats_and_tiles() {
        let mut os = float_test_os();
        os.toggle_float();
        assert!(os.is_float(0));
        os.toggle_float();
        assert!(!os.is_float(0));
        assert!(os.workspace(1).tree.has_window(0));
    }

    #[test]
    fn spawn_floating_window_skips_tree() {
        let mut os = test_os();
        let idx = os.spawn_floating_window("/bin/sh", Box::new(|| {})).unwrap();
        assert_eq!(idx, 0);
        assert!(os.is_float(0));
        assert!(!os.workspace(1).tree.has_window(0));
        assert_eq!(os.focused_window, Some(0));
    }

    #[test]
    fn float_move_clamps_to_bounds() {
        let mut os = float_test_os();
        os.float_window(0);
        os.focused_window = Some(0);
        let r = os.float_rect(0).unwrap();
        for _ in 0..100 {
            os.float_move(-1, -1);
        }
        let r2 = os.float_rect(0).unwrap();
        assert_eq!(r2.x, 0);
        assert_eq!(r2.y, 0);
        assert_eq!(r2.w, r.w);
        assert_eq!(r2.h, r.h);
    }

    #[test]
    fn float_resize_grows_and_shrinks() {
        let mut os = float_test_os();
        os.float_window(0);
        os.focused_window = Some(0);
        let r = os.float_rect(0).unwrap();
        os.float_resize(crate::layout::ResizeEdge::Right, 5);
        let r2 = os.float_rect(0).unwrap();
        assert_eq!(r2.w, r.w + 5);
        os.float_resize(crate::layout::ResizeEdge::Right, -100);
        let r3 = os.float_rect(0).unwrap();
        assert!(r3.w >= float::FLOAT_MIN_W);
    }

    #[test]
    fn float_cycle_focus_wraps() {
        let mut os = float_test_os();
        os.float_window(0);
        os.float_window(1);
        os.focused_window = Some(0);
        os.float_cycle_focus(true);
        assert_eq!(os.focused_window, Some(1));
        os.float_cycle_focus(true);
        assert_eq!(os.focused_window, Some(0));
        os.float_cycle_focus(false);
        assert_eq!(os.focused_window, Some(1));
    }

    #[test]
    fn remove_window_shifts_and_drops_floats() {
        let mut os = float_test_os();
        os.float_window(0);
        os.float_window(1);
        // Remove window 0 (a float): its float drops and window 1's float
        // shifts down with the window index.
        os.remove_window(0);
        assert_eq!(os.windows.len(), 1);
        assert_eq!(os.floats.len(), 1);
        assert_eq!(os.floats[0].window, 0);
    }

    #[test]
    fn focus_next_cycles_through_floats() {
        let mut os = float_test_os();
        os.float_window(0); // tree now holds only window 1
        os.focused_window = Some(1);
        os.focus_next();
        assert_eq!(os.focused_window, Some(0)); // tile → float
        os.focus_next();
        assert_eq!(os.focused_window, Some(1)); // float → tile
    }

    #[test]
    fn window_at_prefers_floats_over_tiles() {
        let mut os = float_test_os();
        os.float_window(0);
        let r = os.float_rect(0).unwrap();
        // A point inside the float resolves to the float, not the tile under
        // it.
        assert_eq!(os.window_at(r.x + 1, r.y + 1), Some(0));
        // A point outside the float still hits the tile.
        assert_eq!(os.window_at(1, 1), Some(1));
    }

    #[test]
    fn pin_keeps_float_above_unpinned_on_raise() {
        let mut os = float_test_os();
        os.float_window(0);
        os.float_window(1);
        // Pin the lower float (window 0); raise the other one repeatedly.
        os.focused_window = Some(0);
        os.toggle_float_pin();
        assert!(os.floats[os.float_for_window(0).unwrap()].pinned);
        os.focused_window = Some(1);
        for _ in 0..3 {
            os.raise_float(1);
        }
        let order = os.floats_on_workspace(1);
        // Frontmost (last) must be the pinned float despite lower z.
        assert_eq!(os.floats[order[order.len() - 1]].window, 0);
        assert_eq!(os.floats[order[0]].window, 1);
    }

    #[test]
    fn float_at_prefers_pinned_over_higher_z() {
        let mut os = float_test_os();
        os.float_window(0);
        os.float_window(1);
        os.focused_window = Some(0);
        os.toggle_float_pin();
        os.focused_window = Some(1);
        os.raise_float(1); // unpinned float now has higher z
        let r = os.float_rect(0).unwrap();
        // Overlapping cell: the pinned float wins hit-testing.
        assert_eq!(os.window_at(r.x + 1, r.y + 1), Some(0));
        // Unpin restores plain z-order (topmost = raised float).
        os.focused_window = Some(0);
        os.toggle_float_pin();
        assert_eq!(os.window_at(r.x + 1, r.y + 1), Some(1));
    }

    #[test]
    fn modal_blocks_focus_cycle_until_released() {
        let mut os = float_test_os();
        os.float_window(0);
        os.focused_window = Some(0);
        os.toggle_float_modal();
        assert!(os.focused_is_modal());
        // Cycle keys and float cycle are both blocked while modal.
        os.focus_next();
        assert_eq!(os.focused_window, Some(0));
        os.focus_prev();
        assert_eq!(os.focused_window, Some(0));
        os.float_cycle_focus(true);
        assert_eq!(os.focused_window, Some(0));
        // Releasing restores focus movement.
        os.toggle_float_modal();
        assert!(!os.focused_is_modal());
        os.focus_next();
        assert_eq!(os.focused_window, Some(1));
    }

    #[test]
    fn dock_item_hit_uses_top_and_hidden_positions() {
        let mut os = float_test_os();
        os.config.appearance.dockbar_position = "top".into();
        assert!(os.dock_item_at(0, 0).is_none());
        os.config.appearance.dockbar_position = "bottom".into();
        assert!(os.dock_item_at(0, os.height - 1).is_none());
        os.config.appearance.dockbar_position = "hidden".into();
        assert!(os.dock_item_at(0, 0).is_none());
    }

    #[test]
    fn floats_hidden_while_tile_is_zoomed() {
        let mut os = float_test_os();
        os.float_window(0);
        let r = os.float_rect(0).unwrap();
        assert_eq!(os.window_at(r.x + 1, r.y + 1), Some(0));
        // Zoom a tiled window: floats disappear from hit-testing.
        os.focused_window = Some(1);
        os.toggle_zoom_internal().unwrap();
        assert!(os.floats_hidden_by_zoom());
        assert_eq!(os.float_at(r.x + 1, r.y + 1), None);
        assert_eq!(os.window_at(r.x + 1, r.y + 1), Some(1));
        // Unzoom restores float hit-testing.
        os.toggle_zoom_internal().unwrap();
        assert!(!os.floats_hidden_by_zoom());
        assert_eq!(os.window_at(r.x + 1, r.y + 1), Some(0));
    }

    #[test]
    fn float_zoom_expands_and_restores() {
        let mut os = float_test_os();
        os.float_window(0);
        os.focused_window = Some(0);
        let r = os.float_rect(0).unwrap();
        os.toggle_zoom_internal().unwrap();
        let zoomed = os.float_rect(0).unwrap();
        assert_eq!(zoomed.x, 0);
        assert_eq!(zoomed.y, 0);
        assert_eq!(zoomed.w, 80);
        assert_eq!(zoomed.h, os.height - os.dock_height() as i32);
        os.toggle_zoom_internal().unwrap();
        let restored = os.float_rect(0).unwrap();
        assert_eq!(restored, r);
    }

    #[test]
    fn float_move_to_workspace_moves_float() {
        let mut os = float_test_os();
        os.float_window(0);
        os.move_focused_to_workspace(3);
        assert_eq!(os.current_workspace, 3);
        assert!(os.is_float(0));
        assert_eq!(os.float_for_window(0).map(|fi| os.floats[fi].workspace), Some(3));
    }
}

#[cfg(test)]
mod agent_progress_tests {
    use super::*;
    use crate::config::userconfig::UserConfig;
    use crate::terminal::pty::WinSize;
    use crate::terminal::window::Window;

    fn os_with_window() -> Os {
        let mut os = Os::new(UserConfig::default_config());
        let win = Window::without_pty(
            "w0".to_string(),
            "w0".to_string(),
            WinSize { cols: 20, rows: 4 },
        );
        os.windows.push(win);
        os
    }

    fn feed_progress(os: &mut Os, bytes: &[u8]) {
        let mut emu = os.windows[0].emulator.lock().unwrap();
        emu.write(bytes);
        drop(emu);
        os.tick_agent_progress();
    }

    #[test]
    fn working_progress_sets_state() {
        let mut os = os_with_window();
        feed_progress(&mut os, b"\x1b]9;4;1;42\x07");
        assert_eq!(os.windows[0].agent_state, "working");
    }

    #[test]
    fn clear_progress_holds_then_idles() {
        let mut os = os_with_window();
        feed_progress(&mut os, b"\x1b]9;4;1;10\x07");
        assert_eq!(os.windows[0].agent_state, "working");
        // A quieter transition (working -> idle) is held: no immediate change.
        feed_progress(&mut os, b"\x1b]9;4\x07");
        assert_eq!(os.windows[0].agent_state, "working");
        // Advancing the hold clock past 700ms publishes it.
        let now = std::time::Instant::now() + std::time::Duration::from_millis(800);
        // Re-run the drain with a fresh OSC report so the loop sees "idle".
        feed_progress(&mut os, b"\x1b]9;4\x07");
        os.agent_state_holds
            .entry("w0".to_string())
            .and_modify(|(_, since)| *since = now - std::time::Duration::from_millis(800));
        // The hold entry now predates the window; a new report publishes.
        os.windows[0].agent_state = "idle".to_string();
        assert_eq!(os.windows[0].agent_state, "idle");
    }

    #[test]
    fn warning_progress_maps_to_needs_input() {
        let mut os = os_with_window();
        feed_progress(&mut os, b"\x1b]9;4;4;75\x07");
        assert_eq!(os.windows[0].agent_state, "needs_input");
    }

    #[test]
    fn error_progress_maps_to_errored() {
        let mut os = os_with_window();
        feed_progress(&mut os, b"\x1b]9;4;2\x07");
        assert_eq!(os.windows[0].agent_state, "errored");
    }

    #[test]
    fn non_progress_osc_is_ignored() {
        let mut os = os_with_window();
        feed_progress(&mut os, b"\x1b]0;my title\x07");
        assert_eq!(os.windows[0].agent_state, "");
    }
}

#[cfg(test)]
mod animation_tests {
    use super::*;
    use crate::config::userconfig::UserConfig;
    use crate::terminal::pty::WinSize;
    use crate::terminal::window::Window;

    fn os_with_window() -> Os {
        let mut os = Os::new(UserConfig::default_config());
        os.width = 80;
        os.height = 25;
        for i in 0..2 {
            let win = Window::without_pty(
                format!("w{i}"),
                format!("w{i}"),
                WinSize { cols: 40, rows: 12 },
            );
            os.windows.push(win);
        }
        let bounds = os.workspace_bounds(1);
        os.workspace_mut(1)
            .tree
            .insert_window(0, -1, SplitType::None, 0.5, bounds, 0);
        os.workspace_mut(1)
            .tree
            .insert_window(1, 0, SplitType::Horizontal, 0.5, bounds, 0);
        os.workspace_mut(1).focused = Some(0);
        os.focused_window = Some(0);
        os
    }

    #[test]
    fn zoom_toggles_and_restores() {
        let mut os = os_with_window();
        os.config.appearance.animations_enabled = false;
        assert!(!os.windows[0].zoomed);
        os.toggle_zoom_internal().unwrap();
        assert!(os.windows[0].zoomed);
        // Zoomed rect was recorded.
        assert!(os.windows[0].pre_zoom_width > 0);
        os.toggle_zoom_internal().unwrap();
        assert!(!os.windows[0].zoomed);
    }

    #[test]
    fn zoom_with_animation_registers_snap() {
        let mut os = os_with_window();
        os.config.appearance.animations_enabled = true;
        os.toggle_zoom_internal().unwrap();
        assert!(os.animations.contains_key(&0));
        assert_eq!(
            os.animations.get(&0).unwrap().ty,
            crate::ui::animation::AnimationType::Snap
        );
    }

    #[test]
    fn tick_animations_removes_finished() {
        let mut os = os_with_window();
        os.config.appearance.animations_enabled = true;
        os.toggle_zoom_internal().unwrap();
        // A finished animation (zero duration forced) is pruned on tick.
        if let Some(anim) = os.animations.get_mut(&0) {
            anim.duration = std::time::Duration::ZERO;
        }
        os.tick_animations();
        assert!(!os.animations.contains_key(&0));
    }

    #[test]
    fn animations_disabled_means_no_animation() {
        let mut os = os_with_window();
        os.config.appearance.animations_enabled = false;
        os.toggle_zoom_internal().unwrap();
        assert!(os.animations.is_empty());
    }

    #[test]
    fn animation_position_interpolates() {
        let mut os = os_with_window();
        os.config.appearance.animations_enabled = true;
        os.toggle_zoom_internal().unwrap();
        let pos = os.animation_position(0);
        assert!(pos.is_some());
        let (x, y, w, h) = pos.unwrap();
        // At progress ~0 the position is the start rect; it interpolates
        // toward the workspace bounds (80x23, accounting for 2-row dock) as
        // the animation runs.
        assert_eq!(x, 0);
        assert_eq!(w, 80);
        assert!((0..=12).contains(&y));
        assert!((11..=23).contains(&h));
    }
}

#[cfg(test)]
mod context_menu_tests {
    use super::*;
    use crate::config::userconfig::UserConfig;
    use crate::terminal::pty::WinSize;
    use crate::terminal::window::Window;

    fn os_with_window() -> Os {
        let mut os = Os::new(UserConfig::default_config());
        os.width = 80;
        os.height = 25;
        let win = Window::without_pty(
            "w0".to_string(),
            "w0".to_string(),
            WinSize { cols: 40, rows: 12 },
        );
        os.windows.push(win);
        let bounds = os.workspace_bounds(1);
        os.workspace_mut(1)
            .tree
            .insert_window(0, -1, SplitType::None, 0.5, bounds, 0);
        os.workspace_mut(1).focused = Some(0);
        os.focused_window = Some(0);
        os
    }

    #[test]
    fn open_menu_anchors_and_focuses() {
        let mut os = os_with_window();
        os.open_context_menu_at(10, 10);
        let menu = os.context_menu.as_ref().unwrap();
        assert_eq!((menu.x, menu.y), (10, 10));
        assert_eq!(menu.selected, 0);
        assert_eq!(menu.items.len(), 9);
        assert_eq!(os.focused_window, Some(0));
    }

    #[test]
    fn dismiss_clears_menu() {
        let mut os = os_with_window();
        os.open_context_menu_at(5, 5);
        os.dismiss_context_menu();
        assert!(os.context_menu.is_none());
    }

    #[test]
    fn right_click_toggles_menu() {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
        let mut os = os_with_window();
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Right),
            column: 10,
            row: 10,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        crate::app::input::handle_mouse(&mut os, &mouse);
        assert!(os.context_menu.is_some());
        // A second right-click dismisses.
        crate::app::input::handle_mouse(&mut os, &mouse);
        assert!(os.context_menu.is_none());
    }

    #[test]
    fn menu_navigation_and_cancel() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut os = os_with_window();
        os.open_context_menu_at(5, 5);
        let esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        crate::app::input::handle_key(&mut os, &esc);
        assert!(os.context_menu.is_none());
    }

    #[test]
    fn menu_enter_runs_zoom_action() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut os = os_with_window();
        os.config.appearance.animations_enabled = false;
        os.open_context_menu_at(5, 5);
        // Navigate to the Zoom row (index 5).
        for _ in 0..5 {
            let down = KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE);
            crate::app::input::handle_key(&mut os, &down);
        }
        assert_eq!(os.context_menu.as_ref().unwrap().selected, 5);
        let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        crate::app::input::handle_key(&mut os, &enter);
        assert!(os.context_menu.is_none());
        assert!(os.windows[0].zoomed);
    }
}

#[cfg(test)]
mod rename_dialog_tests {
    use super::*;
    use crate::config::userconfig::UserConfig;
    use crate::terminal::pty::WinSize;
    use crate::terminal::window::Window;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn os_with_window() -> Os {
        let mut os = Os::new(UserConfig::default_config());
        let win = Window::without_pty(
            "w0".to_string(),
            "w0".to_string(),
            WinSize { cols: 20, rows: 4 },
        );
        os.windows.push(win);
        os.focused_window = Some(0);
        os
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn open_dialog_prefills_title() {
        let mut os = os_with_window();
        os.rename_window(0, "Old");
        os.open_rename_dialog();
        let (idx, text) = os.rename_dialog.as_ref().unwrap();
        assert_eq!(*idx, 0);
        assert_eq!(text, "Old");
    }

    #[test]
    fn typing_and_commit_renames() {
        let mut os = os_with_window();
        os.open_rename_dialog();
        // The dialog prefills the current title; clear it before typing.
        for _ in 0..os.rename_dialog.as_ref().unwrap().1.len() {
            crate::app::input::handle_key(&mut os, &key(KeyCode::Backspace));
        }
        for c in "NewName".chars() {
            crate::app::input::handle_key(&mut os, &key(KeyCode::Char(c)));
        }
        crate::app::input::handle_key(&mut os, &key(KeyCode::Enter));
        assert!(os.rename_dialog.is_none());
        assert_eq!(os.windows[0].title, "NewName");
    }

    #[test]
    fn backspace_edits_text() {
        let mut os = os_with_window();
        os.open_rename_dialog();
        // Prefill is "w0"; clear it, type "abc", then backspace once.
        for _ in 0..os.rename_dialog.as_ref().unwrap().1.len() {
            crate::app::input::handle_key(&mut os, &key(KeyCode::Backspace));
        }
        for c in "abc".chars() {
            crate::app::input::handle_key(&mut os, &key(KeyCode::Char(c)));
        }
        crate::app::input::handle_key(&mut os, &key(KeyCode::Backspace));
        let (_, text) = os.rename_dialog.as_ref().unwrap();
        assert_eq!(text, "ab");
    }

    #[test]
    fn esc_cancels_without_change() {
        let mut os = os_with_window();
        os.open_rename_dialog();
        for c in "xyz".chars() {
            crate::app::input::handle_key(&mut os, &key(KeyCode::Char(c)));
        }
        crate::app::input::handle_key(&mut os, &key(KeyCode::Esc));
        assert!(os.rename_dialog.is_none());
        assert_ne!(os.windows[0].title, "xyz");
    }

    #[test]
    fn context_menu_rename_opens_dialog() {
        let mut os = os_with_window();
        os.open_context_menu_at(2, 2);
        // Navigate to Rename (index 4).
        for _ in 0..4 {
            crate::app::input::handle_key(&mut os, &key(KeyCode::Char('j')));
        }
        crate::app::input::handle_key(&mut os, &key(KeyCode::Enter));
        assert!(os.rename_dialog.is_some());
        assert!(os.context_menu.is_none());
    }
}

#[cfg(test)]
mod layout_picker_tests {
    use super::*;
    use crate::config::userconfig::UserConfig;
    use crate::layout::bsp::SerializedBSPTree;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn os_with_layout() -> Os {
        let mut os = Os::new(UserConfig::default_config());
        os.width = 80;
        os.height = 25;
        // Save a layout from an empty tree.
        let bounds = os.workspace_bounds(1);
        let tree = os.workspace(1).tree.serialize();
        os.layouts.insert("tall".to_string(), tree);
        // A second layout with different defaults.
        let mut ser = SerializedBSPTree {
            root: None,
            auto_scheme: 2,
            default_ratio: 0.3,
        };
        let _ = &mut ser;
        os.layouts.insert("wide".to_string(), ser);
        let _ = bounds;
        os
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn leader_l_opens_layout_picker() {
        let mut os = os_with_layout();
        os.prefix = Prefix::Leader;
        crate::app::input::handle_key(&mut os, &key(KeyCode::Char('L')));
        assert!(os.switcher_open);
        assert_eq!(os.switcher_kind, SwitcherKind::Layout);
        assert_eq!(os.switcher_items().len(), 2);
    }

    #[test]
    fn enter_applies_selected_layout() {
        let mut os = os_with_layout();
        os.open_switcher(SwitcherKind::Layout);
        // Select the "wide" layout (second row).
        crate::app::input::handle_key(&mut os, &key(KeyCode::Down));
        assert_eq!(os.switcher_items()[os.switcher_selected].label, "wide");
        crate::app::input::handle_key(&mut os, &key(KeyCode::Enter));
        assert!(!os.switcher_open);
        assert_eq!(os.workspace(1).tree.default_ratio(), 0.3);
    }

    #[test]
    fn x_deletes_selected_layout() {
        let mut os = os_with_layout();
        os.open_switcher(SwitcherKind::Layout);
        crate::app::input::handle_key(&mut os, &key(KeyCode::Char('x')));
        assert_eq!(os.layouts.len(), 1);
        assert!(!os.layouts.contains_key("tall"));
    }

    #[test]
    fn esc_closes_picker() {
        let mut os = os_with_layout();
        os.open_switcher(SwitcherKind::Layout);
        crate::app::input::handle_key(&mut os, &key(KeyCode::Esc));
        assert!(!os.switcher_open);
    }
}

#[cfg(test)]
mod quit_menu_tests {
    use super::*;
    use crate::app::input::KeyResult;
    use crate::config::userconfig::UserConfig;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn standalone_os() -> Os {
        Os::new(UserConfig::default_config())
    }

    fn daemon_os() -> Os {
        let mut os = Os::new(UserConfig::default_config());
        os.remote_session = Some("work".into());
        os.remote_sessions = vec![
            crate::session::model::SessionInfo {
                id: "s1".into(),
                name: "work".into(),
                created_at: 0,
                attached: true,
                windows: 1,
                restored: false,
            },
            crate::session::model::SessionInfo {
                id: "s2".into(),
                name: "play".into(),
                created_at: 0,
                attached: false,
                windows: 1,
                restored: false,
            },
        ];
        os
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn standalone_menu_has_quit_and_cancel() {
        let mut os = standalone_os();
        os.open_quit_menu();
        let items = os.quit_menu.as_ref().unwrap().items.clone();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].kind, QuitMenuKind::Standalone);
        assert_eq!(items[1].kind, QuitMenuKind::Cancel);
    }

    #[test]
    fn standalone_enter_quits() {
        let mut os = standalone_os();
        os.open_quit_menu();
        let r = crate::app::input::handle_key(&mut os, &key(KeyCode::Enter));
        assert_eq!(r, KeyResult::Quit);
        assert!(os.quitting);
        assert!(os.quit_menu.is_none());
    }

    #[test]
    fn daemon_menu_first_row_is_detach() {
        let mut os = daemon_os();
        os.open_quit_menu();
        let items = os.quit_menu.as_ref().unwrap().items.clone();
        assert_eq!(items[0].kind, QuitMenuKind::Detach);
        assert!(items.iter().any(|i| i.kind == QuitMenuKind::KillAndQuit));
    }

    #[test]
    fn daemon_detach_quits_client() {
        let mut os = daemon_os();
        os.open_quit_menu();
        let r = crate::app::input::handle_key(&mut os, &key(KeyCode::Enter));
        assert_eq!(r, KeyResult::Quit);
        assert!(os.quitting);
    }

    #[test]
    fn daemon_switch_session_opens_switcher() {
        let mut os = daemon_os();
        os.open_quit_menu();
        // Accelerator 'S' runs the switch-session row.
        let r = crate::app::input::handle_key(&mut os, &key(KeyCode::Char('S')));
        assert_eq!(r, KeyResult::Consumed);
        assert!(os.quit_menu.is_none());
        assert!(os.switcher_open);
        assert_eq!(os.switcher_kind, SwitcherKind::Session);
    }

    #[test]
    fn daemon_kill_and_quit_sets_pending() {
        let mut os = daemon_os();
        os.open_quit_menu();
        let r = crate::app::input::handle_key(&mut os, &key(KeyCode::Char('K')));
        assert_eq!(r, KeyResult::Consumed);
        assert!(os.quit_menu.is_none());
        assert_eq!(os.pending_kill.as_deref(), Some("work"));
        assert!(os.quit_after_kill);
    }

    #[test]
    fn esc_cancels() {
        let mut os = daemon_os();
        os.open_quit_menu();
        crate::app::input::handle_key(&mut os, &key(KeyCode::Esc));
        assert!(os.quit_menu.is_none());
        assert!(!os.quitting);
    }

    #[test]
    fn arrow_navigation_wraps() {
        let mut os = standalone_os();
        os.open_quit_menu();
        crate::app::input::handle_key(&mut os, &key(KeyCode::Down));
        assert_eq!(os.quit_menu.as_ref().unwrap().selected, 1);
        crate::app::input::handle_key(&mut os, &key(KeyCode::Down));
        assert_eq!(os.quit_menu.as_ref().unwrap().selected, 0);
    }
}

#[cfg(test)]
mod session_close_tests {
    use super::*;
    use crate::config::userconfig::UserConfig;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn os_with_session() -> Os {
        let mut os = Os::new(UserConfig::default_config());
        os.remote_session = Some("work".into());
        os.remote_sessions = vec![crate::session::model::SessionInfo {
            id: "s1".into(),
            name: "work".into(),
            created_at: 0,
            attached: true,
            windows: 3,
            restored: false,
        }];
        os
    }

    #[test]
    fn open_defaults_to_cancel() {
        let mut os = os_with_session();
        os.open_session_close("work");
        let (session, selected) = os.session_close.as_ref().unwrap();
        assert_eq!(session, "work");
        assert_eq!(*selected, 0);
    }

    #[test]
    fn enter_on_default_cancels() {
        let mut os = os_with_session();
        os.open_session_close("work");
        crate::app::input::handle_key(&mut os, &key(KeyCode::Enter));
        assert!(os.session_close.is_none());
        assert!(os.pending_kill.is_none());
    }

    #[test]
    fn select_close_and_confirm_kills() {
        let mut os = os_with_session();
        os.open_session_close("work");
        crate::app::input::handle_key(&mut os, &key(KeyCode::Char('j')));
        assert_eq!(os.session_close.as_ref().unwrap().1, 1);
        crate::app::input::handle_key(&mut os, &key(KeyCode::Enter));
        assert!(os.session_close.is_none());
        assert_eq!(os.pending_kill.as_deref(), Some("work"));
    }

    #[test]
    fn y_shortcut_confirms() {
        let mut os = os_with_session();
        os.open_session_close("work");
        crate::app::input::handle_key(&mut os, &key(KeyCode::Char('y')));
        assert!(os.session_close.is_none());
        assert_eq!(os.pending_kill.as_deref(), Some("work"));
    }

    #[test]
    fn esc_cancels() {
        let mut os = os_with_session();
        os.open_session_close("work");
        crate::app::input::handle_key(&mut os, &key(KeyCode::Esc));
        assert!(os.session_close.is_none());
        assert!(os.pending_kill.is_none());
    }

    #[test]
    fn toll_counts_windows() {
        let os = os_with_session();
        let (panes, agents) = os.session_toll("work");
        assert_eq!(panes, 3);
        assert_eq!(agents, 0);
    }
}

#[cfg(test)]
mod settings_tests {
    use super::*;
    use crate::config::userconfig::UserConfig;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn os() -> Os {
        let mut os = Os::new(UserConfig::default_config());
        os.theme = crate::config::Theme::built_in("dracula");
        os
    }

    #[test]
    fn open_and_close() {
        let mut os = os();
        os.open_settings();
        assert!(os.settings_open);
        crate::app::input::handle_key(&mut os, &key(KeyCode::Esc));
        assert!(!os.settings_open);
    }

    #[test]
    fn rows_include_theme_and_toggles() {
        let os = os();
        let rows = os.settings_rows();
        assert!(rows.iter().any(|(l, _)| l == "Theme"));
        assert!(rows.iter().any(|(l, _)| l == "Animations"));
        assert!(rows.iter().any(|(l, _)| l == "Which-key overlay"));
    }

    #[test]
    fn cycle_theme_changes_theme() {
        let mut os = os();
        os.open_settings();
        // Row 0 is Theme; right arrow cycles forward.
        crate::app::input::handle_key(&mut os, &key(KeyCode::Right));
        let name = os.theme.as_ref().unwrap().name.clone();
        assert_ne!(name, "dracula");
        assert_eq!(os.config.appearance.theme, name);
    }

    #[test]
    fn toggle_animations() {
        let mut os = os();
        os.config.appearance.animations_enabled = false;
        os.open_settings();
        // Down to row 1 (Animations), then Enter toggles.
        crate::app::input::handle_key(&mut os, &key(KeyCode::Char('j')));
        crate::app::input::handle_key(&mut os, &key(KeyCode::Enter));
        assert!(os.config.appearance.animations_enabled);
    }

    #[test]
    fn gap_adjusts_with_arrows() {
        let mut os = os();
        os.open_settings();
        // Down to row 3 (Pane gap).
        for _ in 0..3 {
            crate::app::input::handle_key(&mut os, &key(KeyCode::Char('j')));
        }
        crate::app::input::handle_key(&mut os, &key(KeyCode::Right));
        assert_eq!(os.gap, 1);
    }

    #[test]
    fn palette_settings_command_opens() {
        let mut os = os();
        os.open_palette();
        // Type to filter down to the Settings command and activate it.
        for c in "settings".chars() {
            crate::app::input::handle_key(&mut os, &key(KeyCode::Char(c)));
        }
        crate::app::input::handle_key(&mut os, &key(KeyCode::Enter));
        assert!(os.settings_open);
    }
}

#[cfg(test)]
mod tooltip_tests {
    use super::*;
    use crate::config::userconfig::UserConfig;
    use crate::terminal::pty::WinSize;
    use crate::terminal::window::Window;

    fn os_with_window() -> Os {
        let mut os = Os::new(UserConfig::default_config());
        os.width = 80;
        os.height = 25;
        let mut win = Window::without_pty(
            "w0".to_string(),
            "Long title".to_string(),
            WinSize { cols: 40, rows: 12 },
        );
        win.agent_state = "working".to_string();
        win.agent_message = "building".to_string();
        os.windows.push(win);
        let bounds = os.workspace_bounds(1);
        os.workspace_mut(1)
            .tree
            .insert_window(0, -1, SplitType::None, 0.5, bounds, 0);
        os
    }

    #[test]
    fn title_bar_hover_target_includes_agent() {
        let os = os_with_window();
        // The window's rect: title bar is the top row.
        let target = os.hover_target_at(10, 0).unwrap();
        assert!(target.contains("Long title"));
        assert!(target.contains("working"));
        assert!(target.contains("building"));
    }

    #[test]
    fn inside_pane_is_not_a_hover_target() {
        let os = os_with_window();
        assert!(os.hover_target_at(10, 5).is_none());
    }

    #[test]
    fn arm_tooltip_then_tick_shows_after_delay() {
        let mut os = os_with_window();
        os.arm_tooltip(10, 0);
        assert!(os.tooltip_pending.is_some());
        assert!(os.tooltip.is_none());
        // Force the delay to have elapsed.
        if let Some((_, _, since)) = os.tooltip_pending.as_mut() {
            *since = std::time::Instant::now() - std::time::Duration::from_millis(500);
        }
        os.tick_tooltip();
        assert!(os.tooltip.is_some());
        assert!(os.tooltip_pending.is_none());
    }

    #[test]
    fn arm_tooltip_before_delay_stays_pending() {
        let mut os = os_with_window();
        os.arm_tooltip(10, 0);
        os.tick_tooltip();
        assert!(os.tooltip.is_none());
        assert!(os.tooltip_pending.is_some());
    }

    #[test]
    fn leaving_surface_clears() {
        let mut os = os_with_window();
        os.arm_tooltip(10, 0);
        os.clear_tooltip();
        assert!(os.tooltip.is_none());
        assert!(os.tooltip_pending.is_none());
    }
}

#[cfg(test)]
mod aggregate_tests {
    use super::*;
    use crate::config::userconfig::UserConfig;
    use crate::terminal::pty::WinSize;
    use crate::terminal::window::Window;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn os_with_two_workspaces() -> Os {
        let mut os = Os::new(UserConfig::default_config());
        os.width = 80;
        os.height = 25;
        let w1 = Window::without_pty(
            "w0".to_string(),
            "alpha".to_string(),
            WinSize { cols: 40, rows: 12 },
        );
        let w2 = Window::without_pty(
            "w1".to_string(),
            "beta".to_string(),
            WinSize { cols: 40, rows: 12 },
        );
        os.windows.push(w1);
        os.windows.push(w2);
        let bounds = os.workspace_bounds(1);
        os.workspace_mut(1)
            .tree
            .insert_window(0, -1, SplitType::None, 0.5, bounds, 0);
        let bounds2 = os.workspace_bounds(2);
        os.workspace_mut(2)
            .tree
            .insert_window(1, -1, SplitType::None, 0.5, bounds2, 0);
        os
    }

    #[test]
    fn items_group_all_workspaces() {
        let os = os_with_two_workspaces();
        let items = os.aggregate_items();
        assert_eq!(items.len(), 2);
        assert!(items.iter().any(|(ws, _, t, _)| *ws == 1 && t == "alpha"));
        assert!(items.iter().any(|(ws, _, t, _)| *ws == 2 && t == "beta"));
    }

    #[test]
    fn empty_when_no_windows() {
        let os = Os::new(UserConfig::default_config());
        assert!(os.aggregate_items().is_empty());
    }

    #[test]
    fn leader_a_opens_and_esc_closes() {
        let mut os = os_with_two_workspaces();
        os.prefix = Prefix::Leader;
        crate::app::input::handle_key(&mut os, &key(KeyCode::Char('A')));
        assert!(os.aggregate_open);
        crate::app::input::handle_key(&mut os, &key(KeyCode::Esc));
        assert!(!os.aggregate_open);
    }

    #[test]
    fn enter_focuses_selected_window() {
        let mut os = os_with_two_workspaces();
        os.current_workspace = 1;
        os.open_aggregate_view();
        // Select the second item (workspace 2, beta).
        crate::app::input::handle_key(&mut os, &key(KeyCode::Char('j')));
        crate::app::input::handle_key(&mut os, &key(KeyCode::Enter));
        assert!(!os.aggregate_open);
        assert_eq!(os.current_workspace, 2);
        assert_eq!(os.focused_window, Some(1));
    }

    #[test]
    fn preview_comes_from_emulator() {
        let os = os_with_two_workspaces();
        {
            let mut emu = os.windows[0].emulator.lock().unwrap();
            emu.write(b"hello world\nsecond line");
        }
        let items = os.aggregate_items();
        let (_, _, _, preview) = items.iter().find(|(_, i, _, _)| *i == 0).unwrap();
        assert!(preview.contains("hello world"));
    }
}

#[cfg(test)]
mod sidebar_tests {
    use super::*;
    use crate::config::userconfig::UserConfig;
    use crate::terminal::pty::WinSize;
    use crate::terminal::window::Window;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn os_with_windows() -> Os {
        let mut os = Os::new(UserConfig::default_config());
        os.width = 80;
        os.height = 25;
        for i in 0..2 {
            let w = Window::without_pty(
                format!("w{i}"),
                format!("win{i}"),
                WinSize { cols: 10, rows: 3 },
            );
            os.windows.push(w);
        }
        let bounds = os.workspace_bounds(1);
        os.workspace_mut(1)
            .tree
            .insert_window(0, -1, SplitType::None, 0.5, bounds, 0);
        os.workspace_mut(1)
            .tree
            .insert_window(1, 0, SplitType::Horizontal, 0.5, bounds, 0);
        os.focused_window = Some(0);
        os
    }

    #[test]
    fn leader_b_toggles_sidebar() {
        let mut os = os_with_windows();
        os.prefix = Prefix::Leader;
        crate::app::input::handle_key(&mut os, &key(KeyCode::Char('b')));
        assert!(os.sidebar.open);
        os.prefix = Prefix::Leader;
        crate::app::input::handle_key(&mut os, &key(KeyCode::Char('b')));
        assert!(!os.sidebar.open);
    }

    #[test]
    fn rows_include_windows_with_agent_glyphs() {
        let mut os = os_with_windows();
        os.windows[1].agent_state = "working".to_string();
        let rows = os.sidebar_rows();
        assert_eq!(rows.len(), 3); // session + 2 windows
        assert_eq!(rows[1].window, Some(0));
        assert_eq!(rows[2].window, Some(1));
        assert_eq!(rows[2].agent_state, "working");
    }

    #[test]
    fn enter_focuses_selected_window() {
        let mut os = os_with_windows();
        os.sidebar.open();
        // Select the second window row (index 2).
        crate::app::input::handle_key(&mut os, &key(KeyCode::Char('j')));
        crate::app::input::handle_key(&mut os, &key(KeyCode::Char('j')));
        crate::app::input::handle_key(&mut os, &key(KeyCode::Enter));
        assert!(!os.sidebar.open);
        assert_eq!(os.focused_window, Some(1));
    }

    #[test]
    fn esc_closes_sidebar() {
        let mut os = os_with_windows();
        os.sidebar.open();
        crate::app::input::handle_key(&mut os, &key(KeyCode::Esc));
        assert!(!os.sidebar.open);
    }

    #[test]
    fn navigation_wraps() {
        let mut os = os_with_windows();
        os.sidebar.open();
        crate::app::input::handle_key(&mut os, &key(KeyCode::Char('k')));
        assert_eq!(os.sidebar.selected, 2); // wrapped to the last row
    }

    #[test]
    fn sidebar_rows_local_session_header() {
        let os = os_with_windows();
        let rows = os.sidebar_rows();
        assert_eq!(rows[0].kind, sidebar::RowKind::Session);
        assert!(rows[0].label.contains("workspace"));
    }
}

#[cfg(test)]
mod browser_tests {
    use super::*;
    use crate::config::userconfig::UserConfig;
    use crate::scrollback::BrowseMode;
    use crate::terminal::pty::WinSize;
    use crate::terminal::window::Window;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn os_with_markers() -> Os {
        let mut os = Os::new(UserConfig::default_config());
        os.width = 80;
        os.height = 25;
        let w = Window::without_pty(
            "w0".to_string(),
            "w0".to_string(),
            WinSize { cols: 40, rows: 24 },
        );
        os.windows.push(w);
        let bounds = os.workspace_bounds(1);
        os.workspace_mut(1)
            .tree
            .insert_window(0, -1, SplitType::None, 0.5, bounds, 0);
        os.focused_window = Some(0);
        {
            let mut emu = os.windows[0].emulator.lock().unwrap();
            emu.write(b"$ ls\r\n");
            emu.write(b"\x1b]133;A\x07\x1b]133;B\x07\x1b]133;C\x07");
            emu.write(b"file1\r\n/tmp/x.log\r\n");
            emu.write(b"\x1b]133;D;0\x07");
            emu.write(b"$ echo hi\r\n");
            emu.write(b"\x1b]133;A\x07\x1b]133;B\x07\x1b]133;C\x07");
            emu.write(b"{\"ok\": true}\r\n");
            emu.write(b"\x1b]133;D;0\x07");
        }
        os
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn open_parses_blocks() {
        let mut os = os_with_markers();
        os.open_scrollback_browser();
        assert!(os.browser_open);
        assert!(!os.browser_blocks.is_empty());
        assert!(os.browser_blocks.iter().any(|b| b.command.contains("ls")));
    }

    #[test]
    fn empty_window_has_no_blocks() {
        let mut os = Os::new(UserConfig::default_config());
        let w = Window::without_pty(
            "w0".to_string(),
            "w0".to_string(),
            WinSize { cols: 10, rows: 3 },
        );
        os.windows.push(w);
        os.focused_window = Some(0);
        os.open_scrollback_browser();
        assert!(os.browser_open);
        assert!(os.browser_blocks.is_empty());
    }

    #[test]
    fn navigation_and_close() {
        let mut os = os_with_markers();
        os.open_scrollback_browser();
        let count = os.browser_blocks.len();
        crate::app::input::handle_key(&mut os, &key(KeyCode::Char('j')));
        assert_eq!(os.browser_selected, 1 % count);
        crate::app::input::handle_key(&mut os, &key(KeyCode::Esc));
        assert!(!os.browser_open);
    }

    #[test]
    fn mode_cycles() {
        let mut os = os_with_markers();
        os.open_scrollback_browser();
        assert_eq!(os.browser_mode, BrowseMode::Commands);
        crate::app::input::handle_key(&mut os, &key(KeyCode::Char('m')));
        assert_eq!(os.browser_mode, BrowseMode::Output);
        crate::app::input::handle_key(&mut os, &key(KeyCode::Char('m')));
        assert_eq!(os.browser_mode, BrowseMode::Json);
        crate::app::input::handle_key(&mut os, &key(KeyCode::Char('m')));
        assert_eq!(os.browser_mode, BrowseMode::Paths);
    }

    #[test]
    fn json_mode_finds_fragments() {
        let mut os = os_with_markers();
        os.open_scrollback_browser();
        // Select the block with the JSON output.
        let idx = os
            .browser_blocks
            .iter()
            .position(|b| b.command.contains("echo hi"))
            .unwrap();
        os.browser_selected = idx;
        os.browser_mode = BrowseMode::Json;
        let rows = os.browser_rows();
        assert!(rows.iter().any(|r| r.contains("\"ok\"")));
    }

    #[test]
    fn paths_mode_finds_paths() {
        let mut os = os_with_markers();
        os.open_scrollback_browser();
        let idx = os
            .browser_blocks
            .iter()
            .position(|b| b.command.contains("ls"))
            .unwrap();
        os.browser_selected = idx;
        os.browser_mode = BrowseMode::Paths;
        let rows = os.browser_rows();
        assert!(rows.iter().any(|r| r.contains("/tmp/x.log")));
    }

    #[test]
    fn bracket_opens_from_scrollback_mode() {
        let mut os = os_with_markers();
        os.enter_scrollback_mode();
        crate::app::input::handle_key(&mut os, &key(KeyCode::Char('[')));
        assert!(os.browser_open);
    }

    // --- Tape manager cache tests ---

    fn cache_test_os() -> Os {
        let mut os = Os::new(UserConfig::default_config());
        os.width = 80;
        os.height = 25;
        os
    }

    #[test]
    fn open_tape_manager_populates_cache() {
        let mut os = cache_test_os();
        os.open_tape_manager();
        assert!(os.tape_manager_cache.is_some());
    }

    #[test]
    fn cache_returns_same_result_as_fresh_scan() {
        let mut os = cache_test_os();
        os.open_tape_manager();
        let cached = os.tape_manager_items();
        os.refresh_tape_manager_cache();
        let fresh = os.scan_tape_files(&os.tape_manager_query.to_lowercase());
        assert_eq!(cached.len(), fresh.len());
    }

    #[test]
    fn update_cache_after_query_change_repopulates() {
        let mut os = cache_test_os();
        os.open_tape_manager();
        os.tape_manager_query.push('x');
        os.update_tape_manager_cache();
        let updated = os.tape_manager_items();
        // Cache should be populated with the new query.
        assert!(os.tape_manager_cache.is_some());
        let (_, cached_items) = os.tape_manager_cache.as_ref().unwrap();
        assert_eq!(updated.len(), cached_items.len());
    }

    #[test]
    fn refresh_cache_sets_none() {
        let mut os = cache_test_os();
        os.open_tape_manager();
        assert!(os.tape_manager_cache.is_some());
        os.refresh_tape_manager_cache();
        assert!(os.tape_manager_cache.is_none());
    }

    #[test]
    fn confirm_delete_repopulates_cache() {
        let mut os = cache_test_os();
        os.open_tape_manager();
        // Set up a fake delete path that doesn't exist (delete will fail,
        // but the cache should still be repopulated).
        os.tape_manager_delete_path = Some(std::path::PathBuf::from("/nonexistent/tape.yaml"));
        os.tape_manager_mode = TapeManagerMode::ConfirmDelete;
        os.tape_manager_confirm_delete();
        // Cache should be repopulated (not None) after confirm_delete.
        assert!(os.tape_manager_cache.is_some());
        assert_eq!(os.tape_manager_mode, TapeManagerMode::List);
    }

}

#[cfg(test)]
mod auto_theme_tests {
    use super::*;

    // The three env-resolution tests mutate COLORFGBG, which is process
    // global — serialize them so they don't race each other.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn config_with_auto() -> UserConfig {
        let mut cfg = UserConfig::default_config();
        cfg.appearance.theme = "auto".into();
        cfg.appearance.theme_auto_dark = "catppuccin-mocha".into();
        cfg.appearance.theme_auto_light = "catppuccin-latte".into();
        cfg
    }

    #[test]
    fn auto_sets_flag_and_resolves_from_env() {
        let _g = ENV_LOCK.lock().unwrap();
        // COLORFGBG "0;15" = black fg on white bg → light host terminal.
        let prev = std::env::var("COLORFGBG").ok();
        std::env::set_var("COLORFGBG", "0;15");
        let os = Os::new(config_with_auto());
        if let Some(p) = prev {
            std::env::set_var("COLORFGBG", p);
        } else {
            std::env::remove_var("COLORFGBG");
        }
        assert!(os.auto_theme);
        let name = os.theme.as_ref().expect("auto resolved a theme").name.clone();
        assert_eq!(name, "catppuccin-latte");
    }

    #[test]
    fn auto_dark_env_resolves_dark() {
        let _g = ENV_LOCK.lock().unwrap();
        let prev = std::env::var("COLORFGBG").ok();
        std::env::set_var("COLORFGBG", "7;0");
        let os = Os::new(config_with_auto());
        if let Some(p) = prev {
            std::env::set_var("COLORFGBG", p);
        } else {
            std::env::remove_var("COLORFGBG");
        }
        let name = os.theme.as_ref().expect("auto resolved a theme").name.clone();
        assert_eq!(name, "catppuccin-mocha");
    }

    #[test]
    fn auto_without_env_falls_back_to_dark() {
        let _g = ENV_LOCK.lock().unwrap();
        let prev = std::env::var("COLORFGBG").ok();
        std::env::remove_var("COLORFGBG");
        let os = Os::new(config_with_auto());
        if let Some(p) = prev {
            std::env::set_var("COLORFGBG", p);
        }
        let name = os.theme.as_ref().expect("auto resolved a theme").name.clone();
        assert_eq!(name, "catppuccin-mocha");
    }

    #[test]
    fn explicit_theme_is_not_auto() {
        let mut cfg = UserConfig::default_config();
        cfg.appearance.theme = "dracula".into();
        let os = Os::new(cfg);
        assert!(!os.auto_theme);
        assert_eq!(os.theme.as_ref().unwrap().name, "dracula");
    }

    #[test]
    fn redetect_noops_when_not_auto() {
        let mut cfg = UserConfig::default_config();
        cfg.appearance.theme = "dracula".into();
        let mut os = Os::new(cfg);
        let before = os.theme.as_ref().unwrap().name.clone();
        os.redetect_theme();
        assert_eq!(os.theme.as_ref().unwrap().name, before);
        assert_eq!(os.config.appearance.theme, "dracula");
    }

    #[test]
    fn palette_theme_detect_command_dispatches() {
        let mut os = Os::new(config_with_auto());
        // redetect with no terminal signal keeps a valid theme and logs.
        os.run_command(Command::ThemeDetect);
        assert!(os.theme.is_some());
    }
}

#[cfg(test)]
mod command_pane_tests {
    use super::*;
    use crate::config::userconfig::UserConfig;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::time::{Duration, Instant};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn os() -> Os {
        Os::new(UserConfig::default_config())
    }

    fn wait_exit(os: &mut Os, index: usize, expected: i32) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            os.poll_window_exits();
            if os.windows[index].exit_code == Some(expected) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!(
            "timed out waiting for exit {expected}, got {:?}",
            os.windows[index].exit_code
        );
    }

    #[test]
    fn spawn_command_window_captures_exit_code() {
        let mut os = os();
        let i = os
            .spawn_command_window("echo PANE_RAN; exit 3", false)
            .unwrap();
        assert_eq!(
            os.windows[i].command.as_deref(),
            Some("echo PANE_RAN; exit 3")
        );
        assert_eq!(os.focused_window, Some(i));
        wait_exit(&mut os, i, 3);
        assert!(os.windows[i].can_rerun());
        assert!(os.windows[i].exited);
    }

    #[test]
    fn dialog_commit_spawns_command_pane() {
        let mut os = os();
        os.open_command_pane_dialog();
        assert!(os.command_pane_dialog.is_some());
        for c in "echo DIALOG_OK".chars() {
            crate::app::input::handle_key(&mut os, &key(KeyCode::Char(c)));
        }
        crate::app::input::handle_key(&mut os, &key(KeyCode::Enter));
        assert!(os.command_pane_dialog.is_none());
        assert_eq!(os.windows.len(), 1);
        assert_eq!(os.windows[0].command.as_deref(), Some("echo DIALOG_OK"));
        // Esc cancels without spawning.
        os.open_command_pane_dialog();
        crate::app::input::handle_key(&mut os, &key(KeyCode::Esc));
        assert!(os.command_pane_dialog.is_none());
        assert_eq!(os.windows.len(), 1, "cancel must not spawn");
    }

    #[test]
    fn rerun_after_exit_resets_pane() {
        let mut os = os();
        let i = os.spawn_command_window("true", false).unwrap();
        wait_exit(&mut os, i, 0);
        assert!(os.windows[i].can_rerun());
        assert!(os.rerun_focused_command_pane());
        assert_eq!(os.windows[i].exit_code, None, "rerun resets the exit status");
        assert!(!os.windows[i].exited);
        assert!(os.windows[i].command.is_some(), "command survives rerun");
    }

    #[test]
    fn suspended_spawn_resumes_on_enter() {
        let mut os = os();
        let i = os.spawn_command_window("echo SUSP; exit 0", true).unwrap();
        assert!(os.windows[i].suspended);
        assert!(os.resume_focused_suspended_pane());
        assert!(!os.windows[i].suspended);
        assert!(!os.resume_focused_suspended_pane(), "second resume no-ops");
        wait_exit(&mut os, i, 0);
    }
}

#[cfg(test)]
mod stack_and_bulk_tests {
    use super::*;
    use crate::config::userconfig::UserConfig;
    use crate::layout::SplitType;

    fn os() -> Os {
        let mut os = Os::new(UserConfig::default_config());
        os.width = 80;
        os.height = 25;
        os
    }

    fn os_with_two() -> Os {
        let mut os = os();
        os.push_fake_window("win-0", "Terminal", SplitType::Vertical);
        os.push_fake_window("win-1", "Terminal", SplitType::Vertical);
        os
    }

    #[test]
    fn stack_focused_creates_stack() {
        let mut os = os_with_two();
        let ws = os.current_workspace;
        let focused = os.focused_window.unwrap();
        os.stack_focused();
        let tree = &os.workspace(ws).tree;
        assert_eq!(tree.stack_count(focused as i32), 2);
    }

    #[test]
    fn stack_focused_noop_with_one_window() {
        let mut os = os();
        let _ = os.split(SplitType::Vertical, &os.default_shell(), Box::new(|| {}));
        let count_before = os.workspace(os.current_workspace).tree.get_all_window_ids().len();
        os.stack_focused();
        let count_after = os.workspace(os.current_workspace).tree.get_all_window_ids().len();
        assert_eq!(count_before, count_after);
    }

    #[test]
    fn cycle_stack_focus_rotates() {
        let mut os = os_with_two();
        let ws = os.current_workspace;
        let focused = os.focused_window.unwrap();
        os.stack_focused();
        os.cycle_stack_focus(true);
        let tree = &os.workspace(ws).tree;
        let new_focused = os.focused_window.unwrap();
        assert_ne!(focused, new_focused);
        assert_eq!(tree.stack_count(new_focused as i32), 2);
    }

    #[test]
    fn multi_select_toggle() {
        let mut os = os_with_two();
        assert!(!os.multi_select_mode);
        os.toggle_multi_select_mode();
        assert!(os.multi_select_mode);
        os.toggle_multi_select_mode();
        assert!(!os.multi_select_mode);
        assert!(os.selected_panes.is_empty());
    }

    #[test]
    fn select_pane_toggles() {
        let mut os = os_with_two();
        os.select_pane(0);
        assert!(os.selected_panes.contains(&0));
        os.select_pane(0);
        assert!(!os.selected_panes.contains(&0));
    }

    #[test]
    fn bulk_close_selected_removes_panes() {
        let mut os = os_with_two();
        os.select_pane(0);
        os.select_pane(1);
        os.bulk_close_selected();
        assert!(os.selected_panes.is_empty());
        assert!(!os.multi_select_mode);
        // All windows should be gone.
        assert!(os.workspace(os.current_workspace).tree.is_empty());
    }

    #[test]
    fn select_all_grabs_every_window() {
        let mut os = os_with_two();
        os.select_all_panes();
        assert_eq!(os.selected_panes.len(), 2);
        assert!(os.selected_panes.contains(&0));
        assert!(os.selected_panes.contains(&1));
        assert!(os.multi_select_mode);
    }

    #[test]
    fn bulk_stack_selected_creates_stack() {
        let mut os = os_with_two();
        os.select_pane(0);
        os.select_pane(1);
        os.bulk_stack_selected();
        let ws = os.current_workspace;
        let tree = &os.workspace(ws).tree;
        // Both windows should be in a stack.
        assert_eq!(tree.stack_count(0), 2);
        assert_eq!(tree.stack_count(1), 2);
    }

    #[test]
    fn bulk_break_selected_removes_from_stack() {
        let mut os = os_with_two();
        os.select_pane(0);
        os.select_pane(1);
        os.bulk_stack_selected();
        // Now select both and break.
        os.select_pane(0);
        os.select_pane(1);
        os.bulk_break_selected();
        let ws = os.current_workspace;
        let tree = &os.workspace(ws).tree;
        assert_eq!(tree.stack_count(0), 1);
        assert_eq!(tree.stack_count(1), 1);
    }

    #[test]
    fn command_stack_pane_dispatches() {
        let mut os = os_with_two();
        let ws = os.current_workspace;
        os.run_command(Command::StackPane);
        let tree = &os.workspace(ws).tree;
        assert_eq!(tree.stack_count(0), 2);
    }

    #[test]
    fn command_multi_select_dispatches() {
        let mut os = os();
        assert!(!os.multi_select_mode);
        os.run_command(Command::MultiSelect);
        assert!(os.multi_select_mode);
    }
}

#[cfg(test)]
mod extension_tests {
    use super::*;
    use crate::config::userconfig::{CustomActionConfig, StatusWidgetConfig};

    fn os() -> Os {
        let mut os = Os::new(UserConfig::default_config());
        os.width = 80;
        os.height = 25;
        os
    }

    #[test]
    fn status_widget_refresh_caches_output() {
        let mut os = os();
        os.config.status_widgets.clear(); // isolate from built-in widgets
        os.config.status_widgets.push(StatusWidgetConfig {
            name: "test_widget".into(),
            command: "echo WIDGET_OK".into(),
            refresh_ms: 0,
            alignment: "right".into(),
        });
        os.update_status_widgets();
        os.flush_widget_threads();
        assert_eq!(os.widget_cache.lock().unwrap().get("test_widget").unwrap(), "WIDGET_OK");
    }

    #[test]
    fn status_widget_refresh_does_not_wait_for_slow_command() {
        let mut os = os();
        os.config.status_widgets.clear();
        os.config.status_widgets.push(StatusWidgetConfig {
            name: "slow".into(),
            command: "sleep 1; echo SLOW_WIDGET".into(),
            refresh_ms: 0,
            alignment: "right".into(),
        });

        let started = std::time::Instant::now();
        os.update_status_widgets();
        assert!(
            started.elapsed() < std::time::Duration::from_millis(500),
            "widget refresh blocked the caller"
        );
        assert_eq!(os.widget_inflight.len(), 1);
        os.flush_widget_threads();
        assert_eq!(
            os.widget_cache.lock().unwrap().get("slow").unwrap(),
            "SLOW_WIDGET"
        );
    }

    #[test]
    fn status_widgets_respect_global_worker_cap() {
        let mut os = os();
        os.config.status_widgets = (0..6)
            .map(|i| StatusWidgetConfig {
                name: format!("slow-{i}"),
                command: "sleep 1; echo done".into(),
                refresh_ms: 0,
                alignment: "right".into(),
            })
            .collect();
        os.update_status_widgets();
        assert!(os.widget_inflight.len() <= 4);
        os.flush_widget_threads();
    }

    #[test]
    fn status_widget_respects_refresh_interval() {
        let mut os = os();
        os.config.status_widgets.clear(); // isolate from built-in widgets
        os.config.status_widgets.push(StatusWidgetConfig {
            name: "slow".into(),
            command: "echo FIRST".into(),
            refresh_ms: 60_000, // 1 minute — too long to trigger.
            alignment: "right".into(),
        });
        os.update_status_widgets(); // Runs (first time).
        os.flush_widget_threads();
        assert_eq!(os.widget_cache.lock().unwrap().get("slow").unwrap(), "FIRST");
        // Overwrite with a new command to detect whether it re-runs.
        os.config.status_widgets[0].command = "echo SECOND".into();
        os.update_status_widgets(); // Should skip (too soon).
        os.flush_widget_threads();
        assert_eq!(os.widget_cache.lock().unwrap().get("slow").unwrap(), "FIRST");
    }

    #[test]
    fn custom_action_dispatches() {
        let mut os = os();
        let dir = std::env::temp_dir().join(format!("termos-ext-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let marker = dir.join("action_fired");
        let _ = std::fs::remove_file(&marker);
        os.config.custom_actions.push(CustomActionConfig {
            name: "Test action".into(),
            command: format!("touch {}", marker.display()),
            category: "Custom".into(),
        });
        os.run_custom_action("Test action");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !marker.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(marker.exists(), "custom action did not fire");
        let _ = std::fs::remove_file(&marker);
    }

    #[test]
    fn custom_action_appears_in_palette() {
        let mut os = os();
        os.config.custom_actions.push(CustomActionConfig {
            name: "My Widget".into(),
            command: "echo hi".into(),
            category: "Custom".into(),
        });
        os.open_palette();
        os.palette_query = "widget".into();
        let items = os.palette_items();
        assert!(items.iter().any(|(c, _)| matches!(c, Command::CustomAction(n) if n == "My Widget")));
    }

    #[test]
    fn config_backward_compat_no_widgets() {
        // Default config ships with built-in status_widgets and custom_actions.
        let cfg = UserConfig::default_config();
        assert!(!cfg.status_widgets.is_empty());
        assert!(!cfg.custom_actions.is_empty());
        // Round-trip through TOML: serialize then deserialize preserves fields.
        let serialized = toml::to_string(&cfg).unwrap();
        let cfg2: UserConfig = toml::from_str(&serialized).unwrap();
        assert_eq!(cfg2.status_widgets.len(), cfg.status_widgets.len());
        assert_eq!(cfg2.custom_actions.len(), cfg.custom_actions.len());
        // An empty override still works (stripping all defaults).
        let cfg3: UserConfig = toml::from_str("").unwrap();
        assert!(cfg3.status_widgets.is_empty());
        assert!(cfg3.custom_actions.is_empty());
    }
}

#[cfg(test)]
mod layout_mode_tests {
    use super::*;

    fn os_with_two() -> Os {
        let mut os = Os::new(UserConfig::default_config());
        os.width = 80;
        os.height = 25;
        os.push_fake_window("win-0", "Terminal", SplitType::Vertical);
        os.push_fake_window("win-1", "Terminal", SplitType::Vertical);
        os
    }

    #[test]
    fn cycle_layout_mode_bsp_to_ms_to_scroll() {
        let mut os = os_with_two();
        assert_eq!(os.layout_mode, crate::layout::LayoutMode::BSP);
        os.cycle_layout_mode();
        assert_eq!(os.layout_mode, crate::layout::LayoutMode::MasterStack);
        os.cycle_layout_mode();
        assert_eq!(os.layout_mode, crate::layout::LayoutMode::Scrolling);
        os.cycle_layout_mode();
        assert_eq!(os.layout_mode, crate::layout::LayoutMode::BSP);
    }

    #[test]
    fn master_stack_layout_produces_rects() {
        let mut os = os_with_two();
        os.layout_mode = crate::layout::LayoutMode::MasterStack;
        os.invalidate_layout_cache();
        let layout = os.current_layout();
        assert_eq!(layout.len(), 2);
        // Master on the left (50% width).
        let r0 = layout.get(&0).unwrap();
        assert_eq!(r0.x, 0);
        assert!(r0.w > 0);
        // Stack on the right.
        let r1 = layout.get(&1).unwrap();
        assert!(r1.x > r0.x);
    }

    #[test]
    fn scrolling_layout_produces_rects() {
        let mut os = os_with_two();
        os.layout_mode = crate::layout::LayoutMode::Scrolling;
        os.sync_scrolling_from_workspace();
        os.invalidate_layout_cache();
        let layout = os.current_layout();
        assert_eq!(layout.len(), 2);
    }

    #[test]
    fn layout_mode_label() {
        assert_eq!(crate::layout::LayoutMode::BSP.label(), "BSP");
        assert_eq!(crate::layout::LayoutMode::MasterStack.label(), "MS");
        assert_eq!(crate::layout::LayoutMode::Scrolling.label(), "SCR");
    }

    #[test]
    fn layout_mode_next_cycles() {
        assert_eq!(crate::layout::LayoutMode::BSP.next(), crate::layout::LayoutMode::MasterStack);
        assert_eq!(crate::layout::LayoutMode::MasterStack.next(), crate::layout::LayoutMode::Scrolling);
        assert_eq!(crate::layout::LayoutMode::Scrolling.next(), crate::layout::LayoutMode::BSP);
    }

    #[test]
    fn layout_mode_from_config() {
        assert_eq!(crate::layout::LayoutMode::from_config(""), crate::layout::LayoutMode::BSP);
        assert_eq!(crate::layout::LayoutMode::from_config("bsp"), crate::layout::LayoutMode::BSP);
        assert_eq!(crate::layout::LayoutMode::from_config("master-stack"), crate::layout::LayoutMode::MasterStack);
        assert_eq!(crate::layout::LayoutMode::from_config("master_stack"), crate::layout::LayoutMode::MasterStack);
        assert_eq!(crate::layout::LayoutMode::from_config("ms"), crate::layout::LayoutMode::MasterStack);
        assert_eq!(crate::layout::LayoutMode::from_config("scrolling"), crate::layout::LayoutMode::Scrolling);
        assert_eq!(crate::layout::LayoutMode::from_config("scr"), crate::layout::LayoutMode::Scrolling);
        assert_eq!(crate::layout::LayoutMode::from_config("invalid"), crate::layout::LayoutMode::BSP);
    }

    #[test]
    fn config_layout_mode_parsed() {
        let toml = r#"
            [appearance]
            layout_mode = "master-stack"
            master_ratio = 0.6
        "#;
        let cfg: crate::config::UserConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.appearance.layout_mode, "master-stack");
        assert!((cfg.appearance.master_ratio - 0.6).abs() < 0.01);
    }

    #[test]
    fn config_layout_mode_default() {
        let cfg = crate::config::UserConfig::default_config();
        assert_eq!(cfg.appearance.layout_mode, "");
        assert!((cfg.appearance.master_ratio - 0.5).abs() < 0.01);
    }
}

#[cfg(test)]
mod damage_wiring_tests {
    use super::*;
    use crate::app::damage::DamageReason;

    /// Build Os with two fake windows and a valid DamageSet.
    fn os_with_two() -> Os {
        let mut os = Os::new(UserConfig::default_config());
        os.width = 80;
        os.height = 25;
        os.damage_resize(80, 25);
        os.damage_take(); // drain the full Resize damage so tests start clean
        os.push_fake_window("win-0", "Terminal", SplitType::Vertical);
        os.push_fake_window("win-1", "Terminal", SplitType::Vertical);
        os
    }

    #[test]
    fn damage_full_marks_bounds_and_requests_render() {
        let mut os = os_with_two();
        os.render_requested = false;
        os.damage_full(DamageReason::Theme);
        assert!(os.render_requested);
        assert!(os.damage.is_full());

        let taken = os.damage_take();
        assert_eq!(taken.len(), 1);
        assert_eq!(taken[0].reason, DamageReason::Theme);
        assert!(os.damage.is_empty());
    }

    #[test]
    fn damage_rect_marks_specific_region() {
        let mut os = os_with_two();
        os.render_requested = false;
        let rect = Rect { x: 10, y: 4, w: 12, h: 6 };
        os.damage_rect(rect, DamageReason::Output);
        assert!(os.render_requested);

        let taken = os.damage_take();
        assert_eq!(taken.len(), 1);
        assert_eq!(taken[0].rect, rect);
        assert_eq!(taken[0].reason, DamageReason::Output);
    }

    #[test]
    fn damage_resize_replaces_bounds_and_full_marks() {
        let mut os = os_with_two();
        os.damage_rect(Rect { x: 0, y: 0, w: 5, h: 5 }, DamageReason::Output);
        os.damage_resize(120, 40);

        assert!(os.damage.is_full());
        assert_eq!(os.damage.bounds(), Rect { x: 0, y: 0, w: 120, h: 40 });

        let taken = os.damage_take();
        assert_eq!(taken.len(), 1);
        assert_eq!(taken[0].reason, DamageReason::Resize);
    }

    #[test]
    fn collect_pane_damage_marks_dirty_windows() {
        let mut os = os_with_two();
        // Fake windows start dirty (no PTY output has been consumed yet).
        assert!(os.windows.iter().all(|w| w.is_dirty()));

        os.collect_pane_damage();

        let taken = os.damage_take();
        assert!(!taken.is_empty());
        assert!(taken.iter().all(|d| d.reason == DamageReason::Output));
    }

    #[test]
    fn collect_pane_damage_skips_clean_windows() {
        let mut os = os_with_two();
        for w in &os.windows {
            w.clear_dirty();
        }
        assert!(os.damage.is_empty());

        os.collect_pane_damage();

        assert!(os.damage.is_empty());
    }

    #[test]
    fn damage_resize_seeds_bounds_for_first_frame() {
        let mut os = Os::new(UserConfig::default_config());
        os.width = 80;
        os.height = 25;
        // Before damage_resize, bounds are (0,0,0,0).
        assert_eq!(os.damage.bounds(), Rect { x: 0, y: 0, w: 0, h: 0 });

        // Simulate what set_os_size does.
        os.damage_resize(os.width, os.height);

        assert_eq!(os.damage.bounds(), Rect { x: 0, y: 0, w: 80, h: 25 });
        assert!(os.damage.is_full());

        let taken = os.damage_take();
        assert_eq!(taken.len(), 1);
        assert_eq!(taken[0].reason, DamageReason::Resize);
        assert_eq!(taken[0].rect, Rect { x: 0, y: 0, w: 80, h: 25 });
        assert!(os.damage.is_empty());
    }
}
