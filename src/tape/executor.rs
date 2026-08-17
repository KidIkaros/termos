//! Tape executor — bridges tape commands to app state, ported from TUIOS
//! `internal/tape/executor.go`.

use super::command::Command;
use super::command::CommandType;

/// Executes tape commands by directly manipulating the app state.
pub trait TapeExecutor {
    /// The focused window's id, if any (Go's `GetFocusedWindowID`).
    fn focused_window_id(&self) -> Option<String>;

    /// Send bytes to a window's PTY.
    fn send_to_window(&mut self, window_id: &str, data: &[u8]) -> Result<(), String>;

    /// Mode switching (`terminal` or `window`).
    fn set_mode(&mut self, mode: &str) -> Result<(), String>;

    /// Window management.
    fn create_new_window(&mut self) -> Result<(), String>;
    fn create_new_window_with_name(&mut self, name: &str) -> Result<(), String>;
    fn close_window(&mut self, window_id: &str) -> Result<(), String>;
    fn close_window_by_name(&mut self, name: &str) -> Result<(), String>;
    fn next_window(&mut self) -> Result<(), String>;
    fn prev_window(&mut self) -> Result<(), String>;
    fn focus_window_by_id(&mut self, window_id: &str) -> Result<(), String>;
    fn focus_window_by_name(&mut self, name: &str) -> Result<(), String>;
    fn rename_window_by_id(&mut self, window_id: &str, name: &str) -> Result<(), String>;
    fn rename_window_by_name(&mut self, old_name: &str, new_name: &str) -> Result<(), String>;
    fn minimize_window_by_id(&mut self, window_id: &str) -> Result<(), String>;
    fn minimize_window_by_name(&mut self, name: &str) -> Result<(), String>;
    fn restore_window_by_id(&mut self, window_id: &str) -> Result<(), String>;
    fn restore_window_by_name(&mut self, name: &str) -> Result<(), String>;

    /// Tiling.
    fn toggle_tiling(&mut self) -> Result<(), String>;
    fn enable_tiling(&mut self) -> Result<(), String>;
    fn disable_tiling(&mut self) -> Result<(), String>;
    fn snap_by_direction(&mut self, direction: &str) -> Result<(), String>;

    /// BSP tiling.
    fn split_horizontal(&mut self) -> Result<(), String>;
    fn split_vertical(&mut self) -> Result<(), String>;
    fn rotate_split(&mut self) -> Result<(), String>;
    fn equalize_splits(&mut self) -> Result<(), String>;
    fn preselect(&mut self, direction: &str) -> Result<(), String>;

    /// Workspaces.
    fn switch_workspace(&mut self, workspace: i32) -> Result<(), String>;
    fn move_window_to_workspace_by_id(&mut self, window_id: &str, workspace: i32)
        -> Result<(), String>;
    fn move_and_follow_workspace_by_id(
        &mut self,
        window_id: &str,
        workspace: i32,
    ) -> Result<(), String>;

    /// Animations.
    fn enable_animations(&mut self) -> Result<(), String>;
    fn disable_animations(&mut self) -> Result<(), String>;
    fn toggle_animations(&mut self) -> Result<(), String>;

    /// New feature commands.
    fn toggle_zoom(&mut self) -> Result<(), String>;
    fn smart_split_focused(&mut self) -> Result<(), String>;
    fn show_command_palette(&mut self) -> Result<(), String>;
    fn save_layout(&mut self, name: &str) -> Result<(), String>;
    fn load_layout(&mut self, name: &str) -> Result<(), String>;

    /// Config commands.
    fn set_config(&mut self, path: &str, value: &str) -> Result<(), String>;
    fn set_theme(&mut self, theme_name: &str) -> Result<(), String>;
    fn set_dockbar_position(&mut self, position: &str) -> Result<(), String>;
    fn set_border_style(&mut self, style: &str) -> Result<(), String>;
    fn show_notification(&mut self, message: &str, notification_type: &str) -> Result<(), String>;
    fn focus_direction(&mut self, direction: &str) -> Result<(), String>;
}

/// Dispatches a parsed command to an executor (Go's `CommandExecutor`).
pub struct CommandExecutor<'a> {
    executor: &'a mut dyn TapeExecutor,
}

impl<'a> CommandExecutor<'a> {
    pub fn new(executor: &'a mut dyn TapeExecutor) -> Self {
        Self { executor }
    }

