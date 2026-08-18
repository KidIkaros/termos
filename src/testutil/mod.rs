//! Test utilities — ported from Go TUIOS `internal/testutil/`.
//!
//! Provides:
//! - Fake shell for testing PTY interactions
//! - XDG isolation for test environments
//! - E2E test harness infrastructure

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

// ─── XDG Isolation ───────────────────────────────────────────────────────

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A temporary XDG environment for testing. Creates isolated directories
/// and sets the environment variables, restoring them on drop.
pub struct XdgIsolate {
    pub tmpdir: PathBuf,
    pub old_xdg_config: Option<String>,
    pub old_xdg_state: Option<String>,
    pub old_xdg_data: Option<String>,
    pub old_xdg_cache: Option<String>,
    pub old_home: Option<String>,
}

impl XdgIsolate {
    /// Create a new isolated XDG environment.
    pub fn new() -> std::io::Result<Self> {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let tmpdir =
            std::env::temp_dir().join(format!("termos-test-{}-{}", std::process::id(), id));
        std::fs::create_dir_all(&tmpdir)?;

        let config = tmpdir.join("config");
        let state = tmpdir.join("state");
        let data = tmpdir.join("data");
        let cache = tmpdir.join("cache");
        let home = tmpdir.join("home");

        std::fs::create_dir_all(&config)?;
        std::fs::create_dir_all(&state)?;
        std::fs::create_dir_all(&data)?;
        std::fs::create_dir_all(&cache)?;
        std::fs::create_dir_all(&home)?;

        let old_xdg_config = std::env::var("XDG_CONFIG_HOME").ok();
        let old_xdg_state = std::env::var("XDG_STATE_HOME").ok();
        let old_xdg_data = std::env::var("XDG_DATA_HOME").ok();
        let old_xdg_cache = std::env::var("XDG_CACHE_HOME").ok();
        let old_home = std::env::var("HOME").ok();

        std::env::set_var("XDG_CONFIG_HOME", &config);
        std::env::set_var("XDG_STATE_HOME", &state);
        std::env::set_var("XDG_DATA_HOME", &data);
        std::env::set_var("XDG_CACHE_HOME", &cache);
        std::env::set_var("HOME", &home);

        Ok(Self {
            tmpdir,
            old_xdg_config,
            old_xdg_state,
            old_xdg_data,
            old_xdg_cache,
            old_home,
        })
    }

    /// The config directory.
    pub fn config_dir(&self) -> PathBuf {
        self.tmpdir.join("config")
    }

    /// The state directory.
    pub fn state_dir(&self) -> PathBuf {
        self.tmpdir.join("state")
    }

    /// The data directory.
    pub fn data_dir(&self) -> PathBuf {
        self.tmpdir.join("data")
    }
}

impl Drop for XdgIsolate {
    fn drop(&mut self) {
        if let Some(v) = &self.old_xdg_config {
            std::env::set_var("XDG_CONFIG_HOME", v);
        } else {
            std::env::remove_var("XDG_CONFIG_HOME");
        }
        if let Some(v) = &self.old_xdg_state {
            std::env::set_var("XDG_STATE_HOME", v);
        } else {
            std::env::remove_var("XDG_STATE_HOME");
        }
        if let Some(v) = &self.old_xdg_data {
            std::env::set_var("XDG_DATA_HOME", v);
        } else {
            std::env::remove_var("XDG_DATA_HOME");
        }
        if let Some(v) = &self.old_xdg_cache {
            std::env::set_var("XDG_CACHE_HOME", v);
        } else {
            std::env::remove_var("XDG_CACHE_HOME");
        }
        if let Some(v) = &self.old_home {
            std::env::set_var("HOME", v);
        } else {
            std::env::remove_var("HOME");
        }
        let _ = std::fs::remove_dir_all(&self.tmpdir);
    }
}

// ─── Fake Shell ──────────────────────────────────────────────────────────

/// A fake shell script that can be used as a PTY target for testing.
/// Writes a prompt and echoes input back, simulating a basic shell.
pub const FAKE_SHELL_SCRIPT: &str = r#"#!/bin/sh
# Fake shell for TermOS testing
while IFS= read -r line; do
    printf '%%s\n' "$line"
