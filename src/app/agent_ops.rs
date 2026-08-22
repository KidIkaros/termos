use std::time::Duration;
use crate::app::agent_alert;
use crate::app::float;
use crate::hooks;
use crate::layout::SplitType;
use super::Prefix;
use super::Os;

impl Os {
    pub(crate) fn window_hook_ctx(&self, index: usize) -> hooks::Context {
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
    pub(crate) fn sync_scrolling_from_workspace(&mut self) {
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
        let mut pending: Vec<(String, String)> = Vec::new();
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
