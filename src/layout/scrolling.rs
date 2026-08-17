//! niri-style scrolling tiling — a port of TUIOS `internal/layout/scrolling.go`.
//!
//! Windows are arranged as columns on an infinite horizontal strip with a
//! viewport that scrolls to follow focus.

use std::collections::HashMap;

use super::Rect;

/// A column in the scrolling layout. Each column holds one or more windows
/// stacked vertically.
#[derive(Debug, Clone, Default)]
pub struct ScrollColumn {
    /// Windows stacked in this column.
    pub window_ids: Vec<i32>,
    /// Width as proportion of screen (0.0-1.0), 0 = default.
    pub proportion: f64,
    /// Fixed width in cells (0 = use proportion).
    pub fixed_width: i32,
}

/// Manages the scrollable tiling strip.
#[derive(Debug, Clone)]
pub struct ScrollingLayout {
    pub columns: Vec<ScrollColumn>,
    /// Index of the focused column.
    pub focused_col: i32,
    /// Scroll offset in cells.
    pub viewport_x: i32,
    /// Default column width proportion (e.g. 0.5).
    pub default_width: f64,
    /// Preset width proportions to cycle through.
    pub preset_widths: Vec<f64>,
    /// Gap between columns in cells.
    pub gap: i32,
}

impl Default for ScrollingLayout {
    fn default() -> Self {
        Self::new()
    }
}

impl ScrollingLayout {
    pub fn new() -> Self {
        Self {
            columns: Vec::new(),
            focused_col: 0,
            viewport_x: 0,
            default_width: 0.55,
            preset_widths: vec![0.333, 0.5, 0.55, 0.667, 0.9],
            gap: 0,
        }
    }

    /// Insert a new column after the focused column and focus it.
    pub fn add_column(&mut self, window_id: i32) {
        let col = ScrollColumn {
            window_ids: vec![window_id],
            ..Default::default()
        };

        let insert_idx: usize =
            if !self.columns.is_empty() && (self.focused_col as usize) < self.columns.len() - 1 {
                let idx = self.focused_col as usize + 1;
                self.columns.insert(idx, col);
                idx
            } else {
                self.columns.push(col);
                self.columns.len() - 1
            };

        self.focused_col = insert_idx as i32;
    }

    /// Remove a window from the layout. If the column becomes empty it is
    /// removed and focus shifts LEFT.
    pub fn remove_window(&mut self, window_id: i32) {
        for i in 0..self.columns.len() {
            if let Some(j) = self.columns[i]
                .window_ids
                .iter()
                .position(|&id| id == window_id)
            {
                self.columns[i].window_ids.remove(j);
                if self.columns[i].window_ids.is_empty() {
                    let removed_idx = i;
                    self.columns.remove(i);
                    if (self.focused_col as usize) >= removed_idx && self.focused_col > 0 {
                        self.focused_col -= 1;
                    }
                    if (self.focused_col as usize) >= self.columns.len() && !self.columns.is_empty()
                    {
                        self.focused_col = (self.columns.len() - 1) as i32;
                    }
                }
                return;
            }
        }
    }

    pub fn focus_left(&mut self) {
        if self.focused_col > 0 {
            self.focused_col -= 1;
        }
    }

    pub fn focus_right(&mut self) {
        if (self.focused_col as usize) < self.columns.len().saturating_sub(1) {
            self.focused_col += 1;
        }
    }

    pub fn move_column_left(&mut self) {
        if self.focused_col > 0 {
            let f = self.focused_col as usize;
            self.columns.swap(f, f - 1);
            self.focused_col -= 1;
        }
    }

    pub fn move_column_right(&mut self) {
        if (self.focused_col as usize) < self.columns.len().saturating_sub(1) {
            let f = self.focused_col as usize;
            self.columns.swap(f, f + 1);
            self.focused_col += 1;
        }
    }

