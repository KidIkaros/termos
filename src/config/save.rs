//! Config file saving — ported from Go TUIOS `internal/config/save.go`.
//!
//! Serializes a `UserConfig` to TOML and writes it atomically (temp file +
//! rename) to avoid leaving a half-written config on disk.

use std::path::Path;

use super::userconfig::UserConfig;

/// The comment block written at the top of a generated config file.
fn config_file_header(config_path: &str) -> String {
    let mut sb = String::new();
    sb.push_str("# TermOS Configuration File\n");
    sb.push_str("# This file allows you to customize appearance and keybindings\n");
    sb.push_str("#\n");
    sb.push_str(&format!("# Configuration location: {}\n", config_path));
    sb.push_str("# Documentation: https://github.com/Gaurav-Gosain/tuios\n");
    sb.push_str("# For keybindings documentation, run: termos keybinds list\n\n");

    sb.push_str("# ============================================================================\n");
    sb.push_str("# APPEARANCE SETTINGS\n");
    sb.push_str("# ============================================================================\n");
    sb.push_str("# Many of these can be changed live from the in-app settings page\n");
    sb.push_str("# (open it with the leader key followed by ',').\n");
    sb.push_str("#\n");
    sb.push_str("# border_style: rounded, single, double, plain, ascii\n");
    sb.push_str("# dockbar_position: bottom, top, hidden\n");
    sb.push_str("# window_title_position: bottom, top, hidden\n");
    sb.push_str("# theme: color theme name (e.g. dracula, nord); empty for terminal colors;\n");
    sb.push_str("#        'auto' detects the host terminal's light/dark preference and picks\n");
    sb.push_str("#        between theme_auto_dark and theme_auto_light.\n");
    sb.push_str("# theme_auto_dark: theme used by 'auto' when the terminal is dark (default catppuccin-mocha)\n");
    sb.push_str("# theme_auto_light: theme used by 'auto' when the terminal is light (default catppuccin-latte)\n");
    sb.push_str("# ============================================================================\n\n");

    sb.push_str("# ============================================================================\n");
    sb.push_str("# KEYBINDINGS\n");
    sb.push_str("# ============================================================================\n");
    sb.push_str("# Set an action to [] to unbind it and hand the key back to the shell.\n");
    sb.push_str("# ============================================================================\n\n");
    sb
}

/// Serialize `config` to TOML and write it to `path` atomically (temp file +
/// rename), creating parent directories as needed.
pub fn save_config(config: &UserConfig, path: &Path) -> Result<(), String> {
    // Create parent directories.
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("failed to create config directory: {}", e))?;
    }

    // Serialize to TOML.
    let toml_data = toml::to_string_pretty(config)
        .map_err(|e| format!("failed to marshal config: {}", e))?;

    // Prepend the header.
    let header = config_file_header(&path.to_string_lossy());
    let content = format!("{}{}", header, toml_data);

    // Write to a temp file in the same directory, then rename for atomicity.
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let tmp_path = dir.join(format!(
        ".termos-config.tmp.{}.{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));

    std::fs::write(&tmp_path, &content)
        .map_err(|e| format!("failed to write temp config file: {}", e))?;

    std::fs::rename(&tmp_path, path)
        .map_err(|e| {
            // Clean up the temp file on failure.
            let _ = std::fs::remove_file(&tmp_path);
            format!("failed to rename temp config file: {}", e)
        })
}

/// Save the config to the XDG config path, creating directories as needed.
pub fn save_user_config(config: &UserConfig) -> Result<(), String> {
    let path = UserConfig::config_path()
        .ok_or_else(|| "no config dir".to_string())?;
    save_config(config, &path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn save_config_writes_valid_toml() {
        let dir = std::env::temp_dir().join("termos_test_save_config");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.toml");

        let cfg = UserConfig::default_config();
        save_config(&cfg, &path).expect("save should succeed");

        // The file should exist and be parseable.
        let data = std::fs::read_to_string(&path).expect("file should exist");
        assert!(data.contains("# TermOS Configuration File"));

        let parsed: UserConfig = toml::from_str(&data).expect("should parse");
        assert_eq!(parsed.appearance.border_style, "rounded");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_config_creates_parent_dirs() {
        let dir = std::env::temp_dir().join("termos_test_save_dirs");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("nested").join("config.toml");

        let cfg = UserConfig::default_config();
        save_config(&cfg, &path).expect("save should succeed");
        assert!(path.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_config_round_trip() {
        let dir = std::env::temp_dir().join("termos_test_save_roundtrip");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.toml");

        let mut cfg = UserConfig::default_config();
        cfg.appearance.border_style = "double".into();
        save_config(&cfg, &path).expect("save should succeed");

        let loaded = UserConfig::load_from(&path);
        assert_eq!(loaded.appearance.border_style, "double");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_file_header_contains_sections() {
        let header = config_file_header("/test/path.toml");
        assert!(header.contains("APPEARANCE SETTINGS"));
        assert!(header.contains("KEYBINDINGS"));
        assert!(header.contains("/test/path.toml"));
    }

    #[test]
    fn save_config_atomic_no_temp_left() {
        let dir = std::env::temp_dir().join("termos_test_save_atomic");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.toml");

        let cfg = UserConfig::default_config();
        save_config(&cfg, &path).expect("save should succeed");

        // No temp files should remain.
        let entries: Vec<PathBuf> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .collect();
        for entry in &entries {
            if entry.file_name().unwrap_or_default().to_string_lossy().starts_with(".termos-config.tmp") {
                panic!("temp file left behind: {:?}", entry);
            }
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}
