//! User configuration — TOML loading and defaults, ported from TUIOS
//! `internal/config/userconfig.go`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::keybindings;

/// The default leader key (Ctrl+B).
pub const DEFAULT_LEADER_KEY: &str = "ctrl+b";

/// The default scrollback line count.
pub const DEFAULT_SCROLLBACK_LINES: i32 = 10_000;

/// The user's custom configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserConfig {
    #[serde(default)]
    pub appearance: AppearanceConfig,
    #[serde(default)]
    pub keybindings: KeybindingsConfig,
    #[serde(default)]
    pub startup: StartupConfig,
    #[serde(default)]
    pub daemon: DaemonConfig,
    /// Lifecycle hooks: event name → shell command or array of shell commands.
    /// Loaded into `hooks::Manager` at startup (see `internal/hooks/hooks.go`).
    #[serde(default)]
    pub hooks: std::collections::HashMap<String, toml::Value>,
    /// Alert/notification sinks, including the `[notifications.agent]` table.
    #[serde(default)]
    pub notifications: NotificationsConfig,
}

/// The `[notifications]` table.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NotificationsConfig {
    /// What tuios does when a pane's agent state changes.
    #[serde(default)]
    pub agent: AgentAlertsConfig,
}

/// The `[notifications.agent]` table. Every toggle is an `Option` so `None`
/// can mean "unset, use the default" and an explicit `false` survives a
/// reload, matching the Go pointer-field design.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentAlertsConfig {
    /// Master switch; false silences every sink including the command.
    /// Default: true.
    #[serde(default)]
    pub enabled: Option<bool>,
    /// Which transitions alert at all.
    #[serde(default)]
    pub states: AgentAlertStates,
    /// In-band terminal notification to the attached client. Default: true.
    #[serde(default)]
    pub notify: Option<bool>,
    /// Make the alert audible (see `sound_mode`). Default: false.
    #[serde(default)]
    pub sound: Option<bool>,
    /// `audio`, `bell`, or `both`. Default: `audio`.
    #[serde(default)]
    pub sound_mode: String,
    /// Shortest gap between audible cues (seconds). Default: 3.
    #[serde(default)]
    pub sound_cooldown_seconds: Option<i64>,
    /// User-supplied cue files.
    #[serde(default)]
    pub sounds: AgentAlertSounds,
    /// Show the message in tuios's dock, clickable, jumping to the pane.
    /// Default: true.
    #[serde(default)]
    pub dock: Option<bool>,
    /// Shell command run on an alert; shorthand for the after-agent-state
    /// hook. Default: empty (nothing runs).
    #[serde(default)]
    pub command: String,
    /// Hold an alert this long, dropping it if the pane leaves the state
    /// before it expires (seconds; 0 alerts immediately). Default: 2.
    #[serde(default)]
    pub settle_seconds: Option<i64>,
    /// Drop alerts for the pane the user is already looking at. Default: true.
    #[serde(default)]
    pub suppress_focused: Option<bool>,
    /// Silence every sink inside `"HH:MM-HH:MM"` (local time; wraps midnight).
    /// Default: empty (never quiet).
    #[serde(default)]
    pub quiet_hours: String,
}

/// One toggle per agent state, naming the states worth interrupting for.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentAlertStates {
    /// Default: true.
    #[serde(default)]
    pub needs_input: Option<bool>,
    /// Default: true.
    #[serde(default)]
    pub errored: Option<bool>,
    /// Default: true.
    #[serde(default)]
    pub done: Option<bool>,
    /// Default: false (the flappy, silence-guessed state).
    #[serde(default)]
    pub idle: Option<bool>,
    /// Default: false.
    #[serde(default)]
    pub working: Option<bool>,
}

/// User-supplied cue files, a path per cue. A path that does not exist falls
/// back to the built-in cue.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentAlertSounds {
    /// Cue for an agent that stopped (`done`, `idle`).
    #[serde(default)]
    pub done: String,
    /// Cue for an agent waiting on a human or failed (`needs_input`, `errored`).
    #[serde(default)]
    pub needs_input: String,
}

/// Appearance settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppearanceConfig {
    pub border_style: String,
    pub hide_window_buttons: bool,
    pub hide_scrollbar: bool,
    pub scrollback_lines: i32,
    pub scroll_lines: i32,
    pub dockbar_position: String,
    pub preferred_shell: String,
    pub theme: String,
    pub shared_borders: bool,
    pub window_title_position: String,
    pub animations_enabled: bool,
    pub confirm_quit: bool,
    pub which_key_enabled: bool,
    pub which_key_position: String,
    pub border_focused_color: Option<String>,
    pub border_unfocused_color: Option<String>,
    pub show_clock: bool,
    pub show_cpu: bool,
    pub show_ram: bool,
    pub max_fps: i32,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            border_style: "rounded".into(),
            hide_window_buttons: false,
            hide_scrollbar: false,
            scrollback_lines: DEFAULT_SCROLLBACK_LINES,
            scroll_lines: 3,
            dockbar_position: "bottom".into(),
            preferred_shell: String::new(),
            theme: String::new(),
            shared_borders: false,
            window_title_position: "bottom".into(),
            animations_enabled: true,
            confirm_quit: false,
            which_key_enabled: true,
            which_key_position: "bottom-right".into(),
            border_focused_color: None,
            border_unfocused_color: None,
            show_clock: false,
            show_cpu: false,
            show_ram: false,
            max_fps: 60,
        }
    }
}

