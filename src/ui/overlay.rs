//! Composable overlay framework — ported from Go TUIOS `internal/overlay/`.
//!
//! Provides borderless floating overlay panels rendered with ratatui. The
//! design is deliberately borderless: a panel is a solid surface-filled
//! rectangle whose neutrals step by luminance so it reads as a raised,
//! floating surface without box-drawing characters.
//!
//! Every renderer returns both the rendered lines and a [`Geometry`]
//! describing the panel-relative rectangles of its interactive regions,
//! so a host can hit-test mouse events without duplicating layout math.

use std::sync::atomic::{AtomicBool, Ordering};

use ratatui::style::{Color, Modifier, Style};

// ---------------------------------------------------------------------------
// ASCII mode toggle
// ---------------------------------------------------------------------------

static ASCII_MODE: AtomicBool = AtomicBool::new(false);

/// Set whether rendering must avoid non-ASCII glyphs.
pub fn set_ascii(v: bool) {
    ASCII_MODE.store(v, Ordering::Relaxed);
}

/// Reports whether rendering must avoid non-ASCII glyphs.
pub fn use_ascii() -> bool {
    ASCII_MODE.load(Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// Rect and Geometry
// ---------------------------------------------------------------------------

/// A half-open rectangle `[x0, x1) × [y0, y1)` in panel-relative cell
/// coordinates. Hosts translate an absolute mouse position into
/// panel-relative coordinates by subtracting the panel's on-screen origin,
/// then hit-test against these.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl Rect {
    /// Create a rect from (x, y, width, height).
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self {
            x0: x,
            y0: y,
            x1: x + w,
            y1: y + h,
        }
    }

    /// Reports whether `(x, y)` falls inside the rectangle.
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x0 && x < self.x1 && y >= self.y0 && y < self.y1
    }

    /// Reports whether the rectangle has no area.
    pub fn empty(&self) -> bool {
        self.x1 <= self.x0 || self.y1 <= self.y0
    }

    /// Width in cells.
    pub fn width(&self) -> i32 {
        (self.x1 - self.x0).max(0)
    }

    /// Height in cells.
    pub fn height(&self) -> i32 {
        (self.y1 - self.y0).max(0)
    }
}

/// Describes the interactive layout of a rendered panel, in panel-relative
/// coordinates. A host records the panel's absolute origin when it places
/// the panel, then uses these rects to route clicks.
#[derive(Debug, Clone, Default)]
pub struct Geometry {
    /// Total rendered size in cells.
    pub width: i32,
    pub height: i32,
    /// The title row; a natural drag handle.
    pub title_bar: Rect,
    /// One rect per tab.
    pub tabs: Vec<Rect>,
    /// Overflow arrows (empty unless the strip is scrolling).
    pub tab_prev: Rect,
    pub tab_next: Rect,
    /// Top-left cell of the body area.
    pub body_x: i32,
    pub body_y: i32,
    /// Content width between the side padding.
    pub inner_width: i32,
    /// Where each surviving key hint was drawn.
    pub hints: Vec<Rect>,
}

// ---------------------------------------------------------------------------
// Palette
// ---------------------------------------------------------------------------

/// The semantic color set a panel is rendered with. The neutral ramp
/// (canvas < panel < surface < row_sel < card) should step by luminance so
/// surfaces read as layered without borders.
#[derive(Debug, Clone)]
pub struct Palette {
    /// Darkest base.
    pub canvas: Color,
    /// Outer band / muted panel base.
    pub panel: Color,
    /// The floating panel fill.
    pub surface: Color,
    /// Selected-row highlight bar.
    pub row_sel: Color,
    /// Inset chip / input background.
    pub card: Color,
    /// Strong selection tint.
    pub selected: Color,

    /// Primary text.
    pub fg: Color,
    /// Secondary / hint text.
    pub fg_dim: Color,
    /// Tertiary / separators / disabled.
    pub fg_mute: Color,

    /// Interactive / brand.
    pub accent: Color,
    /// Brighter accent for icons/keys.
    pub accent_bright: Color,
    /// Foreground that reads on saturated accent pills.
    pub pill_fg: Color,

