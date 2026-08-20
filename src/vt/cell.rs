//! The terminal cell model — the smallest unit of rendered content.
//!
//! A cell carries the character content drawn into a grid slot plus the
//! graphical rendition (SGR) that should paint it. It is the Rust counterpart
//! of `uv.Cell` in the Go codebase.

use std::sync::Arc;

/// A color for a cell's foreground, background, or underline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Color {
    /// Default terminal color (theme foreground/background).
    #[default]
    Default,
    /// One of the 16 ANSI palette slots (0-15).
    Indexed(u8),
    /// One of the 256-color palette slots (0-255).
    Rgb(u8, u8, u8),
}

impl Color {
    pub fn indexed(i: u8) -> Self {
        Color::Indexed(i)
    }

    pub fn rgb(r: u8, g: u8, b: u8) -> Self {
        Color::Rgb(r, g, b)
    }
}

/// Text decoration flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Decoration {
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub double_underline: bool,
    pub blink: bool,
    pub reverse: bool,
    pub hidden: bool,
    pub strikethrough: bool,
    pub overline: bool,
}

/// The graphical rendition (SGR) of a cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Style {
    pub fg: Color,
    pub bg: Color,
    pub underline_color: Option<Color>,
    pub decoration: Decoration,
}

impl Style {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether two styles are visually identical for diffing purposes.
    pub fn eq_visual(&self, other: &Self) -> bool {
        self.fg == other.fg
            && self.bg == other.bg
            && self.underline_color == other.underline_color
            && self.decoration == other.decoration
    }
}

/// A hyperlink attached to a cell (OSC 8).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Link {
    pub url: Option<String>,
}

/// One grid cell.
#[derive(Debug, Clone, Default)]
pub struct Cell {
    /// The character content: a single `char` when occupied, `None` when
    /// blank. Stored inline (no heap allocation) — the VT parser emits one
    /// `char` per `print` call, so a cell never holds more than one code
    /// point.
    pub content: Option<char>,
    /// Display width in columns (1 for normal, 2 for wide CJK, 0 for
    /// continuation cells of a wide rune).
    pub width: u8,
    /// The graphical rendition.
    pub style: Style,
    /// Hyperlink, if any.
    pub link: Link,
    /// Whether this cell was modified since the last clear of the touched set.
    pub dirty: bool,
}

impl Cell {
    pub fn new(content: char, width: u8, style: Style) -> Self {
        Self {
            content: Some(content),
            width,
            style,
            link: Link::default(),
            dirty: true,
        }
    }

    /// A blank cell with explicit width and style (empty content).
    pub fn new_empty(width: u8, style: Style) -> Self {
        Self {
            content: None,
            width,
            style,
            link: Link::default(),
            dirty: true,
        }
    }

    /// A blank cell in the default style.
    pub fn blank() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.content.is_none()
    }

    /// Clear this cell back to a blank default cell.
    pub fn clear(&mut self) {
        *self = Self::default();
        self.dirty = true;
    }
}

/// A cheaply-cloneable reference to a line of cells. Lines are the unit of
/// scrollback storage and of the touched-line set.
pub type Line = Arc<Vec<Cell>>;

/// A single line of cells.
pub fn new_line(width: usize) -> Vec<Cell> {
    vec![Cell::default(); width]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_default() {
        assert_eq!(Color::default(), Color::Default);
    }

    #[test]
    fn color_indexed() {
        let c = Color::indexed(5);
        assert_eq!(c, Color::Indexed(5));
    }

    #[test]
    fn color_rgb() {
        let c = Color::rgb(10, 20, 30);
        assert_eq!(c, Color::Rgb(10, 20, 30));
    }

    #[test]
    fn decoration_default() {
        let d = Decoration::default();
        assert!(!d.bold);
        assert!(!d.dim);
        assert!(!d.italic);
        assert!(!d.underline);
        assert!(!d.reverse);
        assert!(!d.hidden);
        assert!(!d.strikethrough);
        assert!(!d.overline);
    }

    #[test]
    fn style_new() {
        let s = Style::new();
        assert_eq!(s.fg, Color::Default);
        assert_eq!(s.bg, Color::Default);
        assert!(s.underline_color.is_none());
    }

    #[test]
    fn style_eq_visual_same() {
        let s1 = Style::new();
        let s2 = Style::new();
        assert!(s1.eq_visual(&s2));
    }

    #[test]
    fn style_eq_visual_different_fg() {
        let mut s1 = Style::new();
        s1.fg = Color::indexed(1);
        let s2 = Style::new();
        assert!(!s1.eq_visual(&s2));
    }

    #[test]
    fn style_eq_visual_different_bg() {
        let mut s1 = Style::new();
        s1.bg = Color::indexed(2);
        let s2 = Style::new();
        assert!(!s1.eq_visual(&s2));
    }

    #[test]
    fn style_eq_visual_different_underline_color() {
        let mut s1 = Style::new();
        s1.underline_color = Some(Color::indexed(3));
        let s2 = Style::new();
        assert!(!s1.eq_visual(&s2));
    }

    #[test]
    fn cell_new() {
        let c = Cell::new('A', 1, Style::new());
        assert_eq!(c.content, Some('A'));
        assert_eq!(c.width, 1);
        assert!(c.dirty);
    }

    #[test]
    fn cell_blank() {
        let c = Cell::blank();
        assert!(c.content.is_none());
        assert_eq!(c.width, 0);
    }

    #[test]
    fn cell_is_empty() {
        let c = Cell::default();
        assert!(c.is_empty());
        let c2 = Cell::new('x', 1, Style::new());
        assert!(!c2.is_empty());
    }

    #[test]
    fn cell_clear() {
        let mut c = Cell::new('A', 1, Style::new());
        c.clear();
        assert!(c.is_empty());
        assert!(c.dirty);
    }

    #[test]
    fn new_line_creates_correct_width() {
        let line = new_line(10);
        assert_eq!(line.len(), 10);
        for cell in &line {
            assert!(cell.is_empty());
        }
    }

    #[test]
    fn link_default() {
        let link = Link::default();
        assert!(link.url.is_none());
    }

    #[test]
    fn link_with_url() {
        let link = Link {
            url: Some("https://example.com".into()),
        };
        assert_eq!(link.url.as_deref(), Some("https://example.com"));
    }
}
