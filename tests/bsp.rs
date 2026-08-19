//! Integration tests for the BSP tree — verifying the port preserves the
//! semantics of TUIOS `internal/layout/bsp.go`.

use termos::layout::{AutoScheme, BSPTree, Rect, SplitType};

fn bounds() -> Rect {
    Rect {
        x: 0,
        y: 0,
        w: 120,
        h: 40,
    }
}

#[test]
fn first_window_is_root() {
    let mut tree = BSPTree::new();
    tree.insert_window(1, -1, SplitType::None, 0.5, bounds(), 0);
    assert_eq!(tree.window_count(), 1);
    assert!(tree.has_window(1));
    let layout = tree.apply_layout(bounds(), 0);
    assert_eq!(layout[&1], bounds());
}

#[test]
fn split_places_windows_side_by_side_or_stacked() {
    let mut tree = BSPTree::new();
    tree.insert_window(1, -1, SplitType::None, 0.5, bounds(), 0);
    tree.insert_window(2, 1, SplitType::Vertical, 0.5, bounds(), 0);
    let layout = tree.apply_layout(bounds(), 0);
    assert_eq!(layout.len(), 2);
    assert_eq!(layout[&1].w, 60);
    assert_eq!(layout[&2].x, 60);
    assert_eq!(layout[&2].w, 60);
    // Both panes span the full height.
    assert_eq!(layout[&1].h, 40);
    assert_eq!(layout[&2].h, 40);
}

#[test]
fn remove_window_collapses_tree() {
    let mut tree = BSPTree::new();
    tree.insert_window(1, -1, SplitType::None, 0.5, bounds(), 0);
    tree.insert_window(2, 1, SplitType::Vertical, 0.5, bounds(), 0);
    tree.remove_window(2);
    assert_eq!(tree.window_count(), 1);
    assert!(tree.has_window(1));
    // The surviving window takes the whole bounds again.
    let layout = tree.apply_layout(bounds(), 0);
    assert_eq!(layout[&1], bounds());
}

#[test]
fn nested_splits_follow_spiral() {
    let mut tree = BSPTree::new();
    tree.set_auto_scheme(AutoScheme::Spiral);
    tree.insert_window(1, -1, SplitType::None, 0.5, bounds(), 0);
    tree.insert_window(2, 1, SplitType::None, 0.5, bounds(), 0);
    tree.insert_window(3, 2, SplitType::None, 0.5, bounds(), 0);
    let layout = tree.apply_layout(bounds(), 0);
    assert_eq!(layout.len(), 3);
    // Every pane must have a positive area.
    for rect in layout.values() {
        assert!(rect.w >= 1);
        assert!(rect.h >= 1);
    }
}

#[test]
fn equalize_ratios_resets_all() {
    let mut tree = BSPTree::new();
    tree.insert_window(1, -1, SplitType::None, 0.5, bounds(), 0);
    tree.insert_window(2, 1, SplitType::Vertical, 0.7, bounds(), 0);
    tree.insert_window(3, 2, SplitType::Horizontal, 0.3, bounds(), 0);
    tree.equalize_ratios();
    let layout = tree.apply_layout(bounds(), 0);
    // After equalization, the first split is 50/50.
    assert_eq!(layout[&1].w, 60);
}

#[test]
fn gap_reserves_separator_cell() {
    let mut tree = BSPTree::new();
    tree.insert_window(1, -1, SplitType::None, 0.5, bounds(), 0);
    tree.insert_window(2, 1, SplitType::Vertical, 0.5, bounds(), 1);
    let layout = tree.apply_layout(bounds(), 1);
    // With gap=1, the separator cell sits between the panes.
    assert_eq!(layout[&1].w + layout[&2].w + 1, 120);
    // CollectSplits reports the divider.
    let splits = tree.collect_splits(bounds(), 1);
    assert_eq!(splits.len(), 1);
    assert!(splits[0].vertical);
}

#[test]
fn serialize_round_trips() {
    let mut tree = BSPTree::new();
    tree.insert_window(1, -1, SplitType::None, 0.5, bounds(), 0);
    tree.insert_window(2, 1, SplitType::Vertical, 0.6, bounds(), 0);
    tree.insert_window(3, 2, SplitType::Horizontal, 0.4, bounds(), 0);

    let serialized = tree.serialize();
    let restored = BSPTree::deserialize(&serialized);
    assert_eq!(restored.window_count(), 3);
    let a = tree.apply_layout(bounds(), 0);
    let b = restored.apply_layout(bounds(), 0);
    assert_eq!(a, b);
}

#[test]
fn rotate_split_swaps_axis() {
    let mut tree = BSPTree::new();
    tree.insert_window(1, -1, SplitType::None, 0.5, bounds(), 0);
    tree.insert_window(2, 1, SplitType::Vertical, 0.5, bounds(), 0);
    let before = tree.apply_layout(bounds(), 0);
    assert_eq!(before[&1].w, 60);

    tree.rotate_split(1);
    let after = tree.apply_layout(bounds(), 0);
    // Now the split is horizontal: the panes stack.
    assert_eq!(after[&1].h, 20);
}

