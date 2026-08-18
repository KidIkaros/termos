//! Keybind action registry — ported from Go TUIOS `internal/config/registry.go`.
//!
//! Manages the mapping between key chords and action names. Two maps are
//! maintained: one for window-management mode and one for terminal mode.

use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Action name constants
// ---------------------------------------------------------------------------

pub const ACTION_NEW_WINDOW: &str = "new_window";
pub const ACTION_CLOSE_WINDOW: &str = "close_window";
pub const ACTION_NEXT_WINDOW: &str = "next_window";
pub const ACTION_PREV_WINDOW: &str = "prev_window";
pub const ACTION_SPLIT_VERTICAL: &str = "split_vertical";
pub const ACTION_SPLIT_HORIZONTAL: &str = "split_horizontal";
pub const ACTION_TOGGLE_TILING: &str = "toggle_tiling";
pub const ACTION_FULLSCREEN: &str = "fullscreen";
pub const ACTION_RENAME_WINDOW: &str = "rename_window";
pub const ACTION_MOVE_LEFT: &str = "move_left";
pub const ACTION_MOVE_RIGHT: &str = "move_right";
pub const ACTION_MOVE_UP: &str = "move_up";
pub const ACTION_MOVE_DOWN: &str = "move_down";
pub const ACTION_COPY_MODE: &str = "copy_mode";
pub const ACTION_ENTER_PREFIX: &str = "enter_prefix";
pub const ACTION_COMMAND_PALETTE: &str = "command_palette";
pub const ACTION_SETTINGS: &str = "settings";
pub const ACTION_HELP: &str = "help";
pub const ACTION_QUIT: &str = "quit";
pub const ACTION_KILL_WINDOW: &str = "kill_window";
pub const ACTION_SCROLL_UP: &str = "scroll_up";
pub const ACTION_SCROLL_DOWN: &str = "scroll_down";
pub const ACTION_PAGE_UP: &str = "page_up";
pub const ACTION_PAGE_DOWN: &str = "page_down";
pub const ACTION_SWITCH_WORKSPACE: &str = "switch_workspace";
pub const ACTION_NEXT_WORKSPACE: &str = "next_workspace";
pub const ACTION_PREV_WORKSPACE: &str = "prev_workspace";
pub const ACTION_MINIMIZE: &str = "minimize";
pub const ACTION_RESTORE: &str = "restore";
pub const ACTION_THEME_PICKER: &str = "theme_picker";
pub const ACTION_TAPE_MANAGER: &str = "tape_manager";
pub const ACTION_TOGGLE_BORDER: &str = "toggle_border";
pub const ACTION_RESIZE_LEFT: &str = "resize_left";
pub const ACTION_RESIZE_RIGHT: &str = "resize_right";
pub const ACTION_RESIZE_UP: &str = "resize_up";
pub const ACTION_RESIZE_DOWN: &str = "resize_down";

/// The keybind registry: maps chord strings to action names.
pub struct KeybindRegistry {
    wm_map: HashMap<String, String>,
    terminal_map: HashMap<String, String>,
    prefix_map: HashMap<String, String>,
    window_prefix_map: HashMap<String, String>,
    minimize_prefix_map: HashMap<String, String>,
    workspace_prefix_map: HashMap<String, String>,
    debug_prefix_map: HashMap<String, String>,
    tape_prefix_map: HashMap<String, String>,
}

impl KeybindRegistry {
    /// Build a new registry from the default keybindings.
    pub fn new() -> Self {
        let mut reg = Self {
            wm_map: HashMap::new(),
            terminal_map: HashMap::new(),
            prefix_map: HashMap::new(),
            window_prefix_map: HashMap::new(),
            minimize_prefix_map: HashMap::new(),
            workspace_prefix_map: HashMap::new(),
            debug_prefix_map: HashMap::new(),
            tape_prefix_map: HashMap::new(),
        };
        reg.build_defaults();
        reg
    }

