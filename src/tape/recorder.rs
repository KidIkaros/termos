//! Tape recorder — records user interactions as tape commands, ported from
//! TUIOS `internal/tape/recorder.go`.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::command::{Command, CommandType};

/// Records user interactions as tape commands.
#[derive(Debug)]
pub struct Recorder {
    commands: Vec<Command>,
    start_time: Instant,
    last_event_time: Instant,
    enabled: bool,
    /// Buffer for accumulating typed characters.
    typing_buffer: String,
    /// Initial mode when recording started.
    initial_mode: String,
    /// Initial workspace when recording started.
    initial_workspace: i32,
    /// Initial tiling state when recording started.
    initial_tiling: bool,
}

impl Default for Recorder {
    fn default() -> Self {
        Self::new()
    }
}

impl Recorder {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            commands: Vec::new(),
            start_time: now,
            last_event_time: now,
            enabled: false,
            typing_buffer: String::new(),
            initial_mode: String::new(),
            initial_workspace: 1,
            initial_tiling: true,
        }
    }

    /// Begin recording, resetting any prior commands.
    pub fn start(&mut self) {
        self.enabled = true;
        self.start_time = Instant::now();
        self.last_event_time = Instant::now();
        self.commands.clear();
    }

    /// Begin recording and record the initial state (workspace, tiling, mode).
    pub fn start_with_state(&mut self, mode: &str, workspace: i32, tiling_enabled: bool) {
        self.start();
        self.initial_mode = mode.to_string();
        self.initial_workspace = workspace;
        self.initial_tiling = tiling_enabled;

        if workspace > 1 {
            self.commands.push(Command {
                type_: CommandType::SwitchWorkspace,
                args: vec![workspace.to_string()],
                delay: Duration::ZERO,
                line: 1,
                column: 1,
                raw: format!("SwitchWorkspace {workspace}"),
            });
        }
        self.commands.push(Command {
            type_: if tiling_enabled {
                CommandType::EnableTiling
            } else {
                CommandType::DisableTiling
            },
            args: vec![],
            delay: Duration::ZERO,
            line: self.commands.len() + 1,
            column: 1,
            raw: if tiling_enabled {
                "EnableTiling".into()
            } else {
                "DisableTiling".into()
            },
        });
        self.commands.push(Command {
            type_: if mode == "terminal" {
                CommandType::TerminalMode
            } else {
                CommandType::WindowManagementMode
            },
            args: vec![],
            delay: Duration::ZERO,
            line: self.commands.len() + 1,
            column: 1,
            raw: if mode == "terminal" {
                "TerminalMode".into()
            } else {
                "WindowManagementMode".into()
            },
        });
    }

    /// End recording, flushing any pending typed text.
    pub fn stop(&mut self) {
        self.flush_typing_buffer();
        self.enabled = false;
    }

    pub fn is_recording(&self) -> bool {
        self.enabled
    }

    /// Record a key press event (named keys and modifier combos).
    pub fn record_key(&mut self, key: &str) {
        if !self.enabled {
            return;
        }
        self.flush_typing_buffer();
        let now = Instant::now();
        let delay = now.saturating_duration_since(self.last_event_time);
        if let Some(mut cmd) = key_to_command(key, self.commands.len()) {
            cmd.delay = delay;
            self.commands.push(cmd);
            self.last_event_time = now;
        }
    }

    /// Accumulate typed characters (consolidated into one Type command).
    pub fn record_type(&mut self, text: &str) {
        if !self.enabled {
            return;
        }
        self.typing_buffer.push_str(text);
        self.last_event_time = Instant::now();
    }

    fn flush_typing_buffer(&mut self) {
        if self.typing_buffer.is_empty() {
            return;
        }
        let text = std::mem::take(&mut self.typing_buffer);
        let cmd = Command {
            type_: CommandType::Type,
            args: vec![text.clone()],
            delay: Duration::ZERO,
            line: self.commands.len() + 1,
            column: 1,
            raw: format!("Type {text:?}"),
        };
        self.commands.push(cmd);
    }

    /// Record a mode switch, flushing the typing buffer first.
    pub fn record_mode_switch(&mut self, cmd_type: CommandType) {
        if !self.enabled {
            return;
        }
        self.flush_typing_buffer();
        let now = Instant::now();
        let delay = now.saturating_duration_since(self.last_event_time);
        let raw = format!("{cmd_type:?}");
        self.commands.push(Command {
            type_: cmd_type,
            args: vec![],
            delay,
            line: self.commands.len() + 1,
            column: 1,
            raw,
        });
        self.last_event_time = now;
    }

    /// Record a window-management action by name (e.g. `new_window`,
    /// `switch_workspace_3`, `select_window_1`).
    pub fn record_action(&mut self, action: &str, args: &[&str]) {
        if !self.enabled {
            return;
        }
        // Tape-control actions are skipped to avoid recursion.
        if matches!(
            action,
            "toggle_tape_manager" | "stop_recording" | "enter_terminal_mode" | "enter_window_mode"
        ) {
            return;
        }
        self.flush_typing_buffer();
        let now = Instant::now();
        let delay = now.saturating_duration_since(self.last_event_time);

        let str_args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let (cmd_type, raw, args) = match action {
            "new_window" => (CommandType::NewWindow, "NewWindow".to_string(), str_args.clone()),
            "close_window" => (CommandType::CloseWindow, "CloseWindow".to_string(), str_args.clone()),
            "next_window" => (CommandType::NextWindow, "NextWindow".to_string(), str_args.clone()),
            "prev_window" => (CommandType::PrevWindow, "PrevWindow".to_string(), str_args.clone()),
            "minimize_window" => {
                (CommandType::MinimizeWindow, "MinimizeWindow".to_string(), str_args.clone())
            }
            "restore_all" => {
                (CommandType::RestoreWindow, "RestoreWindow".to_string(), str_args.clone())
            }
            "toggle_tiling" => (CommandType::ToggleTiling, "ToggleTiling".to_string(), str_args.clone()),
            "snap_left" => (CommandType::SnapLeft, "SnapLeft".to_string(), str_args.clone()),
            "snap_right" => (CommandType::SnapRight, "SnapRight".to_string(), str_args.clone()),
            "snap_fullscreen" => {
                (CommandType::SnapFullscreen, "SnapFullscreen".to_string(), str_args.clone())
            }
            _ if action.starts_with("switch_workspace_") => {
                let ws = &action["switch_workspace_".len()..];
                (
                    CommandType::SwitchWorkspace,
                    format!("SwitchWorkspace {ws}"),
                    vec![ws.to_string()],
                )
            }
            _ if action.starts_with("select_window_") => {
                let win = &action["select_window_".len()..];
                (
                    CommandType::FocusWindow,
                    format!("FocusWindow {win}"),
                    vec![win.to_string()],
                )
            }
            _ => return, // unknown action, skip
        };

        self.commands.push(Command {
            type_: cmd_type,
            args,
            delay,
            line: self.commands.len() + 1,
            column: 1,
            raw,
        });
        self.last_event_time = now;
    }

    /// Record a workspace switch.
    pub fn record_workspace_switch(&mut self, workspace: i32) {
        if !self.enabled {
            return;
        }
        self.flush_typing_buffer();
        let now = Instant::now();
        let delay = now.saturating_duration_since(self.last_event_time);
        self.commands.push(Command {
            type_: CommandType::SwitchWorkspace,
            args: vec![workspace.to_string()],
            delay,
            line: self.commands.len() + 1,
            column: 1,
            raw: format!("SwitchWorkspace {workspace}"),
        });
        self.last_event_time = now;
    }

    /// Record an explicit sleep command.
    pub fn record_sleep(&mut self, duration: Duration) {
        if !self.enabled {
            return;
        }
        let now = Instant::now();
        let d = format_duration(duration);
        self.commands.push(Command {
            type_: CommandType::Sleep,
            args: vec![d.clone()],
            delay: duration,
            line: self.commands.len() + 1,
            column: 1,
            raw: format!("Sleep {d}"),
        });
        self.last_event_time = now;
    }

    pub fn commands(&self) -> &[Command] {
        &self.commands
    }

    pub fn command_count(&self) -> usize {
        self.commands.len()
    }

    /// The recorded tape as formatted text, wrapped with
    /// DisableAnimations/EnableAnimations and >100ms delays as Sleep lines.
    pub fn string(&self, header: &str) -> String {
        let mut out = String::new();
        if !header.is_empty() {
            out.push_str(&format!("# {header}\n"));
            out.push_str(&format!(
                "# Recorded: {}\n\n",
                format_rfc3339(self.start_time)
            ));
        }
        out.push_str("# Disable animations for consistent playback\n");
        out.push_str("DisableAnimations\n\n");
        for cmd in &self.commands {
            if cmd.delay > Duration::ZERO && cmd.delay.as_millis() > 100 {
                out.push_str(&format!("Sleep {}\n", format_duration(cmd.delay)));
            }
            out.push_str(&cmd.raw);
            out.push('\n');
        }
        out.push_str("\n# Re-enable animations\n");
        out.push_str("EnableAnimations\n");
        out
    }

    pub fn clear(&mut self) {
        self.commands.clear();
        self.typing_buffer.clear();
        self.start_time = Instant::now();
        self.last_event_time = Instant::now();
    }
}

