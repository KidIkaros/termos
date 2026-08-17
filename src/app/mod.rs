//! The window manager — the central application state. Ported from TUIOS
//! `internal/app` (the `OS` struct and its input/render layers).
//!
//! The `Os` struct owns the windows, workspaces, modes, and prefix state. It
//! is a plain state machine: the event loop feeds it input and it produces
//! render state, mirroring the Model-View-Update pattern the Go code gets from
//! Bubble Tea.

pub mod agent_alert;
pub mod input;
pub mod render;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::config::userconfig::UserConfig;
use crate::config::Theme;
use crate::hooks;
use crate::layout::{AutoScheme, BSPTree, PreselectionDir, Rect, SplitType};
use crate::session::model::WindowInfo;
use crate::session::protocol::Message;
use crate::terminal::pty::{PtySink, WinSize};
use crate::terminal::window::Window;
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
}

/// A command the command palette can run. Ported from the TUIOS command list,
/// adapted to the local single-process architecture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
        ];
        for i in 1..=9 {
            cmds.push(Command::SwitchWorkspace(i));
        }
        cmds.push(Command::Quit);
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
    if query.is_empty() {
        return true;
    }
    let text = text.to_lowercase();
    let query = query.to_lowercase();
    let mut it = text.chars();
    for q in query.chars() {
        let found = it.by_ref().find(|&c| c == q);
        if found.is_none() {
            return false;
        }
    }
    true
}

/// Rank how well `text` matches `query`, lower is better. `None` means no
/// match. Prefix matches beat word-boundary matches beat subsequence matches,
/// so the most relevant row sorts to the top of a fuzzy-filtered list.
pub fn fuzzy_rank(text: &str, query: &str) -> Option<usize> {
    if query.is_empty() {
        return Some(0);
    }
    let text_l = text.to_lowercase();
    let query_l = query.to_lowercase();
    if text_l.starts_with(&query_l) {
        return Some(0);
    }
    for word in text_l.split(|c: char| !c.is_alphanumeric()) {
        if word.starts_with(&query_l) {
            return Some(1);
        }
    }
    if matches_query(text, query) {
        return Some(2);
    }
    None
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
    /// Terminal dimensions in cells.
    pub width: i32,
    pub height: i32,
    /// Whether shared borders (tmux-style separators) are on.
    pub shared_borders: bool,
    /// Gap in cells between panes (0 when not shared).
    pub gap: i32,
    /// The auto-split scheme.
    pub auto_scheme: AutoScheme,
    /// Pending preselection direction.
    pub preselection: PreselectionDir,
    /// Notifications to show in the dock.
    pub notifications: Vec<Notification>,
    /// Whether the app is quitting.
    pub quitting: bool,
    /// Whether a quit confirmation is being shown.
    pub show_quit_confirmation: bool,
    /// Command palette state.
    pub palette_open: bool,
    pub palette_query: String,
    pub palette_selected: usize,
    /// Switcher (workspace/window) state.
    pub switcher_open: bool,
    pub switcher_kind: SwitcherKind,
    pub switcher_query: String,
    pub switcher_selected: usize,
    /// Whether vim-like scrollback navigation is active.
    pub scrollback_mode: bool,
    /// Copy-mode cursor position (content coordinates), used while
    /// `scrollback_mode` is active.
    pub copy_cursor_line: usize,
    pub copy_cursor_col: i32,
    /// Whether vim visual selection is active in copy mode.
    pub copy_visual: bool,
    /// The active selection (keyboard visual or mouse drag), if any.
    pub selection: Option<Selection>,
    /// Whether a mouse drag selection is in progress.
    pub mouse_selecting: bool,
    /// The last yanked text (internal clipboard).
    pub clipboard: String,
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
    /// Lifecycle hooks, loaded from the `[hooks]` config section.
    pub hook_manager: hooks::Manager,
    /// Agent alerts parked in their settle window, keyed by window id.
    pending_agent_alerts: HashMap<String, agent_alert::PendingAgentAlert>,
    /// Global audible-cue cooldown across every pane.
    sound_cue: agent_alert::SoundCue,
    /// Host-terminal sequences queued by alerts (OSC 9 / BEL), flushed by the
    /// event loop after each draw so they never interleave a frame.
    host_output: Vec<u8>,
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
    /// Remote `tape exec` progress (current index, total), if one is running.
    pub remote_tape: Option<(usize, usize)>,
    /// A discovered project tape awaiting a trust decision (the review
    /// dialog). `content` is the exact hashed bytes to execute on approval.
    pub project_tape_pending: Option<ProjectTapePending>,
}