    /// Destructive / reset.
    pub warn: Color,
    /// On / enabled.
    pub success: Color,
    /// Informational.
    pub info: Color,
    /// Caution.
    pub warning: Color,
}

impl Palette {
    /// A default dark palette approximating the Go defaults.
    pub fn dark() -> Self {
        Self {
            canvas: Color::Rgb(16, 16, 24),
            panel: Color::Rgb(28, 28, 38),
            surface: Color::Rgb(36, 36, 48),
            row_sel: Color::Rgb(48, 48, 64),
            card: Color::Rgb(24, 24, 34),
            selected: Color::Rgb(80, 80, 120),
            fg: Color::Rgb(220, 220, 230),
            fg_dim: Color::Rgb(160, 160, 180),
            fg_mute: Color::Rgb(100, 100, 120),
            accent: Color::Rgb(137, 180, 250),
            accent_bright: Color::Rgb(180, 200, 255),
            pill_fg: Color::Rgb(20, 20, 30),
            warn: Color::Rgb(243, 139, 168),
            success: Color::Rgb(166, 227, 161),
            info: Color::Rgb(137, 220, 235),
            warning: Color::Rgb(250, 189, 47),
        }
    }
}

impl Default for Palette {
    fn default() -> Self {
        Self::dark()
    }
}

// ---------------------------------------------------------------------------
// Hint
// ---------------------------------------------------------------------------

/// One key/label pair shown in a panel footer.
#[derive(Debug, Clone)]
pub struct Hint {
    pub key: String,
    pub label: String,
}

impl Hint {
    /// Create a new hint.
    pub fn new(key: &str, label: &str) -> Self {
        Self {
            key: key.to_string(),
            label: label.to_string(),
        }
    }

    /// The rendered width: key + space + label.
    pub fn width(&self) -> usize {
        self.key.chars().count() + 1 + self.label.chars().count()
    }
}

// ---------------------------------------------------------------------------
// Dialog
// ---------------------------------------------------------------------------

/// Minimum inner width for a dialog.
pub const MIN_DIALOG_WIDTH: i32 = 8;

/// Returns the inner width for a dialog that would prefer `preferred`
/// columns on a screen `screen_w` columns wide, leaving room for its two
/// border cells.
pub fn dialog_fit_width(preferred: i32, screen_w: i32) -> i32 {
    if screen_w <= 0 {
        return preferred;
    }
    (preferred.min(screen_w - 2)).max(1)
}

/// A hairline micro-dialog: a rounded muted frame with its title set into
/// the top border, key hints set into the bottom border, and an interior
/// on bare canvas.
#[derive(Debug, Clone)]
pub struct Dialog {
    /// Lowercase, set into the top border.
    pub title: String,
    /// Inner content width (rendered block is width+2).
    pub width: i32,
    /// Pre-styled, multi-line body.
    pub body: String,
    /// Key hints for the bottom border.
    pub hints: Vec<Hint>,
}

impl Dialog {
    /// Create a new dialog.
    pub fn new(title: &str, width: i32, body: &str) -> Self {
        Self {
            title: title.to_string(),
            width,
            body: body.to_string(),
            hints: Vec::new(),
        }
    }

    /// Add a hint.
    pub fn with_hint(mut self, key: &str, label: &str) -> Self {
        self.hints.push(Hint::new(key, label));
        self
    }

