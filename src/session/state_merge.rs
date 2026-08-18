//! Session-state merge — ported from Go TUIOS `internal/session/state_merge.go`
//! and `state_events.go`.
//!
//! The daemon and its attached clients both write session state, and they
//! write different parts of it. The daemon owns what a user would be surprised
//! to lose across a detach and reattach: which windows exist, what they are
//! called, which workspace they are on, and what is focused. The client owns
//! what is derived from its own viewport. A client sync that replaced the
//! whole state would silently undo daemon-side mutations; the merge functions
//! here make the daemon's value win on the fields it owns.

/// The agent state of one window, daemon-owned.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WindowState {
    pub id: String,
    pub title: String,
    pub workspace: i32,
    pub agent_state: String,
    pub agent_message: String,
    pub agent_harness: String,
    pub minimized: bool,
    /// The shell-reported working directory (daemon-side /proc read).
    pub cwd: String,
    /// The foreground process command (daemon-side detection).
    pub foreground: String,
}

/// The canonical session state as the daemon sees it.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SessionState {
    pub name: String,
    pub display_name: String,
    pub accent: String,
    pub restored: bool,
    pub resurrection_version: u64,
    pub windows: Vec<WindowState>,
}

impl SessionState {
    /// Find a window by id.
    pub fn window(&self, id: &str) -> Option<&WindowState> {
        self.windows.iter().find(|w| w.id == id)
    }
}

/// Carry over the daemon-owned fields of `canonical` that a client sync
/// omits, so an incomplete sync cannot wipe them. A client never sends the
/// session label, accent, restored flag, or per-window agent state/cwd/
/// foreground, so canonical always wins on those.
pub fn retain_daemon_exclusive(incoming: &mut SessionState, canonical: &SessionState) {
    if incoming.display_name.is_empty() {
        incoming.display_name.clone_from(&canonical.display_name);
    }
    if incoming.accent.is_empty() {
        incoming.accent.clone_from(&canonical.accent);
    }
    // A bool cannot say "not sent"; canonical simply wins.
    incoming.restored = canonical.restored;
    if incoming.resurrection_version == 0 {
        incoming.resurrection_version = canonical.resurrection_version;
    }

    // Per-window daemon-owned fields carried over by id.
    let by_id: std::collections::HashMap<&str, &WindowState> = canonical
        .windows
        .iter()
        .map(|w| (w.id.as_str(), w))
        .collect();
    for w in &mut incoming.windows {
        if let Some(c) = by_id.get(w.id.as_str()) {
            if w.agent_state.is_empty() {
                w.agent_state.clone_from(&c.agent_state);
                w.agent_message.clone_from(&c.agent_message);
                w.agent_harness.clone_from(&c.agent_harness);
            }
            if w.cwd.is_empty() {
                w.cwd.clone_from(&c.cwd);
            }
            w.foreground.clone_from(&c.foreground);
            w.minimized = c.minimized;
        }
    }
}

/// Drop windows from `incoming` whose PTY is no longer live daemon-side.
/// Returns the ids of the dropped windows.
pub fn reconcile_stale(
    incoming: &mut SessionState,
    has_live_pty: impl Fn(&str) -> bool,
) -> Vec<String> {
    let mut dropped = Vec::new();
    incoming.windows.retain(|w| {
        if has_live_pty(&w.id) {
            true
        } else {
            dropped.push(w.id.clone());
            false
        }
    });
    dropped
}

/// A lifecycle event derived from diffing two snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifecycleEvent {
    WindowCreated { id: String, title: String },
    WindowClosed { id: String },
    WindowRetitled { id: String, title: String },
    AgentStateChanged { id: String, state: String },
}

/// The lifecycle-relevant projection of a state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LifecycleSnapshot {
    /// (window id, title, agent state)
    windows: Vec<(String, String, String)>,
}

/// Project a state for lifecycle diffing.
pub fn snapshot_lifecycle(state: &SessionState) -> LifecycleSnapshot {
    LifecycleSnapshot {
        windows: state
            .windows
            .iter()
            .map(|w| (w.id.clone(), w.title.clone(), w.agent_state.clone()))
            .collect(),
    }
}