    /// Look up the action for a chord in window-management mode.
    pub fn get_action(&self, chord: &str) -> Option<&str> {
        let normalized = normalize_chord(chord);
        self.wm_map.get(&normalized).map(|s| s.as_str())
    }

    /// Look up the action for a chord in terminal mode.
    pub fn get_terminal_mode_action(&self, chord: &str) -> Option<&str> {
        let normalized = normalize_chord(chord);
        self.terminal_map.get(&normalized).map(|s| s.as_str())
    }

    /// Look up the action for a chord in the main prefix mode (Ctrl+B).
    pub fn get_prefix_action(&self, chord: &str) -> Option<&str> {
        let normalized = normalize_prefix_chord(chord);
        self.prefix_map.get(&normalized).map(|s| s.as_str())
    }

    /// Look up the action for a chord in window prefix mode (Ctrl+B, t).
    pub fn get_window_prefix_action(&self, chord: &str) -> Option<&str> {
        let normalized = normalize_chord(chord);
        self.window_prefix_map
            .get(&normalized)
            .map(|s| s.as_str())
    }

    /// Look up the action for a chord in minimize prefix mode (Ctrl+B, m).
    pub fn get_minimize_prefix_action(&self, chord: &str) -> Option<&str> {
        let normalized = normalize_prefix_chord(chord);
        self.minimize_prefix_map
            .get(&normalized)
            .map(|s| s.as_str())
    }

    /// Look up the action for a chord in workspace prefix mode (Ctrl+B, w).
    pub fn get_workspace_prefix_action(&self, chord: &str) -> Option<&str> {
        let normalized = normalize_prefix_chord(chord);
        self.workspace_prefix_map
            .get(&normalized)
            .map(|s| s.as_str())
    }

    /// Look up the action for a chord in debug prefix mode (Ctrl+B, D).
    pub fn get_debug_prefix_action(&self, chord: &str) -> Option<&str> {
        let normalized = normalize_prefix_chord(chord);
        self.debug_prefix_map.get(&normalized).map(|s| s.as_str())
    }

    /// Look up the action for a chord in tape prefix mode (Ctrl+B, T).
    pub fn get_tape_prefix_action(&self, chord: &str) -> Option<&str> {
        let normalized = normalize_prefix_chord(chord);
        self.tape_prefix_map.get(&normalized).map(|s| s.as_str())
    }

    /// List all registered action names (deduplicated).
    pub fn actions(&self) -> Vec<String> {
        let mut seen: Vec<String> = self
            .wm_map
            .values()
            .chain(self.terminal_map.values())
            .chain(self.prefix_map.values())
            .chain(self.window_prefix_map.values())
            .chain(self.minimize_prefix_map.values())
            .chain(self.workspace_prefix_map.values())
            .chain(self.debug_prefix_map.values())
            .chain(self.tape_prefix_map.values())
            .cloned()
            .collect();
        seen.sort();
        seen.dedup();
        seen
    }

    /// Reverse lookup: find all chords bound to a given action.
    pub fn chords_for_action(&self, action: &str) -> Vec<&str> {
        self.wm_map
            .iter()
            .chain(self.terminal_map.iter())
            .chain(self.prefix_map.iter())
            .chain(self.window_prefix_map.iter())
            .chain(self.minimize_prefix_map.iter())
            .chain(self.workspace_prefix_map.iter())
            .chain(self.debug_prefix_map.iter())
            .chain(self.tape_prefix_map.iter())
            .filter(|(_, a)| a.as_str() == action)
            .map(|(k, _)| k.as_str())
            .collect()
    }

    /// Register a chord→action mapping in WM mode.
    pub fn register_wm(&mut self, chord: &str, action: &str) {
        let normalized = normalize_chord(chord);
        self.wm_map.insert(normalized, action.to_string());
    }

    /// Register a chord→action mapping in terminal mode.
    pub fn register_terminal(&mut self, chord: &str, action: &str) {
        let normalized = normalize_chord(chord);
        self.terminal_map.insert(normalized, action.to_string());
    }

