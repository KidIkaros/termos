//! The sidebar rail — sessions and their windows with agent-state glyphs.
//!
//! Ported in spirit from Go TUIOS `internal/app/sidebar_*.go`: a right-side
//! rail listing the daemon sessions (or, in local mode, the current session's
//! windows), each row carrying its agent state. Navigation is vim-style
//! (j/k), Enter activates a row, Esc leaves.

/// A row's kind: a session header or a window under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Session,
    Window,
}

/// One row of the sidebar rail.
#[derive(Debug, Clone)]
pub struct SidebarRow {
    pub kind: RowKind,
    pub label: String,
    pub detail: String,
    pub session: Option<String>,
    pub window: Option<usize>,
    pub workspace: i32,
    pub agent_state: String,
}

/// The sidebar state.
#[derive(Debug, Clone, Default)]
pub struct Sidebar {
    pub open: bool,
    pub selected: usize,
}

impl Sidebar {
    /// Create a closed sidebar.
    pub fn new() -> Self {
        Self::default()
    }

    /// Toggle the sidebar open/closed.
    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.selected = 0;
    }

    /// Open the sidebar.
    pub fn open(&mut self) {
        self.open = true;
        self.selected = 0;
    }

    /// Close the sidebar.
    pub fn close(&mut self) {
        self.open = false;
        self.selected = 0;
    }

    /// Move the selection by `delta` rows (wrapping).
    pub fn move_selection(&mut self, delta: i32, count: usize) {
        if count == 0 {
            return;
        }
        self.selected = (self.selected as i32 + delta).rem_euclid(count as i32) as usize;
    }
}

/// Build the sidebar rows for the current app state.
///
/// Local mode shows one session node (the current workspace) with its windows;
/// daemon mode shows every session with the attached session's windows
/// expanded underneath.
pub fn build_rows(
    remote_session: Option<&str>,
    remote_sessions: &[crate::session::model::SessionInfo],
    windows: &[crate::terminal::window::Window],
    workspace: i32,
    window_workspace: impl Fn(usize) -> i32,
) -> Vec<SidebarRow> {
    let mut rows = Vec::new();
    if let Some(current) = remote_session {
        // Session rows first.
        let mut sessions: Vec<&crate::session::model::SessionInfo> =
            remote_sessions.iter().collect();
        sessions.sort_by(|a, b| a.name.cmp(&b.name));
        for s in &sessions {
            let detail = if s.attached {
                format!("{} window(s) · attached", s.windows)
            } else {
                format!("{} window(s)", s.windows)
            };
            rows.push(SidebarRow {
                kind: RowKind::Session,
                label: s.name.clone(),
                detail,
                session: Some(s.name.clone()),
                window: None,
                workspace: 0,
                agent_state: String::new(),
            });
            // Expand the attached session's windows under it.
            if s.name == current {
                for (idx, w) in windows.iter().enumerate() {
                    rows.push(SidebarRow {
                        kind: RowKind::Window,
                        label: w.title.clone(),
                        detail: format!("ws {}", window_workspace(idx)),
                        session: None,
                        window: Some(idx),
                        workspace: window_workspace(idx),
                        agent_state: w.agent_state.clone(),
                    });
                }
            }
        }
    } else {
        // Local mode: one session node plus its windows.
        rows.push(SidebarRow {
            kind: RowKind::Session,
            label: format!("workspace {workspace}"),
            detail: format!("{} window(s)", windows.len()),
            session: None,
            window: None,
            workspace,
            agent_state: String::new(),
        });
        for (idx, w) in windows.iter().enumerate() {
            rows.push(SidebarRow {
                kind: RowKind::Window,
                label: w.title.clone(),
                detail: format!("ws {}", window_workspace(idx)),
                session: None,
                window: Some(idx),
                workspace: window_workspace(idx),
                agent_state: w.agent_state.clone(),
            });
        }
    }
    rows
}

/// The agent-state glyph for a window row (mirrors Go's agentStateIndicator).
pub fn agent_glyph(state: &str) -> &'static str {
    match state {
        "working" => "◐",
        "needs_input" => "✋",
        "idle" => "○",
        "done" => "✓",
        "errored" => "✕",
        _ => " ",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::model::SessionInfo;
    use crate::terminal::pty::WinSize;
    use crate::terminal::window::Window;

    fn info(name: &str, attached: bool) -> SessionInfo {
        SessionInfo {
            id: name.into(),
            name: name.into(),
            created_at: 0,
            attached,
            windows: 1,
            restored: false,
        }
    }

    fn window(id: &str, title: &str, agent: &str) -> Window {
        let mut w = Window::without_pty(
            id.to_string(),
            title.to_string(),
            WinSize { cols: 10, rows: 3 },
        );
        w.agent_state = agent.to_string();
        w
    }

    #[test]
    fn local_mode_has_session_and_windows() {
        let windows = vec![window("w0", "alpha", "working"), window("w1", "beta", "")];
        let rows = build_rows(None, &[], &windows, 1, |_| 1);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].kind, RowKind::Session);
        assert_eq!(rows[0].label, "workspace 1");
        assert_eq!(rows[1].window, Some(0));
        assert_eq!(rows[2].window, Some(1));
        assert_eq!(rows[1].agent_state, "working");
    }

    #[test]
    fn remote_mode_lists_sessions_with_expanded_current() {
        let sessions = vec![info("work", true), info("play", false)];
        let windows = vec![window("w0", "alpha", "")];
        let rows = build_rows(Some("work"), &sessions, &windows, 1, |_| 1);
        // Two session rows + one window row under "work".
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].label, "play");
        assert_eq!(rows[1].label, "work");
        assert!(rows[1].detail.contains("attached"));
        assert_eq!(rows[2].kind, RowKind::Window);
    }

    #[test]
    fn selection_wraps() {
        let mut sb = Sidebar::new();
        sb.open();
        sb.move_selection(-1, 3);
        assert_eq!(sb.selected, 2);
        sb.move_selection(1, 3);
        assert_eq!(sb.selected, 0);
    }

    #[test]
    fn toggle_opens_and_closes() {
        let mut sb = Sidebar::new();
        assert!(!sb.open);
        sb.toggle();
        assert!(sb.open);
        sb.toggle();
        assert!(!sb.open);
    }

    #[test]
    fn agent_glyphs() {
        assert_eq!(agent_glyph("working"), "◐");
        assert_eq!(agent_glyph("needs_input"), "✋");
        assert_eq!(agent_glyph("done"), "✓");
        assert_eq!(agent_glyph("errored"), "✕");
        assert_eq!(agent_glyph("idle"), "○");
        assert_eq!(agent_glyph(""), " ");
        assert_eq!(agent_glyph("none"), " ");
    }
}
