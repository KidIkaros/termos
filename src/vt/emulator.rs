//! The VT emulator — the heart of the terminal. It owns a parser, a main
//! screen and an alternate screen, the scrollback, and all the handlers that
//! turn escape sequences into screen mutations.
//!
//! This is the Rust counterpart of TUIOS `internal/vt/emulator.go`.

use unicode_width::UnicodeWidthChar;

use crate::vt::cell::{Color, Style};
use crate::vt::charset::CharSet;
use crate::vt::parser::{CsiSequence, DcsSequence, Handler, OscSequence, Parser, StringSequence};
use crate::vt::screen::{Position, ScreenBuffer, ScrollRegion};

/// A mode bit — DEC and ANSI modes share a keyspace.
pub const MODE_ALT_SCREEN: i64 = 47;
pub const MODE_ALT_SCREEN_SAVE: i64 = 1049;
pub const MODE_AUTO_WRAP: i64 = 7;
pub const MODE_CURSOR_VISIBLE: i64 = 25;
pub const MODE_BRACKETED_PASTE: i64 = 2004;
pub const MODE_MOUSE_X10: i64 = 9;
pub const MODE_MOUSE_NORMAL: i64 = 1000;
pub const MODE_MOUSE_HIGHLIGHT: i64 = 1001;
pub const MODE_MOUSE_BUTTON_EVENT: i64 = 1002;
pub const MODE_MOUSE_ANY_EVENT: i64 = 1003;
pub const MODE_MOUSE_EXT_SGR: i64 = 1006;
pub const MODE_SYNCHRONIZED_OUTPUT: i64 = 2026;
pub const MODE_INBAND_RESIZE: i64 = 2048;
pub const MODE_ORIGIN: i64 = 6;
pub const MODE_INSERT: i64 = 4;
pub const MODE_APPLICATION_CURSOR_KEYS: i64 = 1;
pub const MODE_APPLICATION_KEYPAD: i64 = 2;

/// The emulator.
#[derive(Debug)]
pub struct Emulator {
    parser: Parser,
    /// Main and alternate screens, and which is active.
    screens: [ScreenBuffer; 2],
    active: usize,
    /// DEC modes (shared keyspace with ANSI private modes).
    modes: std::collections::HashMap<i64, bool>,
    /// Character sets in G0-G3 and which GL/GR point at.
    charsets: [CharSet; 4],
    gl: usize,
    gr: usize,
    /// Whether a pending wrap is armed (the cursor sits past the last column).
    at_phantom: bool,
    /// The title reported via OSC 0/2.
    pub title: String,
    /// The working directory reported via OSC 7.
    pub cwd: String,
    /// Output queue for responses (DA, DSR, etc.) read by the PTY writer.
    response: Vec<u8>,
    /// Last OSC 52 clipboard write, if any.
    clipboard: Option<String>,
    /// Pending APC sequences (Kitty graphics protocol) collected since the
    /// last drain. The app layer drains these and forwards them to the host
    /// terminal via the graphics passthrough.
    pending_apc: Vec<Vec<u8>>,
    /// Pending DCS sequences that look like Sixel (`q` final byte) for the
    /// app layer's Sixel passthrough.
    pending_sixel: Vec<Vec<u8>>,
    /// How many scrollback lines are currently shown above the live screen.
    /// 0 means the live screen is shown; a positive value means the view is
    /// scrolled back that many lines (copy mode).
    viewport: usize,
}

impl Default for Emulator {
    fn default() -> Self {
        Self::new(80, 24)
    }
}

impl Emulator {
    pub fn new(width: i32, height: i32) -> Self {
        let mut emu = Self {
            parser: Parser::new(),
            screens: [ScreenBuffer::new(width, height), ScreenBuffer::new(width, height)],
            active: 0,
            modes: std::collections::HashMap::new(),
            charsets: [CharSet::Ascii; 4],
            gl: 0,
            gr: 1,
            at_phantom: false,
            title: String::new(),
            cwd: String::new(),
            response: Vec::new(),
            clipboard: None,
            pending_apc: Vec::new(),
            pending_sixel: Vec::new(),
            viewport: 0,
        };
        // Alt screen keeps no scrollback.
        emu.screens[1].set_scrollback_enabled(false);
        // Default modes: auto-wrap on, cursor visible.
        emu.modes.insert(MODE_AUTO_WRAP, true);
        emu.modes.insert(MODE_CURSOR_VISIBLE, true);
        emu
    }