/// Diff two snapshots into lifecycle events (created/closed/retitled/agent).
pub fn diff_lifecycle(
    before: &LifecycleSnapshot,
    after: &LifecycleSnapshot,
) -> Vec<LifecycleEvent> {
    let mut events = Vec::new();
    let before_map: std::collections::HashMap<&str, (&str, &str)> = before
        .windows
        .iter()
        .map(|(id, title, state)| (id.as_str(), (title.as_str(), state.as_str())))
        .collect();
    let after_map: std::collections::HashMap<&str, (&str, &str)> = after
        .windows
        .iter()
        .map(|(id, title, state)| (id.as_str(), (title.as_str(), state.as_str())))
        .collect();

    for (id, (title, _)) in &after_map {
        if !before_map.contains_key(id) {
            events.push(LifecycleEvent::WindowCreated {
                id: id.to_string(),
                title: title.to_string(),
            });
        }
    }
    for (id, (_, _)) in &before_map {
        if !after_map.contains_key(id) {
            events.push(LifecycleEvent::WindowClosed { id: id.to_string() });
        }
    }
    for (id, (title, state)) in &after_map {
        if let Some((old_title, old_state)) = before_map.get(id) {
            if old_title != title {
                events.push(LifecycleEvent::WindowRetitled {
                    id: id.to_string(),
                    title: title.to_string(),
                });
            }
            if old_state != state {
                events.push(LifecycleEvent::AgentStateChanged {
                    id: id.to_string(),
                    state: state.to_string(),
                });
            }
        }
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(id: &str, title: &str) -> WindowState {
        WindowState {
            id: id.into(),
            title: title.into(),
            workspace: 1,
            agent_state: String::new(),
            agent_message: String::new(),
            agent_harness: String::new(),
            minimized: false,
            cwd: String::new(),
            foreground: String::new(),
        }
    }

    #[test]
    fn retain_keeps_daemon_labels() {
        let mut incoming = SessionState {
            name: "work".into(),
            ..Default::default()
        };
        let canonical = SessionState {
            name: "work".into(),
            display_name: "Label".into(),
            accent: "green".into(),
            restored: true,
            resurrection_version: 7,
            windows: vec![],
        };
        retain_daemon_exclusive(&mut incoming, &canonical);
        assert_eq!(incoming.display_name, "Label");
        assert_eq!(incoming.accent, "green");
        assert!(incoming.restored);
        assert_eq!(incoming.resurrection_version, 7);
    }

    #[test]
    fn retain_carries_window_agent_state_by_id() {
        let mut incoming = SessionState {
            name: "work".into(),
            windows: vec![window("w1", "t")],
            ..Default::default()
        };
        let mut canonical_win = window("w1", "t");
        canonical_win.agent_state = "working".into();
        canonical_win.agent_harness = "claude-code".into();
        canonical_win.minimized = true;
        let canonical = SessionState {
            name: "work".into(),
            windows: vec![canonical_win],
            ..Default::default()
        };
        retain_daemon_exclusive(&mut incoming, &canonical);
        assert_eq!(incoming.windows[0].agent_state, "working");
        assert_eq!(incoming.windows[0].agent_harness, "claude-code");
        assert!(incoming.windows[0].minimized);
    }

    #[test]
    fn retain_does_not_overwrite_incoming_agent_state() {
        let mut incoming = SessionState {
            name: "work".into(),
            windows: vec![window("w1", "t")],
            ..Default::default()
        };
        incoming.windows[0].agent_state = "idle".into();
        let mut canonical_win = window("w1", "t");
        canonical_win.agent_state = "working".into();
        let canonical = SessionState {
            name: "work".into(),
            windows: vec![canonical_win],
            ..Default::default()
        };
        retain_daemon_exclusive(&mut incoming, &canonical);
        assert_eq!(incoming.windows[0].agent_state, "idle");
    }

    #[test]
    fn reconcile_drops_dead_ptys() {
        let mut state = SessionState {
            name: "work".into(),
            windows: vec![window("w1", "a"), window("w2", "b"), window("w3", "c")],
            ..Default::default()
        };
        let dropped = reconcile_stale(&mut state, |id| id != "w2");
        assert_eq!(dropped, vec!["w2".to_string()]);
        assert_eq!(state.windows.len(), 2);
    }

    #[test]
    fn lifecycle_diff_detects_created_closed_retitled() {
        let before = snapshot_lifecycle(&SessionState {
            name: "work".into(),
            windows: vec![window("w1", "old"), window("w2", "two")],
            ..Default::default()
        });
        let mut after_state = SessionState {
            name: "work".into(),
            windows: vec![window("w1", "new"), window("w3", "three")],
            ..Default::default()
        };
        after_state.windows[0].agent_state = "working".into();
        let after = snapshot_lifecycle(&after_state);

        let events = diff_lifecycle(&before, &after);
        assert!(events.contains(&LifecycleEvent::WindowCreated {
            id: "w3".into(),
            title: "three".into()
        }));
        assert!(events.contains(&LifecycleEvent::WindowClosed { id: "w2".into() }));
        assert!(events.contains(&LifecycleEvent::WindowRetitled {
            id: "w1".into(),
            title: "new".into()
        }));
        assert!(events.contains(&LifecycleEvent::AgentStateChanged {
            id: "w1".into(),
            state: "working".into()
        }));
    }

    #[test]
    fn lifecycle_diff_no_change_is_empty() {
        let s = SessionState {
            name: "work".into(),
            windows: vec![window("w1", "t")],
            ..Default::default()
        };
        let snap = snapshot_lifecycle(&s);
        assert!(diff_lifecycle(&snap, &snap).is_empty());
    }
}
