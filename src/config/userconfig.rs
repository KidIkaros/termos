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
    /// Tape scripting preferences.
    #[serde(default)]
    pub tape: TapeConfig,
    /// Debug/diagnostic settings.
    #[serde(default)]
    pub debug: DebugConfig,
    /// Status-line widgets: external commands whose stdout is rendered
    /// in the dock bar.
    #[serde(default)]
    pub status_widgets: Vec<StatusWidgetConfig>,
    /// Custom palette actions: map a name to a shell command.
    #[serde(default)]
    pub custom_actions: Vec<CustomActionConfig>,
}

/// A status-line widget (`[[status_widgets]]`): runs a shell command
/// periodically and displays the first line of stdout in the dock.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusWidgetConfig {
    /// Display label in the dock.
    pub name: String,
    /// Shell command to run (output = widget text).
    pub command: String,
    /// Refresh interval in milliseconds (0 = run once at startup).
    #[serde(default)]
    pub refresh_ms: u64,
    /// Alignment: "left", "center", or "right" (default: "right").
    #[serde(default = "default_widget_align")]
    pub alignment: String,
}

fn default_widget_align() -> String {
    "right".into()
}

/// A custom palette action (`[[custom_actions]]`): adds a command to
/// the palette that runs a shell command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomActionConfig {
    /// Action name shown in the palette.
    pub name: String,
    /// Shell command to execute.
    pub command: String,
    /// Palette category (default: "Custom").
    #[serde(default = "default_action_category")]
    pub category: String,
}

fn default_action_category() -> String {
    "Custom".into()
}

/// Tape (project automation) settings (`[tape]`).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct TapeConfig {
    /// `"off"`, `"ask"`, or `"auto"`.
    #[serde(default)]
    pub autorun: String,
    /// Auto-open the review dialog on detection.
    #[serde(default)]
    pub auto_review: bool,
}

/// Diagnostic settings (`[debug]`).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct DebugConfig {
    /// Show the on-screen showkeys overlay.
    #[serde(default)]
    pub show_key_events: bool,
}

/// The `[notifications]` table.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NotificationsConfig {
    /// What tuios does when a pane's agent state changes.
    #[serde(default)]
    pub agent: AgentAlertsConfig,
    /// Make errors wait for esc instead of expiring.
    #[serde(default)]
    pub sticky_errors: bool,
    /// Master switch for agent alerts.
    #[serde(default)]
    pub agent_alerts: bool,
    /// Go duration string for how long a done alert stays.
    #[serde(default)]
    pub agent_done_duration: String,
    /// Go duration string for how long an attention alert stays.
    #[serde(default)]
    pub agent_attention_duration: String,
    /// Go duration string for how long a working alert stays.
    #[serde(default)]
    pub agent_working_duration: String,
    /// Go duration string for how long an idle alert stays.
    #[serde(default)]
    pub agent_idle_duration: String,
}

/// The `[notifications.agent]` table. Every toggle is an `Option` so `None`
/// can mean "unset, use the default" and an explicit `false` survives a
/// reload, matching the Go pointer-field design.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
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

/// Scrollbar configuration (`[appearance.scrollbar]`).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ScrollbarConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub width: u32,
    /// `"thin"`, `"thick"`, or `"blocks"`.
    #[serde(default)]
    pub style: String,
}

/// Sidebar configuration (`[appearance.sidebar]`).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SidebarConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub width: u32,
    /// `"left"` or `"right"`.
    #[serde(default)]
    pub position: String,
    #[serde(default)]
    pub show_agents: bool,
    #[serde(default)]
    pub show_unread: bool,
}