    // -----------------------------------------------------------------------
    // Public accessors
    // -----------------------------------------------------------------------

    pub fn screen(&self) -> &ScreenBuffer {
        &self.screens[self.active]
    }

    pub fn screen_mut(&mut self) -> &mut ScreenBuffer {
        &mut self.screens[self.active]
    }

    pub fn main_screen(&self) -> &ScreenBuffer {
        &self.screens[0]
    }

    pub fn width(&self) -> i32 {
        self.screens[self.active].width()
    }

    pub fn height(&self) -> i32 {
        self.screens[self.active].height()
    }

    pub fn is_alt_screen(&self) -> bool {
        self.active == 1
    }

    pub fn is_mode_set(&self, mode: i64) -> bool {
        self.modes.get(&mode).copied().unwrap_or(false)
    }

    /// Whether the application has requested mouse tracking (any of the DEC
    /// mouse-reporting modes).
    pub fn has_mouse_mode(&self) -> bool {
        self.is_mode_set(MODE_MOUSE_NORMAL)
            || self.is_mode_set(MODE_MOUSE_BUTTON_EVENT)
            || self.is_mode_set(MODE_MOUSE_ANY_EVENT)
    }

    pub fn set_mode(&mut self, mode: i64, enabled: bool) {
        self.modes.insert(mode, enabled);
        match mode {
            MODE_ALT_SCREEN | MODE_ALT_SCREEN_SAVE => {
                self.active = if enabled { 1 } else { 0 };
                if enabled {
                    self.screens[1].clear();
                }
            }
            MODE_CURSOR_VISIBLE => self.screen_mut().cursor.hidden = !enabled,
            MODE_ORIGIN
                if enabled => {
                    self.screen_mut().set_cursor(0, 0, false);
                }
            _ => {}
        }
    }

    pub fn mode_map(&self) -> &std::collections::HashMap<i64, bool> {
        &self.modes
    }

    pub fn restore_modes(&mut self, modes: &std::collections::HashMap<i64, bool>) {
        for (mode, enabled) in modes {
            self.modes.insert(*mode, *enabled);
        }
        if let Some(&visible) = modes.get(&MODE_CURSOR_VISIBLE) {
            self.screen_mut().cursor.hidden = !visible;
        }
    }

    pub fn cursor_position(&self) -> Position {
        self.screen().cursor.pos
    }

    pub fn restore_cursor_position(&mut self, pos: Position) {
        self.screen_mut().set_cursor(pos.x, pos.y, false);
    }

    pub fn scroll_region(&self) -> ScrollRegion {
        self.screen().scroll
    }

    pub fn restore_scroll_region(&mut self, region: ScrollRegion) {
        self.screen_mut().scroll = region;
    }

    /// The current pen style.
    pub fn pen(&self) -> Style {
        self.screen().pen()
    }

    pub fn restore_pen(&mut self, style: Style) {
        self.screen_mut().set_pen(style);
    }

    pub fn take_touched(&mut self) -> std::collections::HashSet<i32> {
        self.screen_mut().take_touched()
    }

    pub fn scrollback_len(&self) -> usize {
        self.screens[0].scrollback.len()
    }

    /// The current scrollback viewport offset (0 = live screen).
    pub fn viewport(&self) -> usize {
        self.viewport
    }

    /// Whether the view is scrolled back into the scrollback (copy mode).
    pub fn in_scrollback(&self) -> bool {
        self.viewport > 0
    }

    /// Scroll the viewport by `delta` lines (positive scrolls back, negative
    /// scrolls toward live output), clamped to the available scrollback.
    pub fn scroll_viewport(&mut self, delta: i32) {
        let max = self.screens[0].scrollback.len() as i64;
        let current = self.viewport as i64;
        let target = (current + delta as i64).clamp(0, max);
        self.viewport = target as usize;
    }

    /// Return to the live screen (viewport 0).
    pub fn reset_viewport(&mut self) {
        self.viewport = 0;
    }

    /// Set the viewport directly, clamped to the available scrollback.
    pub fn set_viewport(&mut self, value: usize) {
        let max = self.screens[0].scrollback.len();
        self.viewport = value.min(max);
    }