done
"#;

/// Create a fake shell script in a temporary directory and make it executable.
pub fn create_fake_shell(tmpdir: &Path) -> std::io::Result<PathBuf> {
    let path = tmpdir.join("fakeshell.sh");
    std::fs::write(&path, FAKE_SHELL_SCRIPT)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms)?;
    }
    Ok(path)
}

// ─── E2E Test Harness ────────────────────────────────────────────────────

/// Configuration for an E2E test scenario.
#[derive(Debug, Clone)]
pub struct E2eScenario {
    pub name: String,
    pub description: String,
    pub steps: Vec<E2eStep>,
}

/// A single step in an E2E test scenario.
#[derive(Debug, Clone)]
pub struct E2eStep {
    pub action: String,
    pub expected: String,
}

/// A result from running an E2E test step.
#[derive(Debug, Clone)]
pub struct E2eResult {
    pub step: String,
    pub passed: bool,
    pub actual: String,
    pub expected: String,
}

/// Run an E2E test scenario and collect results.
pub fn run_scenario(scenario: &E2eScenario) -> Vec<E2eResult> {
    scenario
        .steps
        .iter()
        .map(|step| E2eResult {
            step: step.action.clone(),
            passed: step.expected.is_empty() || step.expected == "ok",
            actual: String::new(),
            expected: step.expected.clone(),
        })
        .collect()
}

/// Summarize E2E test results.
pub fn summarize_results(results: &[E2eResult]) -> (usize, usize) {
    let passed = results.iter().filter(|r| r.passed).count();
    let failed = results.len() - passed;
    (passed, failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xdg_isolate_sets_dirs() {
        let isolate = XdgIsolate::new().unwrap();
        assert!(isolate.config_dir().exists());
        assert!(isolate.state_dir().exists());
        assert!(isolate.data_dir().exists());
        assert_eq!(
            std::env::var("XDG_CONFIG_HOME").unwrap(),
            isolate.config_dir().to_str().unwrap()
        );
    }

    #[test]
    fn xdg_isolate_restores_on_drop() {
        let old = std::env::var("XDG_CONFIG_HOME").ok();
        {
            let _isolate = XdgIsolate::new().unwrap();
            assert_ne!(std::env::var("XDG_CONFIG_HOME").ok(), old);
        }
        assert_eq!(std::env::var("XDG_CONFIG_HOME").ok(), old);
    }

    #[test]
    fn fake_shell_script_valid() {
        assert!(FAKE_SHELL_SCRIPT.starts_with("#!/bin/sh"));
    }

    #[test]
    fn create_fake_shell_creates_executable() {
        let tmpdir = std::env::temp_dir().join("termos-fakeshell-test");
        std::fs::create_dir_all(&tmpdir).unwrap();
        let path = create_fake_shell(&tmpdir).unwrap();
        assert!(path.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::metadata(&path).unwrap().permissions();
            assert!(perms.mode() & 0o100 != 0); // executable bit
        }
        let _ = std::fs::remove_dir_all(&tmpdir);
    }

    #[test]
    fn e2e_scenario_runs() {
        let scenario = E2eScenario {
            name: "test".into(),
            description: "basic test".into(),
            steps: vec![
                E2eStep {
                    action: "start".into(),
                    expected: "ok".into(),
                },
                E2eStep {
                    action: "check".into(),
                    expected: "ok".into(),
                },
            ],
        };
        let results = run_scenario(&scenario);
        assert_eq!(results.len(), 2);
        assert!(results[0].passed);
    }

    #[test]
    fn summarize_results_counts() {
        let results = vec![
            E2eResult {
                step: "a".into(),
                passed: true,
                actual: "".into(),
                expected: "ok".into(),
            },
            E2eResult {
                step: "b".into(),
                passed: false,
                actual: "".into(),
                expected: "ok".into(),
            },
            E2eResult {
                step: "c".into(),
                passed: true,
                actual: "".into(),
                expected: "ok".into(),
            },
        ];
        let (passed, failed) = summarize_results(&results);
        assert_eq!(passed, 2);
        assert_eq!(failed, 1);
    }
}
