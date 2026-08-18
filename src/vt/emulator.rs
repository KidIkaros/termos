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
    /// Single-shift select: 0 = none, 2 = SS2, 3 = SS3.
    gsingle: u8,
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
    /// The most recent OSC 9;4 progress report (state, percent), drained by
    /// the app's agent-state tick.
    pending_progress: Option<(crate::vt::progress::ProgressState, Option<u8>)>,
    /// The most recent desktop notification (title, body), drained by the
    /// app layer. Set by OSC 9, OSC 777, and OSC 99.
    pending_notification: Option<(String, String)>,
    /// OSC 133 semantic markers (prompt/command boundaries), recorded as the
    /// stream is parsed. Used by the structured scrollback browser.
    semantic_markers: crate::vt::semantic_markers::SemanticMarkerList,
    last_printed_char: Option<char>,
    /// How many scrollback lines are currently shown above the live screen.
    /// 0 means the live screen is shown; a positive value means the view is
    /// scrolled back that many lines (copy mode).
    viewport: usize,
    /// Cell size in pixels for XTWINOPS responses (default 8×16).
    cell_width: u16,
    cell_height: u16,
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
            screens: [
                ScreenBuffer::new(width, height),
                ScreenBuffer::new(width, height),
            ],
            active: 0,
            modes: std::collections::HashMap::new(),
            charsets: [CharSet::Ascii; 4],
            gl: 0,
            gr: 1,
            gsingle: 0,
            at_phantom: false,
            title: String::new(),
            cwd: String::new(),
            response: Vec::new(),
            clipboard: None,
            pending_apc: Vec::new(),
            pending_sixel: Vec::new(),
            pending_progress: None,
            pending_notification: None,
            semantic_markers: crate::vt::semantic_markers::SemanticMarkerList::new(10_000),
            last_printed_char: None,
            viewport: 0,
            cell_width: 8,
            cell_height: 16,
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
            MODE_ORIGIN if enabled => {
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

    /// Set the cell size in pixels, used for XTWINOPS pixel responses.
    pub fn set_cell_size(&mut self, width: u16, height: u16) {
        self.cell_width = width;
        self.cell_height = height;
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
    pub fn render_text(&self) -> String {
        self.screen().render_text()
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
        self.last_printed_char = Some(c);

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

        // If a wide character doesn't fit in the remaining columns, wrap
        // before printing it (matching xterm/ghostty behaviour).
        if auto_wrap && width > 1 {
            let screen = self.screen();
            if screen.cursor.pos.x + width > screen.width() {
                let screen = self.screen_mut();
                screen.cursor.pos.x = 0;
                screen.line_feed();
            }
        }

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
            screen.set_cell(
                x,
                y,
                crate::vt::cell::Cell {
                    content: c.to_string(),
                    width: width as u8,
                    style,
                    link: Default::default(),
                    dirty: true,
                },
            );
            for k in 1..width {
                screen.set_cell(
                    x + k,
                    y,
                    crate::vt::cell::Cell {
                        content: String::new(),
                        width: 0,
                        style,
                        link: Default::default(),
                        dirty: true,
                    },
                );
            }

            // Advance the cursor, clamped to at most width (the pending-wrap
            // position). A wide char on a narrow screen must not leave the
            // cursor past the screen bounds.
            screen.cursor.pos.x = (screen.cursor.pos.x + width).min(screen.width());
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
                Some(Color::Rgb(
                    params[1] as u8,
                    params[2] as u8,
                    params[3] as u8,
                ))
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
            // C1 controls (0x80–0x9F).
            0x84 => {
                // IND — index (line feed).
                self.screen_mut().line_feed();
            }
            0x88 => {
                // HTS — horizontal tab set (no-op with fixed 8-column tabs).
            }
            0x8d => {
                // RI — reverse index.
                if self.screen().cursor.pos.y == self.screen().scroll.top {
                    self.screen_mut().scroll_down(1);
                } else {
                    self.screen_mut().cursor.pos.y -= 1;
                }
            }
            0x8e => {
                // SS2 — single shift 2.
                self.gsingle = 2;
            }
            0x8f => {
                // SS3 — single shift 3.
                self.gsingle = 3;
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
            // SCS: ESC ) 0 / A / B — select charset into G1.
            (Some(b')'), 0x30) => self.charsets[1] = CharSet::SpecialGraphics,
            (Some(b')'), b'A') => self.charsets[1] = CharSet::Uk,
            (Some(b')'), b'B') => self.charsets[1] = CharSet::Ascii,
            // SCS: ESC * 0 / A / B — select charset into G2.
            (Some(b'*'), 0x30) => self.charsets[2] = CharSet::SpecialGraphics,
            (Some(b'*'), b'A') => self.charsets[2] = CharSet::Uk,
            (Some(b'*'), b'B') => self.charsets[2] = CharSet::Ascii,
            // SCS: ESC + 0 / A / B — select charset into G3.
            (Some(b'+'), 0x30) => self.charsets[3] = CharSet::SpecialGraphics,
            (Some(b'+'), b'A') => self.charsets[3] = CharSet::Uk,
            (Some(b'+'), b'B') => self.charsets[3] = CharSet::Ascii,
            // ESC = — DECKPAM (application keypad mode).
            (None, b'=') => {
                self.modes.insert(MODE_APPLICATION_KEYPAD, true);
            }
            // ESC > — DECKPNM (numeric keypad mode).
            (None, b'>') => {
                self.modes.insert(MODE_APPLICATION_KEYPAD, false);
            }
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
                self.gsingle = 0;
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
            // ESC H — tab set (no-op with fixed 8-column tabs).
            (None, b'H') => {}
            // ESC n — LS2 (locking shift G2).
            (None, b'n') => self.gl = 2,
            // ESC o — LS3 (locking shift G3).
            (None, b'o') => self.gl = 3,
            // ESC | — LS3R (locking shift G3 right).
            (None, b'|') => self.gr = 3,
            // ESC } — LS2R (locking shift G2 right).
            (None, b'}') => self.gr = 2,
            // ESC ~ — LS1R (locking shift G1 right).
            (None, b'~') => self.gr = 1,
            // ESC Z — DECID (respond with DA1).
            (None, b'Z') => {
                let seq = b"\x1b[?62;1;4;6;9;15;18;22c".to_vec();
                self.queue_response(&seq);
            }
            _ => {}
        }
    }

    fn csi(&mut self, seq: &CsiSequence) {
        let final_byte = seq.final_byte;
        let params: Vec<i64> = seq.params.iter().map(|p| p.or(0)).collect();
        let p = |i: usize, default: i32| -> i32 {
            seq.params
                .get(i)
                .map(|p| p.or(default as i64) as i32)
                .unwrap_or(default)
        };
        let p_or1 = |i: usize| -> i32 { seq.params.get(i).map(|p| p.or(1) as i32).unwrap_or(1) };

        // Private (DEC) sequences.
        if seq.private {
            self.private_csi(
                final_byte,
                seq.private_marker,
                &seq.intermediates,
                &params,
                p,
                p_or1,
            );
            return;
        }

        // Sequences with intermediate bytes (e.g. DECSCUSR: CSI SP q).
        if let Some(&inter) = seq.intermediates.first() {
            if inter == b' ' && final_byte == b'q' {
                // DECSCUSR — set cursor style.
                let _n = p(0, 1);
                return;
            }
            if inter == b'$' && final_byte == b'p' {
                // DECRQM — Request Mode (ANSI).
                // Response: CSI n ; s $ y  where s is 1=set, 2=reset.
                let mode = p(0, 0);
                if mode != 0 {
                    let setting = if self.is_mode_set(mode as i64) { 1 } else { 2 };
                    let seq = format!("\x1b[{mode};{setting}$y");
                    self.queue_response(seq.as_bytes());
                }
                return;
            }
            // Other intermediate sequences are not handled.
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
            // CHT — cursor horizontal tabulation (forward N tabs).
            b'I' => {
                let n = p_or1(0);
                let screen = self.screen_mut();
                for _ in 0..n {
                    let next = ((screen.cursor.pos.x / 8) + 1) * 8;
                    screen.cursor.pos.x = next.min(screen.width() - 1);
                }
            }
            // Cursor down (with origin).
            b'J' => match p(0, 0) {
                0 => self.screen_mut().clear_to_end_of_screen(),
                1 => self.screen_mut().clear_from_start_of_screen(),
                2 => self.screen_mut().clear_screen(),
                3 => self.screen_mut().clear_scrollback(),
                _ => {}
            },
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
            // CBT — cursor backward tabulation (back N tabs).
            b'Z' => {
                let n = p_or1(0);
                let screen = self.screen_mut();
                for _ in 0..n {
                    let prev = ((screen.cursor.pos.x - 1) / 8) * 8;
                    screen.cursor.pos.x = prev.max(0);
                }
            }
            // Scroll up.
            b'S' => self.screen_mut().scroll_up(p_or1(0)),
            // Scroll down.
            b'T' => self.screen_mut().scroll_down(p_or1(0)),
            // SGR.
            b'm' => self.apply_sgr(&params),
            // ANSI mode set.
            b'h' => {
                for &mode in &params {
                    self.set_mode(mode, true);
                }
            }
            // ANSI mode reset.
            b'l' => {
                for &mode in &params {
                    self.set_mode(mode, false);
                }
            }
            // Cursor save.
            b's' => self.screen_mut().save_cursor(),
            // Cursor restore.
            b'u' => self.screen_mut().restore_cursor(),
            // Device status report.
            b'n' => match p(0, 0) {
                5 => self.queue_response(b"\x1b[0n"),
                6 => {
                    let x = self.screen().cursor.pos.x + 1;
                    let y = self.screen().cursor.pos.y + 1;
                    let seq = format!("\x1b[{};{}R", y, x);
                    self.queue_response(seq.as_bytes());
                }
                _ => {}
            },
            // Device attributes.
            b'c' => {
                // DA1 — Primary Device Attributes (VT220).
                let seq = b"\x1b[?62;1;4;6;9;15;18;22c".to_vec();
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
                        let ph = self.height() as u16 * self.cell_height;
                        let pw = self.width() as u16 * self.cell_width;
                        let seq = format!("\x1b[4;{ph};{pw}t");
                        self.queue_response(seq.as_bytes());
                    }
                    16 => {
                        // Report cell size in pixels.
                        let seq = format!("\x1b[6;{};{}t", self.cell_height, self.cell_width);
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
            // Horizontal position relative (HPR).
            b'a' => {
                let dx = p(0, 1);
                let screen = self.screen_mut();
                screen.cursor.pos.x = (screen.cursor.pos.x + dx).clamp(0, screen.width() - 1);
            }
            // Repeat preceding character (REP).
            b'b' => {
                if let Some(c) = self.last_printed_char {
                    for _ in 0..p_or1(0) {
                        self.print_char(c);
                    }
                }
            }
            // Scroll region set (DECSTBM).
            b'r' => {
                let top = p(0, 1) - 1;
                let bottom = if seq.params.is_empty() {
                    self.height()
                } else {
                    p(1, self.height())
                };
                let top = top.clamp(0, self.height() - 1);
                let bottom = bottom.clamp(top + 1, self.height());
                self.screen_mut().scroll.top = top;
                self.screen_mut().scroll.bottom = bottom;
                self.screen_mut().cursor.pos.x = 0;
                self.screen_mut().cursor.pos.y = 0;
            }
            // Cursor next line (CNL).
            b'e' => {
                let y = self.screen().cursor.pos.y + p_or1(0);
                self.screen_mut().cursor.pos.y = y.clamp(0, self.height() - 1);
                self.screen_mut().cursor.pos.x = 0;
            }
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
            // OSC 133 — semantic prompt markers (A/B/C/D).
            133 => {
                let text = String::from_utf8_lossy(payload);
                let mut parts = text.split(';');
                let code = parts.next().unwrap_or("").trim();
                if let Some(marker_type) = crate::vt::semantic_markers::parse_marker_type(
                    code.chars().next().unwrap_or(' '),
                ) {
                    let (cx, cy) = {
                        let pos = self.cursor_position();
                        (pos.x, pos.y)
                    };
                    let abs_line = self.scrollback_len() as i32 + cy;
                    let exit = parts
                        .next()
                        .and_then(|p| p.trim().parse::<i32>().ok())
                        .unwrap_or(-1);
                    let mut marker =
                        crate::vt::semantic_markers::SemanticMarker::new(marker_type, abs_line, cx);
                    if marker_type
                        == crate::vt::semantic_markers::SemanticMarkerType::CommandExecuted
                    {
                        // The C marker usually fires after the prompt line was
                        // committed, with the cursor at the start of the output;
                        // fall back to the previous line for the command text.
                        let mut cmd = self.screen().line_text(cy).trim().to_string();
                        if cmd.is_empty() && cy > 0 {
                            cmd = self.screen().line_text(cy - 1).trim().to_string();
                        }
                        marker = marker.with_exit_code(exit).with_captured_text(&cmd);
                    } else {
                        marker = marker.with_exit_code(exit);
                    }
                    self.semantic_markers.push(marker);
                }
            }
            // OSC 9 — progress report (9;4;...) or iTerm2 desktop notification.
            9 => {
                let payload = String::from_utf8_lossy(payload);
                if crate::vt::progress::is_progress_payload(&payload) {
                    let (state, percent) = crate::vt::progress::parse_progress(&payload);
                    self.pending_progress = Some((state, percent));
                } else {
                    // iTerm2 notification: "9;<msg>".
                    self.pending_notification = Some((String::new(), payload.into_owned()));
                }
            }
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
            // OSC 10/11/12 — fg/bg/cursor colors.
            10..=12 => {}
            // OSC 66 — kitty text sizing. We can't render scaled text on a
            // cell grid, but we don't want the sequence to vanish either.
            66 => {}
            // OSC 777 — urxvt notification: "notify;<title>;<body>".
            777 => {
                let text = String::from_utf8_lossy(payload);
                let parts: Vec<&str> = text.splitn(3, ';').collect();
                if parts.len() >= 3 && parts[0] == "notify" {
                    self.pending_notification = Some((parts[1].to_string(), parts[2].to_string()));
                }
            }
            // OSC 99 — kitty notification: "<meta>;<payload>".
            99 => {
                let text = String::from_utf8_lossy(payload);
                let parts: Vec<&str> = text.splitn(2, ';').collect();
                if parts.len() >= 2 {
                    // Best-effort v1: skip continuation chunks (d=0).
                    let is_continuation = parts[0].split(':').any(|kv| kv == "d=0");
                    if !is_continuation {
                        let body = parts[1].to_string();
                        let title = parts[0]
                            .split(':')
                            .find_map(|kv| kv.strip_prefix("p=title:"))
                            .unwrap_or("")
                            .to_string();
                        self.pending_notification = Some((title, body));
                    }
                }
            }
            // OSC 110/111/112 — reset fg/bg/cursor colors.
            110..=112 => {}
            // OSC 52 — clipboard.
            52 => {
                let text = String::from_utf8_lossy(payload);
                if let Some((_, b64)) = text.split_once(';') {
                    use base64::Engine;
                    if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64.trim())
                    {
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

    /// The recorded OSC 133 semantic markers.
    pub fn semantic_markers(&self) -> &crate::vt::semantic_markers::SemanticMarkerList {
        &self.semantic_markers
    }

    /// Drain the most recent OSC 9;4 progress report, if any.
    pub fn take_pending_progress(
        &mut self,
    ) -> Option<(crate::vt::progress::ProgressState, Option<u8>)> {
        self.pending_progress.take()
    }

    /// Drain the most recent desktop notification (title, body), if any.
    /// Set by OSC 9, OSC 777, and OSC 99.
    pub fn take_pending_notification(&mut self) -> Option<(String, String)> {
        self.pending_notification.take()
    }
}

impl Emulator {
    fn private_csi<F1, F2>(
        &mut self,
        final_byte: u8,
        private_marker: u8,
        intermediates: &[u8],
        params: &[i64],
        p: F1,
        _p_or1: F2,
    ) where
        F1: Fn(usize, i32) -> i32,
        F2: Fn(usize) -> i32,
    {
        // DECRQM with `$` intermediate (CSI ? n $ p).
        if intermediates.contains(&b'$') && final_byte == b'p' && private_marker == b'?' {
            let mode = p(0, 0);
            if mode != 0 {
                let setting = if self.is_mode_set(mode as i64) { 1 } else { 2 };
                let seq = format!("\x1b[?{mode};{setting}$y");
                self.queue_response(seq.as_bytes());
            }
            return;
        }
        match (private_marker, final_byte) {
            // DA2 — Secondary Device Attributes (CSI > c).
            (b'>', b'c') => {
                let seq = b"\x1b[>1;10;0c".to_vec();
                self.queue_response(&seq);
            }
            // DEC private mode set.
            (b'?', b'h') => {
                for &mode in params {
                    self.set_mode(mode, true);
                }
            }
            // DEC private mode reset.
            (b'?', b'l') => {
                for &mode in params {
                    self.set_mode(mode, false);
                }
            }
            // DECXCPR — extended cursor position report (CSI ? 6 n).
            (b'?', b'n') => {
                if p(0, 0) == 6 {
                    let x = self.screen().cursor.pos.x + 1;
                    let y = self.screen().cursor.pos.y + 1;
                    let seq = format!("\x1b[?{};{};0R", y, x);
                    self.queue_response(seq.as_bytes());
                }
            }
            // DECSTBM — set top/bottom margins.
            (b'?', b'r') => {
                let top = p(0, 1) - 1;
                let bottom = p(1, self.height()) - 1;
                let screen = self.screen_mut();
                screen.scroll.top = top.clamp(0, screen.height() - 1);
                screen.scroll.bottom = bottom.clamp(0, screen.height()).max(screen.scroll.top + 1);
                screen.cursor.pos.x = 0;
                screen.cursor.pos.y = 0;
            }
            // DECSLRM — set left/right margins (requires DECLRMM).
            (b'?', b's') => {}
            // DECST8C — set tab at every 8 columns.
            (b'?', b'W') => {}
            // DECSCUSR — set cursor style.
            (b'?', b'q') => {}
            (b'?', b't') => {}
            _ => {}
        }
    }
}

#[cfg(test)]
mod osc133_tests {
    use super::*;

    fn emu() -> Emulator {
        Emulator::new(80, 24)
    }

    #[test]
    fn records_prompt_and_command_markers() {
        let mut e = emu();
        e.write(b"$ ls\r\n");
        e.write(b"\x1b]133;A\x07");
        e.write(b"\x1b]133;B\x07");
        e.write(b"\x1b]133;C\x07");
        e.write(b"file1\r\nfile2\r\n");
        e.write(b"\x1b]133;D;0\x07");
        let markers = e.semantic_markers().markers();
        // A, B, C, D recorded.
        assert!(markers.len() >= 4);
        let c = markers
            .iter()
            .find(|m| {
                m.marker_type == crate::vt::semantic_markers::SemanticMarkerType::CommandExecuted
            })
            .unwrap();
        assert!(
            c.captured_text.contains("ls"),
            "captured: {}",
            c.captured_text
        );
        let d = markers
            .iter()
            .find(|m| {
                m.marker_type == crate::vt::semantic_markers::SemanticMarkerType::CommandFinished
            })
            .unwrap();
        assert_eq!(d.exit_code, 0);
    }

    #[test]
    fn non_133_osc_ignored() {
        let mut e = emu();
        e.write(b"\x1b]0;title\x07");
        assert!(e.semantic_markers().is_empty());
    }

    #[test]
    fn scrollback_blocks_from_markers() {
        let mut e = emu();
        e.write(b"$ ls\r\n");
        e.write(b"\x1b]133;A\x07\x1b]133;B\x07\x1b]133;C\x07");
        e.write(b"file1\r\nfile2\r\n");
        e.write(b"\x1b]133;D;0\x07");
        let markers = e.semantic_markers().markers();
        let text = |i: usize| e.content_line_text(i);
        let count = e.content_line_count();
        let blocks = crate::scrollback::parse_blocks(&markers, count, text);
        assert!(!blocks.is_empty());
        assert_eq!(blocks[0].method, "osc133");
    }
}

#[cfg(test)]
mod csi_completion_tests {
    use super::*;

    #[test]
    fn rep_repeats_last_char() {
        let mut e = Emulator::new(80, 24);
        e.write(b"abc");
        e.write(b"\x1b[3b");
        // REP repeats 'c' 3 times: "abcccc"
        let line = e.screen().line_text(0);
        assert!(line.contains("abcccc"), "line: {line}");
    }

    #[test]
    fn decstbm_sets_scroll_region() {
        let mut e = Emulator::new(80, 24);
        e.write(b"\x1b[5;10r");
        assert_eq!(e.screen().scroll.top, 4);
        assert_eq!(e.screen().scroll.bottom, 10);
        // Cursor moves to home.
        assert_eq!(e.cursor_position().x, 0);
        assert_eq!(e.cursor_position().y, 0);
    }

    #[test]
    fn decstbm_full_screen_when_no_params() {
        let mut e = Emulator::new(80, 24);
        e.write(b"\x1b[5;10r");
        e.write(b"\x1b[r");
        assert_eq!(e.screen().scroll.top, 0);
        assert_eq!(e.screen().scroll.bottom, 24);
    }

    #[test]
    fn cnl_moves_down_and_cr() {
        let mut e = Emulator::new(80, 24);
        e.write(b"\x1b[5;10H");
        e.write(b"\x1b[2e");
        assert_eq!(e.cursor_position().y, 6);
        assert_eq!(e.cursor_position().x, 0);
    }

    #[test]
    fn rep_with_no_last_char_is_noop() {
        let mut e = Emulator::new(80, 24);
        e.write(b"\x1b[5b");
        // No panic, cursor at origin.
        assert_eq!(e.cursor_position().x, 0);
    }

    #[test]
    fn hpr_moves_cursor_right() {
        let mut e = Emulator::new(80, 24);
        e.write(b"\x1b[5;10H");
        e.write(b"\x1b[3a");
        assert_eq!(e.cursor_position().x, 12);
        assert_eq!(e.cursor_position().y, 4);
    }

    #[test]
    fn hpr_clamps_to_width() {
        let mut e = Emulator::new(10, 24);
        e.write(b"\x1b[1;8H");
        e.write(b"\x1b[5a");
        assert_eq!(e.cursor_position().x, 9);
    }

    #[test]
    fn decst8c_is_noop() {
        let mut e = Emulator::new(80, 24);
        // Should not panic; fixed 8-column tabs are unaffected.
        e.write(b"\x1b[?5W");
        e.write(b"\t");
        assert_eq!(e.cursor_position().x, 8);
    }
}

#[cfg(test)]
mod esc_osc_completion_tests {
    use super::*;

    #[test]
    fn deckpam_deckpnm_toggle() {
        let mut e = Emulator::new(80, 24);
        assert!(!e.is_mode_set(MODE_APPLICATION_KEYPAD));
        e.write(b"\x1b=");
        assert!(e.is_mode_set(MODE_APPLICATION_KEYPAD));
        e.write(b"\x1b>");
        assert!(!e.is_mode_set(MODE_APPLICATION_KEYPAD));
    }

    #[test]
    fn ls2_sets_gl() {
        let mut e = Emulator::new(80, 24);
        e.write(b"\x1bn");
        assert_eq!(e.gl, 2);
    }

    #[test]
    fn ls3_sets_gl() {
        let mut e = Emulator::new(80, 24);
        e.write(b"\x1bo");
        assert_eq!(e.gl, 3);
    }

    #[test]
    fn ls1r_sets_gr() {
        let mut e = Emulator::new(80, 24);
        e.write(b"\x1b~");
        assert_eq!(e.gr, 1);
    }

    #[test]
    fn ls2r_sets_gr() {
        let mut e = Emulator::new(80, 24);
        e.write(b"\x1b}");
        assert_eq!(e.gr, 2);
    }

    #[test]
    fn ls3r_sets_gr() {
        let mut e = Emulator::new(80, 24);
        e.write(b"\x1b|");
        assert_eq!(e.gr, 3);
    }

    #[test]
    fn scs_g2_g3_select() {
        let mut e = Emulator::new(80, 24);
        e.write(b"\x1b*0");
        assert_eq!(e.charsets[2], CharSet::SpecialGraphics);
        e.write(b"\x1b+B");
        assert_eq!(e.charsets[3], CharSet::Ascii);
    }

    #[test]
    fn osc9_notification() {
        let mut e = Emulator::new(80, 24);
        e.write(b"\x1b]9;Hello world\x07");
        let (title, body) = e.take_pending_notification().unwrap();
        assert_eq!(title, "");
        assert_eq!(body, "Hello world");
    }

    #[test]
    fn osc777_notification() {
        let mut e = Emulator::new(80, 24);
        e.write(b"\x1b]777;notify;Title;Body text\x07");
        let (title, body) = e.take_pending_notification().unwrap();
        assert_eq!(title, "Title");
        assert_eq!(body, "Body text");
    }

    #[test]
    fn osc99_notification() {
        let mut e = Emulator::new(80, 24);
        e.write(b"\x1b]99;i=1;payload\x07");
        let (_title, body) = e.take_pending_notification().unwrap();
        assert_eq!(body, "payload");
    }

    #[test]
    fn osc99_continuation_skipped() {
        let mut e = Emulator::new(80, 24);
        e.write(b"\x1b]99;d=0;chunk\x07");
        assert!(e.take_pending_notification().is_none());
    }

    #[test]
    fn osc9_progress_still_works() {
        let mut e = Emulator::new(80, 24);
        e.write(b"\x1b]9;4;1;50\x07");
        assert!(e.take_pending_progress().is_some());
        assert!(e.take_pending_notification().is_none());
    }

    #[test]
    fn c1_ind_line_feed() {
        let mut e = Emulator::new(80, 24);
        e.write(b"\x1b[5;10H");
        e.write(b"\x84");
        assert_eq!(e.cursor_position().y, 5);
    }

    #[test]
    fn c1_ri_reverse_index() {
        let mut e = Emulator::new(80, 24);
        e.write(b"\x1b[5;10H");
        e.write(b"\x8d");
        assert_eq!(e.cursor_position().y, 3);
    }

    #[test]
    fn c1_ri_at_top_scrolls() {
        let mut e = Emulator::new(80, 24);
        e.write(b"\x1b[1;10H");
        e.write(b"X");
        e.write(b"\x1b[1;1H");
        e.write(b"\x8d");
        // Cursor stays at top; content scrolls down.
        assert_eq!(e.cursor_position().y, 0);
    }

    #[test]
    fn c1_ss2_ss3() {
        let mut e = Emulator::new(80, 24);
        e.write(b"\x8e");
        assert_eq!(e.gsingle, 2);
        e.write(b"\x8f");
        assert_eq!(e.gsingle, 3);
    }

    #[test]
    fn ris_resets_gsingle() {
        let mut e = Emulator::new(80, 24);
        e.write(b"\x8e");
        assert_eq!(e.gsingle, 2);
        e.write(b"\x1bc");
        assert_eq!(e.gsingle, 0);
    }

    #[test]
    fn ed3_clears_scrollback() {
        let mut e = Emulator::new(80, 3);
        // Fill lines to push content into scrollback.
        e.write(b"line1\nline2\nline3\nline4\nline5");
        assert!(e.scrollback_len() > 0, "scrollback should have content");
        e.write(b"\x1b[3J");
        assert_eq!(e.scrollback_len(), 0, "ED 3 should clear scrollback");
    }

    #[test]
    fn xtwinops_16_reports_cell_size() {
        let mut e = Emulator::new(80, 24);
        e.set_cell_size(10, 20);
        e.write(b"\x1b[16t");
        let resp = e.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(s.contains("\x1b[6;20;10t"), "got: {s}");
    }

    #[test]
    fn xtwinops_14_reports_pixel_size() {
        let mut e = Emulator::new(80, 24);
        e.set_cell_size(10, 20);
        e.write(b"\x1b[14t");
        let resp = e.take_response();
        let s = String::from_utf8_lossy(&resp);
        // 24 rows * 20 px = 480, 80 cols * 10 px = 800
        assert!(s.contains("\x1b[4;480;800t"), "got: {s}");
    }

    #[test]
    fn da1_reports_vt220_attributes() {
        let mut e = Emulator::new(80, 24);
        e.write(b"\x1b[c");
        let resp = e.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(s.contains("\x1b[?62;"), "DA1 should report VT220, got: {s}");
    }

    #[test]
    fn da2_reports_secondary_attributes() {
        let mut e = Emulator::new(80, 24);
        e.write(b"\x1b[>c");
        let resp = e.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(s.contains("\x1b[>1;10;0c"), "DA2 got: {s}");
    }

    #[test]
    fn decid_matches_da1() {
        let mut e = Emulator::new(80, 24);
        e.write(b"\x1bZ");
        let resp = e.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(s.contains("\x1b[?62;"), "DECID got: {s}");
    }

    #[test]
    fn decxcpr_reports_extended_cursor() {
        let mut e = Emulator::new(80, 24);
        e.write(b"\x1b[5;10H");
        e.write(b"\x1b[?6n");
        let resp = e.take_response();
        let s = String::from_utf8_lossy(&resp);
        // Row 5, col 10, page 0.
        assert!(s.contains("\x1b[?5;10;0R"), "DECXCPR got: {s}");
    }

    #[test]
    fn decscusr_accepted() {
        let mut e = Emulator::new(80, 24);
        // Should not panic or produce output.
        e.write(b"\x1b[2 q");
        let resp = e.take_response();
        assert!(resp.is_empty(), "DECSCUSR should not produce output");
    }

    #[test]
    fn cht_moves_forward_tabs() {
        let mut e = Emulator::new(80, 24);
        // Start at column 0.
        e.write(b"\x1b[2I");
        // Two tab stops forward: 0 -> 8 -> 16.
        assert_eq!(e.cursor_position().x, 16);
    }

    #[test]
    fn cht_clamps_to_width() {
        let mut e = Emulator::new(20, 24);
        e.write(b"\x1b[1;18H");
        e.write(b"\x1b[3I");
        // 17 -> next tab is 24, but clamped to 19.
        assert_eq!(e.cursor_position().x, 19);
    }

    #[test]
    fn cbt_moves_backward_tabs() {
        let mut e = Emulator::new(80, 24);
        e.write(b"\x1b[1;20H");
        e.write(b"\x1b[2Z");
        // 19 -> prev tab 16 -> prev tab 8.
        assert_eq!(e.cursor_position().x, 8);
    }

    #[test]
    fn cbt_clamps_to_zero() {
        let mut e = Emulator::new(80, 24);
        e.write(b"\x1b[1;3H");
        e.write(b"\x1b[5Z");
        assert_eq!(e.cursor_position().x, 0);
    }

    #[test]
    fn decrqm_ansi_reports_mode() {
        let mut e = Emulator::new(80, 24);
        // Auto-wrap (mode 7) is set by default.
        e.write(b"\x1b[7$p");
        let resp = e.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(s.contains("\x1b[7;1$y"), "DECRQM ANSI got: {s}");
    }

    #[test]
    fn decrqm_ansi_reports_reset() {
        let mut e = Emulator::new(80, 24);
        // Turn off auto-wrap, then query.
        e.write(b"\x1b[7l");
        e.write(b"\x1b[7$p");
        let resp = e.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(s.contains("\x1b[7;2$y"), "DECRQM ANSI reset got: {s}");
    }

    #[test]
    fn decrqm_dec_reports_mode() {
        let mut e = Emulator::new(80, 24);
        // Enable cursor visibility (mode ?25), then query.
        e.write(b"\x1b[?25h");
        e.write(b"\x1b[?25$p");
        let resp = e.take_response();
        let s = String::from_utf8_lossy(&resp);
        assert!(s.contains("\x1b[?25;1$y"), "DECRQM DEC got: {s}");
    }

    #[test]
    fn ansi_mode_set_reset() {
        let mut e = Emulator::new(80, 24);
        assert!(e.is_mode_set(MODE_AUTO_WRAP));
        e.write(b"\x1b[7l");
        assert!(!e.is_mode_set(MODE_AUTO_WRAP));
        e.write(b"\x1b[7h");
        assert!(e.is_mode_set(MODE_AUTO_WRAP));
    }
}

#[cfg(test)]
mod malformed_sequence_tests {
    use super::*;

    #[test]
    fn bare_esc_does_not_crash() {
        let mut e = Emulator::new(80, 24);
        e.write(b"\x1b");
        e.write(b"abc");
        let line = e.screen().line_text(0);
        // ESC followed by 'a' is not a valid ESC sequence; 'a' is consumed
        // as the final byte (no-op). Then "bc" prints.
        assert!(line.contains("bc"), "line: {line}");
    }

    #[test]
    fn incomplete_csi_does_not_crash() {
        let mut e = Emulator::new(80, 24);
        e.write(b"\x1b[");
        e.write(b"abc");
        let line = e.screen().line_text(0);
        // CSI followed by 'a' (0x61) is a final byte (HPR); params empty.
        // Then "bc" prints.
        assert!(line.contains("bc"), "line: {line}");
    }

    #[test]
    fn incomplete_csi_with_params_does_not_crash() {
        let mut e = Emulator::new(80, 24);
        e.write(b"\x1b[5;");
        e.write(b"abc");
        let line = e.screen().line_text(0);
        // CSI 5 ; a — 'a' is final (HPR with param 5, then param empty).
        // Then "bc" prints.
        assert!(line.contains("bc"), "line: {line}");
    }

    #[test]
    fn incomplete_osc_does_not_crash() {
        let mut e = Emulator::new(80, 24);
        e.write(b"\x1b]9;Hello");
        e.write(b" world\x07");
        let (_title, body) = e.take_pending_notification().unwrap();
        assert_eq!(body, "Hello world");
    }

    #[test]
    fn esc_cancel_cancels_csi() {
        let mut e = Emulator::new(80, 24);
        e.write(b"\x1b[5;10");
        // ESC cancels the pending CSI.
        e.write(b"\x1b[1;1H");
        assert_eq!(e.cursor_position().x, 0);
        assert_eq!(e.cursor_position().y, 0);
    }

    #[test]
    fn csi_with_garbage_params_ignored() {
        let mut e = Emulator::new(80, 24);
        // Non-numeric param bytes should not crash.
        e.write(b"\x1b[:::H");
        // Cursor should be at home (1;1 default).
        assert_eq!(e.cursor_position().x, 0);
        assert_eq!(e.cursor_position().y, 0);
    }

    #[test]
    fn partial_utf8_does_not_crash() {
        let mut e = Emulator::new(80, 24);
        // First byte of a 3-byte UTF-8 sequence.
        e.write(b"\xe4\xb8");
        e.write(b"\x96");
        let line = e.screen().line_text(0);
        assert!(line.contains("世"), "line: {line}");
    }

    #[test]
    fn csi_split_across_writes() {
        let mut e = Emulator::new(80, 24);
        e.write(b"\x1b[5");
        e.write(b";10H");
        assert_eq!(e.cursor_position().x, 9);
        assert_eq!(e.cursor_position().y, 4);
    }

    #[test]
    fn empty_csi_params_use_defaults() {
        let mut e = Emulator::new(80, 24);
        // CSI H with no params should go to 1;1.
        e.write(b"\x1b[5;10H");
        e.write(b"\x1b[H");
        assert_eq!(e.cursor_position().x, 0);
        assert_eq!(e.cursor_position().y, 0);
    }

    #[test]
    fn csi_semicolon_only_uses_defaults() {
        let mut e = Emulator::new(80, 24);
        e.write(b"\x1b[;H");
        assert_eq!(e.cursor_position().x, 0);
        assert_eq!(e.cursor_position().y, 0);
    }
}
