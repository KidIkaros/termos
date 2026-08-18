//! Harness registry — ported from Go TUIOS `internal/harness/registry.go`.
//!
//! Loads and manages harness manifests, providing lookup and process
//! identification.

use std::path::Path;

use super::manifest::{parse_manifest, DetectSpec, Manifest};

/// Bundled manifest TOML contents.
const AIDER_TOML: &str = include_str!("../../manifests/aider.toml");
const CLAUDE_CODE_TOML: &str = include_str!("../../manifests/claude-code.toml");
const CODEX_TOML: &str = include_str!("../../manifests/codex.toml");
const CRUSH_TOML: &str = include_str!("../../manifests/crush.toml");
const CURSOR_AGENT_TOML: &str = include_str!("../../manifests/cursor-agent.toml");
const DROID_TOML: &str = include_str!("../../manifests/droid.toml");
const GEMINI_CLI_TOML: &str = include_str!("../../manifests/gemini-cli.toml");
const OPENCODE_TOML: &str = include_str!("../../manifests/opencode.toml");

/// The loaded set of harness manifests, ordered so lookup is deterministic.
pub struct Registry {
    manifests: Vec<Manifest>,
}

/// One manifest that failed to load.
#[derive(Debug)]
pub struct LoadError {
    pub source: String,
    pub error: String,
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}: {}", self.source, self.error)
    }
}

impl std::error::Error for LoadError {}

