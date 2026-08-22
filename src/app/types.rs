use crate::layout::{BSPTree, Rect, SerializedBSPTree};

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
            | Command::OpenAggregate
            | Command::SwitchWorkspace(_) | Command::WorkspaceSwitcher => "Workspace",
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
pub(crate) struct LayoutCacheKey {
    pub(crate) workspace: i32,
    pub(crate) bounds: Rect,
    pub(crate) gap: i32,
    pub(crate) tree: SerializedBSPTree,
}

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
pub(crate) fn cols_overlap(x1: i32, w1: i32, x2: i32, w2: i32) -> bool {
    x1 < x2 + w2 && x2 < x1 + w1
}

/// Whether two y-ranges overlap.
pub(crate) fn rows_overlap(y1: i32, h1: i32, y2: i32, h2: i32) -> bool {
    y1 < y2 + h2 && y2 < y1 + h1
}

/// Expand a rect by `margin` cells on all sides, clamped to non-negative.
pub(crate) fn expand_rect(r: Rect, margin: i32) -> Rect {
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