    /// Register a chord→action mapping in prefix mode.
    pub fn register_prefix(&mut self, chord: &str, action: &str) {
        // Prefix mode preserves case (x and X are different chords).
        let normalized = normalize_prefix_chord(chord);
        self.prefix_map.insert(normalized, action.to_string());
    }

    /// Register a chord→action mapping in window prefix mode.
    pub fn register_window_prefix(&mut self, chord: &str, action: &str) {
        let normalized = normalize_chord(chord);
        self.window_prefix_map.insert(normalized, action.to_string());
    }

    /// Register a chord→action mapping in minimize prefix mode.
    pub fn register_minimize_prefix(&mut self, chord: &str, action: &str) {
        let normalized = normalize_prefix_chord(chord);
        self.minimize_prefix_map
            .insert(normalized, action.to_string());
    }

    /// Register a chord→action mapping in workspace prefix mode.
    pub fn register_workspace_prefix(&mut self, chord: &str, action: &str) {
        let normalized = normalize_prefix_chord(chord);
        self.workspace_prefix_map
            .insert(normalized, action.to_string());
    }

    /// Register a chord→action mapping in debug prefix mode.
    pub fn register_debug_prefix(&mut self, chord: &str, action: &str) {
        let normalized = normalize_prefix_chord(chord);
        self.debug_prefix_map.insert(normalized, action.to_string());
    }

    /// Register a chord→action mapping in tape prefix mode.
    pub fn register_tape_prefix(&mut self, chord: &str, action: &str) {
        let normalized = normalize_prefix_chord(chord);
        self.tape_prefix_map.insert(normalized, action.to_string());
    }