impl Registry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            manifests: Vec::new(),
        }
    }

    /// Load the bundled manifests, then any user manifests from the given
    /// directories. Later directories override earlier ones by ID.
    pub fn load(dirs: &[&Path]) -> (Self, Vec<LoadError>) {
        let mut by_id: std::collections::HashMap<String, Manifest> =
            std::collections::HashMap::new();
        let mut errs = Vec::new();

        // Load bundled manifests.
        let bundled = [
            ("aider.toml", AIDER_TOML),
            ("claude-code.toml", CLAUDE_CODE_TOML),
            ("codex.toml", CODEX_TOML),
            ("crush.toml", CRUSH_TOML),
            ("cursor-agent.toml", CURSOR_AGENT_TOML),
            ("droid.toml", DROID_TOML),
            ("gemini-cli.toml", GEMINI_CLI_TOML),
            ("opencode.toml", OPENCODE_TOML),
        ];

        for (name, toml) in &bundled {
            match parse_manifest(name, toml) {
                Ok(m) => {
                    by_id.insert(m.id.clone(), m);
                }
                Err(e) => errs.push(LoadError {
                    source: name.to_string(),
                    error: e,
                }),
            }
        }

        // Load user manifests from directories.
        for dir in dirs {
            if dir.as_os_str().is_empty() {
                continue;
            }
            let entries = match std::fs::read_dir(dir) {
                Ok(e) => e,
                Err(_) => continue,
            };
            let mut files: Vec<_> = entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().is_some_and(|ext| ext == "toml"))
                .collect();
            files.sort_by_key(|e| e.path());
            for f in files {
                let path = f.path();
                let data = match std::fs::read_to_string(&path) {
                    Ok(d) => d,
                    Err(e) => {
                        errs.push(LoadError {
                            source: path.display().to_string(),
                            error: e.to_string(),
                        });
                        continue;
                    }
                };
                let name = path.display().to_string();
                match parse_manifest(&name, &data) {
                    Ok(m) => {
                        by_id.insert(m.id.clone(), m);
                    }
                    Err(e) => errs.push(LoadError {
                        source: name,
                        error: e,
                    }),
                }
            }
        }

        let mut manifests: Vec<Manifest> = by_id.into_values().collect();
        // Sort by priority (descending), then by id (ascending).
        manifests.sort_by(|a, b| {
            if a.priority != b.priority {
                return b.priority.cmp(&a.priority);
            }
            a.id.cmp(&b.id)
        });

        (Self { manifests }, errs)
    }

    /// List harness IDs in lookup order.
    pub fn ids(&self) -> Vec<&str> {
        self.manifests.iter().map(|m| m.id.as_str()).collect()
    }

    /// Look up a manifest by ID.
    pub fn by_id(&self, id: &str) -> Option<&Manifest> {
        self.manifests.iter().find(|m| m.id == id)
    }

    /// All manifests.
    pub fn manifests(&self) -> &[Manifest] {
        &self.manifests
    }

    /// Identify which harness a process is. Returns the harness ID if matched.
    /// `comm` is the process comm name, `argv` the full command line, `exe`
    /// the resolved executable path. Any may be empty.
    pub fn identify(&self, comm: &str, argv: &[String], exe: &str) -> Option<&str> {
        let comm_base = base_name(comm);
        let argv0 = if argv.is_empty() {
            String::new()
        } else {
            base_name(&argv[0])
        };

        for m in &self.manifests {
            if matches_detect(&m.detect, &comm_base, &argv0, argv, exe) {
                return Some(&m.id);
            }
        }
        None
    }

    /// Classify screen rules for a given harness against pane tail text.
    pub fn classify(&self, id: &str, tail: &[String]) -> Option<(String, usize)> {
        let m = self.by_id(id)?;
        super::classify::classify(m, tail)
    }

    /// How many lines from the bottom this harness's rules see.
    pub fn screen_lines(&self, id: &str) -> i32 {
        match self.by_id(id) {
            Some(m) => super::classify::screen_lines(m),
            None => 0,
        }
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

/// Check if a process matches a detect spec. Any one predicate matching
/// is enough.
fn matches_detect(d: &DetectSpec, comm: &str, argv0: &str, argv: &[String], exe: &str) -> bool {
    if d.comm.iter().any(|c| c == comm) {
        return true;
    }
    if d.argv0.iter().any(|a| a == argv0) {
        return true;
    }
    for want in &d.argv_path {
        for arg in argv {
            if arg.to_lowercase().contains(want) {
                return true;
            }
        }
    }
    if !exe.is_empty() {
        let lower = exe.to_lowercase();
        for pattern in &d.exe_glob {
            if glob_match(pattern, &lower) {
                return true;
            }
        }
    }
    false
}

/// Simple glob matcher supporting `*` wildcards.
fn glob_match(pattern: &str, text: &str) -> bool {
    // Convert glob to a simple check: split by * and ensure all parts
    // appear in order.
    let parts: Vec<&str> = pattern.split('*').filter(|s| !s.is_empty()).collect();
    if parts.is_empty() {
        return true; // pattern is all wildcards
    }
    let mut pos = 0;
    for part in &parts {
        match text[pos..].find(part) {
            Some(idx) => pos += idx + part.len(),
            None => return false,
        }
    }
    // If pattern doesn't end with *, text must end with the last part.
    if !pattern.ends_with('*') {
        return text[pos..].is_empty() || text.ends_with(parts.last().unwrap());
    }
    true
}

/// Script extensions stripped before matching.
const SCRIPT_EXTENSIONS: &[&str] = &[".js", ".mjs", ".cjs", ".ts", ".py"];

/// Reduce a comm or argv token to the base name used for matching:
/// no directory, no trailing NUL, no login-shell "-" prefix, no script
/// extension, lowercased.
fn base_name(s: &str) -> String {
    let s = s.trim().trim_end_matches('\0');
    if s.is_empty() {
        return String::new();
    }
    // Get the file name component.
    let base = std::path::Path::new(s)
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| s.to_string());
    // Strip leading "-" (login shell prefix).
    let base = base.strip_prefix('-').unwrap_or(&base);
    let lower = base.to_lowercase();
    for ext in SCRIPT_EXTENSIONS {
        if let Some(before) = lower.strip_suffix(ext) {
            return before.to_string();
        }
    }
    lower
}

