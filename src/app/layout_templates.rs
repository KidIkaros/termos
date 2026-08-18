//! Layout template persistence — save/load/list/delete layout templates
//! as JSON files in `~/.config/termos/layouts/`.
//!
//! Mirrors Go's `internal/app/layout_templates.go`. Templates store the BSP
//! tree structure plus per-window startup commands, working directories, and
//! tiling configuration.

use std::path::PathBuf;

/// A saved layout template (JSON file in `~/.config/termos/layouts/`).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LayoutTemplate {
    /// Template name (also used as the file stem).
    pub name: String,
    /// Optional description.
    #[serde(default)]
    pub description: String,
    /// Creation time as a Unix timestamp.
    pub created_at: u64,
    /// Schema version (currently 1).
    pub version: u32,
    /// Whether auto-tiling is enabled.
    #[serde(default)]
    pub auto_tiling: bool,
    /// The serialized BSP tree.
    pub tree: crate::layout::bsp::SerializedBSPTree,
    /// Per-window configuration.
    #[serde(default)]
    pub windows: Vec<LayoutWindow>,
    /// Screen dimensions at save time (for proportional scaling).
    #[serde(default)]
    pub screen_width: i32,
    #[serde(default)]
    pub screen_height: i32,
}

/// Per-window configuration in a layout template.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct LayoutWindow {
    /// Custom window title.
    #[serde(default)]
    pub title: String,
    /// Shell command to run on creation (e.g. "vim", "htop").
    #[serde(default)]
    pub command: String,
    /// Working directory for the shell.
    #[serde(default)]
    pub working_dir: String,
}

/// Get the layout templates directory (`~/.config/termos/layouts/`).
pub fn layouts_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("termos")
        .join("layouts")
}

/// Sanitize a layout name for use as a file name.
fn sanitize_name(name: &str) -> String {
    let mut safe = name.replace(std::path::MAIN_SEPARATOR, "_");
    safe = safe.replace(' ', "_");
    safe = safe.replace("..", "_");
    if safe.is_empty() {
        "unnamed".to_string()
    } else {
        safe
    }
}

/// Save a layout template to disk as JSON.
pub fn save_layout_template(template: &LayoutTemplate) -> Result<(), String> {
    let dir = layouts_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{}.json", sanitize_name(&template.name)));
    let json = serde_json::to_string_pretty(template).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())?;
    Ok(())
}

/// Load a layout template from disk by name.
pub fn load_layout_template(name: &str) -> Result<LayoutTemplate, String> {
    let path = layouts_dir().join(format!("{}.json", sanitize_name(name)));
    let json = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    serde_json::from_str(&json).map_err(|e| e.to_string())
}

/// List all saved layout templates (name, created_at).
pub fn list_layout_templates() -> Vec<(String, u64)> {
    let dir = layouts_dir();
    let mut templates = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "json") {
                if let Some(name) = path.file_stem().and_then(|s| s.to_str()) {
                    if let Ok(json) = std::fs::read_to_string(&path) {
                        if let Ok(tmpl) = serde_json::from_str::<LayoutTemplate>(&json) {
                            templates.push((name.to_string(), tmpl.created_at));
                        }
                    }
                }
            }
        }
    }
    templates.sort_by(|a, b| a.0.cmp(&b.0));
    templates
}

/// Delete a layout template by name.
pub fn delete_layout_template(name: &str) -> Result<(), String> {
    let path = layouts_dir().join(format!("{}.json", sanitize_name(name)));
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| e.to_string())
    } else {
        Err(format!("layout '{name}' not found"))
    }
}

/// Generate a tape script from a layout template (mirrors Go's
/// `GenerateTapeScript`).
pub fn generate_tape_script(tmpl: &LayoutTemplate) -> String {
    let mut sb = String::new();
    sb.push_str(&format!("# Auto-generated layout script: {}\n", tmpl.name));
    sb.push_str(&format!("# Created: {}\n\n", tmpl.created_at));

    if tmpl.auto_tiling {
        sb.push_str("EnableTiling\n");
    } else {
        sb.push_str("DisableTiling\n");
    }

    for (i, w) in tmpl.windows.iter().enumerate() {
        if i > 0 {
            sb.push_str("NewWindow\n");
        }
        if !w.title.is_empty() {
            sb.push_str(&format!("RenameWindow \"{}\"\n", w.title));
        }
        if !w.working_dir.is_empty() {
            sb.push_str(&format!("Type cd {}\nEnter\n", w.working_dir));
        }
        if !w.command.is_empty() {
            sb.push_str(&format!("Type {}\nEnter\n", w.command));
        }
        sb.push_str("Sleep 200ms\n");
    }

    sb
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::bsp::{BSPTree, Rect, SplitType};

    #[test]
    fn layout_template_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("XDG_CONFIG_HOME", dir.path());

        let mut tree = BSPTree::new();
        let bounds = Rect {
            x: 0,
            y: 0,
            w: 120,
            h: 40,
        };
        tree.insert_window(1, -1, SplitType::None, 0.5, bounds, 0);
        tree.insert_window(2, 1, SplitType::Vertical, 0.5, bounds, 0);

        let tmpl = LayoutTemplate {
            name: "test-layout".into(),
            description: "A test layout".into(),
            created_at: 1700000000,
            version: 1,
            auto_tiling: true,
            tree: tree.serialize(),
            windows: vec![
                LayoutWindow {
                    title: "editor".into(),
                    command: "vim".into(),
                    working_dir: "/tmp".into(),
                },
                LayoutWindow::default(),
            ],
            screen_width: 120,
            screen_height: 40,
        };

        save_layout_template(&tmpl).unwrap();

        let loaded = load_layout_template("test-layout").unwrap();
        assert_eq!(loaded.name, "test-layout");
        assert!(loaded.auto_tiling);
        assert_eq!(loaded.windows.len(), 2);
        assert_eq!(loaded.windows[0].command, "vim");

        let list = list_layout_templates();
        assert!(list.iter().any(|(n, _)| n == "test-layout"));

        delete_layout_template("test-layout").unwrap();
        assert!(load_layout_template("test-layout").is_err());
    }

    #[test]
    fn generate_tape_script_format() {
        let tmpl = LayoutTemplate {
            name: "dev".into(),
            description: String::new(),
            created_at: 1700000000,
            version: 1,
            auto_tiling: true,
            tree: crate::layout::bsp::SerializedBSPTree::default(),
            windows: vec![LayoutWindow {
                title: "editor".into(),
                command: "vim".into(),
                working_dir: "/home".into(),
            }],
            screen_width: 0,
            screen_height: 0,
        };

        let script = generate_tape_script(&tmpl);
        assert!(script.contains("EnableTiling"));
        assert!(script.contains("RenameWindow \"editor\""));
        assert!(script.contains("Type cd /home"));
        assert!(script.contains("Type vim"));
    }

    #[test]
    fn sanitize_name_handles_special_chars() {
        assert_eq!(sanitize_name("my layout"), "my_layout");
        assert_eq!(sanitize_name(".."), "_");
        assert_eq!(sanitize_name(""), "unnamed");
    }
}