    fn build_defaults(&mut self) {
        // Window management mode defaults.
        let wm_defaults: &[(&str, &str)] = &[
            ("c", ACTION_NEW_WINDOW),
            ("x", ACTION_CLOSE_WINDOW),
            ("n", ACTION_NEXT_WINDOW),
            ("p", ACTION_PREV_WINDOW),
            ("v", ACTION_SPLIT_VERTICAL),
            ("s", ACTION_SPLIT_HORIZONTAL),
            ("t", ACTION_TOGGLE_TILING),
            ("f", ACTION_FULLSCREEN),
            (",", ACTION_RENAME_WINDOW),
            ("h", ACTION_MOVE_LEFT),
            ("l", ACTION_MOVE_RIGHT),
            ("k", ACTION_MOVE_UP),
            ("j", ACTION_MOVE_DOWN),
            ("[", ACTION_COPY_MODE),
            ("?", ACTION_HELP),
            ("q", ACTION_QUIT),
            ("ctrl+b", ACTION_ENTER_PREFIX),
            ("ctrl+c", ACTION_NEW_WINDOW),
            ("ctrl+n", ACTION_NEXT_WINDOW),
            ("ctrl+p", ACTION_PREV_WINDOW),
            ("ctrl+d", ACTION_CLOSE_WINDOW),
            ("alt+1", ACTION_SWITCH_WORKSPACE),
            ("alt+2", ACTION_SWITCH_WORKSPACE),
            ("alt+3", ACTION_SWITCH_WORKSPACE),
            ("alt+4", ACTION_SWITCH_WORKSPACE),
            ("alt+5", ACTION_SWITCH_WORKSPACE),
            ("alt+6", ACTION_SWITCH_WORKSPACE),
            ("alt+7", ACTION_SWITCH_WORKSPACE),
            ("alt+8", ACTION_SWITCH_WORKSPACE),
            ("alt+9", ACTION_SWITCH_WORKSPACE),
        ];
        for (chord, action) in wm_defaults {
            self.register_wm(chord, action);
        }

        // Terminal mode defaults.
        let terminal_defaults: &[(&str, &str)] = &[
            ("ctrl+b", ACTION_ENTER_PREFIX),
            ("ctrl+shift+c", ACTION_COPY_MODE),
        ];
        for (chord, action) in terminal_defaults {
            self.register_terminal(chord, action);
        }

        // Prefix mode defaults (after leader key).
        let prefix_defaults: &[(&str, &str)] = &[
            ("c", ACTION_NEW_WINDOW),
            ("x", ACTION_CLOSE_WINDOW),
            ("r", ACTION_RENAME_WINDOW),
            ("n", ACTION_NEXT_WINDOW),
            ("p", ACTION_PREV_WINDOW),
            ("z", ACTION_FULLSCREEN),
            ("q", ACTION_QUIT),
            ("?", ACTION_HELP),
            ("esc", "prefix_exit_mode"),
            ("[", ACTION_COPY_MODE),
            ("D", "prefix_debug"),
            ("T", "prefix_tape"),
            ("w", "prefix_workspace"),
            ("m", "prefix_minimize"),
            ("t", "prefix_window"),
            ("s", ACTION_SCROLL_UP),
            ("P", ACTION_COMMAND_PALETTE),
            ("S", "prefix_session_switcher"),
            ("W", "prefix_workspace_switcher"),
            ("L", "prefix_layout"),
            ("b", "prefix_toggle_sidebar"),
            ("e", "prefix_explore"),
            ("j", "prefix_jump_notif"),
            ("X", "prefix_close_session"),
            ("d", "prefix_detach"),
            ("space", ACTION_TOGGLE_TILING),
            ("-", "prefix_split_horizontal"),
            ("|", "prefix_split_vertical"),
            ("\\", "prefix_split_vertical"),
            ("R", "prefix_rotate_split"),
            ("=", "prefix_equalize_splits"),
            (",", "prefix_settings"),
        ];
        for (chord, action) in prefix_defaults {
            self.register_prefix(chord, action);
        }

        // Window prefix mode defaults (after leader, t).
        let window_prefix_defaults: &[(&str, &str)] = &[
            ("n", "window_prefix_new"),
            ("x", "window_prefix_close"),
            ("r", "window_prefix_rename"),
            ("tab", "window_prefix_next"),
            ("shift+tab", "window_prefix_prev"),
            ("t", "window_prefix_tiling"),
            ("esc", "window_prefix_cancel"),
        ];
        for (chord, action) in window_prefix_defaults {
            self.register_window_prefix(chord, action);
        }

        // Minimize prefix mode defaults (after leader, m).
        let minimize_prefix_defaults: &[(&str, &str)] = &[
            ("m", "minimize_prefix_focused"),
            ("1", "minimize_prefix_restore_1"),
            ("2", "minimize_prefix_restore_2"),
            ("3", "minimize_prefix_restore_3"),
            ("4", "minimize_prefix_restore_4"),
            ("5", "minimize_prefix_restore_5"),
            ("6", "minimize_prefix_restore_6"),
            ("7", "minimize_prefix_restore_7"),
            ("8", "minimize_prefix_restore_8"),
            ("9", "minimize_prefix_restore_9"),
            ("M", "minimize_prefix_restore_all"),
            ("esc", "minimize_prefix_cancel"),
        ];
        for (chord, action) in minimize_prefix_defaults {
            self.register_minimize_prefix(chord, action);
        }

        // Workspace prefix mode defaults (after leader, w).
        let workspace_prefix_defaults: &[(&str, &str)] = &[
            ("1", "workspace_prefix_switch_1"),
            ("2", "workspace_prefix_switch_2"),
            ("3", "workspace_prefix_switch_3"),
            ("4", "workspace_prefix_switch_4"),
            ("5", "workspace_prefix_switch_5"),
            ("6", "workspace_prefix_switch_6"),
            ("7", "workspace_prefix_switch_7"),
            ("8", "workspace_prefix_switch_8"),
            ("9", "workspace_prefix_switch_9"),
            ("!", "workspace_prefix_move_1"),
            ("@", "workspace_prefix_move_2"),
            ("#", "workspace_prefix_move_3"),
            ("$", "workspace_prefix_move_4"),
            ("%", "workspace_prefix_move_5"),
            ("^", "workspace_prefix_move_6"),
            ("&", "workspace_prefix_move_7"),
            ("*", "workspace_prefix_move_8"),
            ("(", "workspace_prefix_move_9"),
            ("r", "workspace_prefix_rename"),
            ("esc", "workspace_prefix_cancel"),
        ];
        for (chord, action) in workspace_prefix_defaults {
            self.register_workspace_prefix(chord, action);
        }

        // Debug prefix mode defaults (after leader, D).
        let debug_prefix_defaults: &[(&str, &str)] = &[
            ("l", "debug_prefix_logs"),
            ("c", "debug_prefix_cache"),
            ("a", "debug_prefix_animations"),
            ("k", "debug_prefix_showkeys"),
            ("esc", "debug_prefix_cancel"),
        ];
        for (chord, action) in debug_prefix_defaults {
            self.register_debug_prefix(chord, action);
        }

        // Tape prefix mode defaults (after leader, T).
        let tape_prefix_defaults: &[(&str, &str)] = &[
            ("m", "tape_prefix_manager"),
            ("t", "tape_prefix_review"),
            ("r", "tape_prefix_record"),
            ("s", "tape_prefix_stop"),
            ("esc", "tape_prefix_cancel"),
        ];
        for (chord, action) in tape_prefix_defaults {
            self.register_tape_prefix(chord, action);
        }
    }
}