/// Where user manifests live, following XDG.
pub fn user_dir() -> Option<std::path::PathBuf> {
    if let Ok(dir) = std::env::var("TERMOS_HARNESS_DIR") {
        return Some(std::path::PathBuf::from(dir));
    }
    let base = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .or_else(|| std::env::var("HOME").ok().map(|h| format!("{}/.config", h)))?;
    Some(std::path::PathBuf::from(format!(
        "{}/termos/harnesses",
        base
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_bundled() {
        let (reg, errs) = Registry::load(&[]);
        assert!(errs.is_empty(), "load errors: {:?}", errs);
        let ids = reg.ids();
        assert!(ids.contains(&"aider"));
        assert!(ids.contains(&"claude-code"));
        assert!(ids.contains(&"codex"));
        assert!(ids.contains(&"crush"));
        assert!(ids.contains(&"cursor-agent"));
        assert!(ids.contains(&"droid"));
        assert!(ids.contains(&"gemini-cli"));
        assert!(ids.contains(&"opencode"));
    }

    #[test]
    fn by_id_finds_manifest() {
        let (reg, _) = Registry::load(&[]);
        let m = reg.by_id("aider").unwrap();
        assert_eq!(m.display_name, "Aider");
    }

    #[test]
    fn by_id_missing_returns_none() {
        let (reg, _) = Registry::load(&[]);
        assert!(reg.by_id("nonexistent").is_none());
    }

    #[test]
    fn identify_by_comm() {
        let (reg, _) = Registry::load(&[]);
        let id = reg.identify("aider", &["aider".to_string()], "/usr/bin/aider");
        assert_eq!(id, Some("aider"));
    }

    #[test]
    fn identify_by_argv_path() {
        let (reg, _) = Registry::load(&[]);
        let argv = vec![
            "node".to_string(),
            "/usr/lib/node_modules/@anthropic-ai/claude-code/cli.js".to_string(),
        ];
        let id = reg.identify("node", &argv, "/usr/bin/node");
        assert_eq!(id, Some("claude-code"));
    }

    #[test]
    fn identify_no_match() {
        let (reg, _) = Registry::load(&[]);
        let id = reg.identify("bash", &["bash".to_string()], "/usr/bin/bash");
        assert_eq!(id, None);
    }

    #[test]
    fn identify_strips_script_extension() {
        let (reg, _) = Registry::load(&[]);
        // When the script itself is argv[0], base_name strips .js → "aider" → match.
        let argv = vec!["aider.js".to_string()];
        let id = reg.identify("aider.js", &argv, "/usr/bin/aider.js");
        assert_eq!(id, Some("aider"));
    }

    #[test]
    fn base_name_strips_path() {
        assert_eq!(base_name("/usr/bin/aider"), "aider");
    }

    #[test]
    fn base_name_strips_login_prefix() {
        assert_eq!(base_name("-bash"), "bash");
    }

    #[test]
    fn base_name_lowercases() {
        assert_eq!(base_name("Aider"), "aider");
    }

    #[test]
    fn base_name_strips_extension() {
        assert_eq!(base_name("aider.js"), "aider");
        assert_eq!(base_name("aider.py"), "aider");
    }

    #[test]
    fn glob_match_basic() {
        assert!(glob_match("*", "anything"));
        assert!(glob_match("*/claude/*", "/usr/share/claude/versions/1.0"));
        assert!(!glob_match("*/claude/*", "/usr/bin/bash"));
    }

    #[test]
    fn classify_claude_code_permission_prompt() {
        let (reg, _) = Registry::load(&[]);
        let tail = vec![
            "Do you want to proceed?".to_string(),
            "❯ 1. Yes".to_string(),
            "  2. Yes, and don't ask again".to_string(),
        ];
        let (state, _) = reg.classify("claude-code", &tail).unwrap();
        assert_eq!(state, "needs_input");
    }

    #[test]
    fn classify_no_match() {
        let (reg, _) = Registry::load(&[]);
        let tail = vec!["just some output".to_string()];
        assert!(reg.classify("aider", &tail).is_none());
    }

    #[test]
    fn screen_lines_for_enabled() {
        let (reg, _) = Registry::load(&[]);
        assert!(reg.screen_lines("claude-code") > 0);
    }

    #[test]
    fn screen_lines_for_disabled() {
        let (reg, _) = Registry::load(&[]);
        assert_eq!(reg.screen_lines("aider"), 0);
    }
}