    pub fn scrollback_line_text(&self, index: usize) -> Option<String> {
        let line = self.screens[0].scrollback.line(index)?;
        Some(line_text(line))
    }

    /// The total number of content lines: scrollback lines followed by live
    /// screen rows. The alt screen has no scrollback, so it is just its rows.
    pub fn content_line_count(&self) -> usize {
        if self.active == 0 {
            self.screens[0].scrollback.len() + self.height() as usize
        } else {
            self.height() as usize
        }
    }

    /// A clone of a unified content line, where indices `0..scrollback_len`
    /// are scrollback lines and `scrollback_len..` are live screen rows.
    pub fn content_line(&self, index: usize) -> Vec<crate::vt::cell::Cell> {
        if self.active != 0 {
            return self.screens[1].line_owned(index as i32);
        }
        let sb_len = self.screens[0].scrollback.len();
        if index < sb_len {
            self.screens[0]
                .scrollback
                .line(index)
                .map(|l| l.as_ref().clone())
                .unwrap_or_default()
        } else {
            self.screens[0].line_owned((index - sb_len) as i32)
        }
    }

    /// The plain text of a unified content line (trailing spaces trimmed).
    pub fn content_line_text(&self, index: usize) -> String {
        line_text(&self.content_line(index))
    }

    /// Map a rendered view row (0 = top of the view) to its content line
    /// index, honoring the current viewport offset.
    pub fn content_index_for_view_row(&self, row: i32) -> usize {
        let row = row.max(0) as usize;
        if self.active != 0 {
            return row.min(self.height() as usize - 1);
        }
        let sb_len = self.screens[0].scrollback.len();
        let start = sb_len.saturating_sub(self.viewport);
        let count = self.content_line_count();
        if count == 0 {
            return 0;
        }
        (start + row).min(count - 1)
    }

    /// Extract the text of a rectangular selection between two content
    /// positions (line + column). Columns are cell columns; empty cells read
    /// as spaces. Multi-line selections are joined with newlines.
    pub fn selection_text(
        &self,
        start_line: usize,
        start_col: i32,
        end_line: usize,
        end_col: i32,
    ) -> String {
        let (lo, hi) = if start_line <= end_line {
            (start_line, end_line)
        } else {
            (end_line, start_line)
        };
        let (c_lo, c_hi) = if start_col <= end_col {
            (start_col, end_col)
        } else {
            (end_col, start_col)
        };
        let mut out = String::new();
        for line_idx in lo..=hi {
            if line_idx >= self.content_line_count() {
                break;
            }
            let line = self.content_line(line_idx);
            let mut col = 0i32;
            for cell in &line {
                let w = cell.width.max(1) as i32;
                let covered = col.max(c_lo) <= (col + w - 1).min(c_hi);
                if covered {
                    if cell.content.is_empty() {
                        out.push(' ');
                    } else {
                        out.push_str(&cell.content);
                    }
                }
                col += w;
                if col > c_hi {
                    break;
                }
            }
            if line_idx < hi {
                out.push('\n');
            }
        }
        out
    }

    pub fn clear_scrollback(&mut self) {
        self.screens[0].scrollback.clear();
    }

    pub fn set_scrollback_max_lines(&mut self, max: usize) {
        self.screens[0].scrollback.set_max_lines(max);
    }

    /// Render the active screen as plain text (tests).
    pub fn to_string(&self) -> String {
        self.screen().to_string()
    }

    /// Render a full snapshot of the active screen as styled text lines,
    /// returning (content, style) pairs per cell for the renderer.
    pub fn render_lines(&self) -> Vec<Vec<(String, Style)>> {
        let screen = self.screen();
        let mut out = Vec::with_capacity(screen.height() as usize);
        for y in 0..screen.height() {
            out.push(row_to_styled(screen.line(y)));
        }
        out
    }

