//! CLI flag overrides — ported from Go TUIOS `internal/config/overrides.go`.
//!
//! Provides `apply_overrides` to layer CLI flag values on top of a loaded
//! `UserConfig`. The `Overrides` struct uses `Option` fields so `None` means
//! "flag not set, keep config value".

use super::userconfig::UserConfig;

/// CLI flag overrides that take precedence over config file values.
/// `None` / zero values indicate the flag was not set.
#[derive(Debug, Default, Clone)]
pub struct Overrides {
    /// Use ASCII characters instead of Nerd Font icons.
    pub ascii_only: Option<bool>,
    /// Override the window border style.
    pub border_style: Option<String>,
    /// Override the dockbar position.
    pub dockbar_position: Option<String>,
    /// Override hiding window control buttons.
    pub hide_window_buttons: Option<bool>,
    /// Override hiding the scrollbar thumb.
    pub hide_scrollbar: Option<bool>,
    /// Override the window title position.
    pub window_title_position: Option<String>,
    /// Hide the clock overlay (deprecated, use show_clock).
    pub hide_clock: Option<bool>,
    /// Enable the clock overlay.
    pub show_clock: Option<bool>,
    /// Enable the CPU graph in the dock.
    pub show_cpu: Option<bool>,
    /// Enable the RAM usage in the dock.
    pub show_ram: Option<bool>,
    /// Enable shared borders between tiled windows.
    pub shared_borders: Option<bool>,
    /// Override the scrollback buffer size (0 means use default).
    pub scrollback_lines: Option<i32>,
    /// Disable UI animations.
    pub no_animations: Option<bool>,
    /// Always show quit confirmation dialog.
    pub confirm_quit: Option<bool>,
    /// Theme name to load.
    pub theme: Option<String>,
    /// Cap the zoom mode width (0 = fullscreen).
    pub zoom_max_width: Option<i32>,
    /// Disable the which-key popup.
    pub no_which_key: Option<bool>,
    /// --list-themes: list available themes and exit (handled by the CLI).
    pub list_themes: Option<bool>,
    /// --preview-theme <name>: preview a theme's colors and exit.
    pub preview_theme: Option<String>,
    /// --debug: enable debug logging (persistent global flag).
    pub debug: Option<bool>,
    /// --log-level <level>: set the log level (off, error, warn, info, debug, trace).
    pub log_level: Option<String>,
    /// --show-keys: enable the showkeys overlay to display pressed keys.
    pub show_keys: Option<bool>,
}

impl Overrides {
    /// Parse CLI flags from args. Returns `(overrides, remaining_args)`.
    ///
    /// Recognized flags:
    /// - `--border-style <style>`
    /// - `--dockbar-position <top|bottom>`
    /// - `--ascii-only`
    /// - `--theme <name>`
    /// - `--no-which-key`
    /// - `--no-animations`
    /// - `--hide-window-buttons`
    /// - `--hide-scrollbar`
    /// - `--hide-clock`
    /// - `--show-clock`
    /// - `--show-cpu`
    /// - `--show-ram`
    /// - `--shared-borders`
    /// - `--confirm-quit`
    /// - `--window-title-position <pos>`
    /// - `--scrollback-lines <n>`
    /// - `--zoom-max-width <n>`
    /// - `--list-themes`
    /// - `--preview-theme <name>`
    /// - `--debug`
    /// - `--log-level <level>`
    /// - `--show-keys`
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
                "--no-animations" => {
                    ov.no_animations = Some(true);
                }
                "--hide-window-buttons" => {
                    ov.hide_window_buttons = Some(true);
                }
                "--hide-scrollbar" => {
                    ov.hide_scrollbar = Some(true);
                }
                "--hide-clock" => {
                    ov.hide_clock = Some(true);
                }
                "--show-clock" => {
                    ov.show_clock = Some(true);
                }
                "--show-cpu" => {
                    ov.show_cpu = Some(true);
                }
                "--show-ram" => {
                    ov.show_ram = Some(true);
                }
                "--shared-borders" => {
                    ov.shared_borders = Some(true);
                }
                "--confirm-quit" => {
                    ov.confirm_quit = Some(true);
                }
                "--window-title-position" => {
                    i += 1;
                    if let Some(v) = args.get(i) {
                        ov.window_title_position = Some(v.clone());
                    }
                }
                "--scrollback-lines" => {
                    i += 1;
                    if let Some(v) = args.get(i) {
                        if let Ok(n) = v.parse::<i32>() {
                            ov.scrollback_lines = Some(n);
                        }
                    }
                }
                "--zoom-max-width" => {
                    i += 1;
                    if let Some(v) = args.get(i) {
                        if let Ok(n) = v.parse::<i32>() {
                            ov.zoom_max_width = Some(n);
                        }
                    }
                }
                "--list-themes" => {
                    ov.list_themes = Some(true);
                }
                "--preview-theme" => {
                    i += 1;
                    if let Some(v) = args.get(i) {
                        ov.preview_theme = Some(v.clone());
                    }
                }
                "--debug" => {
                    ov.debug = Some(true);
                }
                "--log-level" => {
                    i += 1;
                    if let Some(v) = args.get(i) {
                        ov.log_level = Some(v.clone());
                    }
                }
                "--show-keys" => {
                    ov.show_keys = Some(true);
                }
                _ => {
                    remaining.push(args[i].clone());
                }
            }
            i += 1;
        }
        (ov, remaining)
    }

    /// Apply overrides to a loaded config. CLI flags take precedence over
    /// config file values. Boolean flags are OR'd with the config value.
    pub fn apply(&self, config: &mut UserConfig) {
        apply_overrides(config, self);
    }
}

