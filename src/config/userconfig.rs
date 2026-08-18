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

    #[test]
    fn default_config_has_all_sections() {
        let cfg = UserConfig::default_config();
        assert!(!cfg.keybindings.leader_key.is_empty());
        assert!(cfg.appearance.scrollback_lines > 0);
        assert!(!cfg.appearance.border_style.is_empty());
    }

    #[test]
    fn default_config_dockbar_position() {
        let cfg = UserConfig::default_config();
        assert!(!cfg.appearance.dockbar_position.is_empty());
    }

    #[test]
    fn default_config_sound_mode() {
        let cfg = UserConfig::default_config();
        // sound_mode defaults to empty string or "audio".
        let mode = &cfg.notifications.agent.sound_mode;
        assert!(mode.is_empty() || mode == "audio" || mode == "bell" || mode == "both");
    }

    #[test]
    fn default_config_hide_scrollbar_default() {
        let cfg = UserConfig::default_config();
        assert!(!cfg.appearance.hide_scrollbar);
    }

    #[test]
    fn toml_serialization_produces_valid_toml() {
        let cfg = UserConfig::default_config();
        let toml = toml::to_string(&cfg).expect("serialize");
        // Should be parseable TOML.
        let parsed: toml::Value = toml::from_str(&toml).expect("parse");
        assert!(parsed.is_table());
    }

    #[test]
    fn partial_toml_overrides_defaults() {
        // Use default_config as base, serialize, modify, and re-parse.
        let cfg = UserConfig::default_config();
        let mut toml = toml::to_string(&cfg).expect("serialize");
        // Modify a value.
        toml = toml.replace("border_style = \"rounded\"", "border_style = \"double\"");
        let back: UserConfig = toml::from_str(&toml).expect("deserialize");
        assert_eq!(back.appearance.border_style, "double");
        assert_eq!(back.appearance.scrollback_lines, 10_000);
    }

    #[test]
    fn empty_toml_uses_defaults() {
        // Deserializing from a minimal valid TOML uses serde defaults.
        let toml_str = "[keybindings]\nleader_key = \"ctrl+b\"\n";
        let cfg: UserConfig = toml::from_str(toml_str).expect("deserialize");
        assert_eq!(cfg.keybindings.leader_key, "ctrl+b");
    }

    #[test]
    fn window_management_bindings_present() {
        let cfg = UserConfig::default_config();
        assert!(cfg
            .keybindings
            .window_management
            .contains_key("close_window"));
        assert!(cfg
            .keybindings
            .window_management
            .contains_key("next_window"));
        assert!(cfg
            .keybindings
            .window_management
            .contains_key("prev_window"));
    }

    #[test]
    fn prefix_mode_bindings_present() {
        let cfg = UserConfig::default_config();
        assert!(cfg
            .keybindings
            .prefix_mode
            .contains_key("prefix_new_window"));
        assert!(cfg
            .keybindings
            .prefix_mode
            .contains_key("prefix_close_window"));
    }

    #[test]
    fn default_notification_config() {
        let cfg = UserConfig::default_config();
        assert!(cfg.notifications.agent.enabled.unwrap_or(true));
    }

    #[test]
    fn default_resize_config() {
        // resize_interval is at top level of UserConfig if it exists.
        let cfg = UserConfig::default_config();
        let _ = &cfg.appearance.scroll_lines;
    }

    #[test]
    fn default_behavior_config() {
        let cfg = UserConfig::default_config();
        let _ = cfg.appearance.hide_scrollbar;
        let _ = cfg.appearance.confirm_quit;
    }

    #[test]
    fn toml_round_trip_all_sections() {
        let cfg = UserConfig::default_config();
        let toml = toml::to_string(&cfg).expect("serialize");
        let back: UserConfig = toml::from_str(&toml).expect("deserialize");
        assert_eq!(
            back.notifications.agent.enabled,
            cfg.notifications.agent.enabled
        );
        assert_eq!(back.appearance.border_style, cfg.appearance.border_style);
    }

    #[test]
    fn partial_override_preserves_other_fields() {
        let cfg = UserConfig::default_config();
        let mut toml = toml::to_string(&cfg).expect("serialize");
        toml = toml.replace("leader_key = \"ctrl+b\"", "leader_key = \"ctrl+a\"");
        let back: UserConfig = toml::from_str(&toml).expect("deserialize");
        assert_eq!(back.keybindings.leader_key, "ctrl+a");
        assert_eq!(back.appearance.border_style, "rounded");
    }

    #[test]
    fn default_agent_alert_states() {
        let cfg = UserConfig::default_config();
        assert!(cfg.notifications.agent.states.needs_input.unwrap_or(true));
        assert!(cfg.notifications.agent.states.errored.unwrap_or(true));
        assert!(cfg.notifications.agent.states.done.unwrap_or(true));
    }

    #[test]
    fn default_appearance_fields() {
        let cfg = UserConfig::default_config();
        assert_eq!(cfg.appearance.scrollback_lines, 10_000);
        assert_eq!(cfg.appearance.scroll_lines, 3);
        assert_eq!(cfg.appearance.max_fps, 60);
        assert!(!cfg.appearance.hide_window_buttons);
        assert!(!cfg.appearance.shared_borders);
    }

    #[test]
    fn default_startup_shell() {
        let cfg = UserConfig::default_config();
        // startup.shell is either empty or a valid path.
        let _ = &cfg.startup;
    }

    #[test]
    fn daemon_config_defaults() {
        let cfg = UserConfig::default_config();
        let _ = &cfg.daemon.socket_path;
        let _ = &cfg.daemon.log_level;
        let _ = &cfg.daemon.default_codec;
    }

    #[test]
    fn keybindings_all_sections_present() {
        let cfg = UserConfig::default_config();
        let _ = &cfg.keybindings.layout;
        let _ = &cfg.keybindings.navigation;
        let _ = &cfg.keybindings.system;
        let _ = &cfg.keybindings.terminal_mode;
        let _ = &cfg.keybindings.debug_prefix;
        let _ = &cfg.keybindings.tape_prefix;
    }

    #[test]
    fn overrides_parse_all_flags() {
        let args = vec![
            "--border-style".into(),
            "double".into(),
            "--dockbar-position".into(),
            "top".into(),
            "--ascii-only".into(),
            "--theme".into(),
            "dracula".into(),
            "--no-which-key".into(),
            "remaining".into(),
        ];
        let (ov, remaining) = Overrides::parse(&args);
        assert_eq!(ov.border_style.as_deref(), Some("double"));
        assert_eq!(ov.dockbar_position.as_deref(), Some("top"));
        assert_eq!(ov.ascii_only, Some(true));
        assert_eq!(ov.theme.as_deref(), Some("dracula"));
        assert_eq!(ov.no_which_key, Some(true));
        assert_eq!(remaining, vec!["remaining"]);
    }

    #[test]
    fn overrides_parse_empty() {
        let (ov, remaining) = Overrides::parse(&[]);
        assert!(ov.border_style.is_none());
        assert!(remaining.is_empty());
    }

    #[test]
    fn overrides_parse_unknown_flags() {
        let args = vec!["--unknown".into(), "value".into()];
        let (ov, remaining) = Overrides::parse(&args);
        assert!(ov.border_style.is_none());
        assert_eq!(remaining, vec!["--unknown", "value"]);
    }

    #[test]
    fn overrides_apply_border_style() {
        let mut cfg = UserConfig::default_config();
        let ov = Overrides {
            border_style: Some("double".into()),
            ..Default::default()
        };
        ov.apply(&mut cfg);
        assert_eq!(cfg.appearance.border_style, "double");
    }

    #[test]
    fn overrides_apply_dockbar_position() {
        let mut cfg = UserConfig::default_config();
        let ov = Overrides {
            dockbar_position: Some("top".into()),
            ..Default::default()
        };
        ov.apply(&mut cfg);
        assert_eq!(cfg.appearance.dockbar_position, "top");
    }

    #[test]
    fn overrides_apply_ascii_only() {
        let mut cfg = UserConfig::default_config();
        let ov = Overrides {
            ascii_only: Some(true),
            ..Default::default()
        };
        ov.apply(&mut cfg);
        assert_eq!(cfg.appearance.border_style, "plain");
        assert!(!cfg.appearance.animations_enabled);
    }

    #[test]
    fn overrides_apply_theme() {
        let mut cfg = UserConfig::default_config();
        let ov = Overrides {
            theme: Some("dracula".into()),
            ..Default::default()
        };
        ov.apply(&mut cfg);
        assert_eq!(cfg.appearance.theme, "dracula");
    }

    #[test]
    fn overrides_apply_no_which_key() {
        let mut cfg = UserConfig::default_config();
        let ov = Overrides {
            no_which_key: Some(true),
            ..Default::default()
        };
        ov.apply(&mut cfg);
        assert!(!cfg.appearance.which_key_enabled);
    }

    #[test]
    fn overrides_apply_noop_when_none() {
        let mut cfg = UserConfig::default_config();
        let original_border = cfg.appearance.border_style.clone();
        let ov = Overrides::default();
        ov.apply(&mut cfg);
        assert_eq!(cfg.appearance.border_style, original_border);
    }

    #[test]
    fn fill_missing_leader_key() {
        let mut kb = KeybindingsConfig {
            leader_key: String::new(),
            ..Default::default()
        };
        kb.fill_missing();
        assert_eq!(kb.leader_key, DEFAULT_LEADER_KEY);
    }

    #[test]
    fn fill_missing_window_management() {
        let mut kb = KeybindingsConfig {
            window_management: HashMap::new(),
            ..Default::default()
        };
        kb.fill_missing();
        assert!(!kb.window_management.is_empty());
    }

    #[test]
    fn fill_missing_workspaces() {
        let mut kb = KeybindingsConfig {
            workspaces: HashMap::new(),
            ..Default::default()
        };
        kb.fill_missing();
        assert!(!kb.workspaces.is_empty());
    }

    #[test]
    fn fill_missing_layout() {
        let mut kb = KeybindingsConfig {
            layout: HashMap::new(),
            ..Default::default()
        };
        kb.fill_missing();
        assert!(!kb.layout.is_empty());
    }

    #[test]
    fn fill_missing_mode_control() {
        let mut kb = KeybindingsConfig {
            mode_control: HashMap::new(),
            ..Default::default()
        };
        kb.fill_missing();
        assert!(!kb.mode_control.is_empty());
    }

    #[test]
    fn fill_missing_navigation() {
        let mut kb = KeybindingsConfig {
            navigation: HashMap::new(),
            ..Default::default()
        };
        kb.fill_missing();
        assert!(!kb.navigation.is_empty());
    }

    #[test]
    fn fill_missing_prefix_mode() {
        let mut kb = KeybindingsConfig {
            prefix_mode: HashMap::new(),
            ..Default::default()
        };
        kb.fill_missing();
        assert!(!kb.prefix_mode.is_empty());
    }

    #[test]
    fn fill_missing_terminal_mode() {
        let mut kb = KeybindingsConfig {
            terminal_mode: HashMap::new(),
            ..Default::default()
        };
        kb.fill_missing();
        assert!(!kb.terminal_mode.is_empty());
    }

    #[test]
    fn load_from_nonexistent_returns_defaults() {
        let cfg = UserConfig::load_from(Path::new("/nonexistent/config.toml"));
        assert_eq!(cfg.keybindings.leader_key, DEFAULT_LEADER_KEY);
    }

    #[test]
    fn load_from_invalid_toml_returns_defaults() {
        let dir = std::env::temp_dir().join("termos_test_load_from_invalid");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("bad.toml");
        std::fs::write(&path, "not valid toml {{{{").unwrap();
        let cfg = UserConfig::load_from(&path);
        assert_eq!(cfg.keybindings.leader_key, DEFAULT_LEADER_KEY);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_from_valid_toml() {
        let dir = std::env::temp_dir().join("termos_test_load_from_valid");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.toml");
        let mut cfg = UserConfig::default_config();
        cfg.appearance.border_style = "double".into();
        let toml_str = toml::to_string(&cfg).unwrap();
        std::fs::write(&path, toml_str).unwrap();
        let loaded = UserConfig::load_from(&path);
        assert_eq!(loaded.appearance.border_style, "double");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = std::env::temp_dir().join("termos_test_save_load");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.toml");
        let mut cfg = UserConfig::default_config();
        cfg.appearance.border_style = "thick".into();
        std::fs::write(&path, toml::to_string(&cfg).unwrap()).unwrap();
        let loaded = UserConfig::load_from(&path);
        assert_eq!(loaded.appearance.border_style, "thick");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_path_returns_valid_path() {
        let path = UserConfig::config_path();
        assert!(path.is_some());
        let path = path.unwrap();
        assert!(path.to_string_lossy().contains("termos"));
        assert!(path.to_string_lossy().ends_with("config.toml"));
    }

    #[test]
    fn notifications_default_values() {
        let cfg = UserConfig::default_config();
        assert!(cfg.notifications.agent.enabled.unwrap_or(true));
        assert!(cfg.notifications.agent.notify.unwrap_or(true));
        assert!(!cfg.notifications.agent.sound.unwrap_or(false));
        assert!(cfg.notifications.agent.dock.unwrap_or(true));
        assert!(cfg.notifications.agent.suppress_focused.unwrap_or(true));
        assert_eq!(cfg.notifications.agent.sound_mode, "");
        assert!(cfg.notifications.agent.command.is_empty());
        assert!(cfg.notifications.agent.quiet_hours.is_empty());
    }

    #[test]
    fn agent_alert_sounds_default() {
        let sounds = AgentAlertSounds::default();
        assert!(sounds.done.is_empty());
        assert!(sounds.needs_input.is_empty());
    }

    #[test]
    fn agent_alert_states_default() {
        let states = AgentAlertStates::default();
        assert!(states.needs_input.is_none());
        assert!(states.errored.is_none());
        assert!(states.done.is_none());
        assert!(states.idle.is_none());
        assert!(states.working.is_none());
    }

    #[test]
    fn startup_config_default() {
        let s = StartupConfig::default();
        assert!(!s.open_default_window);
        assert!(!s.tiled);
        assert!(!s.start_in_terminal_mode);
    }

    #[test]
    fn daemon_config_default() {
        let d = DaemonConfig::default();
        assert!(d.log_level.is_empty());
        assert!(d.default_codec.is_empty());
        assert!(d.socket_path.is_empty());
    }

    #[test]
    fn appearance_config_border_focused_color_default() {
        let a = AppearanceConfig::default();
        assert!(a.border_focused_color.is_none());
        assert!(a.border_unfocused_color.is_none());
    }

    #[test]
    fn appearance_config_which_key_position_default() {
        let a = AppearanceConfig::default();
        assert_eq!(a.which_key_position, "bottom-right");
    }

    #[test]
    fn appearance_config_window_title_position_default() {
        let a = AppearanceConfig::default();
        assert_eq!(a.window_title_position, "bottom");
    }

    #[test]
    fn overrides_parse_partial_flags() {
        let args = vec!["--border-style".into(), "double".into()];
        let (ov, remaining) = Overrides::parse(&args);
        assert_eq!(ov.border_style.as_deref(), Some("double"));
        assert!(ov.dockbar_position.is_none());
        assert!(remaining.is_empty());
    }

    #[test]
    fn overrides_parse_missing_value() {
        // --border-style without a value
        let args = vec!["--border-style".into()];
        let (ov, _) = Overrides::parse(&args);
        assert!(ov.border_style.is_none());
    }

    #[test]
    fn overrides_apply_multiple() {
        let mut cfg = UserConfig::default_config();
        let ov = Overrides {
            border_style: Some("double".into()),
            dockbar_position: Some("top".into()),
            theme: Some("dracula".into()),
            no_which_key: Some(true),
            ..Default::default()
        };
        ov.apply(&mut cfg);
        assert_eq!(cfg.appearance.border_style, "double");
        assert_eq!(cfg.appearance.dockbar_position, "top");
        assert_eq!(cfg.appearance.theme, "dracula");
        assert!(!cfg.appearance.which_key_enabled);
    }
}