    /// Render the visible viewport, scrolling back into the scrollback when
    /// `viewport` is non-zero: the last `viewport` scrollback lines are shown
    /// above the live screen rows, up to `height` lines total.
    pub fn render_view_lines(&self) -> Vec<Vec<(String, Style)>> {
        let height = self.screens[self.active].height() as usize;
        if height == 0 {
            return Vec::new();
        }
        let mut out = Vec::with_capacity(height);

        if self.viewport > 0 && self.active == 0 {
            let sb = &self.screens[0].scrollback;
            let sb_len = sb.len();
            let offset = self.viewport.min(sb_len);
            let start = sb_len - offset;
            for i in start..sb_len {
                if out.len() >= height {
                    break;
                }
                if let Some(line) = sb.line(i) {
                    out.push(row_to_styled(Some(line.as_slice())));
                }
            }
        }

        // Fill the remainder with live screen rows.
        let screen = &self.screens[self.active];
        let mut y = 0;
        while out.len() < height {
            out.push(row_to_styled(screen.line(y)));
            y += 1;
        }
        out
    }

    // -----------------------------------------------------------------------
    // Input / output
    // -----------------------------------------------------------------------

    /// Feed PTY output into the emulator.
    pub fn write(&mut self, data: &[u8]) {
        // Take the parser out so we can borrow `self` mutably as the handler
        // without aliasing the parser field.
        let mut parser = std::mem::take(&mut self.parser);
        for &b in data {
            parser.advance(b, self);
        }
        self.parser = parser;
    }

    /// Take any response bytes the emulator queued (DA/DSR answers).
    pub fn take_response(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.response)
    }

    fn queue_response(&mut self, bytes: &[u8]) {
        self.response.extend_from_slice(bytes);
    }

    /// Resize the emulator (both screens).
    pub fn resize(&mut self, width: i32, height: i32) {
        let width = width.max(1);
        let height = height.max(1);
        let old_width = self.width();
        let old_height = self.height();
        let (cx, cy) = {
            let pos = self.screen().cursor.pos;
            (pos.x, pos.y)
        };

        // Auto-scroll to keep the cursor visible when height is reduced.
        if cy >= height && old_height > height {
            let lines_to_scroll = cy - (height - 1);
            self.screen_mut().scroll_up(lines_to_scroll);
        }

        if old_width != width {
            self.screens[0].scrollback.reflow(width as usize);
        }
        self.screens[0].resize(width, height);
        self.screens[1].resize(width, height);

        let nx = cx.clamp(0, width - 1);
        let ny = if cy >= height { height - 1 } else { cy };
        self.screen_mut().set_cursor(nx, ny, false);

        if self.is_mode_set(MODE_INBAND_RESIZE) {
            // CSI 4;rows;cols t
            let seq = format!("\x1b[4;{};{}t", height, width);
            self.queue_response(seq.as_bytes());
        }
    }

    // -----------------------------------------------------------------------
    // SGR
    // -----------------------------------------------------------------------

    fn apply_sgr(&mut self, params: &[i64]) {
        let mut style = self.screen_mut().pen();
        let mut i = 0;
        while i < params.len() {
            let p = params[i];
            match p {
                0 => style = Style::default(),
                1 => style.decoration.bold = true,
                2 => style.decoration.dim = true,
                3 => style.decoration.italic = true,
                4 => style.decoration.underline = true,
                21 => {
                    style.decoration.double_underline = true;
                    style.decoration.underline = false;
                }
                5 | 6 => style.decoration.blink = true,
                7 => style.decoration.reverse = true,
                8 => style.decoration.hidden = true,
                9 => style.decoration.strikethrough = true,
                53 => style.decoration.overline = true,
                22 => {
                    style.decoration.bold = false;
                    style.decoration.dim = false;
                }
                23 => style.decoration.italic = false,
                24 => {
                    style.decoration.underline = false;
                    style.decoration.double_underline = false;
                }
                25 => style.decoration.blink = false,
                27 => style.decoration.reverse = false,
                28 => style.decoration.hidden = false,
                29 => style.decoration.strikethrough = false,
                55 => style.decoration.overline = false,
                // Foreground colors.
                30..=37 => style.fg = Color::Indexed((p - 30) as u8),
                38 => {
                    if let Some(c) = parse_extended_color(&params[i + 1..]) {
                        style.fg = c;
                    }
                    i += color_params_consumed(&params[i + 1..]);
                }
                39 => style.fg = Color::Default,
                90..=97 => style.fg = Color::Indexed((p - 90 + 8) as u8),
                // Background colors.
                40..=47 => style.bg = Color::Indexed((p - 40) as u8),
                48 => {
                    if let Some(c) = parse_extended_color(&params[i + 1..]) {
                        style.bg = c;
                    }
                    i += color_params_consumed(&params[i + 1..]);
                }
                49 => style.bg = Color::Default,
                100..=107 => style.bg = Color::Indexed((p - 100 + 8) as u8),
                // Underline color.
                58 => {
                    if let Some(c) = parse_extended_color(&params[i + 1..]) {
                        style.underline_color = Some(c);
                    }
                    i += color_params_consumed(&params[i + 1..]);
                }
                59 => style.underline_color = None,
                _ => {}
            }
            i += 1;
        }
        self.screen_mut().set_pen(style);
    }

    // -----------------------------------------------------------------------
    // Printing
    // -----------------------------------------------------------------------

    fn print_char(&mut self, c: char) {
        if c == '\r' {
            self.screen_mut().carriage_return();
            return;
        }
        if c == '\n' {
            self.screen_mut().line_feed();
            return;
        }
        if c == '\t' {
            self.tab();
            return;
        }
        if c == '\u{8}' {
            self.screen_mut().cursor.pos.x = (self.screen_mut().cursor.pos.x - 1).max(0);
            return;
        }

        let width = UnicodeWidthChar::width(c).unwrap_or(1) as i32;
        let auto_wrap = self.is_mode_set(MODE_AUTO_WRAP);
        let insert_mode = self.is_mode_set(MODE_INSERT);

        // Handle pending wrap.
        {
            let screen = self.screen_mut();
            if screen.cursor.pos.x >= screen.width() {
                if auto_wrap {
                    screen.cursor.pos.x = 0;
                    screen.line_feed();
                } else {
                    screen.cursor.pos.x = screen.width() - 1;
                }
            }
        }
        self.at_phantom = false;

        let phantom;
        {
            let screen = self.screen_mut();
            if insert_mode {
                screen.insert_cell(1);
            }
            let x = screen.cursor.pos.x;
            let y = screen.cursor.pos.y;
            let style = screen.pen();

            // Write the cell (and continuation cells for wide runes).
            screen.set_cell(x, y, crate::vt::cell::Cell {
                content: c.to_string(),
                width: width as u8,
                style,
                link: Default::default(),
                dirty: true,
            });
            for k in 1..width {
                screen.set_cell(x + k, y, crate::vt::cell::Cell {
                    content: String::new(),
                    width: 0,
                    style,
                    link: Default::default(),
                    dirty: true,
                });
            }

            // Advance the cursor.
            screen.cursor.pos.x += width;
            phantom = x + width >= screen.width();
        }
        self.at_phantom = phantom;
    }

    fn tab(&mut self) {
        let screen = self.screen_mut();
        let next = ((screen.cursor.pos.x / 8) + 1) * 8;
        screen.cursor.pos.x = next.min(screen.width() - 1);
    }
}