    /// Cycle the focused column through preset widths.
    pub fn cycle_width(&mut self) {
        if self.focused_col < 0 || (self.focused_col as usize) >= self.columns.len() {
            return;
        }
        if self.preset_widths.is_empty() {
            return;
        }
        let col = &mut self.columns[self.focused_col as usize];
        let mut current = col.proportion;
        if current == 0.0 {
            current = self.default_width;
        }
        // A prior keyboard resize pins fixed_width; clear it so the cycled
        // preset proportion takes effect.
        col.fixed_width = 0;

        for &w in &self.preset_widths {
            if w > current + 0.01 {
                col.proportion = w;
                return;
            }
        }
        col.proportion = self.preset_widths[0];
    }

    /// Move the window from the next column into the focused column.
    pub fn consume_window(&mut self) {
        if (self.focused_col as usize) >= self.columns.len().saturating_sub(1) {
            return;
        }
        let f = self.focused_col as usize;
        if self.columns[f + 1].window_ids.is_empty() {
            return;
        }
        let window_id = self.columns[f + 1].window_ids.remove(0);
        self.columns[f].window_ids.push(window_id);

        if self.columns[f + 1].window_ids.is_empty() {
            self.columns.remove(f + 1);
        }
    }

    /// Move the last window from the focused column into a new column.
    pub fn expel_window(&mut self) {
        if self.focused_col < 0 || (self.focused_col as usize) >= self.columns.len() {
            return;
        }
        let f = self.focused_col as usize;
        if self.columns[f].window_ids.len() < 2 {
            return;
        }
        let window_id = self.columns[f].window_ids.pop().unwrap();
        let new_col = ScrollColumn {
            window_ids: vec![window_id],
            ..Default::default()
        };
        self.columns.insert(f + 1, new_col);
    }

    /// Width in cells for a column by index.
    pub fn resolve_column_width(&self, col_index: i32, screen_width: i32) -> i32 {
        if col_index < 0 || (col_index as usize) >= self.columns.len() {
            return 0;
        }
        self.resolve_width(&self.columns[col_index as usize], screen_width)
    }

    fn resolve_width(&self, col: &ScrollColumn, screen_width: i32) -> i32 {
        let max_width = screen_width * 9 / 10;
        if col.fixed_width > 0 {
            return col.fixed_width.min(max_width);
        }
        let mut proportion = col.proportion;
        if proportion <= 0.0 {
            proportion = self.default_width;
        }
        ((screen_width as f64 * proportion).max(10.0) as i32).min(max_width)
    }

    /// Total width of all columns in cells.
    pub fn total_strip_width(&self, screen_width: i32) -> i32 {
        let mut total = 0;
        for (i, col) in self.columns.iter().enumerate() {
            total += self.resolve_width(col, screen_width);
            if i < self.columns.len() - 1 {
                total += self.gap;
            }
        }
        total
    }

    /// X position of a column on the virtual strip.
    fn column_x(&self, index: i32, screen_width: i32) -> i32 {
        let mut x = 0;
        for i in 0..index.min(self.columns.len() as i32) {
            x += self.resolve_width(&self.columns[i as usize], screen_width) + self.gap;
        }
        x
    }

    /// Ensure the viewport doesn't scroll past the content.
    pub fn clamp_viewport(&mut self, screen_width: i32) {
        let max_scroll = (self.total_strip_width(screen_width) - screen_width).max(0);
        if self.viewport_x < 0 {
            self.viewport_x = 0;
        }
        if self.viewport_x > max_scroll {
            self.viewport_x = max_scroll;
        }
    }

    /// Only scroll the viewport when the focused column is COMPLETELY
    /// off-screen. If any part is visible, the viewport stays put.
    pub fn ensure_focused_visible(&mut self, screen_width: i32) {
        if self.focused_col < 0 || (self.focused_col as usize) >= self.columns.len() {
            return;
        }
        let col_x = self.column_x(self.focused_col, screen_width);
        let col_w = self.resolve_width(&self.columns[self.focused_col as usize], screen_width);

        let fully_visible =
            col_x >= self.viewport_x && col_x + col_w <= self.viewport_x + screen_width;
        if fully_visible {
            return;
        }

        // Center the focused column so both neighbors peek in.
        self.viewport_x = col_x - (screen_width - col_w) / 2;
        self.clamp_viewport(screen_width);
    }