impl Default for KeybindRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Normalize a chord string: lowercase, canonicalize modifier prefixes.
pub fn normalize_chord(chord: &str) -> String {
    let chord = chord.trim().to_lowercase();
    // Canonicalize "cmd+" → "super+", "opt+" → "alt+"
    let chord = chord.replace("cmd+", "super+");
    let chord = chord.replace("opt+", "alt+");
    // Ensure "ctrl+" not "control+"
    chord.replace("control+", "ctrl+")
}

/// Normalize a chord for prefix mode, preserving case (x ≠ X).
fn normalize_prefix_chord(chord: &str) -> String {
    let chord = chord.trim();
    // Canonicalize "cmd+" → "super+", "opt+" → "alt+" (case-insensitive).
    let chord = chord.replace("cmd+", "super+");
    let chord = chord.replace("CMD+", "super+");
    let chord = chord.replace("Cmd+", "super+");
    let chord = chord.replace("opt+", "alt+");
    let chord = chord.replace("OPT+", "alt+");
    let chord = chord.replace("Opt+", "alt+");
    // Ensure "ctrl+" not "control+" (case-insensitive).
    chord
        .replace("control+", "ctrl+")
        .replace("Control+", "ctrl+")
        .replace("CONTROL+", "ctrl+")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_new_has_defaults() {
        let reg = KeybindRegistry::new();
        assert_eq!(reg.get_action("c"), Some(ACTION_NEW_WINDOW));
        assert_eq!(reg.get_action("x"), Some(ACTION_CLOSE_WINDOW));
        assert_eq!(reg.get_action("ctrl+b"), Some(ACTION_ENTER_PREFIX));
    }

    #[test]
    fn registry_terminal_mode() {
        let reg = KeybindRegistry::new();
        assert_eq!(
            reg.get_terminal_mode_action("ctrl+b"),
            Some(ACTION_ENTER_PREFIX)
        );
        // WM-only binding should not appear in terminal mode.
        assert_eq!(reg.get_terminal_mode_action("c"), None);
    }

    #[test]
    fn registry_chords_for_action() {
        let reg = KeybindRegistry::new();
        let chords = reg.chords_for_action(ACTION_NEXT_WINDOW);
        assert!(chords.contains(&"n"));
        assert!(chords.contains(&"ctrl+n"));
    }

    #[test]
    fn registry_actions_list() {
        let reg = KeybindRegistry::new();
        let actions = reg.actions();
        assert!(actions.contains(&ACTION_NEW_WINDOW.to_string()));
        assert!(actions.contains(&ACTION_QUIT.to_string()));
    }

    #[test]
    fn registry_register_custom() {
        let mut reg = KeybindRegistry::new();
        reg.register_wm("ctrl+e", "my_custom_action");
        assert_eq!(reg.get_action("ctrl+e"), Some("my_custom_action"));
    }

    #[test]
    fn normalize_chord_basic() {
        assert_eq!(normalize_chord("Ctrl+B"), "ctrl+b");
        assert_eq!(normalize_chord("ALT+x"), "alt+x");
        assert_eq!(normalize_chord("cmd+c"), "super+c");
        assert_eq!(normalize_chord("opt+x"), "alt+x");
        assert_eq!(normalize_chord("control+a"), "ctrl+a");
    }

    #[test]
    fn normalize_chord_whitespace() {
        assert_eq!(normalize_chord("  ctrl+b  "), "ctrl+b");
    }

    #[test]
    fn registry_unknown_chord() {
        let reg = KeybindRegistry::new();
        assert_eq!(reg.get_action("zzz"), None);
    }

    #[test]
    fn registry_prefix_action() {
        let reg = KeybindRegistry::new();
        assert_eq!(reg.get_prefix_action("c"), Some(ACTION_NEW_WINDOW));
        assert_eq!(reg.get_prefix_action("x"), Some(ACTION_CLOSE_WINDOW));
        assert_eq!(reg.get_prefix_action("D"), Some("prefix_debug"));
        assert_eq!(reg.get_prefix_action("T"), Some("prefix_tape"));
    }

    #[test]
    fn registry_window_prefix_action() {
        let reg = KeybindRegistry::new();
        assert_eq!(reg.get_window_prefix_action("n"), Some("window_prefix_new"));
        assert_eq!(
            reg.get_window_prefix_action("x"),
            Some("window_prefix_close")
        );
        assert_eq!(
            reg.get_window_prefix_action("esc"),
            Some("window_prefix_cancel")
        );
    }

    #[test]
    fn registry_minimize_prefix_action() {
        let reg = KeybindRegistry::new();
        assert_eq!(
            reg.get_minimize_prefix_action("m"),
            Some("minimize_prefix_focused")
        );
        assert_eq!(
            reg.get_minimize_prefix_action("M"),
            Some("minimize_prefix_restore_all")
        );
    }

    #[test]
    fn registry_workspace_prefix_action() {
        let reg = KeybindRegistry::new();
        assert_eq!(
            reg.get_workspace_prefix_action("1"),
            Some("workspace_prefix_switch_1")
        );
        assert_eq!(
            reg.get_workspace_prefix_action("r"),
            Some("workspace_prefix_rename")
        );
    }

    #[test]
    fn registry_debug_prefix_action() {
        let reg = KeybindRegistry::new();
        assert_eq!(
            reg.get_debug_prefix_action("l"),
            Some("debug_prefix_logs")
        );
        assert_eq!(
            reg.get_debug_prefix_action("k"),
            Some("debug_prefix_showkeys")
        );
    }

    #[test]
    fn registry_tape_prefix_action() {
        let reg = KeybindRegistry::new();
        assert_eq!(
            reg.get_tape_prefix_action("m"),
            Some("tape_prefix_manager")
        );
        assert_eq!(
            reg.get_tape_prefix_action("r"),
            Some("tape_prefix_record")
        );
    }

    #[test]
    fn registry_prefix_unknown_chord() {
        let reg = KeybindRegistry::new();
        assert_eq!(reg.get_prefix_action("zzz"), None);
        assert_eq!(reg.get_debug_prefix_action("zzz"), None);
    }

    #[test]
    fn registry_register_custom_prefix() {
        let mut reg = KeybindRegistry::new();
        reg.register_prefix("ctrl+e", "my_prefix_action");
        assert_eq!(reg.get_prefix_action("ctrl+e"), Some("my_prefix_action"));
    }

    #[test]
    fn registry_actions_includes_prefix() {
        let reg = KeybindRegistry::new();
        let actions = reg.actions();
        assert!(actions.contains(&"prefix_debug".to_string()));
        assert!(actions.contains(&"tape_prefix_manager".to_string()));
    }
}