/// Keybinding configuration tables.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KeybindingsConfig {
    #[serde(default = "default_leader")]
    pub leader_key: String,
    #[serde(default)]
    pub window_management: HashMap<String, Vec<String>>,
    #[serde(default)]
    pub workspaces: HashMap<String, Vec<String>>,
    #[serde(default)]
    pub layout: HashMap<String, Vec<String>>,
    #[serde(default)]
    pub mode_control: HashMap<String, Vec<String>>,
    #[serde(default)]
    pub system: HashMap<String, Vec<String>>,
    #[serde(default)]
    pub navigation: HashMap<String, Vec<String>>,
    #[serde(default)]
    pub restore_minimized: HashMap<String, Vec<String>>,
    #[serde(default)]
    pub prefix_mode: HashMap<String, Vec<String>>,
    #[serde(default)]
    pub window_prefix: HashMap<String, Vec<String>>,
    #[serde(default)]
    pub minimize_prefix: HashMap<String, Vec<String>>,
    #[serde(default)]
    pub workspace_prefix: HashMap<String, Vec<String>>,
    #[serde(default)]
    pub debug_prefix: HashMap<String, Vec<String>>,
    #[serde(default)]
    pub tape_prefix: HashMap<String, Vec<String>>,
    #[serde(default)]
    pub terminal_mode: HashMap<String, Vec<String>>,
}

fn default_leader() -> String {
    DEFAULT_LEADER_KEY.to_string()
}

impl KeybindingsConfig {
    /// Fill in any missing sections with defaults.
    pub fn fill_missing(&mut self) {
        if self.leader_key.is_empty() {
            self.leader_key = DEFAULT_LEADER_KEY.to_string();
        }
        if self.window_management.is_empty() {
            self.window_management = keybindings::default_window_management();
        }
        if self.workspaces.is_empty() {
            self.workspaces = keybindings::default_workspaces();
        }
        if self.layout.is_empty() {
            self.layout = keybindings::default_layout();
        }
        if self.mode_control.is_empty() {
            self.mode_control = keybindings::default_mode_control();
        }
        if self.navigation.is_empty() {
            self.navigation = keybindings::default_navigation();
        }
        if self.prefix_mode.is_empty() {
            self.prefix_mode = keybindings::default_prefix_mode();
        }
        if self.terminal_mode.is_empty() {
            self.terminal_mode = keybindings::default_terminal_mode();
        }
    }
}

/// Startup preferences.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StartupConfig {
    pub open_default_window: bool,
    pub tiled: bool,
    pub start_in_terminal_mode: bool,
}

/// Daemon settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DaemonConfig {
    pub log_level: String,
    pub default_codec: String,
    pub socket_path: String,
}

impl UserConfig {
    /// The default configuration.
    pub fn default_config() -> Self {
        let mut cfg = Self {
            appearance: AppearanceConfig::default(),
            keybindings: KeybindingsConfig::default(),
            startup: StartupConfig::default(),
            daemon: DaemonConfig::default(),
            hooks: std::collections::HashMap::new(),
            notifications: NotificationsConfig::default(),
        };
        cfg.keybindings.fill_missing();
        cfg
    }

