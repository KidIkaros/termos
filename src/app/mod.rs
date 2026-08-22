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
pub mod metrics;
pub mod msg;
pub mod notifications;
pub mod notifications_channel;
pub mod overlay_hit;
pub mod overlay_mouse;
pub mod pixel_canvas;
pub mod render;
pub mod sidebar;
pub mod update;
mod ui_ops;
mod agent_ops;
mod tape_ops;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::config::userconfig::UserConfig;
use crate::config::Theme;
use crate::hooks;
use crate::layout::{AutoScheme, BSPTree, PreselectionDir, Rect, SerializedBSPTree, SplitType};
use crate::session::model::WindowInfo;
use crate::session::protocol::Message;
use crate::terminal::pty::{PtySink, WinSize};
use crate::terminal::window::{SpawnOptions, Window};
use crate::vt::Emulator;

mod types;
pub use types::*;
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
    /// Drag state for widget reorder in the switcher: (grab_row, last_row).
    pub switcher_drag: Option<(usize, usize)>,
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

    // --- System design modules ---

    /// Snowflake ID generator for windows, sessions, and entities.
    pub id_generator: crate::util::Snowflake,
    /// Tiered rate limiter for PTY allocation, input, and notifications.
    pub rate_limiter: crate::util::TieredRateLimiter,
    /// Structured notification pipeline with templates and channels.
    pub notification_pipeline: notifications::NotificationPipeline,
    /// Metrics collector for pane I/O and session aggregation.
    pub metrics: metrics::MetricsCollector,
    /// Widget dashboard registry.
    pub widget_registry: crate::widgets::WidgetRegistry,
    /// Whether the widget dashboard overlay is visible.
    pub dashboard_open: bool,
    /// Whether the dashboard sidebar panel is visible (side-left / side-right mode).
    pub dashboard_sidebar_visible: bool,
    /// Widget IDs enabled in the dashboard. Empty = all enabled.
    pub enabled_widgets: std::collections::HashSet<String>,
    /// Widget reorder undo stack (most recent first).
    pub widget_layout_undo: Vec<crate::widgets::layout::WidgetLayout>,
    /// Widget reorder redo stack.
    pub widget_layout_redo: Vec<crate::widgets::layout::WidgetLayout>,
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
        let dashboard_cfg = config.dashboard.clone();
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
            switcher_drag: None,
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

            // System design modules
            id_generator: crate::util::Snowflake::from_process(),
            rate_limiter: crate::util::TieredRateLimiter::default(),
            notification_pipeline: notifications::NotificationPipeline::new(),
            metrics: metrics::MetricsCollector::new(),
            widget_registry: Self::build_default_widgets(&dashboard_cfg),
            dashboard_open: false,
            dashboard_sidebar_visible: true,
            enabled_widgets: std::collections::HashSet::new(),
            widget_layout_undo: Vec::new(),
            widget_layout_redo: Vec::new(),
        }
    }

    /// Create the widget registry from dashboard config.
    fn build_default_widgets(dash_cfg: &crate::config::userconfig::DashboardConfig) -> crate::widgets::WidgetRegistry {
        let mut reg = crate::widgets::WidgetRegistry::new();

        // Register all built-in widgets.
        reg.register(Box::new(crate::widgets::system::CpuWidget::new()));
        reg.register(Box::new(crate::widgets::system::MemWidget::new()));
        reg.register(Box::new(crate::widgets::system::DiskWidget::new()));
        reg.register(Box::new(crate::widgets::system::NetWidget::new()));
        reg.register(Box::new(crate::widgets::system::ProcWidget::new()));
        reg.register(Box::new(crate::widgets::dev::GitWidget::new()));
        reg.register(Box::new(crate::widgets::dev::BuildWidget::new()));
        reg.register(Box::new(crate::widgets::utility::ClockWidget::new()));
        reg.register(Box::new(crate::widgets::utility::NotesWidget::new()));
        reg.register(Box::new(crate::widgets::utility::ActionsWidget::new()));

        // Build layout from config.
        use crate::widgets::layout::{WidgetLayout, WidgetSlot, DashboardPosition, Side};
        let position = match dash_cfg.position.as_str() {
            "side-left" => DashboardPosition::Side(Side::Left),
            "side-right" => DashboardPosition::Side(Side::Right),
            "bottom" => DashboardPosition::Bottom,
            _ => DashboardPosition::Overlay,
        };

        if dash_cfg.widgets.is_empty() {
            // No explicit placement — auto-layout all registered widgets.
            let ids: Vec<String> = reg.ids().into_iter().map(String::from).collect();
            let mut layout = WidgetLayout::auto_layout(&ids, dash_cfg.columns, dash_cfg.rows, dash_cfg.gap);
            layout.visible = dash_cfg.enabled;
            layout.position = position;
            *reg.layout_mut() = layout;
        } else {
            // Explicit widget placement from TOML.
            let slots: Vec<WidgetSlot> = dash_cfg.widgets.iter().map(|w| WidgetSlot {
                widget_id: w.id.clone(),
                col: w.col,
                row: w.row,
                width: w.width,
                height: w.height,
            }).collect();
            let layout = WidgetLayout {
                columns: dash_cfg.columns,
                rows: dash_cfg.rows,
                gap: dash_cfg.gap,
                slots,
                visible: dash_cfg.enabled,
                position,
            };
            *reg.layout_mut() = layout;
        }

        reg
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

    /// Update the metrics collector. Currently tracks window count and uptime.
    /// I/O per-pane tracking will be wired when the drain thread exposes
    /// atomic counters.
    pub fn tick_metrics(&mut self) {
        // Clean up metrics for windows that no longer exist.
        let active: std::collections::HashSet<String> =
            self.windows.iter().map(|w| w.id.clone()).collect();
        // (metrics cleanup will be added when pane I/O counters are wired)
        let _ = active;
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

    /// The last mouse position as (column, row).
    pub fn last_mouse_pos(&self) -> (i32, i32) {
        self.last_mouse_pos
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

    /// Minimize the focused window: hide it from the tiling layout and show it
    /// as a clickable icon in the dock.  The shell process continues running.
    pub fn minimize_focused(&mut self) {
        if let Some(idx) = self.focused_window {
            if let Some(window) = self.windows.get_mut(idx) {
                if !window.minimized {
                    window.minimized = true;
                    self.begin_minimize(idx as i32);
                    // Move focus to the next tiled window.
                    self.focus_next();
                    self.damage_full(crate::app::damage::DamageReason::Geometry);
                }
            }
        }
    }

    /// Restore a minimized window by its index: unminimize it and move focus
    /// to it so it reappears in the tiling layout.
    pub fn restore_window(&mut self, index: usize) {
        if let Some(window) = self.windows.get_mut(index) {
            if window.minimized {
                window.minimized = false;
                self.begin_restore(index as i32);
                self.focused_window = Some(index);
                self.damage_full(crate::app::damage::DamageReason::Geometry);
            }
        }
    }

    /// Restore the most recently minimized window (or the last window index
    /// that is minimized).  Used by dock click and the `m r` key chord.
    pub fn restore_last_minimized(&mut self) {
        // Walk windows in reverse to find the last minimized one.
        if let Some(idx) = self.windows.iter().enumerate().rev()
            .find(|(_, w)| w.minimized)
            .map(|(i, _)| i)
        {
            self.restore_window(idx);
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
        let layout = match self.layout_mode {
            crate::layout::LayoutMode::BSP => self.current_layout_bsp(),
            crate::layout::LayoutMode::MasterStack => self.current_layout_master_stack(),
            crate::layout::LayoutMode::Scrolling => self.current_layout_scrolling(),
        };
        // Exclude minimized windows — they don't participate in tiling.
        layout.into_iter()
            .filter(|(id, _)| {
                self.windows.get(*id as usize)
                    .map(|w| !w.minimized)
                    .unwrap_or(true)
            })
            .collect()
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
        self.notifications.push(Notification { message: message.clone(), kind: kind.clone() });
        if self.notifications.len() > 5 {
            self.notifications.remove(0);
        }
        // Feed the structured notification pipeline.
        let mut vars = std::collections::HashMap::new();
        vars.insert("message".to_string(), message);
        vars.insert("kind".to_string(), kind);
        self.notification_pipeline.notify(
            "generic",
            &vars,
            notifications_channel::Channel::TuiOverlay,
        );
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
    fn play_alert_sound(&mut self, _cue: &str) {
        // Sound module removed in core split.
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

    fn minimize_window_by_id(&mut self, window_id: &str) -> Result<(), String> {
        if let Ok(idx) = window_id.parse::<usize>() {
            if let Some(w) = self.windows.get_mut(idx) {
                w.minimized = true;
                self.begin_minimize(idx as i32);
                self.damage_full(crate::app::damage::DamageReason::Geometry);
                return Ok(());
            }
        }
        Err(format!("window {window_id} not found"))
    }

    fn minimize_window_by_name(&mut self, _name: &str) -> Result<(), String> {
        Err("minimize by name not yet supported".into())
    }

    fn restore_window_by_id(&mut self, window_id: &str) -> Result<(), String> {
        if let Ok(idx) = window_id.parse::<usize>() {
            self.restore_window(idx);
            return Ok(());
        }
        Err(format!("window {window_id} not found"))
    }

    fn restore_window_by_name(&mut self, _name: &str) -> Result<(), String> {
        Err("restore by name not yet supported".into())
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



#[cfg(test)]
mod tests;
