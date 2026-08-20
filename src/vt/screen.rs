//! The screen buffer — a grid of cells plus the cursor, scroll region, and
//! the line-level operations (insert/delete/scroll) that reshape it.
//!
//! This is the Rust counterpart of `uv.RenderBuffer` + `Screen` in the Go
//! codebase.

use std::collections::HashSet;
use std::sync::Arc;

use crate::vt::cell::{new_line, Cell, Style};
use crate::vt::scrollback::Scrollback;

/// A cursor position (0-indexed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Position {
    pub x: i32,
    pub y: i32,
}

/// The cursor's pen: the style that will paint the next written character.
#[derive(Debug, Clone, Default)]
pub struct CursorPen {
    pub style: Style,
}

/// The cursor state.
#[derive(Debug, Clone, Default)]
pub struct Cursor {
    pub pos: Position,
    pub hidden: bool,
    pub pen: CursorPen,
    /// Saved position (for ESC 7 / CSI s).
    pub saved: Position,
    pub saved_pen: CursorPen,
}

/// The scroll region (margins) in cell coordinates. `min` is top-left,
/// `max` is bottom-right exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScrollRegion {
    pub top: i32,
    pub bottom: i32,
    pub left: i32,
    pub right: i32,
}

impl ScrollRegion {
    pub fn full(width: i32, height: i32) -> Self {
        Self {
            top: 0,
            bottom: height,
            left: 0,
            right: width,
        }
    }
}

/// The screen buffer.
#[derive(Debug)]
pub struct ScreenBuffer {
    width: i32,
    height: i32,
    /// The grid, row-major.
    lines: Vec<Vec<Cell>>,
    /// Lines touched since the last clear (for the renderer's dirty tracking).
    touched: HashSet<i32>,
    /// The cursor.
    pub cursor: Cursor,
    /// The scroll region.
    pub scroll: ScrollRegion,
    /// Scrollback for lines scrolled off the top.
    pub scrollback: Scrollback,
    /// Whether this screen owns scrollback (the alt screen does not).
    has_scrollback: bool,
}

impl ScreenBuffer {
    pub fn new(width: i32, height: i32) -> Self {
        let width = width.max(1);
        let height = height.max(1);
        let lines = vec![new_line(width as usize); height as usize];
        Self {
            width,
            height,
            lines,
            touched: HashSet::new(),
            cursor: Cursor::default(),
            scroll: ScrollRegion::full(width, height),
            scrollback: Scrollback::new(0),
            has_scrollback: true,
        }
    }

    pub fn width(&self) -> i32 {
        self.width
    }

    pub fn height(&self) -> i32 {
        self.height
    }