/// Apply CLI flag overrides to a loaded `UserConfig`.
///
/// CLI flags take precedence over config file values. Boolean flags are OR'd
/// with the config value where appropriate (e.g. `hide_window_buttons`).
pub fn apply_overrides(config: &mut UserConfig, overrides: &Overrides) {
    // ASCII Only — simple flag override.
    if let Some(true) = overrides.ascii_only {
        config.appearance.border_style = "plain".into();
        config.appearance.animations_enabled = false;
    }

    // Border Style — CLI flag takes precedence.
    if let Some(ref bs) = overrides.border_style {
        config.appearance.border_style = bs.clone();
    }

    // Dockbar Position — CLI flag takes precedence.
    if let Some(ref dp) = overrides.dockbar_position {
        config.appearance.dockbar_position = dp.clone();
    }

    // Hide Window Buttons — OR of CLI flag and config.
    if let Some(true) = overrides.hide_window_buttons {
        config.appearance.hide_window_buttons = true;
    }

    // Hide Scrollbar — OR of CLI flag and config.
    if let Some(true) = overrides.hide_scrollbar {
        config.appearance.hide_scrollbar = true;
    }

    // Window Title Position — CLI flag takes precedence.
    if let Some(ref wtp) = overrides.window_title_position {
        config.appearance.window_title_position = wtp.clone();
    }

    // Show Clock — OR of CLI flag and config.
    if let Some(true) = overrides.show_clock {
        config.appearance.show_clock = true;
    }

    // Show CPU — OR of CLI flag and config.
    if let Some(true) = overrides.show_cpu {
        config.appearance.show_cpu = true;
    }

    // Show RAM — OR of CLI flag and config.
    if let Some(true) = overrides.show_ram {
        config.appearance.show_ram = true;
    }

    // Shared Borders — CLI flag OR config.
    if let Some(true) = overrides.shared_borders {
        config.appearance.shared_borders = true;
    }

    // Scrollback Lines — CLI flag takes precedence (clamped to valid range).
    if let Some(lines) = overrides.scrollback_lines {
        if lines > 0 {
            let clamped = lines.clamp(100, 1_000_000);
            config.appearance.scrollback_lines = clamped;
        }
    }

    // No Animations — disables animations.
    if let Some(true) = overrides.no_animations {
        config.appearance.animations_enabled = false;
    }

    // Confirm Quit — always show quit dialog.
    if let Some(true) = overrides.confirm_quit {
        config.appearance.confirm_quit = true;
    }

    // Theme — CLI flag takes precedence.
    if let Some(ref theme) = overrides.theme {
        config.appearance.theme = theme.clone();
    }

    // No Which Key — disables which-key popup.
    if let Some(true) = overrides.no_which_key {
        config.appearance.which_key_enabled = false;
    }

    // Hide Clock — deprecated, hides the clock overlay.
    if let Some(true) = overrides.hide_clock {
        config.appearance.show_clock = false;
    }

    // Zoom Max Width — CLI flag takes precedence.
    if let Some(zmw) = overrides.zoom_max_width {
        if zmw > 0 {
            config.appearance.zoom_max_width = zmw;
        }
    }

    // Show Keys — enable the showkeys overlay.
    if let Some(true) = overrides.show_keys {
        config.debug.show_key_events = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_border_style() {
        let args = vec!["--border-style".into(), "double".into()];
        let (ov, remaining) = Overrides::parse(&args);
        assert_eq!(ov.border_style.as_deref(), Some("double"));
        assert!(remaining.is_empty());
    }

    #[test]
    fn parse_ascii_only() {
        let args = vec!["--ascii-only".into()];
        let (ov, _) = Overrides::parse(&args);
        assert_eq!(ov.ascii_only, Some(true));
    }

    #[test]
    fn parse_no_animations() {
        let args = vec!["--no-animations".into()];
        let (ov, _) = Overrides::parse(&args);
        assert_eq!(ov.no_animations, Some(true));
    }

    #[test]
    fn parse_unknown_flags_pass_through() {
        let args = vec!["--unknown".into(), "value".into()];
        let (ov, remaining) = Overrides::parse(&args);
        assert!(ov.border_style.is_none());
        assert_eq!(remaining, vec!["--unknown", "value"]);
    }

    #[test]
    fn parse_missing_value() {
        let args = vec!["--border-style".into()];
        let (ov, _) = Overrides::parse(&args);
        assert!(ov.border_style.is_none());
    }

    #[test]
    fn apply_border_style() {
        let mut cfg = UserConfig::default_config();
        let ov = Overrides {
            border_style: Some("double".into()),
            ..Default::default()
        };
        apply_overrides(&mut cfg, &ov);
        assert_eq!(cfg.appearance.border_style, "double");
    }

    #[test]
    fn apply_ascii_only() {
        let mut cfg = UserConfig::default_config();
        let ov = Overrides {
            ascii_only: Some(true),
            ..Default::default()
        };
        apply_overrides(&mut cfg, &ov);
        assert_eq!(cfg.appearance.border_style, "plain");
        assert!(!cfg.appearance.animations_enabled);
    }

    #[test]
    fn apply_no_animations() {
        let mut cfg = UserConfig::default_config();
        let ov = Overrides {
            no_animations: Some(true),
            ..Default::default()
        };
        apply_overrides(&mut cfg, &ov);
        assert!(!cfg.appearance.animations_enabled);
    }

    #[test]
    fn apply_show_clock() {
        let mut cfg = UserConfig::default_config();
        assert!(!cfg.appearance.show_clock);
        let ov = Overrides {
            show_clock: Some(true),
            ..Default::default()
        };
        apply_overrides(&mut cfg, &ov);
        assert!(cfg.appearance.show_clock);
    }

    #[test]
    fn apply_scrollback_lines_clamped() {
        let mut cfg = UserConfig::default_config();
        let ov = Overrides {
            scrollback_lines: Some(50),
            ..Default::default()
        };
        apply_overrides(&mut cfg, &ov);
        assert_eq!(cfg.appearance.scrollback_lines, 100);

        let ov2 = Overrides {
            scrollback_lines: Some(2_000_000),
            ..Default::default()
        };
        apply_overrides(&mut cfg, &ov2);
        assert_eq!(cfg.appearance.scrollback_lines, 1_000_000);
    }

    #[test]
    fn apply_noop_when_none() {
        let mut cfg = UserConfig::default_config();
        let original_border = cfg.appearance.border_style.clone();
        let ov = Overrides::default();
        apply_overrides(&mut cfg, &ov);
        assert_eq!(cfg.appearance.border_style, original_border);
    }

    #[test]
    fn apply_multiple_overrides() {
        let mut cfg = UserConfig::default_config();
        let ov = Overrides {
            border_style: Some("double".into()),
            dockbar_position: Some("top".into()),
            theme: Some("dracula".into()),
            no_which_key: Some(true),
            ..Default::default()
        };
        apply_overrides(&mut cfg, &ov);
        assert_eq!(cfg.appearance.border_style, "double");
        assert_eq!(cfg.appearance.dockbar_position, "top");
        assert_eq!(cfg.appearance.theme, "dracula");
        assert!(!cfg.appearance.which_key_enabled);
    }

    #[test]
    fn apply_method_delegates_to_function() {
        let mut cfg = UserConfig::default_config();
        let ov = Overrides {
            border_style: Some("double".into()),
            ..Default::default()
        };
        ov.apply(&mut cfg);
        assert_eq!(cfg.appearance.border_style, "double");
    }

    #[test]
    fn parse_all_flags() {
        let args = vec![
            "--border-style".into(),
            "double".into(),
            "--dockbar-position".into(),
            "top".into(),
            "--ascii-only".into(),
            "--theme".into(),
            "dracula".into(),
            "--no-which-key".into(),
            "--no-animations".into(),
            "--hide-window-buttons".into(),
            "--hide-scrollbar".into(),
            "--show-clock".into(),
            "--show-cpu".into(),
            "--show-ram".into(),
            "--shared-borders".into(),
            "--confirm-quit".into(),
            "--window-title-position".into(),
            "top".into(),
            "--scrollback-lines".into(),
            "5000".into(),
            "--zoom-max-width".into(),
            "120".into(),
            "--debug".into(),
            "--log-level".into(),
            "debug".into(),
            "--show-keys".into(),
            "remaining".into(),
        ];
        let (ov, remaining) = Overrides::parse(&args);
        assert_eq!(ov.border_style.as_deref(), Some("double"));
        assert_eq!(ov.dockbar_position.as_deref(), Some("top"));
        assert_eq!(ov.ascii_only, Some(true));
        assert_eq!(ov.theme.as_deref(), Some("dracula"));
        assert_eq!(ov.no_which_key, Some(true));
        assert_eq!(ov.no_animations, Some(true));
        assert_eq!(ov.hide_window_buttons, Some(true));
        assert_eq!(ov.hide_scrollbar, Some(true));
        assert_eq!(ov.show_clock, Some(true));
        assert_eq!(ov.show_cpu, Some(true));
        assert_eq!(ov.show_ram, Some(true));
        assert_eq!(ov.shared_borders, Some(true));
        assert_eq!(ov.confirm_quit, Some(true));
        assert_eq!(ov.window_title_position.as_deref(), Some("top"));
        assert_eq!(ov.scrollback_lines, Some(5000));
        assert_eq!(ov.zoom_max_width, Some(120));
        assert_eq!(ov.debug, Some(true));
        assert_eq!(ov.log_level.as_deref(), Some("debug"));
        assert_eq!(ov.show_keys, Some(true));
        assert_eq!(remaining, vec!["remaining"]);
    }

    #[test]
    fn parse_debug_flag() {
        let args = vec!["--debug".into()];
        let (ov, remaining) = Overrides::parse(&args);
        assert_eq!(ov.debug, Some(true));
        assert!(remaining.is_empty());
    }

    #[test]
    fn parse_log_level_flag() {
        let args = vec!["--log-level".into(), "trace".into()];
        let (ov, remaining) = Overrides::parse(&args);
        assert_eq!(ov.log_level.as_deref(), Some("trace"));
        assert!(remaining.is_empty());
    }

    #[test]
    fn parse_show_keys_flag() {
        let args = vec!["--show-keys".into()];
        let (ov, remaining) = Overrides::parse(&args);
        assert_eq!(ov.show_keys, Some(true));
        assert!(remaining.is_empty());
    }

    #[test]
    fn show_keys_override_enables_debug_flag() {
        let mut cfg = crate::config::UserConfig::default();
        assert!(!cfg.debug.show_key_events);
        let (ov, _) = Overrides::parse(&["--show-keys".into()]);
        ov.apply(&mut cfg);
        assert!(cfg.debug.show_key_events);
    }

    #[test]
    fn parse_log_level_missing_value() {
        let args = vec!["--log-level".into()];
        let (ov, _) = Overrides::parse(&args);
        assert!(ov.log_level.is_none());
    }
}
