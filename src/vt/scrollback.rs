//! The scrollback ring buffer — lines that have scrolled off the top of the
//! main screen, kept so the user can scroll back and copy.

use crate::vt::cell::{new_line, Cell, Line};

const DEFAULT_MAX_LINES: usize = 10_000;

/// A ring buffer of lines that have scrolled off the top of the screen.
#[derive(Debug)]
pub struct Scrollback {
    /// The stored lines, oldest first.
    lines: Vec<Line>,
    /// Maximum number of lines to keep.
    max_lines: usize,
}

impl Scrollback {
    pub fn new(max_lines: usize) -> Self {
        Self {
            lines: Vec::new(),
            max_lines: if max_lines == 0 {
                DEFAULT_MAX_LINES
            } else {
                max_lines
            },
        }
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn line(&self, index: usize) -> Option<&Line> {
        self.lines.get(index)
    }

    pub fn set_max_lines(&mut self, max: usize) {
        self.max_lines = if max == 0 { DEFAULT_MAX_LINES } else { max };
        self.trim();
    }

    /// Push an owned line, trimming to the max.
    pub fn push_line(&mut self, line: Vec<Cell>) {
        self.lines.push(std::sync::Arc::new(line));
        self.trim();
    }

    /// Push a line and, if it was trimmed, return a recycled line of the same
    /// width to be reused as a fresh blank line (the Go code's
    /// `PushLineOwnedRecycle` optimization).
    pub fn push_line_recycle(&mut self, line: Vec<Cell>) -> Option<Vec<Cell>> {
        let width = line.len();
        self.lines.push(std::sync::Arc::new(line));
        if self.lines.len() > self.max_lines {
            let excess = self.lines.len() - self.max_lines;
            self.lines.drain(..excess);
            return Some(new_line(width));
        }
        None
    }

    fn trim(&mut self) {
        if self.lines.len() > self.max_lines {
            let excess = self.lines.len() - self.max_lines;
            self.lines.drain(..excess);
        }
    }

    pub fn clear(&mut self) {
        self.lines.clear();
    }

    /// Reflow the buffer for a new width (re-wraps soft-wrapped lines).
    /// This is a simplified reflow: it re-wraps every line at the new width
    /// based on cell display widths.
    pub fn reflow(&mut self, new_width: usize) {
        if new_width == 0 {
            return;
        }
        let old: Vec<Line> = self.lines.drain(..).collect();
        let mut reflowed: Vec<Line> = Vec::with_capacity(old.len());

        let mut current: Vec<Cell> = new_line(new_width);
        let mut current_col = 0usize;

        for line in old {
            for cell in line.iter() {
                if current_col >= new_width {
                    reflowed.push(std::sync::Arc::new(std::mem::replace(
                        &mut current,
                        new_line(new_width),
                    )));
                    current_col = 0;
                }
                let w = cell.width.max(1) as usize;
                if current_col + w > new_width {
                    reflowed.push(std::sync::Arc::new(std::mem::replace(
                        &mut current,
                        new_line(new_width),
                    )));
                    current_col = 0;
                }
                current[current_col] = cell.clone();
                // Fill continuation slots for wide runes.
                for k in 1..w {
                    if current_col + k < new_width {
                        current[current_col + k].content.clear();
                        current[current_col + k].width = 0;
                        current[current_col + k].style = cell.style;
                    }
                }
                current_col += w;
            }
            // A hard line break at the end of each stored line.
            reflowed.push(std::sync::Arc::new(std::mem::replace(
                &mut current,
                new_line(new_width),
            )));
            current_col = 0;
        }

        if current_col > 0 {
            reflowed.push(std::sync::Arc::new(current));
        }

        self.lines = reflowed;
        self.trim();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_and_trim() {
        let mut sb = Scrollback::new(3);
        sb.push_line(new_line(2));
        sb.push_line(new_line(2));
        sb.push_line(new_line(2));
        sb.push_line(new_line(2));
        assert_eq!(sb.len(), 3);
    }

    #[test]
    fn reflow_narrows_lines() {
        let mut sb = Scrollback::new(10);
        let mut line = new_line(4);
        for (i, cell) in line.iter_mut().enumerate() {
            cell.content = ((b'a' + i as u8) as char).to_string();
            cell.width = 1;
        }
        sb.push_line(line);
        sb.reflow(2);
        assert_eq!(sb.len(), 2);
    }

    #[test]
    fn new_with_zero_uses_default() {
        let sb = Scrollback::new(0);
        assert_eq!(sb.len(), 0);
    }

    #[test]
    fn line_access() {
        let mut sb = Scrollback::new(5);
        sb.push_line(new_line(3));
        assert!(sb.line(0).is_some());
        assert!(sb.line(1).is_none());
    }

    #[test]
    fn set_max_lines_trims() {
        let mut sb = Scrollback::new(10);
        sb.push_line(new_line(1));
        sb.push_line(new_line(1));
        sb.push_line(new_line(1));
        sb.set_max_lines(2);
        assert_eq!(sb.len(), 2);
    }

    #[test]
    fn set_max_lines_zero_uses_default() {
        let mut sb = Scrollback::new(5);
        sb.push_line(new_line(1));
        sb.set_max_lines(0);
        assert_eq!(sb.len(), 1);
    }

    #[test]
    fn push_line_recycle_no_trim() {
        let mut sb = Scrollback::new(10);
        let recycled = sb.push_line_recycle(new_line(5));
        assert!(recycled.is_none());
    }

    #[test]
    fn push_line_recycle_with_trim() {
        let mut sb = Scrollback::new(2);
        sb.push_line(new_line(1));
        sb.push_line(new_line(1));
        let recycled = sb.push_line_recycle(new_line(3));
        assert!(recycled.is_some());
        assert_eq!(recycled.unwrap().len(), 3);
    }

    #[test]
    fn clear() {
        let mut sb = Scrollback::new(5);
        sb.push_line(new_line(1));
        sb.push_line(new_line(1));
        sb.clear();
        assert!(sb.is_empty());
    }

    #[test]
    fn reflow_widens_lines() {
        let mut sb = Scrollback::new(10);
        let mut line = new_line(2);
        line[0].content = "a".into();
        line[0].width = 1;
        line[1].content = "b".into();
        line[1].width = 1;
        sb.push_line(line);
        sb.reflow(4);
        assert_eq!(sb.len(), 1);
        assert_eq!(sb.line(0).unwrap().len(), 4);
    }

    #[test]
    fn reflow_zero_width_noop() {
        let mut sb = Scrollback::new(10);
        sb.push_line(new_line(5));
        sb.reflow(0);
        assert_eq!(sb.len(), 1);
    }

    #[test]
    fn is_empty_after_push() {
        let mut sb = Scrollback::new(5);
        assert!(sb.is_empty());
        sb.push_line(new_line(1));
        assert!(!sb.is_empty());
    }
}