    /// Scroll the viewport to fully show the focused column (explicit keyboard
    /// navigation).
    pub fn scroll_to_focused_column(&mut self, screen_width: i32) {
        if self.focused_col < 0 || (self.focused_col as usize) >= self.columns.len() {
            return;
        }
        let col_x = self.column_x(self.focused_col, screen_width);
        let col_w = self.resolve_width(&self.columns[self.focused_col as usize], screen_width);

        self.viewport_x = col_x - (screen_width - col_w) / 2;
        self.clamp_viewport(screen_width);
    }

    /// Set focus to the column containing the given window ID. Returns true if
    /// the window was found.
    pub fn focus_column_containing(&mut self, window_id: i32) -> bool {
        for (ci, col) in self.columns.iter().enumerate() {
            if col.window_ids.contains(&window_id) {
                self.focused_col = ci as i32;
                return true;
            }
        }
        false
    }

    /// Compute positions for all columns using the current viewport. Pure —
    /// does not modify viewport_x.
    pub fn compute_positions(
        &self,
        screen_width: i32,
        usable_height: i32,
        top_margin: i32,
    ) -> HashMap<i32, Rect> {
        let mut result = HashMap::new();
        if self.columns.is_empty() {
            return result;
        }

        let mut x = 0;
        for col in &self.columns {
            let col_width = self.resolve_width(col, screen_width);
            let screen_x = x - self.viewport_x;

            let window_count = col.window_ids.len();
            if window_count == 0 {
                x += col_width + self.gap;
                continue;
            }
            let cell_height = usable_height / window_count as i32;
            for (j, &win_id) in col.window_ids.iter().enumerate() {
                let h = if j == window_count - 1 {
                    usable_height - j as i32 * cell_height
                } else {
                    cell_height
                };
                result.insert(
                    win_id,
                    Rect {
                        x: screen_x,
                        y: top_margin + j as i32 * cell_height,
                        w: col_width,
                        h,
                    },
                );
            }
            x += col_width + self.gap;
        }

        result
    }

    /// Total number of windows across all columns.
    pub fn window_count(&self) -> i32 {
        self.columns.iter().map(|c| c.window_ids.len() as i32).sum()
    }

    /// First window ID in the focused column.
    pub fn get_focused_window_id(&self) -> i32 {
        if self.focused_col < 0 || (self.focused_col as usize) >= self.columns.len() {
            return -1;
        }
        self.columns[self.focused_col as usize]
            .window_ids
            .first()
            .copied()
            .unwrap_or(-1)
    }

    pub fn has_window(&self, window_id: i32) -> bool {
        self.columns
            .iter()
            .any(|c| c.window_ids.contains(&window_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_remove_column_updates_focus() {
        let mut s = ScrollingLayout::new();
        s.add_column(1);
        s.add_column(2);
        s.add_column(3);
        assert_eq!(s.columns.len(), 3);
        assert_eq!(s.focused_col, 2);

        s.remove_window(3);
        assert_eq!(s.columns.len(), 2);
        assert_eq!(s.focused_col, 1);
    }

    #[test]
    fn compute_positions_places_windows() {
        let mut s = ScrollingLayout::new();
        s.add_column(1);
        s.add_column(2);
        s.viewport_x = 0;
        let positions = s.compute_positions(120, 24, 0);
        assert_eq!(positions.len(), 2);
        assert_eq!(positions[&1].w, 66); // 120 * 0.55
        assert_eq!(positions[&1].y, 0);
    }

    #[test]
    fn viewport_clamps_to_content() {
        let mut s = ScrollingLayout::new();
        s.add_column(1);
        s.add_column(2);
        s.add_column(3);
        s.viewport_x = 10000;
        s.clamp_viewport(120);
        assert!(s.viewport_x <= (s.total_strip_width(120) - 120).max(0));
    }
}
