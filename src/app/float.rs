//! Floating panes — terminal windows that float above the tiled layout.
//!
//! A floating pane is a `Window` (a live PTY + emulator) that is not part of
//! any workspace's BSP tree. It carries its own screen rect and z-order, is
//! composited above the tiled panes, and can be moved, resized, and cycled
//! with the keyboard or mouse. The window keeps running while floating, so
//! hiding and re-showing a float never interrupts its process.
//!
//! Zellij and tmux 3.7+ both ship floating panes; this is TermOS's port of
//! that interaction model on top of the existing window/emulator machinery.

use crate::layout::{Rect, ResizeEdge};

/// The smallest a floating pane can shrink to (keeps room for borders +
/// content).
pub const FLOAT_MIN_W: i32 = 20;
/// The smallest a floating pane can shrink to (keeps room for borders +
/// content).
pub const FLOAT_MIN_H: i32 = 8;

/// One floating terminal pane.
#[derive(Debug, Clone)]
pub struct FloatPane {
    /// Index into `Os::windows`.
    pub window: usize,
    /// The workspace the float lives on (hidden on other workspaces).
    pub workspace: i32,
    /// Screen-space top-left corner.
    pub x: i32,
    pub y: i32,
    /// Outer size in cells (includes the border ring).
    pub w: i32,
    pub h: i32,
    /// Z-order; higher floats render in front.
    pub z: i32,
    /// Always-on-top: pinned floats stay above unpinned ones regardless of
    /// z-order (raise never moves an unpinned float above a pinned one).
    pub pinned: bool,
    /// Modal: while a modal float is focused, interaction with every other
    /// pane is blocked until the modal state is toggled off (`Ctrl+B F o`)
    /// or the pane is closed.
    pub modal: bool,
}

impl FloatPane {
    /// The float's screen rect.
    pub fn rect(&self) -> Rect {
        Rect {
            x: self.x,
            y: self.y,
            w: self.w,
            h: self.h,
        }
    }

    /// Whether a screen-space cell falls inside the float.
    pub fn contains(&self, column: i32, row: i32) -> bool {
        column >= self.x && column < self.x + self.w && row >= self.y && row < self.y + self.h
    }
}

/// What an in-progress float drag does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloatDragKind {
    /// Moving the whole pane (grab anywhere on the top border row).
    Move,
    /// Resizing from one edge.
    Resize(ResizeEdge),
}

/// Tracks an in-progress mouse drag on a floating pane (move or resize).
///
/// The start rect is captured so each drag is absolute (the new rect is
/// derived from `start_rect` + the cursor delta), which is stable under rapid
/// mouse motion.
#[derive(Debug, Clone, Copy)]
pub struct FloatDragState {
    /// The window being dragged.
    pub window: usize,
    /// Move vs resize.
    pub kind: FloatDragKind,
    /// Cursor position at grab time.
    pub start_x: i32,
    pub start_y: i32,
    /// The float rect captured at grab time.
    pub start_rect: Rect,
}

/// The default geometry for a newly-floated pane: 60% of the workspace,
/// centered.
pub fn default_float_rect(bounds: Rect) -> Rect {
    let w = (bounds.w * 3 / 5).clamp(FLOAT_MIN_W, bounds.w.max(FLOAT_MIN_W));
    let h = (bounds.h * 3 / 5).clamp(FLOAT_MIN_H, bounds.h.max(FLOAT_MIN_H));
    Rect {
        x: bounds.x + (bounds.w - w) / 2,
        y: bounds.y + (bounds.h - h) / 2,
        w,
        h,
    }
}

/// Clamp a rect so it stays fully inside the workspace bounds.
pub fn clamp_rect(r: Rect, bounds: Rect) -> Rect {
    let w = r.w.clamp(FLOAT_MIN_W, bounds.w.max(FLOAT_MIN_W));
    let h = r.h.clamp(FLOAT_MIN_H, bounds.h.max(FLOAT_MIN_H));
    let x = r.x.clamp(bounds.x, (bounds.x + bounds.w - w).max(bounds.x));
    let y = r.y.clamp(bounds.y, (bounds.y + bounds.h - h).max(bounds.y));
    Rect { x, y, w, h }
}