/// Map a key name (or modifier combo) to a tape command.
fn key_to_command(key: &str, line: usize) -> Option<Command> {
    let (cmd_type, raw, args) = match key {
        "enter" => (CommandType::Enter, "Enter".to_string(), vec![]),
        " " => (CommandType::Space, "Space".to_string(), vec![]),
        "backspace" => (CommandType::Backspace, "Backspace".to_string(), vec![]),
        "delete" => (CommandType::Delete, "Delete".to_string(), vec![]),
        "tab" => (CommandType::Tab, "Tab".to_string(), vec![]),
        "esc" => (CommandType::Escape, "Escape".to_string(), vec![]),
        "up" => (CommandType::Up, "Up".to_string(), vec![]),
        "down" => (CommandType::Down, "Down".to_string(), vec![]),
        "left" => (CommandType::Left, "Left".to_string(), vec![]),
        "right" => (CommandType::Right, "Right".to_string(), vec![]),
        "home" => (CommandType::Home, "Home".to_string(), vec![]),
        "end" => (CommandType::End, "End".to_string(), vec![]),
        _ if is_modifier_combo(key) => (CommandType::KeyCombo, key.to_string(), vec![key.to_string()]),
        _ if key.chars().count() == 1 && key.is_ascii() => {
            // Single printable character — record as a Type command.
            return Some(Command {
                type_: CommandType::Type,
                args: vec![key.to_string()],
                delay: Duration::ZERO,
                line: line + 1,
                column: 1,
                raw: format!("Type {key:?}"),
            });
        }
        _ => return None, // unknown key
    };
    Some(Command {
        type_: cmd_type,
        args,
        delay: Duration::ZERO,
        line: line + 1,
        column: 1,
        raw,
    })
}

