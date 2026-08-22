use std::collections::HashMap;
use std::sync::Arc;
use super::{Mode, Prefix, TapeManagerMode, ProjectTapePending};
use super::Os;

impl Os {
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
    pub(crate) fn await_new_window(&mut self) {
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
    pub(crate) fn scan_tape_files(&self, query: &str) -> Vec<std::path::PathBuf> {
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
}
