//! The single message-processing entry point (`Os::update`).
//!
//! This is the Rust analogue of Go's `(*OS).Update(msg tea.Msg)`: every
//! event source funnels into one function that mutates the model and returns
//! side effects for the event loop to execute. Delegating to the existing
//! `handle_key` / `handle_mouse` handlers keeps their behavior byte-for-byte
//! identical; what `update` adds is the uniform envelope (quit detection,
//! host-sequence flushing, and the session switch/kill requests that the
//! switcher sets as pending state).

use super::effect::Effect;
use super::input::{handle_key, handle_mouse, KeyResult};
use super::msg::Msg;
use super::Os;

impl Os {
    /// Process one event, mutating the model and returning side effects.
    pub fn update(&mut self, msg: Msg) -> Vec<Effect> {
        if !matches!(&msg, Msg::Tick) {
            self.request_render();
        }
        match msg {
            Msg::Key(key) => {
                let result = handle_key(self, &key);
                let mut effects = Vec::new();
                if result == KeyResult::Quit || self.quitting {
                    effects.push(Effect::Quit);
                }
                self.append_loop_effects(&mut effects);
                effects
            }
            Msg::Mouse(mouse) => {
                handle_mouse(self, &mouse);
                // Hovering a border changes the host pointer shape (OSC 22).
                self.update_pointer_shape(mouse.column as i32, mouse.row as i32);
                let mut effects = Vec::new();
                self.append_loop_effects(&mut effects);
                effects
            }
            Msg::KeyRelease(_key) => {
                // A release ends a hold; nothing else consumes releases.
                self.hold_mode.end();
                let mut effects = Vec::new();
                self.append_loop_effects(&mut effects);
                effects
            }
            Msg::Resize { cols, rows } => {
                self.width = cols as i32;
                self.height = rows as i32;
                self.damage_resize(cols as i32, rows as i32);
                self.sync_window_sizes();
                Vec::new()
            }
            Msg::Tick => {
                self.poll_window_exits();
                self.tick_agent_progress();
                self.tick_agent_alerts();
                self.tick_notifications();
                self.tick_animations();
                self.tick_tooltip();
                self.tick_script();
                self.tick_metrics();
                self.widget_registry.tick_all();
                // Widget commands are asynchronous; update_status_widgets()
                // reaps completed jobs without waiting for unfinished ones.
                self.update_status_widgets();
                self.sync_window_sizes();
                self.flush_graphics();
                let mut effects = Vec::new();
                self.append_loop_effects(&mut effects);
                effects
            }
            Msg::ConfigReloaded(config) => {
                self.config = *config;
                // Re-resolve the theme exactly as `Os::new` does at startup,
                // so a hot-reloaded `theme` takes effect immediately.
                self.auto_theme = self.config.appearance.theme == "auto";
                if self.config.appearance.theme.is_empty() {
                    self.theme = None;
                } else if self.auto_theme {
                    let mode = crate::util::theme_detect::detect_from_env();
                    let name = crate::util::theme_detect::resolve_auto_theme_name(
                        mode,
                        &self.config.appearance.theme_auto_light,
                        &self.config.appearance.theme_auto_dark,
                    );
                    self.theme = crate::config::Theme::built_in(&name);
                } else {
                    self.theme = crate::config::Theme::built_in(&self.config.appearance.theme);
                }
                self.damage_full(crate::app::damage::DamageReason::Theme);
                self.notify("config reloaded", "info");
                Vec::new()
            }
            Msg::RemoteAgentStateChanged {
                window,
                state,
                message,
                harness,
            } => {
                self.handle_agent_state_changed(&window, &state, &message, &harness);
                Vec::new()
            }
            Msg::RemoteTapeCommand {
                index,
                total,
                command,
            } => {
                self.handle_remote_tape_command(index, total, &command);
                Vec::new()
            }
            Msg::RemoteTapeFinished { total } => {
                self.remote_tape_finished();
                self.notify(format!("tape finished ({total} commands)"), "info");
                Vec::new()
            }
            Msg::RemoteListResult { sessions } => {
                self.remote_sessions = sessions;
                Vec::new()
            }
            Msg::RemoteError(message) => {
                self.notify(message, "error");
                Vec::new()
            }
            Msg::None => {
                // A no-op message still surfaces pending loop effects (session
                // switch/kill requests, queued host sequences), which is what
                // makes it a useful "drain" message for tests and loops.
                let mut effects = Vec::new();
                self.append_loop_effects(&mut effects);
                effects
            }
        }
    }

    /// Append the loop-level effects every input-bearing message shares:
    /// pending session switch/kill (set by the switcher) and host-terminal
    /// sequences queued by agent alerts.
    fn append_loop_effects(&mut self, effects: &mut Vec<Effect>) {
        if let Some(target) = self.pending_switch.take() {
            effects.push(Effect::RequestAttach(target));
        }
        if let Some(target) = self.pending_kill.take() {
            effects.push(Effect::RequestKill(target));
        }
        let host = self.take_host_sequence();
        if !host.is_empty() {
            effects.push(Effect::WriteHost(host));
        }
    }
}

