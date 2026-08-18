//! Sixel graphics state — ported from Go TUIOS `internal/vt/sixel_state.go`.
//!
//! Tracks sixel placements by absolute scrollback line so that images
//! scroll naturally with content and can be cleaned up when scrolled out
//! of view or erased.

use std::sync::Mutex;

/// A sixel image placement tied to an absolute scrollback line.
#[derive(Debug, Clone)]
pub struct SixelPlacement {
    /// Absolute scrollback line where the placement starts.
    pub absolute_line: i32,
    /// Column where the placement starts.
    pub column: i32,
    /// Image width in pixels.
    pub width: i32,
    /// Image height in pixels.
    pub height: i32,
    /// Number of terminal rows the image occupies.
    pub rows: i32,
    /// Number of terminal columns the image occupies.
    pub cols: i32,
    /// Raw DCS sequence for passthrough to host terminal.
    pub raw_sequence: Vec<u8>,
}

/// Thread-safe sixel placement store.
pub struct SixelState {
    inner: Mutex<Vec<SixelPlacement>>,
}

impl std::fmt::Debug for SixelState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.inner.lock().unwrap();
        f.debug_struct("SixelState")
            .field("placements", &inner.len())
            .finish()
    }
}

impl SixelState {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Vec::new()),
        }
    }

    pub fn add(&self, p: SixelPlacement) {
        self.inner.lock().unwrap().push(p);
    }

    pub fn clear(&self) {
        self.inner.lock().unwrap().clear();
    }

    /// Remove placements whose absolute line is above the given top line
    /// (i.e., scrolled out of scrollback).
    pub fn remove_above(&self, top_line: i32) {
        self.inner
            .lock()
            .unwrap()
            .retain(|p| p.absolute_line + p.rows > top_line);
    }

    /// Remove placements that intersect erased rows starting at the given
    /// absolute line.
    pub fn remove_intersecting(&self, start_line: i32, row_count: i32) {
        let end_line = start_line + row_count;
        self.inner.lock().unwrap().retain(|p| {
            let p_end = p.absolute_line + p.rows;
            // Keep if placement ends before erased region or starts after it.
            p_end <= start_line || p.absolute_line >= end_line
        });
    }

    /// Return placements visible within the given absolute line range.
    pub fn visible_placements(&self, top_line: i32, bottom_line: i32) -> Vec<SixelPlacement> {
        self.inner
            .lock()
            .unwrap()
            .iter()
            .filter(|p| {
                let p_end = p.absolute_line + p.rows;
                p_end > top_line && p.absolute_line < bottom_line
            })
            .cloned()
            .collect()
    }

    pub fn count(&self) -> usize {
        self.inner.lock().unwrap().len()
    }

    pub fn all(&self) -> Vec<SixelPlacement> {
        self.inner.lock().unwrap().clone()
    }
}

impl Default for SixelState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placement(line: i32, rows: i32) -> SixelPlacement {
        SixelPlacement {
            absolute_line: line,
            column: 0,
            width: 100,
            height: rows * 16,
            rows,
            cols: 10,
            raw_sequence: vec![],
        }
    }

    #[test]
    fn add_and_visible() {
        let state = SixelState::new();
        state.add(placement(10, 3));
        state.add(placement(20, 5));
        assert_eq!(state.count(), 2);
        let visible = state.visible_placements(0, 15);
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].absolute_line, 10);
    }

    #[test]
    fn remove_above() {
        let state = SixelState::new();
        state.add(placement(0, 3));
        state.add(placement(10, 3));
        state.add(placement(20, 3));
        state.remove_above(10);
        assert_eq!(state.count(), 2);
    }

    #[test]
    fn remove_intersecting() {
        let state = SixelState::new();
        state.add(placement(0, 3));
        state.add(placement(5, 3));
        state.add(placement(10, 3));
        state.remove_intersecting(3, 4);
        // Erased region is lines 3..6 (end = 3+4 = 7).
        // Placement at 0 (rows 0-2, ends at 3) — no overlap, kept.
        // Placement at 5 (rows 5-7, ends at 8) — overlaps lines 5-6, removed.
        // Placement at 10 (rows 10-12) — no overlap, kept.
        assert_eq!(state.count(), 2);

        state.remove_intersecting(0, 20);
        assert_eq!(state.count(), 0);
    }
}