    /// Execute a single parsed command.
    pub fn execute(&mut self, cmd: &Command) -> Result<(), String> {
        let executor = &mut *self.executor;

        // send_repeated sends data to the focused window once per repeat count.
        // Basic key commands carry an optional trailing count (Down 5, ...).
        let send_repeated = |executor: &mut dyn TapeExecutor, data: &[u8]| -> Result<(), String> {
            let id = executor_focused_id(executor);
            for _ in 0..repeat_count(cmd) {
                executor.send_to_window(&id, data)?;
            }
            Ok(())
        };

        match cmd.type_ {
            CommandType::Type => {
                let text = cmd.args.first().ok_or_else(|| missing_arg("Type", "the text to type"))?;
                let id = executor_focused_id(executor);
                executor.send_to_window(&id, text.as_bytes())
            }

            CommandType::Enter => send_repeated(executor, b"\n"),
            CommandType::Space => send_repeated(executor, b" "),
            CommandType::Backspace => send_repeated(executor, b"\x08"),
            CommandType::Tab => send_repeated(executor, b"\t"),
            CommandType::Escape => send_repeated(executor, b"\x1b"),
            CommandType::Delete => send_repeated(executor, b"\x1b[3~"),
            CommandType::Up => send_repeated(executor, b"\x1b[A"),
            CommandType::Down => send_repeated(executor, b"\x1b[B"),
            CommandType::Right => send_repeated(executor, b"\x1b[C"),
            CommandType::Left => send_repeated(executor, b"\x1b[D"),
            CommandType::Home => send_repeated(executor, b"\x1b[H"),
            CommandType::End => send_repeated(executor, b"\x1b[F"),

            // Mode switching.
            CommandType::TerminalMode => executor.set_mode("terminal"),
            CommandType::WindowManagementMode => executor.set_mode("window"),

            // Window management.
            CommandType::NewWindow => {
                if let Some(name) = cmd.args.first() {
                    if !name.is_empty() {
                        return executor.create_new_window_with_name(name);
                    }
                }
                executor.create_new_window()
            }
            CommandType::CloseWindow => {
                if let Some(name) = cmd.args.first() {
                    if !name.is_empty() {
                        return executor.close_window_by_name(name);
                    }
                }
                let id = executor_focused_id(executor);
                executor.close_window(&id)
            }
            CommandType::NextWindow => executor.next_window(),
            CommandType::PrevWindow => executor.prev_window(),
            CommandType::FocusWindow => {
                let target = cmd
                    .args
                    .first()
                    .ok_or_else(|| missing_arg("Focus", "a window name or id"))?;
                // Try as name first (more user-friendly), fall back to ID.
                match executor.focus_window_by_name(target) {
                    Ok(()) => Ok(()),
                    Err(_) => executor.focus_window_by_id(target),
                }
            }
            CommandType::RenameWindow => match cmd.args.len() {
                0 => Err(missing_arg("RenameWindow", "a new name")),
                1 => {
                    let id = executor_focused_id(executor);
                    executor.rename_window_by_id(&id, &cmd.args[0])
                }
                _ => executor.rename_window_by_name(&cmd.args[0], &cmd.args[1]),
            },
            CommandType::MinimizeWindow => {
                if let Some(name) = cmd.args.first() {
                    if !name.is_empty() {
                        return executor.minimize_window_by_name(name);
                    }
                }
                let id = executor_focused_id(executor);
                executor.minimize_window_by_id(&id)
            }
            CommandType::RestoreWindow => {
                if let Some(name) = cmd.args.first() {
                    if !name.is_empty() {
                        return executor.restore_window_by_name(name);
                    }
                }
                let id = executor_focused_id(executor);
                executor.restore_window_by_id(&id)
            }

            // Tiling.
            CommandType::ToggleTiling => executor.toggle_tiling(),
            CommandType::EnableTiling => executor.enable_tiling(),
            CommandType::DisableTiling => executor.disable_tiling(),
            CommandType::SnapLeft => executor.snap_by_direction("left"),
            CommandType::SnapRight => executor.snap_by_direction("right"),
            CommandType::SnapFullscreen => executor.snap_by_direction("fullscreen"),

            // BSP tiling.
            CommandType::Split => {
                let dir = cmd
                    .args
                    .first()
                    .ok_or_else(|| missing_arg("Split", "horizontal or vertical"))?;
                match dir.to_ascii_lowercase().as_str() {
                    "horizontal" | "h" => executor.split_horizontal(),
                    "vertical" | "v" => executor.split_vertical(),
                    other => Err(format!(
                        "unknown Split direction {other:?} (use horizontal or vertical)"
                    )),
                }
            }
            CommandType::RotateSplit => executor.rotate_split(),
            CommandType::EqualizeSplits => executor.equalize_splits(),
            CommandType::Preselect => {
                let dir = cmd
                    .args
                    .first()
                    .ok_or_else(|| missing_arg("Preselect", "left, right, up or down"))?;
                executor.preselect(&dir.to_ascii_lowercase())
            }

            // Workspaces.
            CommandType::SwitchWorkspace => {
                let ws = workspace_arg("Switch", cmd)?;
                executor.switch_workspace(ws)
            }
            CommandType::MoveToWorkspace => {
                let ws = workspace_arg("MoveToWorkspace", cmd)?;
                let id = executor_focused_id(executor);
                executor.move_window_to_workspace_by_id(&id, ws)
            }
            CommandType::MoveAndFollowWorkspace => {
                let ws = workspace_arg("MoveAndFollow", cmd)?;
                let id = executor_focused_id(executor);
                executor.move_and_follow_workspace_by_id(&id, ws)
            }

            CommandType::KeyCombo => {
                let combo = cmd
                    .args
                    .first()
                    .ok_or_else(|| missing_arg("key combo", "a combination such as ctrl+b"))?;
                // Alt+N / opt+N switches workspaces (case-insensitive).
                let lower = combo.to_ascii_lowercase();
                if lower.len() >= 5 && (&lower[..4] == "alt+" || &lower[..4] == "opt+") {
                    if let Ok(ws) = combo[4..].trim().parse::<i32>() {
                        if (1..=9).contains(&ws) {
                            return executor.switch_workspace(ws);
                        }
                    }
                }
                let id = executor_focused_id(executor);
                let bytes = convert_key_combo_to_bytes(combo);
                executor.send_to_window(&id, &bytes)
            }

            // Wait (a Sleep alias) and WaitUntilRegex are handled by the
            // interactive playback loop, which needs to block across ticks
            // while checking timers and screen contents. They are intentional
            // no-ops here so the remote/daemon exec path (fire-and-forget)
            // simply skips them.
            CommandType::Wait | CommandType::WaitUntilRegex => Ok(()),

            CommandType::EnableAnimations => executor.enable_animations(),
            CommandType::DisableAnimations => executor.disable_animations(),
            CommandType::ToggleAnimations => executor.toggle_animations(),

            // New feature commands.
            CommandType::ToggleZoom => executor.toggle_zoom(),
            CommandType::SmartSplit => executor.smart_split_focused(),
            CommandType::CommandPalette => executor.show_command_palette(),
            CommandType::SaveLayout => {
                let name = cmd
                    .args
                    .first()
                    .ok_or_else(|| missing_arg("SaveLayout", "a layout name"))?;
                executor.save_layout(name)
            }
            CommandType::LoadLayout => {
                let name = cmd
                    .args
                    .first()
                    .ok_or_else(|| missing_arg("LoadLayout", "a layout name"))?;
                executor.load_layout(name)
            }

            // Config commands.
            CommandType::SetConfig => {
                if cmd.args.len() < 2 {
                    return Err(missing_arg("Set", "a config path and a value"));
                }
                executor.set_config(&cmd.args[0], &cmd.args[1])
            }
            CommandType::SetTheme => {
                let name = cmd
                    .args
                    .first()
                    .ok_or_else(|| missing_arg("SetTheme", "a theme name"))?;
                executor.set_theme(name)
            }
            CommandType::SetDockbarPosition => {
                let pos = cmd
                    .args
                    .first()
                    .ok_or_else(|| missing_arg("SetDockbarPosition", "top or bottom"))?;
                executor.set_dockbar_position(pos)
            }
            CommandType::SetBorderStyle => {
                let style = cmd
                    .args
                    .first()
                    .ok_or_else(|| missing_arg("SetBorderStyle", "a border style name"))?;
                executor.set_border_style(style)
            }
            CommandType::ShowNotification => {
                let msg = cmd
                    .args
                    .first()
                    .ok_or_else(|| missing_arg("Notify", "a message"))?;
                let kind = cmd.args.get(1).cloned().unwrap_or_else(|| "info".into());
                executor.show_notification(msg, &kind)
            }
            CommandType::FocusDirection => {
                let dir = cmd
                    .args
                    .first()
                    .ok_or_else(|| missing_arg("FocusDirection", "left, right, up or down"))?;
                executor.focus_direction(&dir.to_ascii_lowercase())
            }

            // Other command types are handled elsewhere or ignored.
            _ => Ok(()),
        }
    }
}