    /// The config file path (XDG: `~/.config/tuios/config.toml`).
    pub fn config_path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("termos").join("config.toml"))
    }

    /// Load the user config from the XDG config directory. If no file exists,
    /// returns the defaults (without writing).
    pub fn load() -> Self {
        let Some(path) = Self::config_path() else {
            return Self::default_config();
        };
        let data = match std::fs::read_to_string(&path) {
            Ok(data) => data,
            Err(_) => return Self::default_config(),
        };
        let mut cfg: UserConfig = match toml::from_str(&data) {
            Ok(cfg) => cfg,
            Err(_) => return Self::default_config(),
        };
        cfg.keybindings.fill_missing();
        cfg
    }

    /// Save the config to the XDG config path, creating directories as needed.
    pub fn save(&self) -> Result<(), String> {
        let Some(path) = Self::config_path() else {
            return Err("no config dir".into());
        };
        let toml = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, toml).map_err(|e| e.to_string())
    }

    /// Spawn a file watcher that reloads the config on change. Returns a
    /// receiver that yields a new `UserConfig` whenever the file is modified
    /// (debounced 300ms). Drops cleanly when the receiver is dropped.
    pub fn watch(path: PathBuf) -> Result<std::sync::mpsc::Receiver<UserConfig>, String> {
        use notify_debouncer_mini::{new_debouncer, DebouncedEventKind};
        let (tx, rx) = std::sync::mpsc::channel();
        let tx_clone = tx.clone();
        let watch_path = path.clone();
        let mut debouncer = new_debouncer(
            std::time::Duration::from_millis(300),
            move |res: Result<Vec<notify_debouncer_mini::DebouncedEvent>, _>| {
                if let Ok(events) = res {
                    if events.iter().any(|e| e.kind == DebouncedEventKind::Any) {
                        let cfg = UserConfig::load_from(&path);
                        let _ = tx_clone.send(cfg);
                    }
                }
            },
        )
        .map_err(|e| e.to_string())?;
        debouncer
            .watcher()
            .watch(&watch_path, notify::RecursiveMode::NonRecursive)
            .map_err(|e| e.to_string())?;
        // Keep the debouncer alive for the lifetime of the channel.
        std::mem::forget(debouncer);
        Ok(rx)
    }

    /// Load config from a specific path (used by the watcher).
    pub fn load_from(path: &Path) -> Self {
        let data = match std::fs::read_to_string(path) {
            Ok(data) => data,
            Err(_) => return Self::default_config(),
        };
        let mut cfg: UserConfig = match toml::from_str(&data) {
            Ok(cfg) => cfg,
            Err(_) => return Self::default_config(),
        };
        cfg.keybindings.fill_missing();
        cfg
    }
}

/// CLI flag overrides that take precedence over config file values.
/// Zero values indicate the flag was not set.
#[derive(Debug, Default, Clone)]
pub struct Overrides {
    pub border_style: Option<String>,
    pub dockbar_position: Option<String>,
    pub ascii_only: Option<bool>,
    pub theme: Option<String>,
    pub no_which_key: Option<bool>,
}

impl Overrides {
    /// Parse CLI flags from args. Returns (overrides, remaining_args).
    /// Recognized flags:
    /// --border-style <style>
    /// --dockbar-position <top|bottom>
    /// --ascii-only
    /// --theme <name>
    /// --no-which-key
    pub fn parse(args: &[String]) -> (Self, Vec<String>) {
        let mut ov = Self::default();
        let mut remaining = Vec::new();
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--border-style" => {
                    i += 1;
                    if let Some(v) = args.get(i) {
                        ov.border_style = Some(v.clone());
                    }
                }
                "--dockbar-position" => {
                    i += 1;
                    if let Some(v) = args.get(i) {
                        ov.dockbar_position = Some(v.clone());
                    }
                }
                "--ascii-only" => {
                    ov.ascii_only = Some(true);
                }
                "--theme" => {
                    i += 1;
                    if let Some(v) = args.get(i) {
                        ov.theme = Some(v.clone());
                    }
                }
                "--no-which-key" => {
                    ov.no_which_key = Some(true);
                }
                _ => {
                    remaining.push(args[i].clone());
                }
            }
            i += 1;
        }
        (ov, remaining)
    }

    /// Apply overrides to a loaded config.
    pub fn apply(&self, config: &mut UserConfig) {
        if let Some(ref bs) = self.border_style {
            config.appearance.border_style = bs.clone();
        }
        if let Some(ref dp) = self.dockbar_position {
            config.appearance.dockbar_position = dp.clone();
        }
        if let Some(true) = self.ascii_only {
            // ASCII-only mode: use plain borders and disable animations.
            config.appearance.border_style = "plain".into();
            config.appearance.animations_enabled = false;
        }
        if let Some(ref theme) = self.theme {
            config.appearance.theme = theme.clone();
        }
        if let Some(true) = self.no_which_key {
            config.appearance.which_key_enabled = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_leader_and_bindings() {
        let cfg = UserConfig::default_config();
        assert_eq!(cfg.keybindings.leader_key, "ctrl+b");
        assert!(cfg
            .keybindings
            .prefix_mode
            .contains_key("prefix_new_window"));
        assert!(cfg.keybindings.window_management.contains_key("new_window"));
        assert_eq!(cfg.appearance.border_style, "rounded");
        assert_eq!(cfg.appearance.scrollback_lines, 10_000);
    }

    #[test]
    fn round_trips_through_toml() {
        let cfg = UserConfig::default_config();
        let toml = toml::to_string(&cfg).expect("serialize");
        let back: UserConfig = toml::from_str(&toml).expect("deserialize");
        assert_eq!(back.keybindings.leader_key, "ctrl+b");
        assert_eq!(back.appearance.border_style, "rounded");
    }
}
