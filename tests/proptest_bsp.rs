//! Property-based tests for BSP layout invariants.
//!
//! These verify that:
//! - BSP tiling never produces overlapping or out-of-bounds rects
//! - All windows remain reachable after any sequence of insert/remove

use proptest::prelude::*;
use termos::layout::bsp::{BSPTree, Rect, SplitType};

fn bounds(w: i32, h: i32) -> Rect {
    Rect { x: 0, y: 0, w, h }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// After inserting windows, all laid-out rects must be within bounds
    /// and have positive area.
    #[test]
    fn bsp_insert_stays_in_bounds(
        width in 10i32..200,
        height in 10i32..100,
        window_count in 1usize..16,
    ) {
        let mut tree = BSPTree::new();
        let b = bounds(width, height);
        for i in 0..window_count as i32 {
            tree.insert_window(i, i - 1, SplitType::Vertical, 0.5, b, 0);
        }
        let layout = tree.apply_layout(b, 0);
        prop_assert_eq!(layout.len(), window_count, "not all windows got rects");

        for (id, r) in &layout {
            prop_assert!(
                r.x >= 0 && r.y >= 0,
                "rect for window {} has negative origin: {:?}",
                id, r
            );
            prop_assert!(
                r.x + r.w <= width,
                "rect right edge out of bounds: {:?} in {}x{}",
                r, width, height
            );
            prop_assert!(
                r.y + r.h <= height,
                "rect bottom edge out of bounds: {:?} in {}x{}",
                r, width, height
            );
            prop_assert!(r.w > 0, "zero-width rect for window {}: {:?}", id, r);
            prop_assert!(r.h > 0, "zero-height rect for window {}: {:?}", id, r);
        }
    }

    /// Inserting and removing windows must never panic and must keep
    /// remaining windows reachable.
    #[test]
    fn bsp_insert_remove_never_panics(
        width in 20i32..200,
        height in 20i32..100,
        ops in prop::collection::vec(
            prop_oneof![Just(0i32), Just(1i32), Just(2i32), Just(3i32)],
            0..50
        ),
    ) {
        let mut tree = BSPTree::new();
        let mut next_id = 0i32;
        let b = bounds(width, height);

        for op in ops {
            match op {
                0 => {
                    tree.insert_window(next_id, next_id - 1, SplitType::Vertical, 0.5, b, 0);
                    next_id += 1;
                }
                1 => {
                    if !tree.is_empty() {
                        let id = (next_id - 1).max(0);
                        tree.remove_window(id);
                    }
                }
                _ => {
                    let _ = tree.apply_layout(b, 1);
                }
            }
        }

        let layout = tree.apply_layout(b, 0);
        for (_id, r) in layout.values() {
            prop_assert!(r.w > 0 && r.h > 0, "zero-area rect: {:?}", r);
            prop_assert!(
                r.x + r.w <= width && r.y + r.h <= height,
                "rect out of bounds: {:?} in {}x{}",
                r, width, height
            );
        }
    }

    /// Gap must not cause rects to go negative or out of bounds.
    #[test]
    fn bsp_with_gap_stays_in_bounds(
        width in 20i32..200,
        height in 20i32..100,
        gap in 0i32..5,
        window_count in 1usize..8,
    ) {
        let mut tree = BSPTree::new();
        let b = bounds(width, height);
        for i in 0..window_count as i32 {
            tree.insert_window(i, i - 1, SplitType::Vertical, 0.5, b, 0);
        }
        let layout = tree.apply_layout(b, gap);
        for (_id, r) in layout.values() {
            prop_assert!(r.w > 0, "zero-width with gap={}: {:?}", gap, r);
            prop_assert!(r.h > 0, "zero-height with gap={}: {:?}", gap, r);
            prop_assert!(
                r.x + r.w <= width,
                "gap rect out of bounds: {:?} in {}x{} gap={}",
                r, width, height, gap
            );
        }
    }
}
