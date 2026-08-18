//! Harness manifest types — ported from Go TUIOS `internal/harness/manifest.go`.
//!
//! Describes one coding-agent CLI harness for detection.

use serde::Deserialize;

/// The manifest format version this build understands.
pub const SCHEMA_VERSION: i32 = 1;

/// Default number of screen lines read from the pane bottom.
pub const DEFAULT_SCREEN_LINES: i32 = 6;

/// Describes one harness.
#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub schema_version: i32,
    pub id: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub detect: DetectSpec,
    #[serde(default)]
    pub screen: ScreenSpec,
}

/// How a process is recognised as this harness. Any one predicate matching
/// is enough; they are alternatives, not requirements.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct DetectSpec {
    #[serde(default)]
    pub comm: Vec<String>,
    #[serde(default)]
    pub argv0: Vec<String>,
    #[serde(default)]
    pub argv_path: Vec<String>,
    #[serde(default)]
    pub exe_glob: Vec<String>,
}

/// Optional rules matched against a pane's rendered text.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ScreenSpec {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub lines: i32,
    #[serde(default, rename = "rule")]
    pub rules: Vec<ScreenRule>,
}

/// One screen-text rule. Every string in `all` must be present, at least
/// one in `any` must be present, and none in `not` may be.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ScreenRule {
    pub state: String,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub all: Vec<String>,
    #[serde(default)]
    pub any: Vec<String>,
    #[serde(default)]
    pub not: Vec<String>,
}

/// Parse a manifest from TOML, validating schema version and required fields.
pub fn parse_manifest(name: &str, toml_str: &str) -> Result<Manifest, String> {
    let mut m: Manifest = toml::from_str(toml_str).map_err(|e| format!("{}: {}", name, e))?;

    if m.schema_version != SCHEMA_VERSION {
        return Err(format!(
            "{}: schema_version {}, this build understands {}",
            name, m.schema_version, SCHEMA_VERSION
        ));
    }

    m.id = m.id.trim().to_string();
    if m.id.is_empty() {
        return Err(format!("{}: no id", name));
    }

    // Normalize detect predicates.
    normalize_vec(&mut m.detect.comm);
    normalize_vec(&mut m.detect.argv0);
    normalize_vec(&mut m.detect.argv_path);
    normalize_vec(&mut m.detect.exe_glob);

    if !detect_any(&m.detect) {
        return Err(format!("{}: manifest \"{}\" matches nothing", name, m.id));
    }

    // Validate screen rule states.
    for (i, r) in m.screen.rules.iter().enumerate() {
        if !is_valid_state(&r.state) {
            return Err(format!(
                "{}: manifest \"{}\" screen rule {}: unknown state \"{}\"",
                name, m.id, i, r.state
            ));
        }
    }

    if m.screen.lines <= 0 {
        m.screen.lines = DEFAULT_SCREEN_LINES;
    }

    if m.display_name.is_empty() {
        m.display_name = m.id.clone();
    }

    Ok(m)
}

/// Valid screen rule states.
fn is_valid_state(s: &str) -> bool {
    matches!(s, "working" | "needs_input" | "idle")
}

fn normalize_vec(v: &mut Vec<String>) {
    *v = v
        .iter()
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
}

fn detect_any(d: &DetectSpec) -> bool {
    !d.comm.is_empty() || !d.argv0.is_empty() || !d.argv_path.is_empty() || !d.exe_glob.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_manifest() {
        let toml = r#"
schema_version = 1
id = "test"
display_name = "Test"
priority = 50

[detect]
comm = ["test"]
argv0 = ["test"]
"#;
        let m = parse_manifest("test.toml", toml).unwrap();
        assert_eq!(m.id, "test");
        assert_eq!(m.display_name, "Test");
        assert_eq!(m.priority, 50);
        assert_eq!(m.detect.comm, vec!["test"]);
    }

    #[test]
    fn parse_wrong_schema_version() {
        let toml = r#"
schema_version = 2
id = "test"
[detect]
comm = ["test"]
"#;
        assert!(parse_manifest("test.toml", toml).is_err());
    }

    #[test]
    fn parse_no_id() {
        let toml = r#"
schema_version = 1
id = ""
[detect]
comm = ["test"]
"#;
        assert!(parse_manifest("test.toml", toml).is_err());
    }

    #[test]
    fn parse_no_detect() {
        let toml = r#"
schema_version = 1
id = "test"
[detect]
"#;
        assert!(parse_manifest("test.toml", toml).is_err());
    }

    #[test]
    fn parse_default_display_name() {
        let toml = r#"
schema_version = 1
id = "myagent"
[detect]
comm = ["myagent"]
"#;
        let m = parse_manifest("test.toml", toml).unwrap();
        assert_eq!(m.display_name, "myagent");
    }

    #[test]
    fn parse_default_screen_lines() {
        let toml = r#"
schema_version = 1
id = "test"
[detect]
comm = ["test"]
"#;
        let m = parse_manifest("test.toml", toml).unwrap();
        assert_eq!(m.screen.lines, DEFAULT_SCREEN_LINES);
    }

    #[test]
    fn parse_invalid_screen_state() {
        let toml = r#"
schema_version = 1
id = "test"
[detect]
comm = ["test"]

[[screen.rule]]
state = "bogus"
"#;
        assert!(parse_manifest("test.toml", toml).is_err());
    }

    #[test]
    fn parse_valid_screen_state() {
        let toml = r#"
schema_version = 1
id = "test"
[detect]
comm = ["test"]

[[screen.rule]]
state = "working"
all = ["spinner"]
"#;
        let m = parse_manifest("test.toml", toml).unwrap();
        assert_eq!(m.screen.rules.len(), 1);
        assert_eq!(m.screen.rules[0].state, "working");
    }
}
