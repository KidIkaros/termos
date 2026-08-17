//! Keybinding definitions — ported from TUIOS `internal/config/keybindings.go`.

use std::collections::HashMap;

/// A single keybinding entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keybinding {
    pub key: String,
    pub description: String,
}

/// A section of related keybindings (for the help/which-key overlay).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeybindingSection {
    pub title: String,
    /// Empty for always shown, "tiling" for tiling mode, "!tiling" for
    /// non-tiling.
    pub condition: String,
    pub bindings: Vec<Keybinding>,
}

/// The tmux-style default leader key.
pub const DEFAULT_LEADER_KEY: &str = "ctrl+b";

/// The keybindings for the prefix overlay, by prefix type.
pub fn get_prefix_keybindings(prefix_type: &str, is_daemon_session: bool) -> Vec<Keybinding> {
    match prefix_type {
        "workspace" => vec![
            Keybinding { key: "1-9".into(), description: "Switch to workspace".into() },
            Keybinding { key: "Shift+1-9".into(), description: "Move window to workspace".into() },
            Keybinding { key: "r".into(), description: "Rename workspace".into() },
            Keybinding { key: "Esc".into(), description: "Cancel".into() },
        ],
        "minimize" => vec![
            Keybinding { key: "m".into(), description: "Minimize focused window".into() },
            Keybinding { key: "1-9".into(), description: "Restore window".into() },
            Keybinding { key: "Shift+M".into(), description: "Restore all".into() },
            Keybinding { key: "Esc".into(), description: "Cancel".into() },
        ],
        "window" => vec![
            Keybinding { key: "n".into(), description: "New window".into() },
            Keybinding { key: "x".into(), description: "Close window".into() },
            Keybinding { key: "r".into(), description: "Rename window".into() },
            Keybinding { key: "Tab".into(), description: "Next window".into() },
            Keybinding { key: "Shift+Tab".into(), description: "Previous window".into() },
            Keybinding { key: "t".into(), description: "Toggle tiling mode".into() },
            Keybinding { key: "Esc".into(), description: "Cancel".into() },
        ],
        "debug" => vec![
            Keybinding { key: "l".into(), description: "Toggle log viewer".into() },
            Keybinding { key: "c".into(), description: "Toggle cache statistics".into() },
            Keybinding { key: "k".into(), description: "Toggle showkeys overlay".into() },
            Keybinding { key: "a".into(), description: "Toggle animations".into() },
            Keybinding { key: "Esc".into(), description: "Cancel".into() },
        ],
        "tape" => vec![
            Keybinding { key: "m".into(), description: "Open tape manager".into() },
            Keybinding { key: "t".into(), description: "Review project tape".into() },
            Keybinding { key: "r".into(), description: "Start recording".into() },
            Keybinding { key: "s".into(), description: "Stop recording".into() },
            Keybinding { key: "Esc".into(), description: "Cancel".into() },
        ],
        "layout" => vec![
            Keybinding { key: "l".into(), description: "Load layout".into() },
            Keybinding { key: "s".into(), description: "Save layout".into() },
            Keybinding { key: "Esc".into(), description: "Cancel".into() },
        ],
        _ => {
            let mut bindings = vec![
                Keybinding { key: "c".into(), description: "Create window".into() },
                Keybinding { key: "x".into(), description: "Close window".into() },
                Keybinding { key: "r".into(), description: "Rename window".into() },
                Keybinding { key: ",".into(), description: "Settings".into() },
                Keybinding { key: "n".into(), description: "Next window".into() },
                Keybinding { key: "p".into(), description: "Previous window".into() },
                Keybinding { key: "0-9".into(), description: "Jump to window".into() },
                Keybinding { key: "z".into(), description: "Toggle zoom".into() },
                Keybinding { key: "space".into(), description: "Toggle tiling".into() },
                Keybinding { key: "-".into(), description: "Split horizontal".into() },
                Keybinding { key: "|/\\".into(), description: "Split vertical".into() },
                Keybinding { key: "R".into(), description: "Rotate split".into() },
                Keybinding { key: "=".into(), description: "Equalize splits".into() },
                Keybinding { key: "w".into(), description: "Workspace commands...".into() },
                Keybinding { key: "m".into(), description: "Minimize commands...".into() },
                Keybinding { key: "t".into(), description: "Window commands...".into() },
                Keybinding { key: "D".into(), description: "Debug commands...".into() },
                Keybinding { key: "T".into(), description: "Tape manager...".into() },
                Keybinding { key: "P".into(), description: "Command palette".into() },
                Keybinding { key: "S".into(), description: "Session switcher".into() },
                Keybinding { key: "W".into(), description: "Workspace switcher".into() },
                Keybinding { key: "L".into(), description: "Layout commands...".into() },
                Keybinding { key: "b".into(), description: "Toggle sidebar".into() },
                Keybinding { key: "e".into(), description: "Focus/leave sidebar".into() },
                Keybinding { key: "j".into(), description: "Jump to newest message".into() },
                Keybinding { key: "X".into(), description: "Close session".into() },
            ];

            if is_daemon_session {
                bindings.push(Keybinding { key: "d".into(), description: "Detach session".into() });
                bindings.push(Keybinding { key: "Esc".into(), description: "Window mode".into() });
            } else {
                bindings.push(Keybinding { key: "d/Esc".into(), description: "Window mode".into() });
            }

            bindings.push(Keybinding { key: "[".into(), description: "Scrollback mode".into() });
            bindings.push(Keybinding { key: "s".into(), description: "Scrollback browser".into() });
            bindings.push(Keybinding { key: "?".into(), description: "Toggle help".into() });

            if is_daemon_session {
                bindings.push(Keybinding { key: "q".into(), description: "Quit menu".into() });
            } else {
                bindings.push(Keybinding { key: "q".into(), description: "Quit".into() });
            }

            bindings
        }
    }
}