/// Convenience wrapper for callers that only need the quit decision.
pub fn update_returns_quit(effects: &[Effect]) -> bool {
    effects.iter().any(|e| matches!(e, Effect::Quit))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::userconfig::UserConfig;
    use crossterm::event::KeyEvent;

    fn test_os() -> Os {
        Os::new(UserConfig::default_config())
    }

    fn key(code: crossterm::event::KeyCode, modifiers: crossterm::event::KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn config_reload_applies_theme_and_startup_config() {
        let mut os = test_os();
        assert!(os.theme.is_none()); // default config has no theme

        let mut cfg = UserConfig::default_config();
        cfg.appearance.theme = "nord".into();
        cfg.appearance.animations_enabled = true;
        os.update(Msg::ConfigReloaded(Box::new(cfg)));

        // The new theme is resolved immediately, matching `Os::new` startup.
        let theme = os.theme.as_ref().expect("reload should resolve the theme");
        assert_eq!(theme.name, "nord");
        // The rest of the config is swapped in too.
        assert!(os.config.appearance.animations_enabled);
    }

    #[test]
    fn config_reload_clears_theme_when_empty() {
        let mut os = test_os();
        let mut cfg = UserConfig::default_config();
        cfg.appearance.theme = "nord".into();
        os.update(Msg::ConfigReloaded(Box::new(cfg)));
        assert!(os.theme.is_some());

        let cfg = UserConfig::default_config();
        os.update(Msg::ConfigReloaded(Box::new(cfg)));
        assert!(os.theme.is_none(), "reload with empty theme clears it");
    }

    #[test]
    fn none_msg_no_effects() {
        let mut os = test_os();
        let effects = os.update(Msg::None);
        assert!(effects.is_empty());
    }

    #[test]
    fn leader_key_sets_prefix() {
        let mut os = test_os();
        os.update(Msg::Key(key(
            crossterm::event::KeyCode::Char('b'),
            crossterm::event::KeyModifiers::CONTROL,
        )));
        assert_eq!(os.prefix, crate::app::Prefix::Leader);
    }

    #[test]
    fn window_mode_key_enters_terminal() {
        let mut os = test_os();
        os.update(Msg::Key(key(
            crossterm::event::KeyCode::Char('i'),
            crossterm::event::KeyModifiers::NONE,
        )));
        assert_eq!(os.mode, crate::app::Mode::Terminal);
    }

    #[test]
    fn quit_key_emits_quit_effect() {
        let mut os = test_os();
        // Ctrl+C in terminal mode with no window is consumed; use the
        // window-management quit path instead: prefix 'q' isn't bound, so
        // drive the quit confirmation via the leader + explicit state.
        os.prefix = crate::app::Prefix::None;
        os.show_quit_confirmation = true;
        let effects = os.update(Msg::Key(key(
            crossterm::event::KeyCode::Char('y'),
            crossterm::event::KeyModifiers::NONE,
        )));
        assert!(update_returns_quit(&effects));
    }

    #[test]
    fn resize_msg_syncs_dimensions() {
        let mut os = test_os();
        os.update(Msg::Resize {
            cols: 100,
            rows: 40,
        });
        assert_eq!(os.width, 100);
        assert_eq!(os.height, 40);
    }

    #[test]
    fn config_reload_notifies() {
        let mut os = test_os();
        os.update(Msg::ConfigReloaded(Box::new(UserConfig::default_config())));
        assert!(!os.notifications.is_empty());
    }

    #[test]
    fn remote_error_notifies() {
        let mut os = test_os();
        os.update(Msg::RemoteError("boom".into()));
        assert!(os
            .notifications
            .iter()
            .any(|n| n.message == "boom" && n.kind == "error"));
    }

    #[test]
    fn remote_list_result_caches() {
        let mut os = test_os();
        let sessions = vec![crate::session::model::SessionInfo {
            id: "s1".into(),
            name: "work".into(),
            created_at: 0,
            attached: true,
            windows: 2,
            restored: false,
        }];
        os.update(Msg::RemoteListResult { sessions });
        assert_eq!(os.remote_sessions.len(), 1);
        assert_eq!(os.remote_sessions[0].name, "work");
    }

    #[test]
    fn pending_switch_becomes_effect() {
        let mut os = test_os();
        os.pending_switch = Some("other".into());
        let effects = os.update(Msg::None);
        assert!(effects
            .iter()
            .any(|e| matches!(e, Effect::RequestAttach(t) if t == "other")));
        // The pending state is consumed.
        assert!(os.pending_switch.is_none());
    }

    #[test]
    fn pending_kill_becomes_effect() {
        let mut os = test_os();
        os.pending_kill = Some("doomed".into());
        let effects = os.update(Msg::None);
        assert!(effects
            .iter()
            .any(|e| matches!(e, Effect::RequestKill(t) if t == "doomed")));
        assert!(os.pending_kill.is_none());
    }

    #[test]
    fn host_sequence_becomes_write_host() {
        let mut os = test_os();
        os.host_output.extend_from_slice(b"\x07");
        let effects = os.update(Msg::None);
        assert!(effects
            .iter()
            .any(|e| matches!(e, Effect::WriteHost(b) if b == b"\x07")));
    }

    #[test]
    fn tick_runs_maintenance() {
        let mut os = test_os();
        // Must not panic and must not quit.
        let effects = os.update(Msg::Tick);
        assert!(!update_returns_quit(&effects));
    }
}