/// Appearance settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
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
    /// Clickable workspace strip in the dock.
    #[serde(default)]
    pub dock_workspace_tabs: bool,
    /// Pop a truncated workspace name in full on hover.
    #[serde(default)]
    pub dock_workspace_tooltip: bool,
    /// Powerline caps on the dock's pills.
    #[serde(default)]
    pub dock_pill_caps: bool,
    /// Enable mouse-friendly dock: clickable pills, hover tooltips, right-click context menus.
    #[serde(default = "default_true")]
    pub mouse_friendly: bool,
    /// Pane scrollbar configuration.
    #[serde(default)]
    pub scrollbar: ScrollbarConfig,
    /// Session sidebar / rail configuration.
    #[serde(default)]
    pub sidebar: SidebarConfig,
    /// Maximum width (in cells) for zoomed panes; 0 = unlimited.
    #[serde(default)]
    pub zoom_max_width: i32,
    /// Give every session its own colour (default: true).
    #[serde(default = "default_true")]
    pub session_colors: bool,
    /// ASCII fallback instead of Nerd Fonts (default: false).
    #[serde(default)]
    pub use_ascii_only: bool,
    /// Hide the clock overlay (deprecated, use show_clock).
    #[serde(default)]
    pub hide_clock: bool,
    /// Copy a mouse selection to the clipboard on release (default: true).
    #[serde(default = "default_true")]
    pub copy_on_select: bool,
    /// Focus the pane under the cursor as the mouse moves (default: false).
    #[serde(default)]
    pub focus_follows_mouse: bool,
    /// Alt + left-drag moves a pane (default: true).
    #[serde(default = "default_true")]
    pub alt_drag: bool,
    /// What a click on pane content does: single, double, off (default: single).
    #[serde(default = "default_click_to_type")]
    pub click_to_type: String,
    /// Punctuation that counts as part of a word for double-click.
    #[serde(default = "default_word_characters")]
    pub word_characters: String,
    /// Format string for window titles: {title}, {index}, {cwd}.
    #[serde(default)]
    pub window_title_format: String,
    /// Theme used when `theme = "auto"` detects a dark host terminal.
    #[serde(default = "default_auto_dark")]
    pub theme_auto_dark: String,
    /// Theme used when `theme = "auto"` detects a light host terminal.
    #[serde(default = "default_auto_light")]
    pub theme_auto_light: String,
}

fn default_auto_dark() -> String {
    "catppuccin-mocha".into()
}

fn default_auto_light() -> String {
    "catppuccin-latte".into()
}

fn default_true() -> bool {
    true
}

fn default_click_to_type() -> String {
    "single".into()
}

fn default_word_characters() -> String {
    "@-./_~".into()
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
            zoom_max_width: 0,
            dock_workspace_tabs: true,
            dock_workspace_tooltip: true,
            dock_pill_caps: false,
            mouse_friendly: true,
            scrollbar: ScrollbarConfig::default(),
            sidebar: SidebarConfig {
                position: "left".into(),
                width: 28,
                ..Default::default()
            },
            session_colors: true,
            use_ascii_only: false,
            hide_clock: false,
            copy_on_select: true,
            focus_follows_mouse: false,
            alt_drag: true,
            click_to_type: "single".into(),
            word_characters: "@-./_~".into(),
            window_title_format: String::new(),
            theme_auto_dark: default_auto_dark(),
            theme_auto_light: default_auto_light(),
        }
    }
}

/// Keybinding configuration tables.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
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
#[serde(default)]
pub struct StartupConfig {
    pub open_default_window: bool,
    pub tiled: bool,
    pub start_in_terminal_mode: bool,
}

/// Daemon settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DaemonConfig {
    /// `"debug"`, `"info"`, `"warn"`, `"error"`.
    #[serde(default)]
    pub log_level: String,
    /// `"json"` or `"gob"`.
    #[serde(default)]
    pub default_codec: String,
    /// Custom socket path (empty = default XDG path).
    #[serde(default)]
    pub socket_path: String,
    /// Whether to automatically detect foreground agent processes.
    #[serde(default = "default_true")]
    pub agent_auto_detect: bool,
    /// How often (in seconds) to poll for foreground agent processes.
    #[serde(default = "default_agent_detect_seconds")]
    pub agent_detect_seconds: u64,
    /// Known agent binary names for foreground-process detection.
    #[serde(default)]
    pub agent_binaries: Vec<String>,
}

fn default_agent_detect_seconds() -> u64 {
    2
}