/// The default window-management keybindings (the `keybindings.window_management`
/// TOML table).
pub fn default_window_management() -> HashMap<String, Vec<String>> {
    let mut m = HashMap::new();
    m.insert("new_window".into(), vec!["n".into()]);
    m.insert("close_window".into(), vec!["w".into(), "x".into()]);
    m.insert("rename_window".into(), vec!["r".into()]);
    m.insert("minimize_window".into(), vec!["m".into()]);
    m.insert("restore_all".into(), vec!["M".into()]);
    m.insert("toggle_zoom".into(), vec!["z".into()]);
    m.insert("copy_selection".into(), vec!["c".into()]);
    m.insert("next_window".into(), vec!["tab".into()]);
    m.insert("prev_window".into(), vec!["shift+tab".into()]);
    for i in 1..=9 {
        m.insert(format!("select_window_{i}"), vec![i.to_string()]);
    }
    m
}

/// The default prefix-mode keybindings.
pub fn default_prefix_mode() -> HashMap<String, Vec<String>> {
    let mut m = HashMap::new();
    m.insert("prefix_new_window".into(), vec!["c".into()]);
    m.insert("prefix_close_window".into(), vec!["x".into()]);
    m.insert("prefix_rename_window".into(), vec!["r".into()]);
    m.insert("prefix_settings".into(), vec![",".into()]);
    m.insert("prefix_next_window".into(), vec!["n".into(), "tab".into()]);
    m.insert("prefix_prev_window".into(), vec!["p".into(), "shift+tab".into()]);
    for i in 0..=9 {
        m.insert(format!("prefix_select_{i}"), vec![i.to_string()]);
    }
    m.insert("prefix_toggle_tiling".into(), vec!["space".into()]);
    m.insert("prefix_workspace".into(), vec!["w".into()]);
    m.insert("prefix_minimize".into(), vec!["m".into()]);
    m.insert("prefix_window".into(), vec!["t".into()]);
    m.insert("prefix_detach".into(), vec!["d".into()]);
    m.insert("prefix_close_session".into(), vec!["X".into()]);
    m.insert("prefix_exit_mode".into(), vec!["esc".into()]);
    m.insert("prefix_selection".into(), vec!["[".into()]);
    m.insert("prefix_help".into(), vec!["?".into()]);
    m.insert("prefix_debug".into(), vec!["D".into()]);
    m.insert("prefix_tape".into(), vec!["T".into()]);
    m.insert("prefix_quit".into(), vec!["q".into()]);
    m.insert("prefix_fullscreen".into(), vec!["z".into()]);
    m.insert("prefix_split_horizontal".into(), vec!["-".into()]);
    m.insert("prefix_split_vertical".into(), vec!["|".into(), "\\".into()]);
    m.insert("prefix_rotate_split".into(), vec!["R".into()]);
    m.insert("prefix_equalize_splits".into(), vec!["=".into()]);
    m.insert("prefix_scrollback".into(), vec!["s".into()]);
    m.insert("prefix_command_palette".into(), vec!["P".into()]);
    m.insert("prefix_toggle_sidebar".into(), vec!["b".into()]);
    m.insert("prefix_session_switcher".into(), vec!["S".into()]);
    m.insert("prefix_workspace_switcher".into(), vec!["W".into()]);
    m.insert("prefix_layout".into(), vec!["L".into()]);
    m
}