/// The border interaction a screen cell starts on a float, if any.
///
/// The top border row is a move handle (the pane's title lives there); any
/// other border edge starts a resize. Corners on the top row move.
pub fn float_edge_at(f: &FloatPane, column: i32, row: i32) -> Option<FloatDragKind> {
    if !f.contains(column, row) {
        return None;
    }
    let on_border = column == f.x
        || column == f.x + f.w - 1
        || row == f.y
        || row == f.y + f.h - 1;
    if !on_border {
        return None;
    }
    if row == f.y {
        return Some(FloatDragKind::Move);
    }
    let edge = if column == f.x {
        ResizeEdge::Left
    } else if column == f.x + f.w - 1 {
        ResizeEdge::Right
    } else if row == f.y + f.h - 1 {
        ResizeEdge::Bottom
    } else {
        ResizeEdge::Top
    };
    Some(FloatDragKind::Resize(edge))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds() -> Rect {
        Rect {
            x: 0,
            y: 0,
            w: 80,
            h: 24,
        }
    }

    fn pane(x: i32, y: i32, w: i32, h: i32, z: i32) -> FloatPane {
        FloatPane {
            window: 0,
            workspace: 1,
            x,
            y,
            w,
            h,
            z,
            pinned: false,
            modal: false,
        }
    }

    #[test]
    fn default_rect_is_centered_three_fifths() {
        let r = default_float_rect(bounds());
        assert_eq!(r.w, 48);
        assert_eq!(r.h, 14);
        assert_eq!(r.x, 16); // (80 - 48) / 2
        assert_eq!(r.y, 5); // (24 - 14) / 2
    }

    #[test]
    fn default_rect_small_screen() {
        let r = default_float_rect(Rect {
            x: 0,
            y: 0,
            w: 30,
            h: 10,
        });
        assert!(r.w >= FLOAT_MIN_W);
        assert!(r.h >= FLOAT_MIN_H);
        assert!(r.w <= 30 && r.h <= 10);
    }

    #[test]
    fn clamp_keeps_rect_inside() {
        let r = clamp_rect(
            Rect {
                x: -5,
                y: 40,
                w: 50,
                h: 10,
            },
            bounds(),
        );
        assert!(r.x >= 0);
        assert!(r.y + r.h <= 24);
        assert_eq!(r.w, 50);
    }

    #[test]
    fn clamp_shrinks_oversized() {
        let r = clamp_rect(
            Rect {
                x: 0,
                y: 0,
                w: 200,
                h: 100,
            },
            bounds(),
        );
        assert_eq!(r.w, 80);
        assert_eq!(r.h, 24);
    }

    #[test]
    fn contains_hits_inside_only() {
        let f = pane(10, 5, 30, 12, 1);
        assert!(f.contains(10, 5));
        assert!(f.contains(39, 16));
        assert!(!f.contains(40, 16));
        assert!(!f.contains(10, 17));
    }

    #[test]
    fn top_row_is_move() {
        let f = pane(10, 5, 30, 12, 1);
        assert_eq!(float_edge_at(&f, 10, 5), Some(FloatDragKind::Move));
        assert_eq!(float_edge_at(&f, 25, 5), Some(FloatDragKind::Move));
        assert_eq!(float_edge_at(&f, 39, 5), Some(FloatDragKind::Move));
    }

    #[test]
    fn side_edges_resize() {
        let f = pane(10, 5, 30, 12, 1);
        assert_eq!(
            float_edge_at(&f, 10, 10),
            Some(FloatDragKind::Resize(ResizeEdge::Left))
        );
        assert_eq!(
            float_edge_at(&f, 39, 10),
            Some(FloatDragKind::Resize(ResizeEdge::Right))
        );
        assert_eq!(
            float_edge_at(&f, 25, 16),
            Some(FloatDragKind::Resize(ResizeEdge::Bottom))
        );
    }

    #[test]
    fn interior_is_not_a_handle() {
        let f = pane(10, 5, 30, 12, 1);
        assert_eq!(float_edge_at(&f, 25, 10), None);
    }
}
