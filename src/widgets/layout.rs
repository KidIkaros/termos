//! Widget layout engine — grid-based positioning for dashboard widgets.
//!
//! The layout divides the available space into a grid of slots. Each slot
//! can hold one widget. The grid is configured via TOML:
//!
//! ```toml
//! [dashboard]
//! columns = 3
//! rows = 2
//! gap = 1
//!
//! [[dashboard.widgets]]
//! id = "cpu"
//! col = 0
//! row = 0
//! width = 1
//! height = 1
//! ```

use ratatui::layout::Rect;

/// A slot in the widget grid.
#[derive(Debug, Clone)]
pub struct WidgetSlot {
    /// Widget ID to render in this slot.
    pub widget_id: String,
    /// Grid column (0-indexed).
    pub col: u16,
    /// Grid row (0-indexed).
    pub row: u16,
    /// Number of columns this slot spans.
    pub width: u16,
    /// Number of rows this slot spans.
    pub height: u16,
}

/// The layout configuration for the widget dashboard.
#[derive(Debug, Clone)]
pub struct WidgetLayout {
    /// Number of columns in the grid.
    pub columns: u16,
    /// Number of rows in the grid.
    pub rows: u16,
    /// Gap between widgets (in cells).
    pub gap: u16,
    /// Widget slots in the grid.
    pub slots: Vec<WidgetSlot>,
    /// Whether the dashboard overlay is visible.
    pub visible: bool,
    /// Dashboard position: top, bottom, or overlay.
    pub position: DashboardPosition,
}

/// Where the dashboard is displayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DashboardPosition {
    /// Full-screen overlay (toggled with a keybinding).
    Overlay,
    /// Side panel (left or right).
    Side(Side),
    /// Bottom bar (above the status bar).
    Bottom,
}

/// Which side for a side panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

impl WidgetLayout {
    /// Create a default layout: 3 columns, 2 rows, gap of 1.
    pub fn new() -> Self {
        Self {
            columns: 3,
            rows: 2,
            gap: 1,
            slots: Vec::new(),
            visible: false,
            position: DashboardPosition::Overlay,
        }
    }

    /// Create a layout from a list of slots.
    pub fn with_slots(slots: Vec<WidgetSlot>) -> Self {
        let mut layout = Self::new();
        layout.slots = slots;
        layout
    }

    /// Calculate the pixel rectangles for each slot given the total area.
    pub fn compute_rects(&self, area: Rect) -> Vec<(String, Rect)> {
        if self.slots.is_empty() {
            return Vec::new();
        }

        let gap = self.gap;
        let total_width = area.width.saturating_sub(gap * (self.columns.saturating_sub(1)));
        let total_height = area.height.saturating_sub(gap * (self.rows.saturating_sub(1)));
        let cell_width = total_width / self.columns.max(1);
        let cell_height = total_height / self.rows.max(1);

        self.slots
            .iter()
            .map(|slot| {
                let x = area.x + slot.col * (cell_width + gap);
                let y = area.y + slot.row * (cell_height + gap);
                let w = cell_width * slot.width;
                let h = cell_height * slot.height;
                let rect = Rect {
                    x,
                    y,
                    width: w.min(area.x + area.width - x),
                    height: h.min(area.y + area.height - y),
                };
                (slot.widget_id.clone(), rect)
            })
            .collect()
    }

    /// Add a widget slot.
    pub fn add_slot(&mut self, slot: WidgetSlot) {
        self.slots.push(slot);
    }

    /// Remove a widget slot by widget ID.
    pub fn remove_slot(&mut self, widget_id: &str) {
        self.slots.retain(|s| s.widget_id != widget_id);
    }

    /// Toggle dashboard visibility.
    pub fn toggle(&mut self) {
        self.visible = !self.visible;
    }

    /// Auto-layout: place widgets in a grid, filling left-to-right, top-to-bottom.
    pub fn auto_layout(widget_ids: &[String], columns: u16, rows: u16, gap: u16) -> Self {
        let mut slots = Vec::new();
        for (i, id) in widget_ids.iter().enumerate() {
            let col = (i as u16) % columns;
            let row = (i as u16) / columns;
            if row >= rows {
                break; // No more room
            }
            slots.push(WidgetSlot {
                widget_id: id.clone(),
                col,
                row,
                width: 1,
                height: 1,
            });
        }
        Self {
            columns,
            rows,
            gap,
            slots,
            visible: false,
            position: DashboardPosition::Overlay,
        }
    }
}

impl Default for WidgetLayout {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_rects_basic() {
        let layout = WidgetLayout {
            columns: 2,
            rows: 2,
            gap: 0,
            slots: vec![
                WidgetSlot { widget_id: "a".into(), col: 0, row: 0, width: 1, height: 1 },
                WidgetSlot { widget_id: "b".into(), col: 1, row: 0, width: 1, height: 1 },
                WidgetSlot { widget_id: "c".into(), col: 0, row: 1, width: 1, height: 1 },
                WidgetSlot { widget_id: "d".into(), col: 1, row: 1, width: 1, height: 1 },
            ],
            visible: true,
            position: DashboardPosition::Overlay,
        };

        let area = Rect::new(0, 0, 100, 50);
        let rects = layout.compute_rects(area);
        assert_eq!(rects.len(), 4);

        // Widget "a" at (0,0), 50x25
        assert_eq!(rects[0].1, Rect::new(0, 0, 50, 25));
        // Widget "b" at (50,0), 50x25
        assert_eq!(rects[1].1, Rect::new(50, 0, 50, 25));
        // Widget "c" at (0,25), 50x25
        assert_eq!(rects[2].1, Rect::new(0, 25, 50, 25));
        // Widget "d" at (50,25), 50x25
        assert_eq!(rects[3].1, Rect::new(50, 25, 50, 25));
    }

    #[test]
    fn compute_rects_with_gap() {
        let layout = WidgetLayout {
            columns: 2,
            rows: 1,
            gap: 2,
            slots: vec![
                WidgetSlot { widget_id: "a".into(), col: 0, row: 0, width: 1, height: 1 },
                WidgetSlot { widget_id: "b".into(), col: 1, row: 0, width: 1, height: 1 },
            ],
            visible: true,
            position: DashboardPosition::Overlay,
        };

        let area = Rect::new(0, 0, 100, 20);
        let rects = layout.compute_rects(area);
        assert_eq!(rects.len(), 2);
        // Each cell: (100 - 2) / 2 = 49 wide
        assert_eq!(rects[0].1.width, 49);
        assert_eq!(rects[1].1.x, 51); // 0 + 49 + 2
    }

    #[test]
    fn auto_layout_fills_grid() {
        let ids: Vec<String> = (0..6).map(|i| format!("w{i}")).collect();
        let layout = WidgetLayout::auto_layout(&ids, 3, 2, 1);
        assert_eq!(layout.slots.len(), 6);
        assert_eq!(layout.slots[0].col, 0);
        assert_eq!(layout.slots[0].row, 0);
        assert_eq!(layout.slots[3].col, 0);
        assert_eq!(layout.slots[3].row, 1);
    }

    #[test]
    fn auto_layout_truncates_at_capacity() {
        let ids: Vec<String> = (0..10).map(|i| format!("w{i}")).collect();
        let layout = WidgetLayout::auto_layout(&ids, 2, 2, 0);
        // 2x2 = 4 slots max
        assert_eq!(layout.slots.len(), 4);
    }

    #[test]
    fn toggle_visibility() {
        let mut layout = WidgetLayout::new();
        assert!(!layout.visible);
        layout.toggle();
        assert!(layout.visible);
        layout.toggle();
        assert!(!layout.visible);
    }
}
