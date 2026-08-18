//! Config schema validation — ported from Go TUIOS `internal/config/validation.go`.

use super::userconfig::UserConfig;

/// Severity of a validation finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// A single validation error or warning.
#[derive(Debug, Clone)]
pub struct ValidationError {
    pub field: String,
    pub key: String,
    pub message: String,
    pub severity: Severity,
}

/// The result of validating a config.
#[derive(Debug, Clone, Default)]
pub struct ValidationResult {
    pub errors: Vec<ValidationError>,
    pub warnings: Vec<ValidationError>,
}

impl ValidationResult {
    /// True if there are no errors.
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }

    /// True if there are any warnings.
    pub fn has_warnings(&self) -> bool {
        !self.warnings.is_empty()
    }
}

/// Validate all config sections.
pub fn validate_config(config: &UserConfig) -> ValidationResult {
    let mut result = ValidationResult::default();

    validate_appearance(config, &mut result);
    validate_daemon(config, &mut result);
    validate_startup(config, &mut result);
    validate_notifications(config, &mut result);
    validate_tape(config, &mut result);
    validate_debug(config, &mut result);

    result
}

fn validate_appearance(config: &UserConfig, result: &mut ValidationResult) {
    let valid_border_styles = ["rounded", "single", "double", "plain", "ascii"];
    if !valid_border_styles.contains(&config.appearance.border_style.as_str()) {
        result.errors.push(ValidationError {
            field: "appearance".into(),
            key: "border_style".into(),
            message: format!(
                "border_style must be one of {:?}, got \"{}\"",
                valid_border_styles, config.appearance.border_style
            ),
            severity: Severity::Error,
        });
    }

    let valid_dockbar = ["top", "bottom", "left", "right", "hidden"];
    if !valid_dockbar.contains(&config.appearance.dockbar_position.as_str()) {
        result.errors.push(ValidationError {
            field: "appearance".into(),
            key: "dockbar_position".into(),
            message: format!(
                "dockbar_position must be one of {:?}, got \"{}\"",
                valid_dockbar, config.appearance.dockbar_position
            ),
            severity: Severity::Error,
        });
    }

    if config.appearance.scrollback_lines <= 0 {
        result.errors.push(ValidationError {
            field: "appearance".into(),
            key: "scrollback_lines".into(),
            message: "scrollback_lines must be > 0".into(),
            severity: Severity::Error,
        });
    }

    // Validate window_title_position.
    let valid_title_positions = ["top", "bottom", "hidden"];
    if !valid_title_positions.contains(&config.appearance.window_title_position.as_str()) {
        result.errors.push(ValidationError {
            field: "appearance".into(),
            key: "window_title_position".into(),
            message: format!(
                "window_title_position must be one of {:?}, got \"{}\"",
                valid_title_positions, config.appearance.window_title_position
            ),
            severity: Severity::Error,
        });
    }

    // Validate which_key_position.
    let valid_whichkey = [
        "bottom-right",
        "bottom-left",
        "top-right",
        "top-left",
        "center",
    ];
    if !valid_whichkey.contains(&config.appearance.which_key_position.as_str()) {
        result.errors.push(ValidationError {
            field: "appearance".into(),
            key: "which_key_position".into(),
            message: format!(
                "which_key_position must be one of {:?}, got \"{}\"",
                valid_whichkey, config.appearance.which_key_position
            ),
            severity: Severity::Error,
        });
    }

    // Validate max_fps.
    if config.appearance.max_fps <= 0 || config.appearance.max_fps > 240 {
        result.errors.push(ValidationError {
            field: "appearance".into(),
            key: "max_fps".into(),
            message: "max_fps must be between 1 and 240".into(),
            severity: Severity::Error,
        });
    }

    // Validate scroll_lines.
    if config.appearance.scroll_lines <= 0 || config.appearance.scroll_lines > 50 {
        result.errors.push(ValidationError {
            field: "appearance".into(),
            key: "scroll_lines".into(),
            message: "scroll_lines must be between 1 and 50".into(),
            severity: Severity::Error,
        });
    }

    // Validate sidebar position.
    if !config.appearance.sidebar.position.is_empty() {
        let valid_sidebar_positions = ["left", "right", "hidden"];
        if !valid_sidebar_positions.contains(&config.appearance.sidebar.position.as_str()) {
            result.errors.push(ValidationError {
                field: "appearance.sidebar".into(),
                key: "position".into(),
                message: format!(
                    "sidebar.position must be one of {:?}, got \"{}\"",
                    valid_sidebar_positions, config.appearance.sidebar.position
                ),
                severity: Severity::Error,
            });
        }
    }

    // Validate scrollbar style.
    if !config.appearance.scrollbar.style.is_empty() {
        let valid_scrollbar_styles = ["thin", "thick", "blocks", "track"];
        if !valid_scrollbar_styles.contains(&config.appearance.scrollbar.style.as_str()) {
            result.errors.push(ValidationError {
                field: "appearance.scrollbar".into(),
                key: "style".into(),
                message: format!(
                    "scrollbar.style must be one of {:?}, got \"{}\"",
                    valid_scrollbar_styles, config.appearance.scrollbar.style
                ),
                severity: Severity::Error,
            });
        }
    }
}