fn is_modifier_combo(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    lower.starts_with("ctrl+") || lower.starts_with("alt+") || lower.starts_with("shift+")
}

/// Format a duration the way Go's `time.Duration.String()` does (`500ms`,
/// `1m30s`, `1.5s`). Sub-second values use a decimal (`411.29ms`), and
/// anything under a millisecond rounds up to `1ms` — the only units emitted
/// are ones the tape lexer round-trips (`h`, `m`, `s`, `ms`).
pub fn format_duration(d: Duration) -> String {
    let nanos = d.as_nanos();
    if nanos == 0 {
        return "0s".to_string();
    }
    let mut out = String::new();
    let mut rest = nanos;
    for (unit, name) in [
        (3_600_000_000_000u128, "h"),
        (60_000_000_000, "m"),
        (1_000_000_000, "s"),
        (1_000_000, "ms"),
    ] {
        if rest >= unit {
            let whole = rest / unit;
            if whole > 0 && !rest.is_multiple_of(unit) && unit <= 1_000_000_000 {
                // Decimal sub-second value: `1.5s`, `411.29ms`. Three digits
                // of fraction keep recorded delays readable and re-parseable.
                let frac = rest % unit;
                let millis = if unit == 1_000_000_000 {
                    frac / 1_000_000
                } else {
                    frac / 1_000
                };
                out.push_str(&format!("{whole}.{millis:03}{name}"));
                return out;
            }
            out.push_str(&format!("{whole}{name}"));
            rest %= unit;
        }
    }
    if out.is_empty() {
        "1ms".to_string() // sub-millisecond: round up so it round-trips
    } else {
        out
    }
}

fn format_rfc3339(_start: Instant) -> String {
    // The wall-clock instant of recording start; Instant has no calendar
    // mapping, so approximate with the process clock at serialization time.
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_epoch_utc(secs)
}

