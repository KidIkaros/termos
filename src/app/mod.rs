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

    #[test]
    fn minimize_focused_hides_window_from_layout() {
        let mut os = os_with_two();
        os.focused_window = Some(0);
        let before = os.current_layout();
        assert!(before.contains_key(&0));

        os.minimize_focused();

        assert!(os.windows[0].minimized);
        let after = os.current_layout();
        assert!(!after.contains_key(&0), "minimized window should not be in layout");
        // Focus should have moved to window 1.
        assert_eq!(os.focused_window, Some(1));
    }

    #[test]
    fn restore_window_brings_it_back() {
        let mut os = os_with_two();
        os.focused_window = Some(0);
        os.minimize_focused();
        assert!(os.windows[0].minimized);

        os.restore_window(0);

        assert!(!os.windows[0].minimized);
        assert_eq!(os.focused_window, Some(0));
        let layout = os.current_layout();
        assert!(layout.contains_key(&0));
    }

    #[test]
    fn restore_last_minimized_picks_last() {
        let mut os = os_with_two();
        os.focused_window = Some(0);
        os.minimize_focused();
        os.focused_window = Some(1);
        os.minimize_focused();
        // Both minimized.
        assert!(os.windows[0].minimized);
        assert!(os.windows[1].minimized);

        os.restore_last_minimized();

        // Last minimized (index 1) should be restored.
        assert!(!os.windows[1].minimized);
        assert!(os.windows[0].minimized);
    }

    #[test]
    fn dock_items_include_minimized_windows() {
        use crate::app::dock::get_dock_items;
        let mut os = os_with_two();
        os.focused_window = Some(0);
        os.minimize_focused();

        let items = get_dock_items(&os);
        assert_eq!(items.len(), 1);
        assert!(items[0].minimized);
        assert_eq!(items[0].window_index, 0);
    }

    #[test]
    fn dock_items_empty_when_no_minimized() {
        use crate::app::dock::get_dock_items;
        let os = os_with_two();
        let items = get_dock_items(&os);
        assert!(items.is_empty());
    }

    #[test]
    fn dock_count_includes_all_minimized() {
        use crate::app::dock::build_dock_left_text;
        let mut os = os_with_two();
        os.focused_window = Some(0);
        os.minimize_focused();
        os.focused_window = Some(1);
        os.minimize_focused();
        // build_dock_left_text counts via BSP tree which retains minimized IDs.
        let (_, trail, _) = build_dock_left_text(&os);
        assert!(trail.contains(":2 "), "trail should contain ':2 ' but got: {trail}");
    }

    #[test]
    fn all_minimized_layout_empty() {
        let mut os = os_with_two();
        os.focused_window = Some(0);
        os.minimize_focused();
        os.focused_window = Some(1);
        os.minimize_focused();
        // current_layout should be empty (all minimized).
        let layout = os.current_layout();
        assert!(layout.is_empty());
    }

    #[test]
    fn all_minimized_dock_items_count() {
        use crate::app::dock::get_dock_items;
        let mut os = os_with_two();
        os.focused_window = Some(0);
        os.minimize_focused();
        os.focused_window = Some(1);
        os.minimize_focused();
        let items = get_dock_items(&os);
        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|i| i.minimized));
    }
}