fn validate_daemon(config: &UserConfig, result: &mut ValidationResult) {
    if config.daemon.log_level.is_empty() {
        result.warnings.push(ValidationError {
            field: "daemon".into(),
            key: "log_level".into(),
            message: "log_level is empty, using default".into(),
            severity: Severity::Warning,
        });
    }

    // Validate log_level value.
    let valid_log_levels = ["off", "debug", "info", "warn", "error"];
    if !config.daemon.log_level.is_empty()
        && !valid_log_levels.contains(&config.daemon.log_level.as_str())
    {
        result.errors.push(ValidationError {
            field: "daemon".into(),
            key: "log_level".into(),
            message: format!(
                "log_level must be one of {:?}, got \"{}\"",
                valid_log_levels, config.daemon.log_level
            ),
            severity: Severity::Error,
        });
    }

    // Validate default_codec.
    let valid_codecs = ["json", "gob"];
    if !config.daemon.default_codec.is_empty()
        && !valid_codecs.contains(&config.daemon.default_codec.as_str())
    {
        result.errors.push(ValidationError {
            field: "daemon".into(),
            key: "default_codec".into(),
            message: format!(
                "default_codec must be one of {:?}, got \"{}\"",
                valid_codecs, config.daemon.default_codec
            ),
            severity: Severity::Error,
        });
    }
}

fn validate_startup(config: &UserConfig, result: &mut ValidationResult) {
    // StartupConfig has boolean fields only; no numeric validation needed.
    if !config.startup.tiled && !config.startup.open_default_window {
        result.warnings.push(ValidationError {
            field: "startup".into(),
            key: "open_default_window".into(),
            message: "open_default_window is false and tiled is false; session will start empty"
                .into(),
            severity: Severity::Warning,
        });
    }
}

fn validate_notifications(config: &UserConfig, result: &mut ValidationResult) {
    let valid_sound_modes = ["audio", "bell", "both"];
    if !config.notifications.agent.sound_mode.is_empty()
        && !valid_sound_modes.contains(&config.notifications.agent.sound_mode.as_str())
    {
        result.errors.push(ValidationError {
            field: "notifications.agent".into(),
            key: "sound_mode".into(),
            message: format!(
                "sound_mode must be one of {:?}, got \"{}\"",
                valid_sound_modes, config.notifications.agent.sound_mode
            ),
            severity: Severity::Error,
        });
    }
}

/// Validate the `[tape]` section.
fn validate_tape(config: &UserConfig, result: &mut ValidationResult) {
    let valid_autorun = ["off", "ask", "auto"];
    if !config.tape.autorun.is_empty()
        && !valid_autorun.contains(&config.tape.autorun.as_str())
    {
        result.warnings.push(ValidationError {
            field: "tape".into(),
            key: "autorun".into(),
            message: format!(
                "'{}' is not a valid value (allowed: {:?}); falling back to default",
                config.tape.autorun, valid_autorun
            ),
            severity: Severity::Warning,
        });
    }
}

/// Validate the `[debug]` section.
fn validate_debug(_config: &UserConfig, _result: &mut ValidationResult) {
    // DebugConfig currently has only a single boolean field; no validation
    // is needed. This function exists for future expansion.
}