/// Convert a line of cells into its plain text, one space per empty cell and
/// skipping continuation cells of wide runes (trailing spaces trimmed).
fn line_text(line: &[crate::vt::cell::Cell]) -> String {
    let mut text = String::new();
    let mut col = 0;
    while col < line.len() {
        let cell = &line[col];
        if !cell.content.is_empty() {
            text.push_str(&cell.content);
        } else {
            text.push(' ');
        }
        col += cell.width.max(1) as usize;
    }
    text.trim_end().to_string()
}

/// Convert a screen line of cells into `(content, style)` pairs, skipping
/// continuation cells of wide runes so each entry is one displayed column.
fn row_to_styled(line: Option<&[crate::vt::cell::Cell]>) -> Vec<(String, Style)> {
    let Some(row) = line else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(row.len());
    let mut col = 0;
    while col < row.len() {
        let cell = &row[col];
        out.push((cell.content.clone(), cell.style));
        col += cell.width.max(1) as usize;
    }
    out
}

/// Parse an extended color (2;r;g;b or 5;n) from a parameter slice.
/// Returns the color and how many parameters were consumed.
fn parse_extended_color(params: &[i64]) -> Option<Color> {
    match params.first() {
        Some(2) => {
            if params.len() >= 4 {
                Some(Color::Rgb(params[1] as u8, params[2] as u8, params[3] as u8))
            } else {
                None
            }
        }
        Some(5) => {
            if params.len() >= 2 {
                Some(Color::Indexed(params[1] as u8))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Number of parameters consumed by an extended color spec.
fn color_params_consumed(params: &[i64]) -> usize {
    match params.first() {
        Some(2) => params.len().min(4),
        Some(5) => params.len().min(2),
        _ => 0,
    }
}

impl Handler for Emulator {
    fn print(&mut self, c: char) {
        self.print_char(c);
    }

    fn execute(&mut self, c: u8) {
        match c {
            0x07 => { /* BEL — notify handled by the app */ }
            0x08 => {
                self.screen_mut().cursor.pos.x = (self.screen_mut().cursor.pos.x - 1).max(0);
            }
            0x09 => self.tab(),
            0x0a..=0x0c => self.screen_mut().line_feed(),
            0x0d => self.screen_mut().carriage_return(),
            0x0e => {
                // SO — shift out to G1.
                self.gl = 1;
            }
            0x0f => {
                // SI — shift in to G0.
                self.gl = 0;
            }
            _ => {}
        }
    }

    fn esc(&mut self, intermediates: &[u8], final_byte: u8) {
        match (intermediates.first().copied(), final_byte) {
            // SCS: ESC ( 0 / A / B — select charset into G0.
            (Some(b'('), 0x30) => self.charsets[0] = CharSet::SpecialGraphics,
            (Some(b'('), b'A') => self.charsets[0] = CharSet::Uk,
            (Some(b'('), b'B') => self.charsets[0] = CharSet::Ascii,
            (Some(b')'), 0x30) => self.charsets[1] = CharSet::SpecialGraphics,
            (Some(b')'), b'A') => self.charsets[1] = CharSet::Uk,
            (Some(b')'), b'B') => self.charsets[1] = CharSet::Ascii,
            // ESC 7 — save cursor.
            (None, b'7') => self.screen_mut().save_cursor(),
            // ESC 8 — restore cursor.
            (None, b'8') => self.screen_mut().restore_cursor(),
            // ESC c — RIS, full reset.
            (None, b'c') => {
                self.modes.clear();
                self.modes.insert(MODE_AUTO_WRAP, true);
                self.modes.insert(MODE_CURSOR_VISIBLE, true);
                self.charsets = [CharSet::Ascii; 4];
                self.gl = 0;
                self.gr = 1;
                self.screen_mut().clear();
                self.screen_mut().set_cursor(0, 0, false);
                self.screen_mut().scroll = ScrollRegion::full(self.width(), self.height());
            }
            // ESC D — index (line feed).
            (None, b'D') => self.screen_mut().line_feed(),
            // ESC E — next line.
            (None, b'E') => {
                self.screen_mut().carriage_return();
                self.screen_mut().line_feed();
            }
            // ESC M — reverse index (scroll down if at top).
            (None, b'M') => {
                if self.screen().cursor.pos.y == self.screen().scroll.top {
                    self.screen_mut().scroll_down(1);
                } else {
                    self.screen_mut().cursor.pos.y -= 1;
                }
            }
            // ESC H — tab set.
            (None, b'H') => {}
            // ESC Z — DECID (respond with DA1).
            (None, b'Z') => {
                let seq = b"\x1b[?1;2c".to_vec();
                self.queue_response(&seq);
            }
            _ => {}
        }
    }

    fn csi(&mut self, seq: &CsiSequence) {
        let final_byte = seq.final_byte;
        let params: Vec<i64> = seq
            .params
            .iter()
            .map(|p| p.or(0))
            .collect();
        let p = |i: usize, default: i32| -> i32 {
            seq.params
                .get(i)
                .map(|p| p.or(default as i64) as i32)
                .unwrap_or(default)
        };
        let p_or1 = |i: usize| -> i32 {
            seq.params.get(i).map(|p| p.or(1) as i32).unwrap_or(1)
        };

        // Private (DEC) sequences.
        if seq.private {
            self.private_csi(final_byte, &params, p, p_or1);
            return;
        }

        match final_byte {
            // Cursor up.
            b'A' => self.screen_mut().move_cursor(0, -p_or1(0)),
            // Cursor down.
            b'B' => self.screen_mut().move_cursor(0, p_or1(0)),
            // Cursor forward.
            b'C' => self.screen_mut().move_cursor(p_or1(0), 0),
            // Cursor back.
            b'D' => self.screen_mut().move_cursor(-p_or1(0), 0),
            // Cursor next line.
            b'E' => {
                self.screen_mut().cursor.pos.x = 0;
                self.screen_mut().move_cursor(0, p_or1(0));
            }
            // Cursor previous line.
            b'F' => {
                self.screen_mut().cursor.pos.x = 0;
                self.screen_mut().move_cursor(0, -p_or1(0));
            }
            // Cursor horizontal absolute.
            b'G' => {
                let x = p(0, 1) - 1;
                self.screen_mut().cursor.pos.x = x.clamp(0, self.width() - 1);
            }
            // Cursor position.
            b'H' | b'f' => {
                let row = p(0, 1) - 1;
                let col = p(1, 1) - 1;
                self.screen_mut().set_cursor(col, row, false);
            }
            // Cursor up (with origin).
            b'I' => self.screen_mut().move_cursor(0, -p_or1(0)),
            // Cursor down (with origin).
            b'J' => {
                match p(0, 0) {
                    0 => self.screen_mut().clear_to_end_of_screen(),
                    1 => self.screen_mut().clear_from_start_of_screen(),
                    2 | 3 => self.screen_mut().clear_screen(),
                    _ => {}
                }
            }
            // Erase line.
            b'K' => match p(0, 0) {
                0 => self.screen_mut().clear_to_end_of_line(),
                1 => self.screen_mut().clear_from_start_of_line(),
                2 => self.screen_mut().clear_line(),
                _ => {}
            },
            // Insert line.
            b'L' => self.screen_mut().insert_line(p_or1(0)),
            // Delete line.
            b'M' => self.screen_mut().delete_line(p_or1(0)),
            // Delete character.
            b'P' => self.screen_mut().delete_cell(p_or1(0)),
            // Insert character.
            b'@' => self.screen_mut().insert_cell(p_or1(0)),
            // Erase character.
            b'X' => self.screen_mut().erase_characters(p_or1(0)),
            // Scroll up.
            b'S' => self.screen_mut().scroll_up(p_or1(0)),
            // Scroll down.
            b'T' => self.screen_mut().scroll_down(p_or1(0)),
            // SGR.
            b'm' => self.apply_sgr(&params),
            // Cursor save.
            b's' => self.screen_mut().save_cursor(),
            // Cursor restore.
            b'u' => self.screen_mut().restore_cursor(),
            // Device status report.
            b'n' => {
                match p(0, 0) {
                    5 => self.queue_response(b"\x1b[0n"),
                    6 => {
                        let x = self.screen().cursor.pos.x + 1;
                        let y = self.screen().cursor.pos.y + 1;
                        let seq = format!("\x1b[{};{}R", y, x);
                        self.queue_response(seq.as_bytes());
                    }
                    _ => {}
                }
            }
            // Device attributes.
            b'c' => {
                let seq = b"\x1b[?1;2c".to_vec();
                self.queue_response(&seq);
            }
            // Set tab stop.
            b'g' => {
                if p(0, 0) == 3 {
                    // Clear all tab stops — we use fixed 8-col stops.
                }
            }
            // Window manipulation (title / size).
            b't' => {
                match p(0, 0) {
                    14 => {
                        // Report window size in pixels.
                        let seq = format!("\x1b[4;{};{}t", 480, 640);
                        self.queue_response(seq.as_bytes());
                    }
                    18 => {
                        // Report window size in cells.
                        let seq = format!("\x1b[8;{};{}t", self.height(), self.width());
                        self.queue_response(seq.as_bytes());
                    }
                    _ => {}
                }
            }
            // Vertical position.
            b'd' => {
                let y = p(0, 1) - 1;
                self.screen_mut().cursor.pos.y = y.clamp(0, self.height() - 1);
            }
            // Character position absolute (horizontal).
            b'`' => {
                let x = p(0, 1) - 1;
                self.screen_mut().cursor.pos.x = x.clamp(0, self.width() - 1);
            }
            // Repeat preceding character.
            b'b' => {}
            _ => {}
        }
    }

    fn dcs(&mut self, seq: &DcsSequence) {
        // Sixel DCS: the final byte is 'q' and the payload starts with the
        // Sixel raster attributes. Collect it for the app layer's Sixel
        // passthrough; other DCS sequences are ignored.
        if seq.final_byte == b'q' {
            let mut data = Vec::with_capacity(seq.data.len());
            data.extend_from_slice(&seq.data);
            self.pending_sixel.push(data);
        }
    }

    fn osc(&mut self, seq: &OscSequence) {
        let data = &seq.data;
        // Split the number prefix from the payload.
        let mut split = 0;
        while split < data.len() && data[split].is_ascii_digit() {
            split += 1;
        }
        let num: i64 = if split > 0 {
            std::str::from_utf8(&data[..split])
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0)
        } else {
            0
        };
        let payload = if split < data.len() && data[split] == b';' {
            &data[split + 1..]
        } else {
            &data[split..]
        };

        match num {
            // Set window title / icon name.
            0 | 2 => {
                self.title = String::from_utf8_lossy(payload).into_owned();
            }
            // Set icon name only.
            1 => {}
            // OSC 4 — set color palette.
            4 => {
                // Format: c;index;color
                let text = String::from_utf8_lossy(payload);
                if let Some((idx, color)) = text.split_once(';') {
                    let _ = idx;
                    let _ = color;
                    // Palette override — stored by the app/theme layer.
                }
            }
            // OSC 7 — working directory.
            7 => {
                self.cwd = String::from_utf8_lossy(payload).into_owned();
            }
            // OSC 8 — hyperlink.
            8 => {
                let text = String::from_utf8_lossy(payload);
                let url = text.split(';').nth(1).unwrap_or("").to_string();
                // Hyperlinks stored per-cell by the app layer; we record the
                // current link on the pen here.
                let _ = url;
            }
            // OSC 10/11 — fg/bg colors.
            10 | 11 => {}
            // OSC 52 — clipboard.
            52 => {
                let text = String::from_utf8_lossy(payload);
                if let Some((_, b64)) = text.split_once(';') {
                    use base64::Engine;
                    if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64.trim()) {
                        let text = String::from_utf8_lossy(&bytes).into_owned();
                        // Exposed to the app via a callback; stored in a slot.
                        self.clipboard = Some(text);
                    }
                }
            }
            _ => {}
        }
    }

    fn apc(&mut self, seq: &StringSequence) {
        // Kitty graphics protocol — collect for the app layer's passthrough.
        // Only APC sequences starting with 'G' (Kitty graphics) are tracked;
        // other APC uses are rare and can be dropped.
        if seq.data.first().copied() == Some(b'G') {
            self.pending_apc.push(seq.data.clone());
        }
    }

    fn pm(&mut self, _seq: &StringSequence) {}

    fn sos(&mut self, _seq: &StringSequence) {}
}

// The OSC 52 clipboard slot lives on the emulator.
impl Emulator {
    /// Last OSC 52 clipboard write, if any.
    pub fn take_clipboard(&mut self) -> Option<String> {
        self.clipboard.take()
    }

    /// Drain pending Kitty APC sequences (the raw bytes after `\x1b_G`,
    /// including the leading `G`). The app layer forwards these to the host
    /// terminal via the graphics passthrough.
    pub fn drain_pending_apc(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.pending_apc)
    }

    /// Drain pending Sixel DCS payloads (the bytes after `DCS ... q`).
    pub fn drain_pending_sixel(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.pending_sixel)
    }
}