/// The focused window id, or an empty string when none (sends go nowhere).
fn executor_focused_id(executor: &dyn TapeExecutor) -> String {
    executor
        .focused_window_id()
        .unwrap_or_default()
}

/// Reports a tape command that was given no argument to act on.
fn missing_arg(command: &str, want: &str) -> String {
    format!("{command} needs {want}")
}

/// Parse a workspace number argument; a non-numeric argument is an error
/// rather than silently becoming workspace 0.
fn workspace_arg(command: &str, cmd: &Command) -> Result<i32, String> {
    let arg = cmd
        .args
        .first()
        .ok_or_else(|| missing_arg(command, "a workspace number"))?;
    arg.trim()
        .parse::<i32>()
        .map_err(|_| format!("{command}: {arg:?} is not a workspace number"))
}

/// The trailing repeat count of a basic key command, taken from its first
/// argument. Defaults to 1 when absent or not a positive int.
fn repeat_count(cmd: &Command) -> usize {
    match cmd.args.first() {
        Some(n) => n.parse::<usize>().unwrap_or(1).max(1),
        None => 1,
    }
}

/// Convert a key combination string to the bytes sent to a PTY:
/// `Ctrl+b` → `[0x02]`, `Alt+x` → `[0x1b, 'x']`, `Shift+Tab` → back-tab.
pub fn convert_key_combo_to_bytes(combo_str: &str) -> Vec<u8> {
    let parts: Vec<&str> = combo_str.split('+').collect();
    if parts.len() < 2 {
        return combo_str.as_bytes().to_vec();
    }

    let mut ctrl = false;
    let mut alt = false;
    let mut shift = false;
    let mut key = "";
    for part in parts {
        let p = part.trim();
        match p.to_ascii_lowercase().as_str() {
            "ctrl" => ctrl = true,
            "alt" | "opt" => alt = true,
            "shift" => shift = true,
            _ => key = p,
        }
    }
    if key.is_empty() {
        return combo_str.as_bytes().to_vec();
    }

    let mut result = Vec::new();
    let key_bytes = key.as_bytes();
    if key_bytes.len() == 1 {
        let k = key_bytes[0];
        if ctrl {
            // Ctrl+letter/digit: ASCII control character (uppercase & 0x1F).
            if k.is_ascii_alphabetic() {
                result.push(k.to_ascii_uppercase() & 0x1f);
            } else if k.is_ascii_digit() {
                result.push(k & 0x1f);
            } else {
                result.push(k);
            }
        } else if alt {
            result.push(0x1b);
            result.push(k);
        } else if shift {
            result.push(k.to_ascii_uppercase());
        } else {
            result.push(k);
        }
    } else {
        let lower_key = key.to_ascii_lowercase();
        // Shift+Tab is the back-tab sequence.
        if lower_key == "tab" && shift && !ctrl && !alt {
            return b"\x1b[Z".to_vec();
        }
        let special: &[u8] = match lower_key.as_str() {
            "space" => b" ",
            "enter" | "return" => b"\n",
            "tab" => b"\t",
            "escape" | "esc" => b"\x1b",
            "backspace" => b"\x08",
            "delete" => b"\x7f",
            _ => key.as_bytes(),
        };
        if alt {
            result.push(0x1b);
        }
        result.extend_from_slice(special);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// A recording test double (Go's `nopExecutor`): records calls, returns
    /// Ok for the ops under test, Err for the rest.
    type RecordedSends = Rc<RefCell<Vec<(String, Vec<u8>)>>>;
    struct Recording {
        sends: RecordedSends,
    }

    macro_rules! nop_ops {
        ($ty:ident) => {
            impl TapeExecutor for $ty {
                fn focused_window_id(&self) -> Option<String> {
                    Some("w1".to_string())
                }
                fn send_to_window(&mut self, id: &str, data: &[u8]) -> Result<(), String> {
                    self.sends.borrow_mut().push((id.to_string(), data.to_vec()));
                    Ok(())
                }
                fn set_mode(&mut self, _m: &str) -> Result<(), String> { Ok(()) }
                fn create_new_window(&mut self) -> Result<(), String> { Ok(()) }
                fn create_new_window_with_name(&mut self, _n: &str) -> Result<(), String> { Ok(()) }
                fn close_window(&mut self, _w: &str) -> Result<(), String> { Ok(()) }
                fn close_window_by_name(&mut self, _n: &str) -> Result<(), String> { Ok(()) }
                fn next_window(&mut self) -> Result<(), String> { Ok(()) }
                fn prev_window(&mut self) -> Result<(), String> { Ok(()) }
                fn focus_window_by_id(&mut self, _w: &str) -> Result<(), String> { Ok(()) }
                fn focus_window_by_name(&mut self, _n: &str) -> Result<(), String> { Ok(()) }
                fn rename_window_by_id(&mut self, _w: &str, _n: &str) -> Result<(), String> { Ok(()) }
                fn rename_window_by_name(&mut self, _o: &str, _n: &str) -> Result<(), String> { Ok(()) }
                fn minimize_window_by_id(&mut self, _w: &str) -> Result<(), String> { Ok(()) }
                fn minimize_window_by_name(&mut self, _n: &str) -> Result<(), String> { Ok(()) }
                fn restore_window_by_id(&mut self, _w: &str) -> Result<(), String> { Ok(()) }
                fn restore_window_by_name(&mut self, _n: &str) -> Result<(), String> { Ok(()) }
                fn toggle_tiling(&mut self) -> Result<(), String> { Ok(()) }
                fn enable_tiling(&mut self) -> Result<(), String> { Ok(()) }
                fn disable_tiling(&mut self) -> Result<(), String> { Ok(()) }
                fn snap_by_direction(&mut self, _d: &str) -> Result<(), String> { Ok(()) }
                fn split_horizontal(&mut self) -> Result<(), String> { Ok(()) }
                fn split_vertical(&mut self) -> Result<(), String> { Ok(()) }
                fn rotate_split(&mut self) -> Result<(), String> { Ok(()) }
                fn equalize_splits(&mut self) -> Result<(), String> { Ok(()) }
                fn preselect(&mut self, _d: &str) -> Result<(), String> { Ok(()) }
                fn switch_workspace(&mut self, _w: i32) -> Result<(), String> { Ok(()) }
                fn move_window_to_workspace_by_id(&mut self, _w: &str, _x: i32) -> Result<(), String> { Ok(()) }
                fn move_and_follow_workspace_by_id(&mut self, _w: &str, _x: i32) -> Result<(), String> { Ok(()) }
                fn enable_animations(&mut self) -> Result<(), String> { Ok(()) }
                fn disable_animations(&mut self) -> Result<(), String> { Ok(()) }
                fn toggle_animations(&mut self) -> Result<(), String> { Ok(()) }
                fn toggle_zoom(&mut self) -> Result<(), String> { Ok(()) }
                fn smart_split_focused(&mut self) -> Result<(), String> { Ok(()) }
                fn show_command_palette(&mut self) -> Result<(), String> { Ok(()) }
                fn save_layout(&mut self, _n: &str) -> Result<(), String> { Ok(()) }
                fn load_layout(&mut self, _n: &str) -> Result<(), String> { Ok(()) }
                fn set_config(&mut self, _p: &str, _v: &str) -> Result<(), String> { Ok(()) }
                fn set_theme(&mut self, _n: &str) -> Result<(), String> { Ok(()) }
                fn set_dockbar_position(&mut self, _p: &str) -> Result<(), String> { Ok(()) }
                fn set_border_style(&mut self, _s: &str) -> Result<(), String> { Ok(()) }
                fn show_notification(&mut self, _m: &str, _t: &str) -> Result<(), String> { Ok(()) }
                fn focus_direction(&mut self, _d: &str) -> Result<(), String> { Ok(()) }
            }
        };
    }

    nop_ops!(Recording);

    impl Recording {
        fn new() -> (Self, RecordedSends) {
            let sends = Rc::new(RefCell::new(Vec::new()));
            (Self { sends: sends.clone() }, sends)
        }
    }

    fn cmd(type_: CommandType, args: &[&str]) -> Command {
        Command {
            type_,
            args: args.iter().map(|s| s.to_string()).collect(),
            delay: std::time::Duration::ZERO,
            line: 1,
            column: 1,
            raw: String::new(),
        }
    }

    #[test]
    fn convert_key_combo() {
        let cases = [
            ("Shift+Tab", b"\x1b[Z".to_vec()),
            ("Shift+a", b"A".to_vec()),
            ("Shift+z", b"Z".to_vec()),
            ("Ctrl+a", b"\x01".to_vec()),
            ("Alt+a", b"\x1ba".to_vec()),
            // Ctrl takes precedence over Alt in the tape encoder (Go's
            // if/else chain), so no ESC prefix.
            ("Ctrl+Alt+D", b"\x04".to_vec()),
            ("space", b"space".to_vec()), // no modifier → sent as-is
            ("Alt+enter", b"\x1b\n".to_vec()),
        ];
        for (combo, want) in cases {
            assert_eq!(convert_key_combo_to_bytes(combo), want, "combo {combo}");
        }
    }

    #[test]
    fn repeat_count_defaults() {
        assert_eq!(repeat_count(&cmd(CommandType::Enter, &[])), 1);
        assert_eq!(repeat_count(&cmd(CommandType::Enter, &["5"])), 5);
        assert_eq!(repeat_count(&cmd(CommandType::Enter, &["abc"])), 1);
        assert_eq!(repeat_count(&cmd(CommandType::Enter, &["0"])), 1);
        assert_eq!(repeat_count(&cmd(CommandType::Enter, &["-3"])), 1);
    }

    #[test]
    fn execute_rejects_unusable_commands() {
        let (mut rec, _) = Recording::new();
        let cases = [
            ("Type with no text", cmd(CommandType::Type, &[])),
            ("Split with no direction", cmd(CommandType::Split, &[])),
            (
                "Split with a bad direction",
                cmd(CommandType::Split, &["sideways"]),
            ),
            ("Focus with no target", cmd(CommandType::FocusWindow, &[])),
            ("Preselect with no direction", cmd(CommandType::Preselect, &[])),
            ("RenameWindow with no name", cmd(CommandType::RenameWindow, &[])),
            ("SaveLayout with no name", cmd(CommandType::SaveLayout, &[])),
            (
                "Set with no value",
                cmd(CommandType::SetConfig, &["a.b"]),
            ),
            ("Notify with no message", cmd(CommandType::ShowNotification, &[])),
            ("Switch with no workspace", cmd(CommandType::SwitchWorkspace, &[])),
            (
                "Switch with a non-numeric workspace",
                cmd(CommandType::SwitchWorkspace, &["main"]),
            ),
        ];
        for (name, c) in cases {
            let mut ce = CommandExecutor::new(&mut rec);
            assert!(
                ce.execute(&c).is_err(),
                "{name}: an unusable command must report why it did nothing"
            );
        }
    }

    #[test]
    fn execute_sends_repeated_keys_to_focused_window() {
        let (mut rec, sends) = Recording::new();
        {
            let mut ce = CommandExecutor::new(&mut rec);
            ce.execute(&cmd(CommandType::Down, &["3"])).unwrap();
            ce.execute(&cmd(CommandType::Type, &["hi"])).unwrap();
        }
        let sends = sends.borrow();
        // Down 3 sends three separate writes to the focused window.
        assert_eq!(
            sends[..3],
            vec![
                ("w1".to_string(), b"\x1b[B".to_vec()),
                ("w1".to_string(), b"\x1b[B".to_vec()),
                ("w1".to_string(), b"\x1b[B".to_vec()),
            ]
        );
        assert_eq!(sends[3], ("w1".to_string(), b"hi".to_vec()));
    }

    #[test]
    fn alt_plus_digit_switches_workspace() {
        let (mut rec, sends) = Recording::new();
        {
            let mut ce = CommandExecutor::new(&mut rec);
            ce.execute(&cmd(CommandType::KeyCombo, &["Alt+3"])).unwrap();
        }
        assert!(sends.borrow().is_empty(), "Alt+N must switch, not type");
    }
}