/// The default workspace keybindings (alt+1..9 on Linux, opt+1..9 on macOS).
pub fn default_workspaces() -> HashMap<String, Vec<String>> {
    let mut m = HashMap::new();
    for i in 1..=9 {
        m.insert(format!("switch_workspace_{i}"), vec![format!("alt+{i}")]);
        m.insert(
            format!("move_and_follow_{i}"),
            vec![format!("alt+shift+{i}")],
        );
    }
    m
}

/// The default layout keybindings (snap, swap, resize, BSP split).
pub fn default_layout() -> HashMap<String, Vec<String>> {
    let mut m = HashMap::new();
    m.insert("snap_left".into(), vec!["h".into()]);
    m.insert("snap_right".into(), vec!["l".into()]);
    m.insert("snap_fullscreen".into(), vec!["f".into()]);
    m.insert("unsnap".into(), vec!["u".into()]);
    m.insert("toggle_tiling".into(), vec!["t".into()]);
    m.insert("swap_left".into(), vec!["H".into(), "ctrl+left".into()]);
    m.insert("swap_right".into(), vec!["L".into(), "ctrl+right".into()]);
    m.insert("swap_up".into(), vec!["K".into(), "ctrl+up".into()]);
    m.insert("swap_down".into(), vec!["J".into(), "ctrl+down".into()]);
    m.insert("split_horizontal".into(), vec!["-".into()]);
    m.insert("split_vertical".into(), vec!["|".into(), "\\".into()]);
    m.insert("rotate_split".into(), vec!["R".into()]);
    m.insert("equalize_splits".into(), vec!["=".into()]);
    m.insert("preselect_left".into(), vec!["alt+h".into()]);
    m.insert("preselect_right".into(), vec!["alt+l".into()]);
    m.insert("preselect_up".into(), vec!["alt+k".into()]);
    m.insert("preselect_down".into(), vec!["alt+j".into()]);
    m
}

/// The default mode-control keybindings (vim-like mode entry/exit).
pub fn default_mode_control() -> HashMap<String, Vec<String>> {
    let mut m = HashMap::new();
    m.insert("enter_terminal_mode".into(), vec!["i".into(), "enter".into()]);
    m.insert("enter_window_mode".into(), vec!["esc".into()]);
    m.insert("toggle_help".into(), vec!["?".into()]);
    m.insert("quit".into(), vec!["q".into()]);
    m
}

/// The default navigation keybindings.
pub fn default_navigation() -> HashMap<String, Vec<String>> {
    let mut m = HashMap::new();
    m.insert("nav_up".into(), vec!["up".into()]);
    m.insert("nav_down".into(), vec!["down".into()]);
    m.insert("nav_left".into(), vec!["left".into()]);
    m.insert("nav_right".into(), vec!["right".into()]);
    m
}

/// The default terminal-mode keybindings (direct, no prefix).
pub fn default_terminal_mode() -> HashMap<String, Vec<String>> {
    let mut m = HashMap::new();
    m.insert("terminal_next_window".into(), vec!["alt+n".into()]);
    m.insert("terminal_prev_window".into(), vec!["alt+p".into()]);
    m.insert("terminal_exit_mode".into(), vec!["alt+esc".into()]);
    m.insert("terminal_focus_left".into(), vec!["alt+left".into()]);
    m.insert("terminal_focus_right".into(), vec!["alt+right".into()]);
    m.insert("terminal_focus_up".into(), vec!["alt+up".into()]);
    m.insert("terminal_focus_down".into(), vec!["alt+down".into()]);
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn general_prefix_bindings_include_core_actions() {
        let bindings = get_prefix_keybindings("", false);
        let keys: Vec<&str> = bindings.iter().map(|b| b.key.as_str()).collect();
        assert!(keys.contains(&"c"));
        assert!(keys.contains(&"n"));
        assert!(keys.contains(&"-"));
        assert!(keys.contains(&"P"));
        assert!(keys.contains(&"q"));
    }

    #[test]
    fn daemon_mode_changes_detach_description() {
        let bindings = get_prefix_keybindings("", true);
        let detach = bindings.iter().find(|b| b.key == "d");
        assert_eq!(detach.unwrap().description, "Detach session");
    }

    #[test]
    fn default_prefix_selects_cover_0_through_9() {
        let m = default_prefix_mode();
        for i in 0..=9 {
            assert!(m.contains_key(&format!("prefix_select_{i}")));
        }
    }
}