/// A discovered `.tuios.tape` waiting on the trust review.
#[derive(Debug, Clone)]
pub struct ProjectTapePending {
    pub path: String,
    pub hash: String,
    pub content: Vec<u8>,
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
        let theme = if config.appearance.theme.is_empty() {
            None
        } else {
            Theme::built_in(&config.appearance.theme)
        };
        let shared_borders = config.appearance.shared_borders;
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
            mode: Mode::WindowManagement,
            prefix: Prefix::None,
            workspaces,
            current_workspace: 1,
            config,
            theme,
            width: 80,
            height: 24,
            shared_borders,
            gap: if shared_borders { 1 } else { 0 },
            auto_scheme: AutoScheme::Spiral,
            preselection: PreselectionDir::None,
            notifications: Vec::new(),
            quitting: false,
            show_quit_confirmation: false,
            palette_open: false,
            palette_query: String::new(),
            palette_selected: 0,
            switcher_open: false,
            switcher_kind: SwitcherKind::Workspace,
            switcher_query: String::new(),
            switcher_selected: 0,
            scrollback_mode: false,
            copy_cursor_line: 0,
            copy_cursor_col: 0,
            copy_visual: false,
            selection: None,
            mouse_selecting: false,
            clipboard: String::new(),
            remote_session: None,
            remote_sessions: Vec::new(),
            pending_switch: None,
            pending_kill: None,
            remote_commands: None,
            pending_split: None,
            hook_manager,
            pending_agent_alerts: HashMap::new(),
            sound_cue: agent_alert::SoundCue::new(),
            host_output: Vec::new(),
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
            remote_tape: None,
            project_tape_pending: None,
        }
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
        self.fire_hook(
            hooks::Event::AfterLayoutChange,
            hooks::Context {
                layout: "bsp".into(),
                ..hooks::Context::default()
            },
        );
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
                if self.windows.get(focused).map(|w| w.id == window_id).unwrap_or(false) {
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
                    if self.windows.get(focused).map(|w| w.id == p.window_id).unwrap_or(false) {
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

    /// Drain the queued host-terminal sequences.
    pub fn take_host_sequence(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.host_output)
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
        self.script_wait_regex =
            Some((re, std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms)));
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
            .map(|emu| re.is_match(&emu.to_string()))
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
            return Some(if total == 0 { 100 } else { index.saturating_mul(100).checked_div(total).unwrap_or(100) });
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
        let content = recorder.string("Recorded in TUIOS");
        let name = format!("recording-{}", crate::tape::tapes::timestamp_stamp());
        match crate::tape::tapes::save_tape(&name, &content) {
            Ok(path) => {
                self.notify(format!("saved {count} commands to {}", path.display()), "info");
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
        self.prefix = Prefix::None;
    }

    /// The tape files for the manager overlay, filtered by the query.
    pub fn tape_manager_items(&self) -> Vec<std::path::PathBuf> {
        let Ok(files) = crate::tape::tapes::list_tapes() else {
            return Vec::new();
        };
        let query = self.tape_manager_query.to_lowercase();
        files
            .into_iter()
            .filter(|p| {
                query.is_empty()
                    || p.file_name()
                        .map(|n| n.to_string_lossy().to_lowercase().contains(&query))
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
                self.notify(format!("project tape is ineligible: {}", result.reason), "error");
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

    fn workspace_mut(&mut self, number: i32) -> &mut Workspace {
        self.workspaces.entry(number).or_insert_with(|| Workspace::new(number))
    }

    fn workspace(&self, number: i32) -> &Workspace {
        self.workspaces.get(&number).expect("workspace exists")
    }

    /// The windows on the current workspace, in layout order (BSP IDs).
    pub fn current_workspace_windows(&self) -> Vec<i32> {
        let ws = self.workspace(self.current_workspace);
        ws.tree.get_all_window_ids()
    }

    /// True if the given window index is on the current workspace.
    pub fn window_on_current_workspace(&self, index: usize) -> bool {
        let ws = self.workspace(self.current_workspace);
        ws.tree.get_all_window_ids().contains(&(index as i32))
    }

    // -----------------------------------------------------------------------
    // Window creation
    // -----------------------------------------------------------------------

    /// Spawn a new shell window on the current workspace. If there is a
    /// focused window, it splits (BSP); otherwise the window becomes the root.
    pub fn spawn_window(&mut self, shell: &str, wake: Box<dyn Fn() + Send + 'static>) -> Result<usize, String> {
        let index = self.windows.len();
        let id = format!("win-{index}");
        let size = WinSize {
            cols: 80,
            rows: 24,
        };
        let env = vec![("TUIOS_ENV".to_string(), "1".to_string())];
        let window = Window::spawn(id, "Terminal", size, shell, None, wake, &env)
            .map_err(|e| e.to_string())?;
        self.windows.push(window);

        // Insert into the current workspace's BSP tree.
        let ws = self.current_workspace;
        let bounds = self.workspace_bounds(ws);
        let focused = self.workspace(ws).focused;
        let gap = self.gap;
        let tree = &mut self.workspace_mut(ws).tree;
        match focused {
            Some(f) => {
                tree.insert_window(
                    index as i32,
                    f as i32,
                    SplitType::None,
                    0.5,
                    bounds,
                    gap,
                );
            }
            None => {
                tree.insert_window(index as i32, -1, SplitType::None, 0.5, bounds, gap);
            }
        }
        self.workspace_mut(ws).focused = Some(index);
        self.focused_window = Some(index);
        self.record_action("new_window", &[]);
        let ctx = self.window_hook_ctx(index);
        self.fire_hook(hooks::Event::AfterNewWindow, ctx);
        Ok(index)
    }

    /// The usable bounds of a workspace, minus the dock bar.
    pub fn workspace_bounds(&self, _ws: i32) -> Rect {
        let dock_height = 1;
        Rect {
            x: 0,
            y: 0,
            w: self.width,
            h: (self.height - dock_height).max(1),
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
        // The workspace that owns this window (current if unknown).
        let mut target_ws = self.current_workspace;
        for ws_num in 1..=9 {
            if self.workspace(ws_num).tree.has_window(index as i32) {
                target_ws = ws_num;
                break;
            }
        }

        self.windows.remove(index);

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
                let new_id = if old_id > index as i32 { old_id - 1 } else { old_id };
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
        if target_ws == self.current_workspace {
            self.focused_window = remaining.first().map(|&i| i as usize);
            if self.focused_window.is_none() {
                self.mode = Mode::WindowManagement;
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
        let ws = self.current_workspace;
        let ids = self.workspace(ws).tree.get_all_window_ids();
        if ids.is_empty() {
            self.focused_window = None;
            return;
        }
        let current = self.focused_window;
        let next = match current {
            Some(c) => {
                let pos = ids.iter().position(|&id| id == c as i32).unwrap_or(0);
                ids[(pos + 1) % ids.len()]
            }
            None => ids[0],
        };
        if self.focused_window != Some(next as usize) {
            self.focused_window = Some(next as usize);
            self.workspace_mut(ws).focused = Some(next as usize);
            self.record_action("next_window", &[]);
            let ctx = self.window_hook_ctx(next as usize);
            self.fire_hook(hooks::Event::AfterFocusChange, ctx);
        }
    }

    pub fn focus_prev(&mut self) {
        let ws = self.current_workspace;
        let ids = self.workspace(ws).tree.get_all_window_ids();
        if ids.is_empty() {
            self.focused_window = None;
            return;
        }
        let current = self.focused_window;
        let next = match current {
            Some(c) => {
                let pos = ids.iter().position(|&id| id == c as i32).unwrap_or(0);
                ids[(pos + ids.len() - 1) % ids.len()]
            }
            None => ids[0],
        };
        if self.focused_window != Some(next as usize) {
            self.focused_window = Some(next as usize);
            self.workspace_mut(ws).focused = Some(next as usize);
            self.record_action("prev_window", &[]);
            let ctx = self.window_hook_ctx(next as usize);
            self.fire_hook(hooks::Event::AfterFocusChange, ctx);
        }
    }

    /// Focus the window at the given index (if on the current workspace).
    pub fn focus_window(&mut self, index: usize) {
        let ws = self.current_workspace;
        if self.workspace(ws).tree.has_window(index as i32)
            && self.focused_window != Some(index)
        {
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
    }

    /// Move the focused window to another workspace and follow it.
    pub fn move_focused_to_workspace(&mut self, number: i32) {
        if !(1..=9).contains(&number) || number == self.current_workspace {
            return;
        }
        let Some(focused) = self.focused_window else {
            return;
        };
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
    pub fn split(&mut self, direction: SplitType, shell: &str, wake: Box<dyn Fn() + Send + 'static>) -> Result<usize, String> {
        let index = self.windows.len();
        let id = format!("win-{index}");
        let size = WinSize { cols: 40, rows: 12 };
        let env = vec![("TUIOS_ENV".to_string(), "1".to_string())];
        let window = Window::spawn(id, "Terminal", size, shell, None, wake, &env)
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

    /// Resize all windows to their BSP layout rects. Windows whose size
    /// actually changed after the initial layout fire the after-resize hook.
    pub fn sync_window_sizes(&mut self) {
        let ws = self.current_workspace;
        let bounds = self.workspace_bounds(ws);
        let layout = self.workspace(ws).tree.apply_layout(bounds, self.gap);
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
        for (index, rect) in resized {
            let mut ctx = self.window_hook_ctx(index);
            ctx.width = rect.w;
            ctx.height = rect.h;
            self.fire_hook(hooks::Event::AfterResize, ctx);
        }
    }

    /// The layout rects for the current workspace.
    pub fn current_layout(&self) -> HashMap<i32, Rect> {
        let ws = self.current_workspace;
        let bounds = self.workspace_bounds(ws);
        self.workspace(ws).tree.apply_layout(bounds, self.gap)
    }

    /// The focused window's emulator, if any.
    pub fn focused_emulator(&self) -> Option<Arc<Mutex<Emulator>>> {
        self.focused_window
            .and_then(|index| self.windows.get(index))
            .map(|w| Arc::clone(&w.emulator))
    }

    pub fn notify(&mut self, message: impl Into<String>, kind: impl Into<String>) {
        self.notifications.push(Notification {
            message: message.into(),
            kind: kind.into(),
        });
        if self.notifications.len() > 5 {
            self.notifications.remove(0);
        }
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
    pub fn palette_items(&self) -> Vec<Command> {
        let mut items: Vec<(usize, Command)> = Command::all()
            .into_iter()
            .filter_map(|c| fuzzy_rank(&c.label(), &self.palette_query).map(|r| (r, c)))
            .collect();
        items.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.label().cmp(&b.1.label())));
        items.into_iter().map(|(_, c)| c).collect()
    }

    pub fn palette_move(&mut self, delta: i32) {
        let len = self.palette_items().len();
        if len == 0 {
            return;
        }
        let sel = self.palette_selected as i32 + delta;
        self.palette_selected = sel.rem_euclid(len as i32) as usize;
    }

    /// Run the selected command and close the palette.
    pub fn activate_palette(&mut self) {
        let items = self.palette_items();
        let cmd = items.get(self.palette_selected).copied();
        self.close_palette();
        if let Some(cmd) = cmd {
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
            Command::EqualizeSplits => {
                let ws = self.current_workspace;
                self.workspace_mut(ws).tree.equalize_ratios();
            }
            Command::Scrollback => self.enter_scrollback_mode(),
            Command::SwitchWorkspace(i) => self.switch_workspace(i),
            Command::Quit => {
                self.show_quit_confirmation = true;
            }
        }
    }

    fn default_shell(&self) -> String {
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
    pub fn toggle_visual(&mut self) {
        let Some(i) = self.focused_window else {
            return;
        };
        if self.copy_visual {
            self.copy_visual = false;
            self.selection = None;
        } else {
            self.copy_visual = true;
            self.selection = Some(Selection {
                window: i,
                anchor_line: self.copy_cursor_line,
                anchor_col: self.copy_cursor_col,
                cursor_line: self.copy_cursor_line,
                cursor_col: self.copy_cursor_col,
            });
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
            emu.selection_text(sel.anchor_line, sel.anchor_col, sel.cursor_line, sel.cursor_col)
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
        use std::io::Write;
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
        let seq = format!("\x1b]52;c;{b64}\x07");
        let mut out = std::io::stdout();
        let _ = out.write_all(seq.as_bytes());
        let _ = out.flush();
    }

    /// The content position (line, column) under a screen cell coordinate for
    /// a window.
    pub fn content_position_at(&self, window: usize, column: i32, row: i32) -> Option<(usize, i32)> {
        let layout = self.current_layout();
        let rect = layout.get(&(window as i32))?;
        // The pane border consumes the outer ring; content starts one cell in.
        let rel_row = (row - rect.y - 1).max(0);
        let rel_col = (column - rect.x - 1).max(0);
        let w = self.windows.get(window)?;
        let emu = w.emulator.lock().ok()?;
        let line = emu.content_index_for_view_row(rel_row);
        Some((line, rel_col))
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
        let mut best: Option<(usize, i32)> = None;
        for (window_id, rect) in layout {
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
        let idx = self.spawn_window(&shell, Box::new(|| {})).map_err(|e| e.to_string())?;
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
        Err("zoom is not implemented in this port".into())
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

    fn save_layout(&mut self, _name: &str) -> Result<(), String> {
        Err("SaveLayout is not implemented in this port".into())
    }

    fn load_layout(&mut self, _name: &str) -> Result<(), String> {
        Err("LoadLayout is not implemented in this port".into())
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

    fn focus_direction(&mut self, _direction: &str) -> Result<(), String> {
        Err("directional focus is not implemented in this port".into())
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
    fn fuzzy_match_is_subsequence_case_insensitive() {
        assert!(matches_query("Switch to workspace 3", "sw3"));
        assert!(matches_query("New window", ""));
        assert!(matches_query("New window", "nw"));
        assert!(!matches_query("New window", "zq"));
    }

    #[test]
    fn palette_filters_commands() {
        let mut os = test_os();
        os.open_palette();
        os.palette_query = "close".into();
        assert_eq!(os.palette_items(), vec![Command::CloseWindow]);
    }

    #[test]
    fn palette_ranks_best_match_first() {
        let mut os = test_os();
        os.open_palette();
        // "quit" also subsequence-matches "equalize splits"; the prefix match
        // on "Quit" must rank first.
        os.palette_query = "quit".into();
        assert_eq!(os.palette_items().first(), Some(&Command::Quit));
    }

    #[test]
    fn palette_move_wraps_and_activate_runs_command() {
        let mut os = test_os();
        os.open_palette();
        os.palette_query = "workspace 3".into();
        let items = os.palette_items();
        assert_eq!(items, vec![Command::SwitchWorkspace(3)]);
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
        let seen: Arc<Mutex<Vec<(hooks::Event, hooks::Context)>>> = Arc::new(Mutex::new(Vec::new()));
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
        assert!(seen.lock().unwrap().iter().any(|(e, _)| *e == hooks::Event::AfterFocusChange));

        // Closing the focused window fires after-close-window.
        os.close_focused_window();
        os.hook_manager.wait();
        assert!(seen.lock().unwrap().iter().any(|(e, _)| *e == hooks::Event::AfterCloseWindow));

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
        os.toggle_visual();
        assert!(os.copy_visual);
        let sel = os.selection.as_ref().unwrap();
        assert_eq!(sel.window, 0);
        assert_eq!(sel.anchor_line, sel.cursor_line);
        // Esc clears visual selection.
        os.toggle_visual();
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
        os.script_sleep_until =
            Some(std::time::Instant::now() - std::time::Duration::from_secs(1));
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
    fn render_overlay_does_not_panic_on_offset_rects() {
        // Regression: overlays narrower than the screen (which-key, switcher,
        // tape manager) used absolute indexing into an offset block buffer.
        use crate::app::render::render;
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let mut os = test_os();
        os.prefix = Prefix::Tape; // narrow which-key popup
        let mut terminal = Terminal::new(TestBackend::new(80, 25)).unwrap();
        terminal.draw(|f| render(&os, f.buffer_mut())).unwrap();
        os.prefix = Prefix::None;
        os.tape_manager_open = true; // tape manager overlay
        terminal.draw(|f| render(&os, f.buffer_mut())).unwrap();
        os.tape_manager_open = false;
        os.switcher_open = true; // switcher overlay
        terminal.draw(|f| render(&os, f.buffer_mut())).unwrap();
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
        terminal.draw(|f| render(&os, f.buffer_mut())).unwrap();
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
        let types: Vec<_> = recorder
            .commands()
            .iter()
            .map(|c| c.type_)
            .collect();
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
            // Unsupported ops report why.
            let zoom_cmd = Command {
                type_: CommandType::ToggleZoom,
                args: vec![],
                delay: std::time::Duration::ZERO,
                line: 1,
                column: 1,
                raw: "ToggleZoom".into(),
            };
            assert!(ce.execute(&zoom_cmd).is_err());
        }

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
        os.hook_manager.register(hooks::Event::AfterAgentState, "dummy");
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
        assert!(os.notifications.is_empty(), "focused pane must be suppressed");
        assert!(os.take_host_sequence().is_empty());
    }

    #[test]
    fn agent_alert_settle_window_parks_then_fires() {
        let mut os = os_with_window();
        os.focused_window = None; // nothing focused → nothing suppressed
        os.hook_manager.register(hooks::Event::AfterAgentState, "dummy");
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
}