/// Render a Unix timestamp as `YYYY-MM-DDTHH:MM:SSZ` (UTC).
fn format_epoch_utc(secs: u64) -> String {
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mth = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if mth <= 2 { y + 1 } else { y };
    format!("{year:04}-{mth:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_typing_consolidated() {
        let mut r = Recorder::new();
        r.start();
        r.record_type("hel");
        r.record_type("lo");
        r.record_key("enter");
        r.stop();
        let cmds = r.commands();
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0].type_, CommandType::Type);
        assert_eq!(cmds[0].args[0], "hello");
        assert_eq!(cmds[0].raw, r#"Type "hello""#);
        assert_eq!(cmds[1].type_, CommandType::Enter);
        assert_eq!(cmds[1].raw, "Enter");
    }

    #[test]
    fn record_key_maps_named_keys() {
        let mut r = Recorder::new();
        r.start();
        r.record_key("up");
        r.record_key("ctrl+b");
        r.record_key("?");
        r.stop();
        let types: Vec<CommandType> = r.commands().iter().map(|c| c.type_).collect();
        assert_eq!(types, vec![CommandType::Up, CommandType::KeyCombo, CommandType::Type]);
        assert_eq!(r.commands()[1].raw, "ctrl+b");
    }

    #[test]
    fn start_with_state_records_initial_state() {
        let mut r = Recorder::new();
        r.start_with_state("terminal", 3, true);
        let types: Vec<CommandType> = r.commands().iter().map(|c| c.type_).collect();
        assert_eq!(
            types,
            vec![
                CommandType::SwitchWorkspace,
                CommandType::EnableTiling,
                CommandType::TerminalMode
            ]
        );
    }

    #[test]
    fn record_action_maps_names() {
        let mut r = Recorder::new();
        r.start();
        r.record_action("new_window", &[]);
        r.record_action("switch_workspace_2", &[]);
        r.record_action("select_window_1", &[]);
        r.record_action("toggle_tape_manager", &[]); // skipped
        r.record_action("bogus", &[]); // skipped
        r.stop();
        let types: Vec<CommandType> = r.commands().iter().map(|c| c.type_).collect();
        assert_eq!(
            types,
            vec![
                CommandType::NewWindow,
                CommandType::SwitchWorkspace,
                CommandType::FocusWindow
            ]
        );
        assert_eq!(r.commands()[1].args[0], "2");
        assert_eq!(r.commands()[2].args[0], "1");
    }

    #[test]
    fn string_output_wraps_and_escapes() {
        let mut r = Recorder::new();
        r.start();
        r.record_type("say \"hi\"");
        r.record_key("enter");
        r.stop();
        let s = r.string("my tape");
        assert!(s.starts_with("# my tape\n"));
        assert!(s.contains("DisableAnimations"));
        assert!(s.contains("EnableAnimations"));
        // %q-style escaping: quotes round-trip through the lexer.
        assert!(s.contains("say \\\"hi\\\""), "got: {s}");
        assert!(s.contains("Enter"));
    }

    #[test]
    fn delays_above_100ms_become_sleep_lines() {
        let mut r = Recorder::new();
        r.start();
        r.record_key("enter");
        // Simulate a delay by recording a sleep directly.
        r.record_sleep(Duration::from_millis(500));
        r.record_key("enter");
        r.stop();
        let s = r.string("");
        assert!(s.contains("Sleep 500ms"), "got: {s}");
    }

    #[test]
    fn duration_formatting() {
        assert_eq!(format_duration(Duration::from_millis(500)), "500ms");
        assert_eq!(format_duration(Duration::from_secs(2)), "2s");
        assert_eq!(format_duration(Duration::from_millis(1500)), "1.500s");
        assert_eq!(format_duration(Duration::from_secs(90)), "1m30s");
        assert_eq!(format_duration(Duration::from_secs(3723)), "1h2m3s");
        assert_eq!(format_duration(Duration::ZERO), "0s");
        assert_eq!(format_duration(Duration::from_nanos(500)), "1ms");
        // Every format must round-trip through the lexer + parser.
        for d in [
            Duration::from_millis(411),
            Duration::from_millis(289),
            Duration::from_millis(1500),
            Duration::from_secs(90),
            Duration::from_secs(3723),
        ] {
            let s = format_duration(d);
            let parsed = crate::tape::command::parse_duration(&s);
            assert!(parsed.is_some(), "{s} did not re-parse");
        }
    }

    #[test]
    fn not_recording_ignores_events() {
        let mut r = Recorder::new();
        r.record_key("enter");
        r.record_type("x");
        r.record_action("new_window", &[]);
        assert!(r.commands().is_empty());
    }

    #[test]
    fn epoch_formatting() {
        // 2026-01-01T00:00:00Z = 1767225600
        assert_eq!(format_epoch_utc(1767225600), "2026-01-01T00:00:00Z");
    }
}