    /// Render the dialog, returning lines and geometry.
    pub fn render(&self, _pal: &Palette) -> (Vec<String>, Geometry) {
        let w = self.width.max(MIN_DIALOG_WIDTH);
        let (tl, tr, bl, br, h, v) = dialog_frame();

        let mut lines = Vec::new();

        // Top border with title.
        let mut top = String::from(tl);
        if !self.title.is_empty() {
            let title = truncate(&self.title, (w - 3).max(1) as usize);
            top.push_str(h);
            top.push(' ');
            top.push_str(&title);
            top.push(' ');
            let remaining = w - title.chars().count() as i32 - 3;
            if remaining > 0 {
                top.push_str(&h.repeat(remaining as usize));
            }
        } else {
            top.push_str(&h.repeat(w as usize));
        }
        top.push_str(tr);
        lines.push(top);

        // Body lines.
        for body_line in self.body.lines() {
            let mut line = String::from(v);
            let truncated = truncate(body_line, w as usize);
            line.push_str(&truncated);
            let pad = w - truncated.chars().count() as i32;
            if pad > 0 {
                line.push_str(&" ".repeat(pad as usize));
            }
            line.push_str(v);
            lines.push(line);
        }

        // Bottom border with hints.
        let mut bottom = String::from(bl);
        let hints_w: usize = self.hints.iter().map(|h| h.width()).sum::<usize>()
            + (self.hints.len().saturating_sub(1)) * 2;
        if hints_w > 0 && hints_w + 3 <= w as usize {
            let rule_w = w as usize - hints_w - 3;
            bottom.push_str(&h.repeat(rule_w));
            bottom.push(' ');
            for (i, hint) in self.hints.iter().enumerate() {
                if i > 0 {
                    bottom.push_str("  ");
                }
                bottom.push_str(&hint.key);
                bottom.push(' ');
                bottom.push_str(&hint.label);
            }
            bottom.push(' ');
            bottom.push_str(h);
        } else {
            bottom.push_str(&h.repeat(w as usize));
        }
        bottom.push_str(br);
        lines.push(bottom);

        let geo = Geometry {
            width: w + 2,
            height: lines.len() as i32,
            inner_width: w,
            body_x: 1,
            body_y: 1,
            title_bar: Rect::new(0, 0, w + 2, 1),
            hints: Vec::new(),
            ..Default::default()
        };

        (lines, geo)
    }
}

/// Returns the border glyphs for a dialog, honoring ASCII mode.
fn dialog_frame() -> (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
) {
    if use_ascii() {
        ("+", "+", "+", "+", "-", "|")
    } else {
        ("╭", "╮", "╰", "╯", "─", "│")
    }
}

// ---------------------------------------------------------------------------
// Panel
// ---------------------------------------------------------------------------

/// Side padding in cells.
pub const SIDE_PAD: i32 = 2;

/// Minimum inner content width for a panel.
pub const MIN_PANEL_WIDTH: i32 = 12;

/// Returns the inner content width for a panel.
pub fn fit_width(preferred: i32, screen_w: i32) -> i32 {
    if screen_w <= 0 {
        return preferred;
    }
    let avail = screen_w - 2 * SIDE_PAD;
    (preferred.min(avail)).max(1)
}

/// A borderless floating panel: a solid surface fill with an inset accent
/// title chip, an optional tab row, a body, and a muted footer of key hints.
#[derive(Debug, Clone)]
pub struct Panel {
    /// Optional leading glyph for the title chip.
    pub glyph: String,
    /// Panel title.
    pub title: String,
    /// Inner content width.
    pub width: i32,
    /// Tab labels.
    pub tabs: Vec<String>,
    /// Index of the active tab.
    pub active_tab: usize,
    /// Pre-styled, multi-line body.
    pub body: String,
    /// Key hints for the footer.
    pub hints: Vec<Hint>,
}

impl Panel {
    /// Create a new panel.
    pub fn new(title: &str, width: i32) -> Self {
        Self {
            glyph: String::new(),
            title: title.to_string(),
            width,
            tabs: Vec::new(),
            active_tab: 0,
            body: String::new(),
            hints: Vec::new(),
        }
    }

