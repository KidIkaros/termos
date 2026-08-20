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
    /// The base character: a single `char` when occupied, `None` when blank.
    /// Stored inline (no heap allocation) — the VT parser emits one base
    /// `char` per `print` call.
    pub content: Option<char>,
    /// Display width in columns (1 for normal, 2 for wide CJK, 0 for
    /// continuation cells of a wide rune).
    pub width: u8,
    /// Zero-width combining marks that render together with `content` as a
    /// single grapheme (e.g. `e` + U+0301 → `é`). The first
    /// [`MAX_COMBINING`] marks are stored inline (no heap); any beyond that
    /// spill into [`Self::combining_overflow`]. Real scripts (Devanagari
    /// virama+matra stacks, Hangul jamo, polytonic Greek, Vietnamese)
    /// fit within the inline budget; the spill exists for pathological
    /// linguistic sequences and is shared via `Arc` so scrollback copies
    /// never duplicate it.
    pub combining: [char; MAX_COMBINING],
    /// How many combining marks are live in total (inline + overflow).
    pub combining_len: u8,
    /// Marks beyond the inline [`MAX_COMBINING`]. `None` for the common case.
    pub combining_overflow: Option<Arc<Vec<char>>>,
    /// The graphical rendition.
    pub style: Style,
    /// Hyperlink, if any.
    pub link: Link,
    /// Whether this cell was modified since the last clear of the touched set.
    pub dirty: bool,
}

/// How many zero-width combining marks a single cell holds inline before
/// spilling to the heap. Covers Latin/Greek/Cyrillic accented text (1-3
/// marks), Devanagari consonant stacks (virama + matras + nukta, ~3), Hangul
/// jamo (lead + vowel + final, 2), and Vietnamese stacking. The inline budget
/// keeps the render hot path allocation-free for every realistic script.
pub const MAX_COMBINING: usize = 4;

impl Cell {
    pub fn new(content: char, width: u8, style: Style) -> Self {
        Self {
            content: Some(content),
            width,
            combining: ['\0'; MAX_COMBINING],
            combining_len: 0,
            combining_overflow: None,
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
            combining: ['\0'; MAX_COMBINING],
            combining_len: 0,
            combining_overflow: None,
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

    /// The inline (first [`MAX_COMBINING`]) combining marks.
    pub fn combining_slice(&self) -> &[char] {
        &self.combining[..(self.combining_len as usize).min(MAX_COMBINING)]
    }

    /// Visit every combining mark (inline then overflow) in order.
    pub fn for_each_combining(&self, mut f: impl FnMut(char)) {
        for &m in self.combining_slice() {
            f(m);
        }
        if let Some(overflow) = &self.combining_overflow {
            for &m in overflow.iter() {
                f(m);
            }
        }
    }

    /// Append the full grapheme (base + combining marks) to `out`.
    pub fn push_grapheme_into(&self, out: &mut String) {
        if let Some(ch) = self.content {
            out.push(ch);
        } else {
            out.push(' ');
        }
        self.for_each_combining(|m| out.push(m));
    }

    /// The full grapheme this cell renders (base + combining marks), or
    /// `None` when blank. Allocates — prefer [`Self::push_grapheme_into`]
    /// in hot paths.
    pub fn grapheme(&self) -> Option<String> {
        self.content.map(|ch| {
            let mut s = String::with_capacity(1 + self.combining_len as usize);
            s.push(ch);
            self.for_each_combining(|m| s.push(m));
            s
        })
    }

    /// Attach a zero-width combining mark. Inline until [`MAX_COMBINING`],
    /// then spilled to a shared heap buffer — unbounded.
    pub fn push_combining(&mut self, c: char) {
        let idx = self.combining_len as usize;
        if idx < MAX_COMBINING {
            self.combining[idx] = c;
        } else {
            let overflow = self.combining_overflow.get_or_insert_with(|| Arc::new(Vec::new()));
            Arc::make_mut(overflow).push(c);
        }
        self.combining_len += 1;
        self.dirty = true;
    }

    /// Clear this cell back to a blank default cell.
    pub fn clear(&mut self) {
        *self = Self::default();
        self.dirty = true;
    }
}

/// One rendered column: the base char, its zero-width combining marks, and
/// the style. Inline marks are stored inline; spills share the cell's
/// overflow buffer via `Arc`, so building a styled row never copies mark
/// data (only refcount bumps). Clone, not Copy — constructing a row copies
/// the small fields and bumps a refcount at most.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyledChar {
    pub content: char,
    pub combining: [char; MAX_COMBINING],
    pub combining_len: u8,
    pub combining_overflow: Option<Arc<Vec<char>>>,
    /// Terminal width of this column (1, or 2 for a wide lead). The renderer
    /// uses it to leave the continuation cell blank so the terminal's wide
    /// glyph is not overwritten by the next column.
    pub width: u8,
    pub style: Style,
}

impl StyledChar {
    pub fn combining_slice(&self) -> &[char] {
        &self.combining[..(self.combining_len as usize).min(MAX_COMBINING)]
    }

    /// Visit every combining mark (inline then overflow) in order.
    pub fn for_each_combining(&self, mut f: impl FnMut(char)) {
        for &m in self.combining_slice() {
            f(m);
        }
        if let Some(overflow) = &self.combining_overflow {
            for &m in overflow.iter() {
                f(m);
            }
        }
    }

    /// Whether this column renders as more than a single code point.
    pub fn has_combining(&self) -> bool {
        self.combining_len > 0
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
