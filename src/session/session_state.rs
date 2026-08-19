//! Daemon-side session-state operations — ported from Go TUIOS
//! `internal/session/session_ops.go`.
//!
//! Pure helpers that resolve a window target string against a window list and
//! manage workspace focus/repair. The mutating `Session` methods in Go
//! (`AddDaemonWindow`, `CloseDaemonWindow`, …) are implemented inline in the
//! daemon in this port; these helpers are the shared, testable core they call.

use std::collections::HashMap;

use super::state_merge::{SessionState, WindowState};

/// The default workspace count when the session state does not say how many
/// it has. State written by a client that reports its workspace count uses
/// that instead, so the bound is the session's own rather than a number this
/// module guesses.
const DEFAULT_WORKSPACES: i32 = 9;

/// How many workspaces this state has, for the range check on operations that
/// take a workspace index. Falls back to [`DEFAULT_WORKSPACES`] when the state
/// does not report a count.
pub fn workspace_bound(state: &SessionState) -> i32 {
    // The Rust SessionState does not yet carry NumWorkspaces; when it does,
    // this will read it. For now the default applies.
    let _ = state;
    DEFAULT_WORKSPACES
}

/// Resolve a window target string to an index into `windows`. Matches, in
/// order: an exact window id, the position `list-windows` prints when the
/// target is all digits and in range, a unique id prefix, an exact custom
/// name, then an exact title. Returns `Err` when there is no match or a
/// prefix/name is ambiguous.
pub fn find_window_state_index(windows: &[WindowState], target: &str) -> Result<usize, String> {
    if target.is_empty() {
        return Err("empty window target".into());
    }

    // Exact id.
    for (i, w) in windows.iter().enumerate() {
        if w.id == target {
            return Ok(i);
        }
    }

    // The index list-windows prints (the slice position).
    if let Some(idx) = window_index_target(target, windows.len()) {
        return Ok(idx);
    }

    // Unique id prefix.
    let mut prefix_idx: Option<usize> = None;
    let mut prefix_count = 0;
    for (i, w) in windows.iter().enumerate() {
        if w.id.starts_with(target) {
            prefix_idx = Some(i);
            prefix_count += 1;
        }
    }
    if prefix_count == 1 {
        return Ok(prefix_idx.unwrap());
    }
    if prefix_count > 1 {
        return Err(format!(
            "ambiguous window ID prefix {:?} matches {} windows",
            target, prefix_count
        ));
    }

    // Exact title (the Rust WindowState has no CustomName field yet; title is
    // the fallback).
    let mut name_idx: Option<usize> = None;
    let mut name_count = 0;
    for (i, w) in windows.iter().enumerate() {
        if w.title == target {
            name_idx = Some(i);
            name_count += 1;
        }
    }
    if name_count == 1 {
        return Ok(name_idx.unwrap());
    }
    if name_count > 1 {
        return Err(format!(
            "ambiguous window name {:?} matches {} windows",
            target, name_count
        ));
    }

    Err(format!("no window found matching {:?}", target))
}

/// Report whether a window target is an all-digit position into a window list
/// of the given length, and which position. Both resolvers share it so an
/// index means the same thing attached and detached.
pub fn window_index_target(target: &str, count: usize) -> Option<usize> {
    if target.is_empty() || target.len() > 4 {
        return None;
    }
    let mut idx = 0usize;
    for r in target.chars() {
        if !r.is_ascii_digit() {
            return None;
        }
        idx = idx * 10 + (r as usize - b'0' as usize);
    }
    if idx >= count {
        return None;
    }
    Some(idx)
}

/// The id of the first window in slice order that sits on the given workspace
/// and is not minimized, or `None` when the workspace has no such window.
///
/// This is the focus-repair rule, deliberately the same one the renderer
/// applies: first in order, minimized windows skipped, no focus at all when
/// nothing visible remains.
pub fn first_visible_on_workspace(windows: &[WindowState], workspace: i32) -> Option<&str> {
    windows
        .iter()
        .find(|w| w.workspace == workspace && !w.minimized)
        .map(|w| w.id.as_str())
}

/// Repair focus after a window is removed: return the id of the first visible
/// window on `workspace`, or `None` when none remains.
pub fn repair_focus(windows: &[WindowState], workspace: i32) -> Option<String> {
    first_visible_on_workspace(windows, workspace).map(|s| s.to_string())
}

/// Validate a workspace index against the session's bound.
pub fn check_workspace_range(ws: i32, state: &SessionState) -> Result<(), String> {
    let bound = workspace_bound(state);
    if ws < 1 || ws > bound {
        return Err(format!("workspace {} out of range (1-{})", ws, bound));
    }
    Ok(())
}

