//! The session tree — the shared data model behind session-management
//! surfaces (switcher, sidebar). Ported from TUIOS `internal/sessiontree`,
//! which is pure data with no rendering or app imports.

use serde::{Deserialize, Serialize};

/// Distinguishes a session row from a window row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeKind {
    Session,
    Window,
}

/// One row in the tree: a session, or a window under it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub kind: NodeKind,
    /// Session name for a session node, window ID for a window node.
    pub id: String,
    pub title: String,
    /// Raw agent-state string ("" | "working" | "needs_input" | "idle" |
    /// "done" | "errored").
    pub agent_state: String,
    /// The unread bit of a "done" state; meaningless for other states.
    pub done_seen: bool,
    /// Unix nanosecond stamp of when the pane entered `agent_state`, 0 unknown.
    pub state_at: i64,
    pub workspace: i32,
    pub attached: bool,
    pub is_current: bool,
    pub window_count: usize,
    pub restored: bool,
    pub children: Vec<Node>,
}

/// The full set of sessions, each with its windows when known.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Tree {
    pub sessions: Vec<Node>,
}

/// Caller's per-window data.
#[derive(Debug, Clone)]
pub struct WindowInput {
    pub id: String,
    pub title: String,
    pub agent_state: String,
    pub done_seen: bool,
    pub state_at: i64,
    pub focused: bool,
    pub workspace: i32,
}

/// Caller's per-session data.
#[derive(Debug, Clone)]
pub struct SessionInput {
    pub name: String,
    /// The user's label; never reaches ID. Empty when unset.
    pub display_name: String,
    pub attached: bool,
    pub is_current: bool,
    pub window_count: usize,
    pub restored: bool,
    pub current_workspace: i32,
    pub windows: Vec<WindowInput>,
}

/// Rank agent states so a session's roll-up surfaces urgent states. Higher
/// wins: errored > needs_input > done-unseen > working > done-seen > idle.
pub fn agent_rank(state: &str, done_seen: bool) -> i32 {
    match state {
        "errored" => 6,
        "needs_input" => 5,
        "done" => {
            if done_seen {
                2
            } else {
                4
            }
        }
        "working" => 3,
        "idle" => 1,
        _ => 0,
    }
}

/// The highest-priority state among the given raw states, treating every
/// "done" as unseen.
pub fn roll_up_state(states: &[String]) -> String {
    let mut best = String::new();
    let mut best_rank = 0;
    for s in states {
        if agent_rank(s, false) > best_rank {
            best_rank = agent_rank(s, false);
            best = s.clone();
        }
    }
    best
}

/// Build one session node, rolling its windows' states up and disambiguating
/// colliding titles.
pub fn build_session(s: SessionInput) -> Node {
    let title = if s.display_name.is_empty() {
        s.name.clone()
    } else {
        s.display_name.clone()
    };
    let mut node = Node {
        kind: NodeKind::Session,
        id: s.name,
        title,
        agent_state: String::new(),
        done_seen: false,
        state_at: 0,
        workspace: s.current_workspace,
        attached: s.attached,
        is_current: s.is_current,
        window_count: s.window_count,
        restored: s.restored,
        children: Vec::new(),
    };
    if s.windows.is_empty() {
        return node;
    }

    let mut children = Vec::with_capacity(s.windows.len());
    let mut best_rank = 0;
    for w in s.windows {
        let r = agent_rank(&w.agent_state, w.done_seen);
        if r > best_rank {
            node.agent_state = w.agent_state.clone();
            node.done_seen = w.done_seen;
            best_rank = r;
        }
        children.push(Node {
            kind: NodeKind::Window,
            id: w.id,
            title: w.title,
            agent_state: w.agent_state,
            done_seen: w.done_seen,
            state_at: w.state_at,
            workspace: w.workspace,
            attached: false,
            is_current: w.focused,
            window_count: 0,
            restored: false,
            children: Vec::new(),
        });
    }
    node.children = disambiguate(children);
    node.window_count = node.children.len();
    node
}

/// Make every window row self-identifying by appending its 1-based position to
/// rows whose titles collide.
fn disambiguate(mut children: Vec<Node>) -> Vec<Node> {
    let mut counts = std::collections::HashMap::new();
    for c in &children {
        *counts.entry(c.title.clone()).or_insert(0usize) += 1;
    }
    for (i, c) in children.iter_mut().enumerate() {
        if counts.get(&c.title).copied().unwrap_or(0) > 1 {
            c.title = format!("{} {}", c.title, i + 1);
        }
    }
    children
}

/// Build the full tree, preserving session order.
pub fn build(sessions: Vec<SessionInput>) -> Tree {
    Tree {
        sessions: sessions.into_iter().map(build_session).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(id: &str, title: &str, state: &str) -> WindowInput {
        WindowInput {
            id: id.to_string(),
            title: title.to_string(),
            agent_state: state.to_string(),
            done_seen: false,
            state_at: 0,
            focused: false,
            workspace: 1,
        }
    }

    #[test]
    fn agent_rank_orders_states() {
        assert!(agent_rank("errored", false) > agent_rank("needs_input", false));
        assert!(agent_rank("done", false) > agent_rank("working", false));
        assert!(agent_rank("done", true) < agent_rank("working", false));
        assert_eq!(agent_rank("idle", false), 1);
    }

    #[test]
    fn build_session_rolls_up_state() {
        let node = build_session(SessionInput {
            name: "dev".into(),
            display_name: String::new(),
            attached: false,
            is_current: true,
            window_count: 0,
            restored: false,
            current_workspace: 1,
            windows: vec![win("w0", "Terminal", "working"), win("w1", "Editor", "errored")],
        });
        assert_eq!(node.agent_state, "errored");
        assert_eq!(node.window_count, 2);
        assert_eq!(node.children.len(), 2);
    }

    #[test]
    fn disambiguates_colliding_titles() {
        let node = build_session(SessionInput {
            name: "dev".into(),
            display_name: String::new(),
            attached: false,
            is_current: false,
            window_count: 0,
            restored: false,
            current_workspace: 0,
            windows: vec![win("w0", "Terminal", ""), win("w1", "Terminal", "")],
        });
        let titles: Vec<&str> = node.children.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles, vec!["Terminal 1", "Terminal 2"]);
    }

    #[test]
    fn display_name_wins_title() {
        let node = build_session(SessionInput {
            name: "dev".into(),
            display_name: "Payments API".into(),
            attached: false,
            is_current: false,
            window_count: 0,
            restored: false,
            current_workspace: 0,
            windows: vec![],
        });
        assert_eq!(node.title, "Payments API");
        assert_eq!(node.id, "dev");
    }
}
