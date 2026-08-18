//! Agent section: filtering, sorting, and priority.
//!
//! Ported from Go TUIOS `internal/app/sidebar_agents.go`. The agents section
//! lists every pane running an agent, sorted by priority or recency, filtered
//! to all sessions or just the current one.

use super::{AgentEntry, NavRow, RowKind};

/// The agent filter values.
pub const FILTER_ALL: &str = "all";
pub const FILTER_SESSION: &str = "session";

/// The agent sort values.
pub const SORT_PRIORITY: &str = "priority";
pub const SORT_RECENT: &str = "recent";

/// Agent priority: higher = more important.
///
/// - errored = 5
/// - needs_input = 4
/// - working = 3
/// - done (unread) = 2
/// - done (seen) = 1
/// - idle/unknown = 0
pub fn priority(state: &str, done_seen: bool) -> i32 {
    super::agent_priority(state, done_seen)
}

/// Whether a state is an attention state (needs input or errored).
pub fn is_attention(state: &str) -> bool {
    super::sidebar_attention(state)
}

/// Filter agent entries by the given filter mode.
pub fn filter_entries<'a>(
    entries: &'a [AgentEntry],
    filter: &str,
    current_session: &str,
) -> Vec<&'a AgentEntry> {
    match filter {
        FILTER_SESSION => entries.iter().filter(|e| e.session_id == current_session).collect(),
        _ => entries.iter().collect(),
    }
}

/// Sort agent entries by the given sort mode.
pub fn sort_entries<'a>(entries: Vec<&'a AgentEntry>, sort: &str) -> Vec<&'a AgentEntry> {
    let mut sorted = entries;
    match sort {
        SORT_RECENT => {
            sorted.sort_by_key(|e| std::cmp::Reverse(e.state_at));
        }
        _ => {
            // Priority sort: stable, highest priority first.
            sorted.sort_by_key(|e| std::cmp::Reverse(priority(&e.state, e.done_seen)));
        }
    }
    sorted
}

/// Build nav rows from agent entries (for keyboard navigation).
pub fn agent_nav_rows(entries: &[AgentEntry]) -> Vec<NavRow> {
    entries
        .iter()
        .map(|e| NavRow {
            kind: RowKind::Agent,
            session_id: e.session_id.clone(),
            window_id: e.window_id.clone(),
            window_index: e.window_index,
        })
        .collect()
}

/// Collect agent entries from sidebar rows.
pub fn collect_from_rows(rows: &[super::SidebarRow], session_label: &str) -> Vec<AgentEntry> {
    rows.iter()
        .filter(|r| r.kind == RowKind::Window && !r.agent_state.is_empty() && r.agent_state != "none")
        .map(|r| AgentEntry {
            session_id: r.session.clone().unwrap_or_default(),
            window_id: r.window_id.clone().unwrap_or_default(),
            title: r.label.clone(),
            state: r.agent_state.clone(),
            done_seen: r.done_seen,
            state_at: r.agent_state_at,
            window_index: r.window.map(|i| i as i32).unwrap_or(-1),
            session_label: session_label.to_string(),
            foreign: r.foreign,
        })
        .collect()
}

/// The count of attention entries (needs_input or errored).
pub fn attention_count(entries: &[AgentEntry]) -> usize {
    entries.iter().filter(|e| is_attention(&e.state)).count()
}

/// The count of unread done entries.
pub fn unread_done_count(entries: &[AgentEntry]) -> usize {
    entries
        .iter()
        .filter(|e| e.state == "done" && !e.done_seen)
        .count()
}

/// The count of working entries.
pub fn working_count(entries: &[AgentEntry]) -> usize {
    entries.iter().filter(|e| e.state == "working").count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(session: &str, state: &str, seen: bool, at: u64) -> AgentEntry {
        AgentEntry {
            session_id: session.into(),
            window_id: "w1".into(),
            title: "test".into(),
            state: state.into(),
            done_seen: seen,
            state_at: at,
            window_index: 0,
            session_label: session.into(),
            foreign: false,
        }
    }

    #[test]
    fn priority_ranking() {
        assert_eq!(priority("errored", false), 5);
        assert_eq!(priority("needs_input", false), 4);
        assert_eq!(priority("working", false), 3);
        assert_eq!(priority("done", false), 2);
        assert_eq!(priority("done", true), 1);
        assert_eq!(priority("idle", false), 0);
    }

    #[test]
    fn filter_all_returns_everything() {
        let entries = vec![
            entry("a", "working", false, 0),
            entry("b", "done", false, 0),
        ];
        let filtered = filter_entries(&entries, FILTER_ALL, "a");
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn filter_session_returns_only_matching() {
        let entries = vec![
            entry("a", "working", false, 0),
            entry("b", "done", false, 0),
        ];
        let filtered = filter_entries(&entries, FILTER_SESSION, "a");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].session_id, "a");
    }

    #[test]
    fn sort_priority_puts_errored_first() {
        let entries = [
            entry("a", "working", false, 0),
            entry("b", "errored", false, 0),
            entry("c", "idle", false, 0),
        ];
        let refs: Vec<&AgentEntry> = entries.iter().collect();
        let sorted = sort_entries(refs, SORT_PRIORITY);
        assert_eq!(sorted[0].state, "errored");
        assert_eq!(sorted[1].state, "working");
        assert_eq!(sorted[2].state, "idle");
    }

    #[test]
    fn sort_recent_puts_newest_first() {
        let entries = [
            entry("a", "working", false, 100),
            entry("b", "done", false, 200),
            entry("c", "idle", false, 50),
        ];
        let refs: Vec<&AgentEntry> = entries.iter().collect();
        let sorted = sort_entries(refs, SORT_RECENT);
        assert_eq!(sorted[0].state_at, 200);
        assert_eq!(sorted[1].state_at, 100);
        assert_eq!(sorted[2].state_at, 50);
    }

    #[test]
    fn attention_count_correct() {
        let entries = vec![
            entry("a", "working", false, 0),
            entry("b", "needs_input", false, 0),
            entry("c", "errored", false, 0),
            entry("d", "done", false, 0),
        ];
        assert_eq!(attention_count(&entries), 2);
    }

    #[test]
    fn unread_done_count_correct() {
        let entries = vec![
            entry("a", "done", false, 0),
            entry("b", "done", true, 0),
            entry("c", "working", false, 0),
        ];
        assert_eq!(unread_done_count(&entries), 1);
    }

    #[test]
    fn working_count_correct() {
        let entries = vec![
            entry("a", "working", false, 0),
            entry("b", "working", false, 0),
            entry("c", "done", false, 0),
        ];
        assert_eq!(working_count(&entries), 2);
    }
}