    pub fn cell(&self, x: i32, y: i32) -> Option<&Cell> {
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            return None;
        }
        self.lines[y as usize].get(x as usize)
    }

    pub fn cell_mut(&mut self, x: i32, y: i32) -> Option<&mut Cell> {
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            return None;
        }
        self.lines[y as usize].get_mut(x as usize)
    }

    pub fn line(&self, y: i32) -> Option<&[Cell]> {
        if y < 0 || y >= self.height {
            return None;
        }
        Some(&self.lines[y as usize])
    }

    pub fn take_touched(&mut self) -> HashSet<i32> {
        std::mem::take(&mut self.touched)
    }

    pub fn touch_line(&mut self, y: i32) {
        if y >= 0 && y < self.height {
            self.touched.insert(y);
        }
    }

    pub fn touch_all(&mut self) {
        for y in 0..self.height {
            self.touched.insert(y);
        }
    }

    /// Set a cell, marking the line touched.
    pub fn set_cell(&mut self, x: i32, y: i32, cell: Cell) {
        if let Some(slot) = self.cell_mut(x, y) {
            *slot = cell;
            self.touch_line(y);
        }
    }

    /// Clear a cell back to blank.
    pub fn blank_cell(&mut self, x: i32, y: i32) {
        if let Some(slot) = self.cell_mut(x, y) {
            *slot = Cell::default();
            self.touch_line(y);
        }
    }

    /// A blank cell in the cursor's current pen.
    fn blank_with_pen(&self) -> Cell {
        Cell {
            content: None,
            width: 1,
            style: self.cursor.pen.style,
            ..Default::default()
        }
    }

    pub fn resize(&mut self, width: i32, height: i32) {
        let width = width.max(1);
        let height = height.max(1);
        self.width = width;
        self.height = height;
        self.lines.resize(height as usize, new_line(width as usize));
        for line in self.lines.iter_mut() {
            line.resize(width as usize, Cell::default());
        }
        // If narrowing cut a wide rune in half, blank the dangling lead at
        // the last column so no reader sees a 2-cell rune in a 1-cell space.
        if width > 0 {
            let last = width - 1;
            for line in self.lines.iter_mut() {
                if line[last as usize].width > 1 {
                    line[last as usize] = Cell::default();
                }
            }
        }
        self.scroll = ScrollRegion::full(width, height);
        self.cursor.pos.x = self.cursor.pos.x.clamp(0, width - 1);
        self.cursor.pos.y = self.cursor.pos.y.clamp(0, height - 1);
        self.touch_all();
    }

    /// Clear the whole screen (not scrollback).
    pub fn clear(&mut self) {
        for y in 0..self.height {
            for x in 0..self.width {
                self.lines[y as usize][x as usize] = Cell::default();
            }
            self.touch_line(y);
        }
    }

    pub fn clear_area(&mut self, x: i32, y: i32, w: i32, h: i32) {
        for yy in y..(y + h).min(self.height) {
            for xx in x..(x + w).min(self.width) {
                if xx >= 0 && yy >= 0 {
                    self.lines[yy as usize][xx as usize] = Cell::default();
                }
            }
            self.touch_line(yy);
        }
    }

    /// Clear from the cursor to the end of the line (EL 0).
    pub fn clear_to_end_of_line(&mut self) {
        let x = self.cursor.pos.x;
        let y = self.cursor.pos.y;
        if y < 0 || y >= self.height {
            return;
        }
        for xx in x..self.width {
            self.lines[y as usize][xx as usize] = Cell::default();
        }
        self.touch_line(y);
    }

    /// Clear from the start of the line to the cursor (EL 1).
    pub fn clear_from_start_of_line(&mut self) {
        let x = self.cursor.pos.x;
        let y = self.cursor.pos.y;
        if y < 0 || y >= self.height {
            return;
        }
        for xx in 0..=x.min(self.width - 1) {
            self.lines[y as usize][xx as usize] = Cell::default();
        }
        self.touch_line(y);
    }

    /// Clear the whole line under the cursor (EL 2).
    pub fn clear_line(&mut self) {
        let y = self.cursor.pos.y;
        if y < 0 || y >= self.height {
            return;
        }
        for xx in 0..self.width {
            self.lines[y as usize][xx as usize] = Cell::default();
        }
        self.touch_line(y);
    }

    /// Clear from the cursor to the end of the screen (ED 0).
    pub fn clear_to_end_of_screen(&mut self) {
        self.clear_to_end_of_line();
        for y in (self.cursor.pos.y + 1)..self.height {
            for x in 0..self.width {
                self.lines[y as usize][x as usize] = Cell::default();
            }
            self.touch_line(y);
        }
    }

    /// Clear from the start of the screen to the cursor (ED 1).
    pub fn clear_from_start_of_screen(&mut self) {
        for y in 0..self.cursor.pos.y {
            for x in 0..self.width {
                self.lines[y as usize][x as usize] = Cell::default();
            }
            self.touch_line(y);
        }
        self.clear_from_start_of_line();
    }

    /// Clear the whole screen (ED 2).
    pub fn clear_screen(&mut self) {
        self.clear();
    }

    /// Clear scrollback and screen (ED 3).
    pub fn clear_scrollback(&mut self) {
        if self.has_scrollback {
            self.scrollback.clear();
        }
        self.clear();
    }

    // -----------------------------------------------------------------------
    // Cursor movement
    // -----------------------------------------------------------------------

    pub fn set_cursor(&mut self, x: i32, y: i32, margins: bool) {
        if margins {
            let sy = self.scroll.top + y;
            self.cursor.pos.y = sy.clamp(self.scroll.top, self.scroll.bottom - 1);
            let sx = self.scroll.left + x;
            self.cursor.pos.x = sx.clamp(self.scroll.left, self.scroll.right - 1);
        } else {
            self.cursor.pos.y = y.clamp(0, self.height - 1);
            self.cursor.pos.x = x.clamp(0, self.width - 1);
        }
    }

    pub fn move_cursor(&mut self, dx: i32, dy: i32) {
        let nx = self.cursor.pos.x + dx;
        let ny = self.cursor.pos.y + dy;
        // Bounded by screen.
        self.cursor.pos.x = nx.clamp(0, self.width - 1);
        self.cursor.pos.y = ny.clamp(0, self.height - 1);
    }

    pub fn save_cursor(&mut self) {
        self.cursor.saved = self.cursor.pos;
        self.cursor.saved_pen = self.cursor.pen.clone();
    }

    pub fn restore_cursor(&mut self) {
        self.cursor.pos = self.cursor.saved;
        // The saved position may predate a resize; clamp it to the current
        // screen so DECRC can never leave the cursor out of bounds.
        self.cursor.pos.x = self.cursor.pos.x.clamp(0, self.width - 1);
        self.cursor.pos.y = self.cursor.pos.y.clamp(0, self.height - 1);
        self.cursor.pen = self.cursor.saved_pen.clone();
    }

    // -----------------------------------------------------------------------
    // Insert / delete
    // -----------------------------------------------------------------------

    /// Insert `n` blank cells at the cursor, pushing cells right.
    pub fn insert_cell(&mut self, n: i32) {
        if n <= 0 {
            return;
        }
        let x = self.cursor.pos.x;
        let y = self.cursor.pos.y;
        if y < self.scroll.top || y >= self.scroll.bottom {
            return;
        }
        let right = self.scroll.right;
        let n = n.min(right - x);
        if n <= 0 {
            return;
        }
        let blank = self.blank_with_pen();
        let row = &mut self.lines[y as usize];
        // Drop the cells that fall off the right edge of the scroll region,
        // then insert `n` blanks at `x`. `drain` + `splice` move cells in
        // place instead of cloning the whole row twice.
        row.drain((right - n) as usize..right as usize);
        row.splice(
            x as usize..x as usize,
            std::iter::repeat_with(|| blank.clone()).take(n as usize),
        );
        self.touch_line(y);
    }

    /// Delete `n` cells at the cursor, pulling cells left.
    pub fn delete_cell(&mut self, n: i32) {
        if n <= 0 {
            return;
        }
        let x = self.cursor.pos.x;
        let y = self.cursor.pos.y;
        if y < self.scroll.top || y >= self.scroll.bottom {
            return;
        }
        let right = self.scroll.right;
        let n = n.min(right - x);
        if n <= 0 {
            return;
        }
        let blank = self.blank_with_pen();
        let row = &mut self.lines[y as usize];
        // Remove `n` cells at `x`, pulling the rest left in place, then pad
        // the scroll region's trailing edge with blanks.
        row.drain(x as usize..(x + n) as usize);
        let insert_at = (right - n) as usize;
        row.splice(
            insert_at..insert_at,
            std::iter::repeat_with(|| blank.clone()).take(n as usize),
        );
        self.touch_line(y);
    }

    /// Erase `n` characters (ECH) from the cursor, leaving blanks.
    pub fn erase_characters(&mut self, n: i32) {
        if n <= 0 {
            return;
        }
        let x = self.cursor.pos.x;
        let y = self.cursor.pos.y;
        if y < 0 || y >= self.height {
            return;
        }
        let right = (x + n).min(self.width);
        let blank = self.blank_with_pen();
        for i in x..right {
            self.lines[y as usize][i as usize] = blank.clone();
        }
        self.touch_line(y);
    }

    /// Insert `n` blank lines at the cursor, discarding lines past the bottom
    /// margin.
    pub fn insert_line(&mut self, n: i32) {
        if n <= 0 {
            return;
        }
        let y = self.cursor.pos.y;
        if y < self.scroll.top || y >= self.scroll.bottom {
            return;
        }
        let n = n.min(self.scroll.bottom - y);
        let blank = new_line(self.width as usize);
        for _ in 0..n {
            self.lines.insert(y as usize, blank.clone());
            self.lines.remove(self.scroll.bottom as usize);
        }
        for yy in self.scroll.top..self.scroll.bottom {
            self.touch_line(yy);
        }
    }

    /// Delete `n` lines at the cursor, pulling lines up from below.
    pub fn delete_line(&mut self, n: i32) {
        if n <= 0 {
            return;
        }
        let y = self.cursor.pos.y;
        if y < self.scroll.top || y >= self.scroll.bottom {
            return;
        }
        let n = n.min(self.scroll.bottom - y);
        let blank = new_line(self.width as usize);
        for _ in 0..n {
            self.lines.remove(y as usize);
            self.lines
                .insert(self.scroll.bottom as usize - 1, blank.clone());
        }
        for yy in self.scroll.top..self.scroll.bottom {
            self.touch_line(yy);
        }
    }

    // -----------------------------------------------------------------------
    // Scrolling
    // -----------------------------------------------------------------------

    /// Scroll content up `n` lines within the scroll region. Lines scrolled
    /// past the top margin are saved to scrollback when the region is the full
    /// screen width and starts at the top.
    pub fn scroll_up(&mut self, n: i32) {
        if n <= 0 {
            return;
        }
        let save = self.has_scrollback
            && self.scroll.top == 0
            && self.scroll.left == 0
            && self.scroll.right == self.width;

        // Preserve cursor position.
        let cx = self.cursor.pos.x;
        let cy = self.cursor.pos.y;

        let top = self.scroll.top;
        let bottom = self.scroll.bottom;
        let n = n.min(bottom - top);

        // Save the departing lines to scrollback, collecting recycled blank
        // buffers when the scrollback is at capacity.
        let mut blank_bufs: Vec<Vec<Cell>> =
            Vec::with_capacity(if save { n as usize } else { 0 });
        if save {
            for i in 0..n {
                let line = self.lines[(top + i) as usize].clone();
                match self.scrollback.push_line_recycle(line) {
                    Some(recycled) => blank_bufs.push(recycled),
                    None => blank_bufs.push(new_line(self.width as usize)),
                }
            }
        }

        // Slide lines up.
        for y in top..(bottom - n) {
            self.lines[y as usize] = std::mem::take(&mut self.lines[(y + n) as usize]);
        }
        // Blank the vacated bottom lines, reusing recycled buffers when the
        // departing lines were saved to scrollback.
        let mut blanks = blank_bufs.into_iter();
        for y in (bottom - n)..bottom {
            self.lines[y as usize] = blanks
                .next()
                .unwrap_or_else(|| new_line(self.width as usize));
        }
        for y in top..bottom {
            self.touch_line(y);
        }

        self.cursor.pos.x = cx;
        self.cursor.pos.y = cy;
    }

    /// Scroll content down `n` lines within the scroll region.
    pub fn scroll_down(&mut self, n: i32) {
        if n <= 0 {
            return;
        }
        let cx = self.cursor.pos.x;
        let cy = self.cursor.pos.y;
        let top = self.scroll.top;
        let bottom = self.scroll.bottom;
        let n = n.min(bottom - top);

        // Slide lines down.
        for y in ((top + n)..bottom).rev() {
            self.lines[y as usize] = std::mem::take(&mut self.lines[(y - n) as usize]);
        }
        // Blank the vacated top lines.
        for y in top..(top + n) {
            self.lines[y as usize] = new_line(self.width as usize);
        }
        for y in top..bottom {
            self.touch_line(y);
        }
        self.cursor.pos.x = cx;
        self.cursor.pos.y = cy;
    }

    /// Move the cursor to the next line, scrolling the region if at the bottom.
    pub fn line_feed(&mut self) {
        if self.cursor.pos.y + 1 >= self.scroll.bottom {
            // Scroll the region up.
            self.scroll_up(1);
        } else {
            self.cursor.pos.y += 1;
        }
    }

    pub fn carriage_return(&mut self) {
        self.cursor.pos.x = self.scroll.left;
    }

    /// Render the screen as plain text (for tests and copy).
    pub fn render_text(&self) -> String {
        let mut out = String::new();
        for y in 0..self.height {
            let mut line = String::new();
            let mut col = 0;
            let row = &self.lines[y as usize];
            while col < self.width {
                let cell = &row[col as usize];
                cell.push_grapheme_into(&mut line);
                col += cell.width.max(1) as i32;
            }
            out.push_str(line.trim_end());
            if y < self.height - 1 {
                out.push('\n');
            }
        }
        out
    }

    /// A snapshot of a line as plain text (used by scrollback/copy).
    pub fn line_text(&self, y: i32) -> String {
        let mut line = String::new();
        if y < 0 || y >= self.height {
            return line;
        }
        let mut col = 0;
        let row = &self.lines[y as usize];
        while col < self.width {
            let cell = &row[col as usize];
            cell.push_grapheme_into(&mut line);
            col += cell.width.max(1) as i32;
        }
        line.trim_end().to_string()
    }

    /// Copy a full line out of the buffer (used when pushing to scrollback).
    pub fn line_owned(&self, y: i32) -> Vec<Cell> {
        if y < 0 || y >= self.height {
            return Vec::new();
        }
        self.lines[y as usize].clone()
    }

    pub fn set_scrollback_enabled(&mut self, enabled: bool) {
        self.has_scrollback = enabled;
    }

    pub fn scrollback_enabled(&self) -> bool {
        self.has_scrollback
    }

    /// The current pen style.
    pub fn pen(&self) -> Style {
        self.cursor.pen.style
    }

    pub fn set_pen(&mut self, style: Style) {
        self.cursor.pen.style = style;
    }

    /// Direct access to a line for the renderer.
    pub fn lines(&self) -> &[Vec<Cell>] {
        &self.lines
    }
}