/// Format a validation result as a human-readable report.
pub fn format_errors(result: &ValidationResult) -> String {
    let mut out = String::new();
    if !result.errors.is_empty() {
        out.push_str("Configuration errors:\n");
        for e in &result.errors {
            out.push_str(&format!("  [{}] {}: {}\n", e.field, e.key, e.message));
        }
    }
    if !result.warnings.is_empty() {
        out.push_str("Configuration warnings:\n");
        for w in &result.warnings {
            out.push_str(&format!("  [{}] {}: {}\n", w.field, w.key, w.message));
        }
    }
    if out.is_empty() {
        out.push_str("Configuration is valid.\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_config_passes() {
        let config = UserConfig::default_config();
        let result = validate_config(&config);
        assert!(result.is_valid(), "errors: {:?}", result.errors);
    }

    #[test]
    fn invalid_border_style() {
        let mut config = UserConfig::default_config();
        config.appearance.border_style = "invalid".into();
        let result = validate_config(&config);
        assert!(!result.is_valid());
        assert!(result.errors.iter().any(|e| e.key == "border_style"));
    }

    #[test]
    fn invalid_dockbar_position() {
        let mut config = UserConfig::default_config();
        config.appearance.dockbar_position = "sideways".into();
        let result = validate_config(&config);
        assert!(!result.is_valid());
        assert!(result.errors.iter().any(|e| e.key == "dockbar_position"));
    }

    #[test]
    fn invalid_scrollback_lines() {
        let mut config = UserConfig::default_config();
        config.appearance.scrollback_lines = 0;
        let result = validate_config(&config);
        assert!(!result.is_valid());
        assert!(result.errors.iter().any(|e| e.key == "scrollback_lines"));
    }

    #[test]
    fn invalid_sound_mode() {
        let mut config = UserConfig::default_config();
        config.notifications.agent.sound_mode = "invalid".into();
        let result = validate_config(&config);
        assert!(!result.is_valid());
        assert!(result.errors.iter().any(|e| e.key == "sound_mode"));
    }

    #[test]
    fn format_errors_valid() {
        let result = ValidationResult::default();
        let out = format_errors(&result);
        assert!(out.contains("valid"));
    }

    #[test]
    fn format_errors_with_errors() {
        let mut result = ValidationResult::default();
        result.errors.push(ValidationError {
            field: "test".into(),
            key: "foo".into(),
            message: "bad value".into(),
            severity: Severity::Error,
        });
        let out = format_errors(&result);
        assert!(out.contains("errors"));
        assert!(out.contains("bad value"));
    }

    #[test]
    fn invalid_window_title_position() {
        let mut config = UserConfig::default_config();
        config.appearance.window_title_position = "sideways".into();
        let result = validate_config(&config);
        assert!(!result.is_valid());
        assert!(result.errors.iter().any(|e| e.key == "window_title_position"));
    }

    #[test]
    fn invalid_which_key_position() {
        let mut config = UserConfig::default_config();
        config.appearance.which_key_position = "middle".into();
        let result = validate_config(&config);
        assert!(!result.is_valid());
        assert!(result.errors.iter().any(|e| e.key == "which_key_position"));
    }

    #[test]
    fn invalid_max_fps() {
        let mut config = UserConfig::default_config();
        config.appearance.max_fps = 0;
        let result = validate_config(&config);
        assert!(!result.is_valid());
        assert!(result.errors.iter().any(|e| e.key == "max_fps"));
    }

    #[test]
    fn invalid_scroll_lines() {
        let mut config = UserConfig::default_config();
        config.appearance.scroll_lines = 0;
        let result = validate_config(&config);
        assert!(!result.is_valid());
        assert!(result.errors.iter().any(|e| e.key == "scroll_lines"));
    }

    #[test]
    fn invalid_tape_autorun_warns() {
        let mut config = UserConfig::default_config();
        config.tape.autorun = "invalid".into();
        let result = validate_config(&config);
        assert!(result.is_valid()); // warnings don't make it invalid
        assert!(result.warnings.iter().any(|w| w.key == "autorun"));
    }

    #[test]
    fn valid_tape_autorun() {
        let mut config = UserConfig::default_config();
        config.tape.autorun = "auto".into();
        let result = validate_config(&config);
        assert!(!result.warnings.iter().any(|w| w.key == "autorun"));
    }

    #[test]
    fn invalid_daemon_log_level() {
        let mut config = UserConfig::default_config();
        config.daemon.log_level = "verbose".into();
        let result = validate_config(&config);
        assert!(!result.is_valid());
        assert!(result.errors.iter().any(|e| e.key == "log_level"));
    }

    #[test]
    fn invalid_daemon_codec() {
        let mut config = UserConfig::default_config();
        config.daemon.default_codec = "xml".into();
        let result = validate_config(&config);
        assert!(!result.is_valid());
        assert!(result.errors.iter().any(|e| e.key == "default_codec"));
    }

    #[test]
    fn invalid_sidebar_position() {
        let mut config = UserConfig::default_config();
        config.appearance.sidebar.position = "up".into();
        let result = validate_config(&config);
        assert!(!result.is_valid());
        assert!(result.errors.iter().any(|e| e.key == "position"));
    }

    #[test]
    fn invalid_scrollbar_style() {
        let mut config = UserConfig::default_config();
        config.appearance.scrollbar.style = "fancy".into();
        let result = validate_config(&config);
        assert!(!result.is_valid());
        assert!(result.errors.iter().any(|e| e.key == "style"));
    }

    #[test]
    fn default_config_validates_with_new_fields() {
        let config = UserConfig::default_config();
        let result = validate_config(&config);
        assert!(result.is_valid(), "errors: {:?}", result.errors);
    }
}