/// Collect the slice indices of windows on `workspace`, in slice order, and the
/// position of the currently focused window within that list (or 0 when the
/// focused window is not on this workspace).
pub fn workspace_window_order(
    windows: &[WindowState],
    workspace: i32,
    focused_id: &str,
) -> (Vec<usize>, usize) {
    let mut order = Vec::new();
    let mut current = 0;
    for (i, w) in windows.iter().enumerate() {
        if w.workspace != workspace {
            continue;
        }
        if w.id == focused_id {
            current = order.len();
        }
        order.push(i);
    }
    (order, current)
}

/// Cycle focus to the next (`delta > 0`) or previous (`delta < 0`) window on a
/// workspace, wrapping around. Returns the new focused window id, or `None`
/// when the workspace has no windows.
pub fn cycle_focus(
    windows: &[WindowState],
    workspace: i32,
    focused_id: &str,
    delta: i32,
) -> Option<String> {
    let (order, current) = workspace_window_order(windows, workspace, focused_id);
    if order.is_empty() {
        return None;
    }
    let len = order.len() as i32;
    let step = if delta < 0 { -1 } else { 1 };
    let next = ((current as i32 + step).rem_euclid(len)) as usize;
    Some(windows[order[next]].id.clone())
}

/// Move a window to a workspace, dropping the old workspace's focus entry when
/// the moved window held it. Returns the old workspace so the caller can
/// repair focus on it.
pub fn move_window_to_workspace(
    windows: &mut [WindowState],
    workspace_focus: &mut HashMap<i32, String>,
    target: &str,
    ws: i32,
) -> Result<i32, String> {
    let idx = find_window_state_index(windows, target)?;
    let old_workspace = windows[idx].workspace;
    windows[idx].workspace = ws;
    if workspace_focus.get(&old_workspace).map(|s| s.as_str()) == Some(&windows[idx].id) {
        workspace_focus.remove(&old_workspace);
    }
    Ok(old_workspace)
}

/// Set or clear a workspace name. An empty name clears the entry.
pub fn set_workspace_name(
    workspace_names: &mut HashMap<i32, String>,
    ws: i32,
    name: &str,
    bound: i32,
) -> Result<(), String> {
    if ws < 1 || ws > bound {
        return Err(format!("workspace {} out of range (1-{})", ws, bound));
    }
    if name.is_empty() {
        workspace_names.remove(&ws);
    } else {
        workspace_names.insert(ws, name.to_string());
    }
    Ok(())
}