#[test]
fn resize_split_moves_divider() {
    let mut tree = BSPTree::new();
    tree.insert_window(1, -1, SplitType::None, 0.5, bounds(), 0);
    tree.insert_window(2, 1, SplitType::Vertical, 0.5, bounds(), 0);

    use termos::layout::ResizeEdge;
    // Drag the divider to x=30 (from window 1's right edge).
    let moved = tree.resize_split(1, ResizeEdge::Right, 30, bounds(), 0);
    assert!(moved);
    let layout = tree.apply_layout(bounds(), 0);
    assert_eq!(layout[&1].w, 30);
    assert_eq!(layout[&2].x, 30);
}

// -----------------------------------------------------------------------
// Stacked pane tests
// -----------------------------------------------------------------------

#[test]
fn push_to_stack_wraps_two_windows() {
    let mut tree = BSPTree::new();
    tree.insert_window(1, -1, SplitType::None, 0.5, bounds(), 0);
    tree.insert_window(2, 1, SplitType::Vertical, 0.5, bounds(), 0);
    // Window 1 is left, window 2 is right.
    let before = tree.apply_layout(bounds(), 0);
    assert_eq!(before[&1].w, 60);
    assert_eq!(before[&2].x, 60);

    // Stack 1 (active) with 2 (inactive).
    tree.push_to_stack(1, 2);
    assert_eq!(tree.stack_count(1), 2);
    assert_eq!(tree.stack_count(2), 2);
    let windows = tree.stack_windows(1);
    assert!(windows.contains(&1));
    assert!(windows.contains(&2));

    // Layout: only the active pane gets real space; the other is a 1-cell
    // title bar.
    let after = tree.apply_layout(bounds(), 0);
    // Both windows should still be in the layout.
    assert!(after.contains_key(&1));
    assert!(after.contains_key(&2));
}

#[test]
fn pop_from_stack_restores_as_split() {
    let mut tree = BSPTree::new();
    tree.insert_window(1, -1, SplitType::None, 0.5, bounds(), 0);
    tree.insert_window(2, 1, SplitType::Vertical, 0.5, bounds(), 0);
    tree.push_to_stack(1, 2);
    assert_eq!(tree.stack_count(1), 2);

    // Pop window 2 out of the stack.
    let popped = tree.pop_from_stack(2);
    assert!(popped);
    assert_eq!(tree.stack_count(1), 1);
    assert_eq!(tree.stack_count(2), 1);
    // Both windows still exist in the layout.
    let layout = tree.apply_layout(bounds(), 0);
    assert!(layout.contains_key(&1));
    assert!(layout.contains_key(&2));
}

#[test]
fn cycle_stack_focus_swaps_active() {
    let mut tree = BSPTree::new();
    tree.insert_window(1, -1, SplitType::None, 0.5, bounds(), 0);
    tree.insert_window(2, 1, SplitType::Vertical, 0.5, bounds(), 0);
    tree.push_to_stack(1, 2);
    // 1 is active (left), 2 is title bar (right).
    assert_eq!(tree.stack_depth(1), 0);
    assert_eq!(tree.stack_depth(2), 1);

    // Cycle forward: 2 becomes active.
    let new_active = tree.cycle_stack_focus(1, true);
    assert_eq!(new_active, 2);
    assert_eq!(tree.stack_depth(2), 0);
    assert_eq!(tree.stack_depth(1), 1);

    // Cycle back: 1 becomes active again.
    let new_active = tree.cycle_stack_focus(2, false);
    assert_eq!(new_active, 1);
}

#[test]
fn non_stacked_window_stack_count_is_one() {
    let mut tree = BSPTree::new();
    tree.insert_window(1, -1, SplitType::None, 0.5, bounds(), 0);
    assert_eq!(tree.stack_count(1), 1);
    assert_eq!(tree.find_stack_root(1), None);
}

#[test]
fn push_to_stack_noop_on_same_window() {
    let mut tree = BSPTree::new();
    tree.insert_window(1, -1, SplitType::None, 0.5, bounds(), 0);
    tree.push_to_stack(1, 1); // noop
    assert_eq!(tree.stack_count(1), 1);
}

#[test]
fn three_window_layout_after_stack() {
    let mut tree = BSPTree::new();
    tree.insert_window(1, -1, SplitType::None, 0.5, bounds(), 0);
    tree.insert_window(2, 1, SplitType::Vertical, 0.5, bounds(), 0);
    tree.insert_window(3, 1, SplitType::Vertical, 0.5, bounds(), 0);
    // Stack 2 (active) with 1 (inactive).
    tree.push_to_stack(2, 1);
    assert_eq!(tree.stack_count(2), 2);
    assert_eq!(tree.stack_count(1), 2);
    // Window 3 is unaffected.
    assert_eq!(tree.stack_count(3), 1);

    let layout = tree.apply_layout(bounds(), 0);
    assert_eq!(layout.len(), 3);
}