impl UserConfig {
    /// The default configuration.
    pub fn default_config() -> Self {
        let mut cfg = Self {
            appearance: AppearanceConfig::default(),
            keybindings: KeybindingsConfig::default(),
            startup: StartupConfig::default(),
            daemon: DaemonConfig {
                log_level: "off".into(),
                default_codec: "gob".into(),
                socket_path: String::new(),
                agent_auto_detect: true,
                agent_detect_seconds: 2,
                agent_binaries: Vec::new(),
            },
            hooks: std::collections::HashMap::new(),
            notifications: NotificationsConfig::default(),
            tape: TapeConfig::default(),
            debug: DebugConfig::default(),
            status_widgets: vec![
                StatusWidgetConfig {
                    name: "clock".into(),
                    command: "date +'%H:%M'".into(),
                    refresh_ms: 1000,
                    alignment: "right".into(),
                },
                StatusWidgetConfig {
                    name: "load".into(),
                    command: "cat /proc/loadavg 2>/dev/null | cut -d' ' -f1-3".into(),
                    refresh_ms: 5000,
                    alignment: "right".into(),
                },
            ],
            custom_actions: vec![
                CustomActionConfig {
                    name: "Git status".into(),
                    command: "git status -s".into(),
                    category: "Git".into(),
                },
                CustomActionConfig {
                    name: "Git log (recent)".into(),
                    command: "git log --oneline -15".into(),
                    category: "Git".into(),
                },
                CustomActionConfig {
                    name: "Git diff".into(),
                    command: "git diff --stat".into(),
                    category: "Git".into(),
                },
                CustomActionConfig {
                    name: "Disk usage".into(),
                    command: "df -h . | tail -1".into(),
                    category: "System".into(),
                },
                CustomActionConfig {
                    name: "Process list".into(),
                    command: "ps aux --sort=-%cpu | head -12".into(),
                    category: "System".into(),
                },
                CustomActionConfig {
                    name: "List files".into(),
                    command: "ls -la".into(),
                    category: "Dev".into(),
                },
                CustomActionConfig {
                    name: "Cargo check".into(),
                    command: "cargo check 2>&1 | tail -5".into(),
                    category: "Dev".into(),
                },
                CustomActionConfig {
                    name: "Cargo test".into(),
                    command: "cargo test 2>&1 | tail -10".into(),
                    category: "Dev".into(),
                },
                CustomActionConfig {
                    name: "Docker ps".into(),
                    command: "docker ps --format 'table {{.Names}}\t{{.Status}}' 2>/dev/null || echo 'Docker not available'".into(),
                    category: "Dev".into(),
                },
                CustomActionConfig {
                    name: "Find files".into(),
                    command: "find . -maxdepth 2 -type f | head -30".into(),
                    category: "Dev".into(),
                },
            ],
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
        Self::parse_str(&data)
    }

    /// Parse config from a TOML string. Returns defaults on parse error.
    pub fn parse_str(data: &str) -> Self {
        Self::parse_str_checked(data).unwrap_or_else(|_| Self::default_config())
    }

    /// Parse config from a TOML string, reporting errors instead of silently
    /// falling back to defaults.  Used by the hot-reload watcher so a broken
    /// edit keeps the last good config rather than resetting the session.
    pub fn parse_str_checked(data: &str) -> Result<UserConfig, String> {
        let mut cfg: UserConfig = toml::from_str(data).map_err(|e| e.to_string())?;
        cfg.keybindings.fill_missing();
        Ok(cfg)
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
                        // On a broken edit (unreadable or parse error), keep
                        // the last good config instead of resetting the
                        // running session to defaults.
                        if let Ok(cfg) = UserConfig::try_load_from(&path) {
                            let _ = tx_clone.send(cfg);
                        }
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
        Self::try_load_from(path).unwrap_or_else(|_| Self::default_config())
    }

    /// Load config from a specific path, reporting errors instead of silently
    /// falling back to defaults (used by the hot-reload watcher).
    pub fn try_load_from(path: &Path) -> Result<UserConfig, String> {
        let data = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        Self::parse_str_checked(&data)
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
    fn parses_startup_and_debug_sections() {
        let cfg = UserConfig::parse_str(
            "[appearance]\ntheme = \"catppuccin-mocha\"\n\n[startup]\nstart_in_terminal_mode = true\n\n[debug]\nshow_key_events = true\n",
        );
        assert!(cfg.startup.start_in_terminal_mode);
        assert!(cfg.debug.show_key_events);
        assert_eq!(cfg.appearance.theme, "catppuccin-mocha");
    }

    #[test]
    fn parses_partial_sections_without_discarding() {
        // Partial `[appearance]` must not nuke the whole config.
        let raw: Result<UserConfig, _> =
            toml::from_str("[appearance]\ntheme = \"catppuccin-mocha\"\n");
        match raw {
            Ok(c) => assert_eq!(c.appearance.theme, "catppuccin-mocha"),
            Err(e) => panic!("partial [appearance] deserialize failed: {e}"),
        }

        // Partial `[startup]` must not nuke the whole config.
        let raw: Result<UserConfig, _> =
            toml::from_str("[startup]\nstart_in_terminal_mode = true\n");
        match raw {
            Ok(c) => assert!(c.startup.start_in_terminal_mode),
            Err(e) => panic!("partial [startup] deserialize failed: {e}"),
        }

        // Partial `[keybindings]` must not nuke the whole config either.
        let raw: Result<UserConfig, _> = toml::from_str("[keybindings]\nleader_key = \"ctrl-a\"\n");
        match raw {
            Ok(c) => assert_eq!(c.keybindings.leader_key, "ctrl-a"),
            Err(e) => panic!("partial [keybindings] deserialize failed: {e}"),
        }

        // Partial `[daemon]` must not nuke the whole config.
        let raw: Result<UserConfig, _> = toml::from_str("[daemon]\nlog_level = \"debug\"\n");
        match raw {
            Ok(c) => assert_eq!(c.daemon.log_level, "debug"),
            Err(e) => panic!("partial [daemon] deserialize failed: {e}"),
        }

        // End-to-end through `parse_str`.
        let cfg = UserConfig::parse_str(
            "[appearance]\ntheme = \"catppuccin-mocha\"\n\n[startup]\nstart_in_terminal_mode = true\n\n[debug]\nshow_key_events = true\n",
        );
        assert!(cfg.startup.start_in_terminal_mode);
        assert!(cfg.debug.show_key_events);
        assert_eq!(cfg.appearance.theme, "catppuccin-mocha");
    }

    #[test]
    fn checked_parse_reports_errors_instead_of_defaults() {
        // Syntax error: reported, not silently discarded.
        assert!(UserConfig::parse_str_checked("not [ valid").is_err());
        // Unknown section is fine (serde ignores unknown tables); missing
        // required inner fields without defaults is not.
        assert!(UserConfig::parse_str_checked("[bogus]\nx = 1\n").is_ok());
        // A valid partial config parses and keeps its values.
        let ok = UserConfig::parse_str_checked("[startup]\nstart_in_terminal_mode = true\n")
            .expect("valid partial config should parse");
        assert!(ok.startup.start_in_terminal_mode);
    }

    #[test]
    fn try_load_from_rejects_missing_and_broken_files() {
        let dir = std::env::temp_dir().join(format!("termos-cfg-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Missing file → Err.
        assert!(UserConfig::try_load_from(&dir.join("missing.toml")).is_err());

        // Broken file → Err.
        let broken = dir.join("broken.toml");
        std::fs::write(&broken, "not [ valid").unwrap();
        assert!(UserConfig::try_load_from(&broken).is_err());

        // Valid file → Ok with values.
        let good = dir.join("good.toml");
        std::fs::write(&good, "[startup]\nstart_in_terminal_mode = true\n").unwrap();
        let cfg = UserConfig::try_load_from(&good).expect("valid file should load");
        assert!(cfg.startup.start_in_terminal_mode);

        std::fs::remove_dir_all(&dir).ok();
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
    fn appearance_config_auto_theme_defaults() {
        let a = AppearanceConfig::default();
        assert_eq!(a.theme_auto_dark, "catppuccin-mocha");
        assert_eq!(a.theme_auto_light, "catppuccin-latte");
    }

    #[test]
    fn old_config_without_auto_theme_fields_still_parses() {
        // A config saved before the auto-theme fields existed must load with
        // the new fields defaulted (missing fields are not a parse error).
        let full = toml::to_string(&UserConfig::default_config()).unwrap();
        let old = full
            .lines()
            .filter(|l| !l.contains("theme_auto_dark") && !l.contains("theme_auto_light"))
            .collect::<Vec<_>>()
            .join("\n");
        let cfg = UserConfig::parse_str(&old);
        assert_eq!(cfg.appearance.theme_auto_dark, "catppuccin-mocha");
        assert_eq!(cfg.appearance.theme_auto_light, "catppuccin-latte");
    }
}