/// A reference to a screen line shared with the renderer.
pub type ScreenLine = Arc<Vec<Cell>>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vt::cell::Color;

    #[test]
    fn screen_buffer_new() {
        let s = ScreenBuffer::new(80, 24);
        assert_eq!(s.width(), 80);
        assert_eq!(s.height(), 24);
    }

    #[test]
    fn screen_buffer_min_size() {
        let s = ScreenBuffer::new(0, 0);
        assert_eq!(s.width(), 1);
        assert_eq!(s.height(), 1);
    }

    #[test]
    fn screen_buffer_cell_access() {
        let s = ScreenBuffer::new(10, 5);
        assert!(s.cell(0, 0).is_some());
        assert!(s.cell(9, 4).is_some());
        assert!(s.cell(10, 0).is_none());
        assert!(s.cell(0, 5).is_none());
        assert!(s.cell(-1, 0).is_none());
    }

    #[test]
    fn screen_buffer_cell_mut() {
        let mut s = ScreenBuffer::new(10, 5);
        assert!(s.cell_mut(0, 0).is_some());
        assert!(s.cell_mut(10, 0).is_none());
    }

    #[test]
    fn screen_buffer_line() {
        let s = ScreenBuffer::new(10, 5);
        assert!(s.line(0).is_some());
        assert_eq!(s.line(0).unwrap().len(), 10);
        assert!(s.line(5).is_none());
        assert!(s.line(-1).is_none());
    }

    #[test]
    fn screen_buffer_set_cell() {
        let mut s = ScreenBuffer::new(10, 5);
        s.set_cell(5, 2, Cell::new('X', 1, Style::new()));
        let cell = s.cell(5, 2).unwrap();
        assert_eq!(cell.content, Some('X'));
    }

    #[test]
    fn screen_buffer_blank_cell() {
        let mut s = ScreenBuffer::new(10, 5);
        s.set_cell(5, 2, Cell::new('X', 1, Style::new()));
        s.blank_cell(5, 2);
        assert!(s.cell(5, 2).unwrap().is_empty());
    }

    #[test]
    fn screen_buffer_touch() {
        let mut s = ScreenBuffer::new(10, 5);
        s.touch_line(3);
        let touched = s.take_touched();
        assert!(touched.contains(&3));
        assert!(touched.len() == 1);
    }

    #[test]
    fn screen_buffer_touch_all() {
        let mut s = ScreenBuffer::new(10, 5);
        s.touch_all();
        let touched = s.take_touched();
        assert_eq!(touched.len(), 5);
    }

    #[test]
    fn screen_buffer_resize() {
        let mut s = ScreenBuffer::new(10, 5);
        s.set_cell(5, 2, Cell::new('X', 1, Style::new()));
        s.resize(20, 10);
        assert_eq!(s.width(), 20);
        assert_eq!(s.height(), 10);
    }

    #[test]
    fn screen_buffer_clear() {
        let mut s = ScreenBuffer::new(10, 5);
        s.set_cell(5, 2, Cell::new('X', 1, Style::new()));
        s.clear();
        assert!(s.cell(5, 2).unwrap().is_empty());
    }

    #[test]
    fn screen_buffer_clear_area() {
        let mut s = ScreenBuffer::new(10, 5);
        s.set_cell(3, 2, Cell::new('X', 1, Style::new()));
        s.clear_area(2, 1, 3, 3);
        assert!(s.cell(3, 2).unwrap().is_empty());
    }

    #[test]
    fn screen_buffer_clear_to_end_of_line() {
        let mut s = ScreenBuffer::new(10, 5);
        s.set_cell(5, 2, Cell::new('X', 1, Style::new()));
        s.set_cell(8, 2, Cell::new('Y', 1, Style::new()));
        s.cursor.pos = Position { x: 3, y: 2 };
        s.clear_to_end_of_line();
        assert!(s.cell(3, 2).unwrap().is_empty());
        assert!(s.cell(5, 2).unwrap().is_empty());
    }

    #[test]
    fn screen_buffer_clear_from_start_of_line() {
        let mut s = ScreenBuffer::new(10, 5);
        s.set_cell(2, 2, Cell::new('X', 1, Style::new()));
        s.cursor.pos = Position { x: 5, y: 2 };
        s.clear_from_start_of_line();
        assert!(s.cell(2, 2).unwrap().is_empty());
    }

    #[test]
    fn screen_buffer_clear_line() {
        let mut s = ScreenBuffer::new(10, 5);
        s.set_cell(5, 2, Cell::new('X', 1, Style::new()));
        s.cursor.pos = Position { x: 0, y: 2 };
        s.clear_line();
        assert!(s.cell(5, 2).unwrap().is_empty());
    }

    #[test]
    fn screen_buffer_clear_to_end_of_screen() {
        let mut s = ScreenBuffer::new(10, 5);
        s.set_cell(5, 3, Cell::new('X', 1, Style::new()));
        s.cursor.pos = Position { x: 0, y: 2 };
        s.clear_to_end_of_screen();
        assert!(s.cell(5, 3).unwrap().is_empty());
    }

    #[test]
    fn screen_buffer_clear_from_start_of_screen() {
        let mut s = ScreenBuffer::new(10, 5);
        s.set_cell(5, 1, Cell::new('X', 1, Style::new()));
        s.cursor.pos = Position { x: 5, y: 2 };
        s.clear_from_start_of_screen();
        assert!(s.cell(5, 1).unwrap().is_empty());
    }

    #[test]
    fn screen_buffer_set_cursor() {
        let mut s = ScreenBuffer::new(10, 5);
        s.set_cursor(5, 3, false);
        assert_eq!(s.cursor.pos, Position { x: 5, y: 3 });
    }

    #[test]
    fn screen_buffer_set_cursor_with_margins() {
        let mut s = ScreenBuffer::new(10, 5);
        s.scroll = ScrollRegion {
            top: 1,
            bottom: 4,
            left: 2,
            right: 8,
        };
        s.set_cursor(1, 1, true);
        assert!(s.cursor.pos.x >= 2);
        assert!(s.cursor.pos.y >= 1);
    }

    #[test]
    fn screen_buffer_move_cursor() {
        let mut s = ScreenBuffer::new(10, 5);
        s.cursor.pos = Position { x: 5, y: 2 };
        s.move_cursor(2, 1);
        assert_eq!(s.cursor.pos, Position { x: 7, y: 3 });
    }

    #[test]
    fn screen_buffer_move_cursor_clamps() {
        let mut s = ScreenBuffer::new(10, 5);
        s.cursor.pos = Position { x: 5, y: 2 };
        s.move_cursor(100, 100);
        assert_eq!(s.cursor.pos, Position { x: 9, y: 4 });
    }

    #[test]
    fn screen_buffer_save_restore_cursor() {
        let mut s = ScreenBuffer::new(10, 5);
        s.cursor.pos = Position { x: 5, y: 2 };
        s.save_cursor();
        s.cursor.pos = Position { x: 0, y: 0 };
        s.restore_cursor();
        assert_eq!(s.cursor.pos, Position { x: 5, y: 2 });
    }

    #[test]
    fn screen_buffer_insert_cell() {
        let mut s = ScreenBuffer::new(10, 5);
        s.set_cell(5, 2, Cell::new('X', 1, Style::new()));
        s.cursor.pos = Position { x: 1, y: 2 };
        s.insert_cell(2);
        // "X" shifts right by 2, from col 5 to col 7.
        assert_eq!(s.cell(7, 2).unwrap().content, Some('X'));
    }

    #[test]
    fn screen_buffer_delete_cell() {
        let mut s = ScreenBuffer::new(10, 5);
        s.set_cell(5, 2, Cell::new('X', 1, Style::new()));
        s.cursor.pos = Position { x: 3, y: 2 };
        s.delete_cell(2);
        assert!(s.cell(5, 2).unwrap().is_empty());
    }

    #[test]
    fn screen_buffer_scroll_up() {
        let mut s = ScreenBuffer::new(10, 5);
        s.set_cell(5, 0, Cell::new('X', 1, Style::new()));
        s.scroll_up(1);
        assert!(s.cell(5, 0).unwrap().is_empty());
    }

    #[test]
    fn screen_buffer_scroll_down() {
        let mut s = ScreenBuffer::new(10, 5);
        s.set_cell(5, 4, Cell::new('X', 1, Style::new()));
        s.scroll_down(1);
        assert!(s.cell(5, 4).unwrap().is_empty());
    }

    #[test]
    fn screen_buffer_pen() {
        let mut s = ScreenBuffer::new(10, 5);
        let pen = s.pen();
        assert_eq!(pen, Style::new());
        let mut style = Style::new();
        style.fg = Color::indexed(1);
        s.set_pen(style);
        assert_eq!(s.pen().fg, Color::indexed(1));
    }

    #[test]
    fn screen_buffer_scrollback_enabled() {
        let s = ScreenBuffer::new(10, 5);
        assert!(s.scrollback_enabled());
    }

    #[test]
    fn screen_buffer_lines() {
        let s = ScreenBuffer::new(10, 5);
        assert_eq!(s.lines().len(), 5);
    }

    #[test]
    fn scroll_region_full() {
        let r = ScrollRegion::full(80, 24);
        assert_eq!(r.top, 0);
        assert_eq!(r.bottom, 24);
        assert_eq!(r.left, 0);
        assert_eq!(r.right, 80);
    }
}