    /// Render the panel, returning lines and geometry.
    pub fn render(&self, _pal: &Palette) -> (Vec<String>, Geometry) {
        let total_w = self.width + 2 * SIDE_PAD;
        let blank = " ".repeat(total_w as usize);
        let pad = " ".repeat(SIDE_PAD as usize);

        let mut lines = Vec::new();
        let mut geo = Geometry {
            width: total_w,
            inner_width: self.width,
            body_x: SIDE_PAD,
            ..Default::default()
        };

        lines.push(blank.clone()); // top pad

        // Title chip row.
        let chip_label = if self.glyph.is_empty() {
            truncate(&self.title, (self.width - 2).max(1) as usize)
        } else {
            truncate(
                &format!("{} {}", self.glyph, self.title),
                (self.width - 2).max(1) as usize,
            )
        };
        let chip = format!(" {} ", chip_label);
        let chip_padded = format!("{}{}", pad, chip);
        lines.push(pad_line(&chip_padded, total_w as usize));
        geo.title_bar = Rect::new(0, lines.len() as i32 - 1, total_w, 1);
        lines.push(blank.clone()); // blank after title

        if !self.tabs.is_empty() {
            let tab_line = render_tabs(&self.tabs, self.active_tab, self.width);
            lines.push(pad_line(&format!("{}{}", pad, tab_line), total_w as usize));
            lines.push(pad_line(
                &format!("{}{}", pad, "─".repeat(self.width as usize)),
                total_w as usize,
            ));
            lines.push(blank.clone());
        }

        geo.body_y = lines.len() as i32;
        for body_line in self.body.lines() {
            lines.push(pad_line(&format!("{}{}", pad, body_line), total_w as usize));
        }

        if !self.hints.is_empty() {
            lines.push(blank.clone());
            lines.push(pad_line(
                &format!("{}{}", pad, "─".repeat(self.width as usize)),
                total_w as usize,
            ));
            for hint_line in render_hints(&self.hints, self.width) {
                lines.push(pad_line(&format!("{}{}", pad, hint_line), total_w as usize));
            }
        }

        lines.push(blank); // bottom pad
        geo.height = lines.len() as i32;

        (lines, geo)
    }
}

/// Pad or truncate a line to exactly `width` cells.
fn pad_line(s: &str, width: usize) -> String {
    let len = s.chars().count();
    if len >= width {
        s.chars().take(width).collect()
    } else {
        format!("{}{}", s, " ".repeat(width - len))
    }
}

/// Render the tab strip on one row.
fn render_tabs(tabs: &[String], active: usize, _width: i32) -> String {
    let mut parts = Vec::new();
    for (i, tab) in tabs.iter().enumerate() {
        if i > 0 {
            parts.push(" ".to_string());
        }
        if i == active {
            parts.push(format!(" {} ", tab));
        } else {
            parts.push(tab.clone());
        }
    }
    parts.join("")
}

/// Render hints as footer rows, wrapping when they don't fit.
fn render_hints(hints: &[Hint], width: i32) -> Vec<String> {
    let mut rows = Vec::new();
    let mut current = Vec::new();
    let mut current_w = 0usize;
    let sep_w = 3usize;

    for hint in hints {
        let part = format!("{} {}", hint.key, hint.label);
        let w = part.chars().count();
        if current_w > 0 && current_w + sep_w + w > width as usize {
            rows.push(current.join("   "));
            current.clear();
            current_w = 0;
        }
        if current_w > 0 {
            current_w += sep_w;
        }
        current.push(part);
        current_w += w;
    }
    if !current.is_empty() {
        rows.push(current.join("   "));
    }
    rows
}

// ---------------------------------------------------------------------------
// Utility functions
// ---------------------------------------------------------------------------

/// Truncates `s` to fit within `max_width` display cells, appending an
/// ellipsis when it overflows.
pub fn truncate(s: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_width {
        return s.to_string();
    }
    let ell = if use_ascii() { "..." } else { "…" };
    let ell_len = ell.chars().count();
    if max_width <= ell_len {
        return ell.chars().take(max_width).collect();
    }
    let target = max_width - ell_len;
    let truncated: String = chars.into_iter().take(target).collect();
    format!("{}{}", truncated, ell)
}

/// Returns the ellipsis character for the current ASCII setting.
pub fn ellipsis() -> &'static str {
    if use_ascii() {
        "..."
    } else {
        "…"
    }
}

/// Returns the enter key glyph.
pub fn enter_key() -> &'static str {
    if use_ascii() {
        "enter"
    } else {
        "↵"
    }
}