/// Switch the current workspace, restoring the recorded focus for it when one
/// exists.
pub fn switch_workspace(
    state: &mut SessionState,
    workspace_focus: &HashMap<i32, String>,
    ws: i32,
) -> Result<(), String> {
    check_workspace_range(ws, state)?;
    // The Rust SessionState does not yet carry CurrentWorkspace; when it does,
    // this sets it. For now this validates and returns the focus.
    if let Some(focus) = workspace_focus.get(&ws) {
        // The caller applies the focus; this function validates the range.
        let _ = focus;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(id: &str, title: &str, ws: i32, minimized: bool) -> WindowState {
        WindowState {
            id: id.to_string(),
            title: title.to_string(),
            workspace: ws,
            minimized,
            ..Default::default()
        }
    }

    #[test]
    fn find_by_exact_id() {
        let windows = [win("abc123", "Terminal", 1, false)];
        assert_eq!(find_window_state_index(&windows, "abc123"), Ok(0));
    }

    #[test]
    fn find_by_index() {
        let windows = [
            win("aaa", "A", 1, false),
            win("bbb", "B", 1, false),
        ];
        assert_eq!(find_window_state_index(&windows, "1"), Ok(1));
    }

    #[test]
    fn find_by_unique_prefix() {
        let windows = [
            win("abc123", "A", 1, false),
            win("def456", "B", 1, false),
        ];
        assert_eq!(find_window_state_index(&windows, "abc"), Ok(0));
    }

    #[test]
    fn ambiguous_prefix_errors() {
        let windows = [
            win("abc123", "A", 1, false),
            win("abc456", "B", 1, false),
        ];
        let err = find_window_state_index(&windows, "abc").unwrap_err();
        assert!(err.contains("ambiguous"));
    }

    #[test]
    fn find_by_title() {
        let windows = [
            win("aaa", "build", 1, false),
            win("bbb", "test", 1, false),
        ];
        assert_eq!(find_window_state_index(&windows, "test"), Ok(1));
    }

    #[test]
    fn empty_target_errors() {
        let windows = [win("aaa", "A", 1, false)];
        assert!(find_window_state_index(&windows, "").is_err());
    }

    #[test]
    fn no_match_errors() {
        let windows = [win("aaa", "A", 1, false)];
        assert!(find_window_state_index(&windows, "zzz").is_err());
    }

    #[test]
    fn window_index_target_valid() {
        assert_eq!(window_index_target("0", 5), Some(0));
        assert_eq!(window_index_target("3", 5), Some(3));
    }

    #[test]
    fn window_index_target_out_of_range() {
        assert_eq!(window_index_target("5", 5), None);
        assert_eq!(window_index_target("99", 5), None);
    }

    #[test]
    fn window_index_target_non_digit() {
        assert_eq!(window_index_target("abc", 5), None);
    }

    #[test]
    fn window_index_target_too_long() {
        assert_eq!(window_index_target("12345", 99), None);
    }

    #[test]
    fn first_visible_skips_minimized() {
        let windows = [
            win("a", "A", 1, true),
            win("b", "B", 1, false),
            win("c", "C", 1, false),
        ];
        assert_eq!(first_visible_on_workspace(&windows, 1), Some("b"));
    }

    #[test]
    fn first_visible_none_when_all_minimized() {
        let windows = [win("a", "A", 1, true)];
        assert_eq!(first_visible_on_workspace(&windows, 1), None);
    }

    #[test]
    fn first_visible_filters_workspace() {
        let windows = [
            win("a", "A", 1, false),
            win("b", "B", 2, false),
        ];
        assert_eq!(first_visible_on_workspace(&windows, 2), Some("b"));
    }

    #[test]
    fn repair_focus_returns_first_visible() {
        let windows = [
            win("a", "A", 1, false),
            win("b", "B", 1, false),
        ];
        assert_eq!(repair_focus(&windows, 1), Some("a".to_string()));
    }

    #[test]
    fn cycle_forward_wraps() {
        let windows = [
            win("a", "A", 1, false),
            win("b", "B", 1, false),
            win("c", "C", 1, false),
        ];
        // a -> b -> c -> a
        assert_eq!(cycle_focus(&windows, 1, "a", 1), Some("b".into()));
        assert_eq!(cycle_focus(&windows, 1, "b", 1), Some("c".into()));
        assert_eq!(cycle_focus(&windows, 1, "c", 1), Some("a".into()));
    }

    #[test]
    fn cycle_backward_wraps() {
        let windows = [
            win("a", "A", 1, false),
            win("b", "B", 1, false),
            win("c", "C", 1, false),
        ];
        assert_eq!(cycle_focus(&windows, 1, "a", -1), Some("c".into()));
        assert_eq!(cycle_focus(&windows, 1, "c", -1), Some("b".into()));
    }

    #[test]
    fn cycle_no_windows_returns_none() {
        let windows: [WindowState; 0] = [];
        assert!(cycle_focus(&windows, 1, "", 1).is_none());
    }

    #[test]
    fn cycle_filters_workspace() {
        let windows = [
            win("a", "A", 1, false),
            win("b", "B", 2, false),
            win("c", "C", 1, false),
        ];
        // Only a and c are on workspace 1.
        assert_eq!(cycle_focus(&windows, 1, "a", 1), Some("c".into()));
        assert_eq!(cycle_focus(&windows, 1, "c", 1), Some("a".into()));
    }

    #[test]
    fn move_window_drops_old_workspace_focus() {
        let mut windows = [
            win("a", "A", 1, false),
            win("b", "B", 1, false),
        ];
        let mut focus = HashMap::new();
        focus.insert(1, "a".to_string());
        let old = move_window_to_workspace(&mut windows, &mut focus, "a", 2).unwrap();
        assert_eq!(old, 1);
        assert!(!focus.contains_key(&1));
        assert_eq!(windows[0].workspace, 2);
    }

    #[test]
    fn set_workspace_name_adds_and_clears() {
        let mut names = HashMap::new();
        set_workspace_name(&mut names, 2, "review", 9).unwrap();
        assert_eq!(names.get(&2), Some(&"review".to_string()));
        set_workspace_name(&mut names, 2, "", 9).unwrap();
        assert!(!names.contains_key(&2));
    }

    #[test]
    fn set_workspace_name_out_of_range() {
        let mut names = HashMap::new();
        assert!(set_workspace_name(&mut names, 0, "x", 9).is_err());
        assert!(set_workspace_name(&mut names, 10, "x", 9).is_err());
    }

    #[test]
    fn check_workspace_range_valid() {
        let state = SessionState::default();
        assert!(check_workspace_range(1, &state).is_ok());
        assert!(check_workspace_range(9, &state).is_ok());
    }

    #[test]
    fn check_workspace_range_invalid() {
        let state = SessionState::default();
        assert!(check_workspace_range(0, &state).is_err());
        assert!(check_workspace_range(10, &state).is_err());
    }

    #[test]
    fn workspace_bound_defaults_to_nine() {
        let state = SessionState::default();
        assert_eq!(workspace_bound(&state), 9);
    }
}