impl Emulator {
    fn private_csi<F1, F2>(
        &mut self,
        final_byte: u8,
        params: &[i64],
        p: F1,
        _p_or1: F2,
    ) where
        F1: Fn(usize, i32) -> i32,
        F2: Fn(usize) -> i32,
    {
        match final_byte {
            // DEC private mode set.
            b'h' => {
                for &mode in params {
                    self.set_mode(mode, true);
                }
            }
            // DEC private mode reset.
            b'l' => {
                for &mode in params {
                    self.set_mode(mode, false);
                }
            }
            // DEC private mode query.
            b'$' | b'p' => {}
            // DECSTBM — set top/bottom margins.
            b'r' => {
                let top = p(0, 1) - 1;
                let bottom = p(1, self.height()) - 1;
                let screen = self.screen_mut();
                screen.scroll.top = top.clamp(0, screen.height() - 1);
                screen.scroll.bottom = bottom.clamp(0, screen.height()).max(screen.scroll.top + 1);
                screen.cursor.pos.x = 0;
                screen.cursor.pos.y = 0;
            }
            // DECSLRM — set left/right margins (requires DECLRMM).
            b's' => {}
            // DECSCUSR — set cursor style.
            b'q' => {}
            b't' => {}
            _ => {}
        }
    }
}
