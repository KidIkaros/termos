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
    /// The character content (a grapheme cluster).
    pub content: String,
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
    pub fn new(content: impl Into<String>, width: u8, style: Style) -> Self {
        Self {
            content: content.into(),
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
        self.content.is_empty()
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
