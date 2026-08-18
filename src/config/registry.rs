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
}

impl KeybindRegistry {
    /// Build a new registry from the default keybindings.
    pub fn new() -> Self {
        let mut reg = Self {
            wm_map: HashMap::new(),
            terminal_map: HashMap::new(),
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

    /// List all registered action names (deduplicated).
    pub fn actions(&self) -> Vec<String> {
        let mut seen: Vec<String> = self
            .wm_map
            .values()
            .chain(self.terminal_map.values())
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
}