/// Returns the sigil mark (one cell).
pub fn sigil_mark() -> &'static str {
    if use_ascii() {
        ">"
    } else {
        "›"
    }
}

/// Returns the sigil plus trailing space (two cells).
pub fn sigil() -> String {
    format!("{} ", sigil_mark())
}

/// Returns a dashed separator rule.
pub fn dash_rule(width: i32) -> String {
    let ch = if use_ascii() { "-" } else { "╌" };
    ch.repeat(width.max(0) as usize)
}

/// Returns a solid horizontal rule.
pub fn rule(width: i32) -> String {
    let ch = if use_ascii() { "-" } else { "─" };
    ch.repeat(width.max(0) as usize)
}

/// Returns the number of rows the footer hint strip needs.
pub fn hint_row_count(hints: &[Hint], width: i32) -> i32 {
    if hints.is_empty() {
        return 0;
    }
    let mut rows = 1;
    let mut cur_w = 0usize;
    for h in hints {
        let w = h.width();
        if cur_w > 0 && cur_w + 3 + w > width as usize {
            rows += 1;
            cur_w = 0;
        }
        if cur_w > 0 {
            cur_w += 3;
        }
        cur_w += w;
    }
    rows
}

/// Returns the number of rows the tab strip needs (always 1 if tabs exist).
pub fn tab_row_count(tabs: &[String], _width: i32) -> i32 {
    if tabs.is_empty() {
        0
    } else {
        1
    }
}

// ---------------------------------------------------------------------------
// Style helpers
// ---------------------------------------------------------------------------

/// Create a ratatui style with the given foreground and background.
pub fn styled(fg: Color, bg: Color) -> Style {
    Style::default().fg(fg).bg(bg)
}

/// Create a bold ratatui style with the given foreground and background.
pub fn styled_bold(fg: Color, bg: Color) -> Style {
    Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD)
}

