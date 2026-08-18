//! The named-action dispatcher — the Rust analogue of Go's
//! `internal/input/actions.go` ActionDispatcher.
//!
//! Every action the window manager can perform has a stable string name.
//! The dispatcher maps those names to handlers over `Os`, so keybindings,
//! the command palette, the context menu, and tape scripts can all address
//! the same action without knowing how it is implemented.

use super::Os;

/// Run a named action. Returns `false` when the name is unknown.
pub fn dispatch(os: &mut Os, action: &str) -> bool {
    match action {
        "new_window" => {
            let shell = os.default_shell();
            let _ = os.spawn_window(&shell, Box::new(|| {}));
        }
        "close_window" => os.close_focused_window(),
        "next_window" => os.focus_next(),
        "prev_window" => os.focus_prev(),
        "split_horizontal" => {
            let shell = os.default_shell();
            let _ = os.split(
                crate::layout::SplitType::Horizontal,
                &shell,
                Box::new(|| {}),
            );
        }
        "split_vertical" => {
            let shell = os.default_shell();
            let _ = os.split(crate::layout::SplitType::Vertical, &shell, Box::new(|| {}));
        }
        "toggle_zoom" => {
            let _ = os.toggle_zoom_internal();
        }
        "equalize_splits" => {
            let ws = os.current_workspace;
            os.workspace_mut(ws).tree.equalize_ratios();
            os.sync_window_sizes();
        }
        "rotate_split" => {
            if let Some(focused) = os.focused_window {
                let ws = os.current_workspace;
                os.workspace_mut(ws).tree.rotate_split(focused as i32);
                os.sync_window_sizes();
            }
        }
        "swap_left" => os.swap_focused_with(crate::layout::PreselectionDir::Left),
        "swap_right" => os.swap_focused_with(crate::layout::PreselectionDir::Right),
        "swap_up" => os.swap_focused_with(crate::layout::PreselectionDir::Up),
        "swap_down" => os.swap_focused_with(crate::layout::PreselectionDir::Down),
        "snap_left" => os.snap_half(true),
        "snap_right" => os.snap_half(false),
        "toggle_help" => {
            os.help_open = !os.help_open;
        }
        "command_palette" => os.open_palette(),
        "settings" => os.open_settings(),
        "quit" => os.open_quit_menu(),
        "copy_selection" => os.yank_selection(),
        "paste_clipboard" => {
            let text = os.clipboard.clone();
            if !text.is_empty() {
                os.write_to_focused(text.as_bytes());
            }
        }
        "clear_selection" => {
            os.selection = None;
            os.copy_visual = false;
            os.mouse_selecting = false;
        }
        "scrollback" => os.enter_scrollback_mode(),
        "focus_sidebar" => os.sidebar.open(),
        "toggle_sidebar" => os.sidebar.toggle(),
        "toggle_tape_manager" => {
            os.tape_manager_open = !os.tape_manager_open;
        }
        "enter_terminal_mode" => os.enter_terminal_mode(),
        "enter_window_mode" => os.leave_terminal_mode(),
        "rename_window" => os.open_rename_dialog(),
        _ => {
            // Parameterized actions: switch_workspace_N, move_and_follow_N.
            if let Some(n) = action.strip_prefix("switch_workspace_") {
                if let Ok(ws) = n.parse::<i32>() {
                    os.switch_workspace(ws);
                    return true;
                }
            }
            if let Some(n) = action.strip_prefix("move_and_follow_") {
                if let Ok(ws) = n.parse::<i32>() {
                    os.move_window_and_follow(ws);
                    return true;
                }
            }
            if let Some(n) = action.strip_prefix("select_window_") {
                if let Ok(idx) = n.parse::<usize>() {
                    if idx < os.windows.len() {
                        os.focus_window(idx);
                    }
                    return true;
                }
            }
            return false;
        }
    }
    true
}

/// Every action name the dispatcher understands.
pub fn all_action_names() -> Vec<&'static str> {
    vec![
        "new_window",
        "close_window",
        "next_window",
        "prev_window",
        "split_horizontal",
        "split_vertical",
        "toggle_zoom",
        "equalize_splits",
        "rotate_split",
        "swap_left",
        "swap_right",
        "swap_up",
        "swap_down",
        "snap_left",
        "snap_right",
        "toggle_help",
        "command_palette",
        "settings",
        "quit",
        "copy_selection",
        "paste_clipboard",
        "clear_selection",
        "scrollback",
        "focus_sidebar",
        "toggle_sidebar",
        "toggle_tape_manager",
        "enter_terminal_mode",
        "enter_window_mode",
        "rename_window",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::userconfig::UserConfig;
    use crate::layout::SplitType;
    use crate::terminal::pty::WinSize;
    use crate::terminal::window::Window;

    fn os_with_window() -> Os {
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
        os.workspace_mut(1).focused = Some(0);
        os.focused_window = Some(0);
        os
    }

    #[test]
    fn unknown_action_returns_false() {
        let mut os = os_with_window();
        assert!(!dispatch(&mut os, "frobnicate"));
    }

    #[test]
    fn next_and_prev_window() {
        let mut os = os_with_window();
        dispatch(&mut os, "next_window");
        assert_eq!(os.focused_window, Some(1));
        dispatch(&mut os, "prev_window");
        assert_eq!(os.focused_window, Some(0));
    }

    #[test]
    fn swap_left_swaps_windows() {
        let mut os = os_with_window();
        // Focus window 1 (bottom half), swap up against window 0.
        os.focus_window(1);
        dispatch(&mut os, "swap_up");
        // Both windows still exist after the swap.
        assert_eq!(os.windows.len(), 2);
    }

    #[test]
    fn switch_workspace_param() {
        let mut os = os_with_window();
        assert!(dispatch(&mut os, "switch_workspace_3"));
        assert_eq!(os.current_workspace, 3);
    }

    #[test]
    fn move_and_follow_param() {
        let mut os = os_with_window();
        assert!(dispatch(&mut os, "move_and_follow_2"));
        assert_eq!(os.current_workspace, 2);
        // The focused window moved with it.
        assert!(os.workspace(2).tree.has_window(0));
    }

    #[test]
    fn copy_and_paste_roundtrip() {
        let mut os = os_with_window();
        os.clipboard = "hello".into();
        // Paste writes to the focused window (PTY-less: no-op, no panic).
        dispatch(&mut os, "paste_clipboard");
        assert_eq!(os.clipboard, "hello");
    }

    #[test]
    fn scrollback_and_sidebar_actions() {
        let mut os = os_with_window();
        dispatch(&mut os, "scrollback");
        assert!(os.scrollback_mode);
        dispatch(&mut os, "toggle_sidebar");
        assert!(os.sidebar.open);
    }

    #[test]
    fn settings_and_palette() {
        let mut os = os_with_window();
        dispatch(&mut os, "settings");
        assert!(os.settings_open);
        dispatch(&mut os, "command_palette");
        assert!(os.palette_open);
    }
}
