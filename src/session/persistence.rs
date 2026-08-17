//! Session persistence ("resurrection") — ported from TUIOS
//! `internal/session/resurrection.go`. Sessions are saved as JSON so their
//! window shells and workspaces can be respawned when the daemon restarts.

use std::path::PathBuf;

use super::model::{SessionState, WindowState};

/// The state directory: `$XDG_STATE_HOME/tuios/sessions` (or the platform
/// default).
pub fn state_dir() -> Option<PathBuf> {
    dirs::state_dir().map(|d| d.join("tuios").join("sessions"))
}

/// The state file for a session name.
pub fn state_path(name: &str) -> Option<PathBuf> {
    state_dir().map(|d| d.join(format!("{name}.json")))
}

/// Save a session's window definitions.
pub fn save(name: &str, windows: &[WindowState]) -> Result<(), String> {
    let Some(dir) = state_dir() else {
        return Err("no state directory".into());
    };
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let state = SessionState {
        name: name.to_string(),
        windows: windows.to_vec(),
    };
    let json = serde_json::to_string_pretty(&state).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{name}.json"));
    std::fs::write(&path, json).map_err(|e| e.to_string())
}

/// Load a session's window definitions.
pub fn load(name: &str) -> Option<SessionState> {
    let path = state_path(name)?;
    let data = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&data).ok()
}

/// All saved session names, ordered for a stable restore.
pub fn list_saved() -> Vec<SessionState> {
    let Some(dir) = state_dir() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut states: Vec<SessionState> = entries
        .flatten()
        .filter(|e| e.path().extension().map(|x| x == "json").unwrap_or(false))
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .filter_map(|s| serde_json::from_str::<SessionState>(&s).ok())
        .collect();
    states.sort_by(|a, b| a.name.cmp(&b.name));
    states
}

/// Remove a session's state file (an explicit kill must not resurrect).
pub fn remove(name: &str) {
    if let Some(path) = state_path(name) {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_windows() {
        // Use a temp dir via TMPDIR-independent override is not exposed, so
        // exercise save/load through the public API and clean up.
        let name = "__tuios_test_session__";
        remove(name);
        let windows = vec![
            WindowState {
                title: "Terminal".to_string(),
                shell: "/bin/sh".to_string(),
                workspace: 1,
            },
            WindowState {
                title: "Editor".to_string(),
                shell: "/bin/sh".to_string(),
                workspace: 2,
            },
        ];
        save(name, &windows).unwrap();
        let loaded = load(name).expect("loaded");
        assert_eq!(loaded.windows.len(), 2);
        assert_eq!(loaded.windows[1].workspace, 2);
        remove(name);
        assert!(load(name).is_none());
    }
}