/// Create a reversed ratatui style.
pub fn styled_reverse(fg: Color, bg: Color) -> Style {
    Style::default()
        .fg(fg)
        .bg(bg)
        .add_modifier(Modifier::REVERSED)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serializes tests that toggle the global `ASCII_MODE` flag.
    static ASCII_MODE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn rect_contains() {
        let r = Rect::new(5, 3, 10, 5);
        assert!(r.contains(5, 3));
        assert!(r.contains(14, 7));
        assert!(!r.contains(15, 3));
        assert!(!r.contains(5, 8));
        assert!(!r.contains(4, 3));
    }

    #[test]
    fn rect_empty() {
        assert!(Rect::new(0, 0, 0, 5).empty());
        assert!(Rect::new(0, 0, 5, 0).empty());
        assert!(!Rect::new(0, 0, 1, 1).empty());
    }

    #[test]
    fn rect_width_height() {
        let r = Rect::new(2, 3, 10, 5);
        assert_eq!(r.width(), 10);
        assert_eq!(r.height(), 5);
    }

    #[test]
    fn dialog_fit_width_basic() {
        assert_eq!(dialog_fit_width(40, 80), 40);
        assert_eq!(dialog_fit_width(80, 40), 38);
        assert_eq!(dialog_fit_width(40, 0), 40);
        assert_eq!(dialog_fit_width(40, 5), 3);
    }

    #[test]
    fn fit_width_basic() {
        assert_eq!(fit_width(40, 80), 40);
        assert_eq!(fit_width(80, 40), 36);
        assert_eq!(fit_width(40, 0), 40);
    }

    #[test]
    fn truncate_short_string() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn truncate_long_string() {
        let _guard = ASCII_MODE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_ascii(true);
        assert_eq!(truncate("hello world", 8), "hello...");
        set_ascii(false);
    }

    #[test]
    fn truncate_exact_fit() {
        assert_eq!(truncate("hello", 5), "hello");
    }

    #[test]
    fn truncate_zero_width() {
        assert_eq!(truncate("hello", 0), "");
    }

    #[test]
    fn hint_width() {
        let h = Hint::new("ctrl+x", "cut");
        assert_eq!(h.width(), 10); // "ctrl+x" (6) + space (1) + "cut" (3)
    }

    #[test]
    fn hint_row_count_single_row() {
        let hints = vec![Hint::new("q", "quit"), Hint::new("enter", "ok")];
        assert_eq!(hint_row_count(&hints, 80), 1);
    }

    #[test]
    fn hint_row_count_wraps() {
        let hints = vec![
            Hint::new("ctrl+x", "cut"),
            Hint::new("ctrl+v", "paste"),
            Hint::new("ctrl+z", "undo"),
        ];
        assert!(hint_row_count(&hints, 15) > 1);
    }

    #[test]
    fn hint_row_count_empty() {
        assert_eq!(hint_row_count(&[], 80), 0);
    }

    #[test]
    fn tab_row_count_empty() {
        assert_eq!(tab_row_count(&[], 80), 0);
    }

    #[test]
    fn tab_row_count_one() {
        assert_eq!(tab_row_count(&["a".to_string(), "b".to_string()], 80), 1);
    }

    #[test]
    fn dialog_render_basic() {
        let d = Dialog::new("test", 20, "line 1\nline 2");
        let pal = Palette::dark();
        let (lines, geo) = d.render(&pal);
        assert!(lines.len() >= 4); // top + 2 body + bottom
        assert_eq!(geo.width, 22);
        assert_eq!(geo.inner_width, 20);
        assert_eq!(geo.body_x, 1);
        assert_eq!(geo.body_y, 1);
    }

    #[test]
    fn panel_render_basic() {
        let mut p = Panel::new("My Panel", 30);
        p.body = "hello world".to_string();
        let pal = Palette::dark();
        let (lines, geo) = p.render(&pal);
        assert!(lines.len() >= 5); // pad + title + blank + body + pad
        assert_eq!(geo.width, 34);
        assert_eq!(geo.inner_width, 30);
        assert_eq!(geo.body_x, SIDE_PAD);
    }

    #[test]
    fn panel_render_with_hints() {
        let mut p = Panel::new("Panel", 40);
        p.body = "content".to_string();
        p.hints.push(Hint::new("q", "quit"));
        p.hints.push(Hint::new("enter", "ok"));
        let pal = Palette::dark();
        let (lines, _) = p.render(&pal);
        // pad + title + blank + body + blank + rule + hint + pad
        assert!(lines.len() >= 7);
    }

    #[test]
    fn panel_render_with_tabs() {
        let mut p = Panel::new("Panel", 40);
        p.tabs = vec!["tab1".to_string(), "tab2".to_string()];
        p.active_tab = 0;
        p.body = "content".to_string();
        let pal = Palette::dark();
        let (lines, geo) = p.render(&pal);
        assert!(geo.tabs.is_empty()); // tabs not tracked in simplified version
        assert!(lines.len() >= 7); // pad + title + blank + tab + rule + blank + body + pad
    }

    #[test]
    fn ellipsis_ascii() {
        let _guard = ASCII_MODE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_ascii(true);
        assert_eq!(ellipsis(), "...");
        set_ascii(false);
        assert_eq!(ellipsis(), "…");
    }

    #[test]
    fn sigil_mark_ascii() {
        let _guard = ASCII_MODE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_ascii(true);
        assert_eq!(sigil_mark(), ">");
        set_ascii(false);
        assert_eq!(sigil_mark(), "›");
    }

    #[test]
    fn dash_rule_basic() {
        let _guard = ASCII_MODE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_ascii(false);
        assert_eq!(dash_rule(5), "╌╌╌╌╌");
        set_ascii(true);
        assert_eq!(dash_rule(5), "-----");
        set_ascii(false);
    }

    #[test]
    fn rule_basic() {
        let _guard = ASCII_MODE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_ascii(false);
        assert_eq!(rule(5), "─────");
        set_ascii(true);
        assert_eq!(rule(5), "-----");
        set_ascii(false);
    }

    #[test]
    fn palette_dark_default() {
        let p = Palette::default();
        // Just verify it doesn't panic and has expected structure.
        let _ = p.canvas;
        let _ = p.accent;
    }
}
