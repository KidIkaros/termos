//! Screen diff protocol for daemon mode — ported from Go TUIOS
//! `internal/terminal/window_diff.go`.
//!
//! In daemon mode, screen updates can be sent as cell-level diffs rather than
//! raw byte streams. This bypasses the VT parser entirely: the daemon sends
//! only the cells that changed, plus cursor position and alt-screen state.
//! The client applies them directly to the emulator's screen buffer.

use crate::terminal::window::Window;
use crate::vt::{Cell, Color, Decoration, Style};

// ---------------------------------------------------------------------------
// DiffCell — minimal cell representation for the wire protocol
// ---------------------------------------------------------------------------

/// A minimal cell representation for the screen diff protocol.
///
/// Avoids importing the session package (which would create a cycle). The
/// fields mirror Go's `DiffCell` but use Rust types: packed RGBA colors become
/// `u32`, attrs stay a bitmask, and content is a `String`.
///
/// Ported from Go `DiffCell`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DiffCell {
    /// Row (0-indexed).
    pub row: i32,
    /// Column (0-indexed).
    pub col: i32,
    /// The character content (a grapheme cluster).
    pub content: String,
    /// Display width in columns (1 for normal, 2 for wide CJK, 0 for
    /// continuation).
    pub width: u8,
    /// Packed RGBA foreground color (0 = default).
    pub fg: u32,
    /// Packed RGBA background color (0 = default).
    pub bg: u32,
    /// Attribute bitmask (bold/dim/italic/underline/etc.).
    pub attrs: u16,
    /// Packed RGBA underline color (0 = default).
    pub ul_color: u32,
    /// Underline style (0 = none).
    pub ul_style: u8,
}

impl DiffCell {
    /// Whether this cell is a blank/default cell.
    pub fn is_blank(&self) -> bool {
        self.content.is_empty()
            && self.fg == 0
            && self.bg == 0
            && self.attrs == 0
            && self.ul_color == 0
    }
}

// ---------------------------------------------------------------------------
// Color packing/unpacking
// ---------------------------------------------------------------------------

/// Pack an RGBA color into a `u32`. Returns 0 for `Color::Default` (which
/// means "default terminal color" on the wire).
pub fn pack_color(color: Color) -> u32 {
    match color {
        Color::Default => 0,
        Color::Indexed(i) => {
            // Pack indexed colors as 0x01 in the alpha channel to distinguish
            // from RGB. The index goes in the low byte.
            0x0100_0000 | u32::from(i)
        }
        Color::Rgb(r, g, b) => {
            // Pack as 0xAARRGGBB with full opacity.
            0xFF00_0000 | (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)
        }
    }
}

/// Unpack a `u32` into a `Color`. 0 means `Color::Default`.
pub fn unpack_color(rgba: u32) -> Color {
    if rgba == 0 {
        return Color::Default;
    }
    let alpha = (rgba >> 24) & 0xFF;
    if alpha == 0x01 {
        // Indexed color.
        let idx = (rgba & 0xFF) as u8;
        return Color::Indexed(idx);
    }
    // RGB color.
    let r = ((rgba >> 16) & 0xFF) as u8;
    let g = ((rgba >> 8) & 0xFF) as u8;
    let b = (rgba & 0xFF) as u8;
    Color::Rgb(r, g, b)
}

// ---------------------------------------------------------------------------
// Attribute bitmask
// ---------------------------------------------------------------------------

/// Attribute bit flags matching the `Decoration` field order.
pub const ATTR_BOLD: u16 = 1 << 0;
pub const ATTR_DIM: u16 = 1 << 1;
pub const ATTR_ITALIC: u16 = 1 << 2;
pub const ATTR_UNDERLINE: u16 = 1 << 3;
pub const ATTR_DOUBLE_UNDERLINE: u16 = 1 << 4;
pub const ATTR_BLINK: u16 = 1 << 5;
pub const ATTR_REVERSE: u16 = 1 << 6;
pub const ATTR_HIDDEN: u16 = 1 << 7;
pub const ATTR_STRIKETHROUGH: u16 = 1 << 8;
pub const ATTR_OVERLINE: u16 = 1 << 9;

/// Pack a `Decoration` into the attribute bitmask.
pub fn pack_attrs(deco: Decoration) -> u16 {
    let mut attrs = 0u16;
    if deco.bold {
        attrs |= ATTR_BOLD;
    }
    if deco.dim {
        attrs |= ATTR_DIM;
    }
    if deco.italic {
        attrs |= ATTR_ITALIC;
    }
    if deco.underline {
        attrs |= ATTR_UNDERLINE;
    }
    if deco.double_underline {
        attrs |= ATTR_DOUBLE_UNDERLINE;
    }
    if deco.blink {
        attrs |= ATTR_BLINK;
    }
    if deco.reverse {
        attrs |= ATTR_REVERSE;
    }
    if deco.hidden {
        attrs |= ATTR_HIDDEN;
    }
    if deco.strikethrough {
        attrs |= ATTR_STRIKETHROUGH;
    }
    if deco.overline {
        attrs |= ATTR_OVERLINE;
    }
    attrs
}

/// Unpack the attribute bitmask into a `Decoration`.
pub fn unpack_attrs(attrs: u16) -> Decoration {
    Decoration {
        bold: attrs & ATTR_BOLD != 0,
        dim: attrs & ATTR_DIM != 0,
        italic: attrs & ATTR_ITALIC != 0,
        underline: attrs & ATTR_UNDERLINE != 0,
        double_underline: attrs & ATTR_DOUBLE_UNDERLINE != 0,
        blink: attrs & ATTR_BLINK != 0,
        reverse: attrs & ATTR_REVERSE != 0,
        hidden: attrs & ATTR_HIDDEN != 0,
        strikethrough: attrs & ATTR_STRIKETHROUGH != 0,
        overline: attrs & ATTR_OVERLINE != 0,
    }
}

// ---------------------------------------------------------------------------
// DiffCell ↔ Cell conversion
// ---------------------------------------------------------------------------

/// Convert a `DiffCell` into a `Cell` for direct screen buffer insertion.
pub fn diff_cell_to_cell(diff: &DiffCell) -> Cell {
    let style = Style {
        fg: unpack_color(diff.fg),
        bg: unpack_color(diff.bg),
        underline_color: if diff.ul_color != 0 {
            Some(unpack_color(diff.ul_color))
        } else {
            None
        },
        decoration: unpack_attrs(diff.attrs),
    };
    Cell {
        content: diff.content.clone(),
        width: diff.width,
        style,
        link: Default::default(),
        dirty: true,
    }
}

/// Convert a `Cell` into a `DiffCell` for wire serialization.
pub fn cell_to_diff_cell(cell: &Cell, row: i32, col: i32) -> DiffCell {
    DiffCell {
        row,
        col,
        content: cell.content.clone(),
        width: cell.width,
        fg: pack_color(cell.style.fg),
        bg: pack_color(cell.style.bg),
        attrs: pack_attrs(cell.style.decoration),
        ul_color: cell.style.underline_color.map_or(0, pack_color),
        ul_style: 0,
    }
}

// ---------------------------------------------------------------------------
// Screen diff application
// ---------------------------------------------------------------------------

/// Apply a screen diff: write changed cells directly into the terminal
/// emulator's screen buffer, bypassing the VT parser entirely.
///
/// Ported from Go `ApplyScreenDiff`. Used by the event-based screen diff
/// protocol to update daemon windows without risk of byte-stream corruption.
///
/// After applying the cells, the cursor position and alt-screen state are
/// set, and the window's new-output flag is signaled.
pub fn apply_screen_diff(
    window: &Window,
    cells: &[DiffCell],
    cursor_x: i32,
    cursor_y: i32,
    cursor_hidden: bool,
    is_alt_screen: bool,
) {
    let emulator = &window.emulator;
    if let Ok(mut emu) = emulator.lock() {
        for c in cells {
            let cell = diff_cell_to_cell(c);
            emu.screen_mut().set_cell(c.col, c.row, cell);
        }

        // Set cursor position.
        emu.restore_cursor_position(crate::vt::Position {
            x: cursor_x,
            y: cursor_y,
        });

        // Set cursor visibility.
        emu.set_mode(crate::vt::emulator::MODE_CURSOR_VISIBLE, !cursor_hidden);

        // Set alt-screen mode if it changed.
        let current_alt = emu.is_alt_screen();
        if current_alt != is_alt_screen {
            emu.set_mode(crate::vt::emulator::MODE_ALT_SCREEN, is_alt_screen);
        }
    }

    // Signal new output so the render path picks up the diff.
    window.signal_new_output();
}

// ---------------------------------------------------------------------------
// Screen diff computation
// ---------------------------------------------------------------------------

/// Compute a minimal diff between two screen states.
///
/// Given an "old" and "new" set of cells (as flat vectors of `DiffCell`),
/// returns only the cells that differ. This is the diff the daemon sends to
/// clients: only changed cells travel on the wire.
///
/// Ported from the Go diff logic (inline in the daemon's screen serializer).
pub fn compute_diff(old: &[DiffCell], new: &[DiffCell]) -> Vec<DiffCell> {
    let mut diff = Vec::new();
    let mut old_map: std::collections::HashMap<(i32, i32), &DiffCell> =
        old.iter().map(|c| ((c.row, c.col), c)).collect();

    for new_cell in new {
        let key = (new_cell.row, new_cell.col);
        match old_map.remove(&key) {
            Some(old_cell) if old_cell == new_cell => {
                // Unchanged — skip.
            }
            _ => {
                // Changed or new — include in diff.
                diff.push(new_cell.clone());
            }
        }
    }

    // Cells in old but not in new are now blank — send blank DiffCells.
    for (_, old_cell) in old_map {
        let mut blank = DiffCell {
            row: old_cell.row,
            col: old_cell.col,
            ..Default::default()
        };
        blank.content.clear();
        blank.width = 1;
        diff.push(blank);
    }

    diff
}

/// Serialize a screen diff for transmission to remote clients.
///
/// The format is a simple length-prefixed binary encoding:
/// - 4 bytes: number of cells (u32 LE)
/// - For each cell:
///   - 4 bytes: row (i32 LE)
///   - 4 bytes: col (i32 LE)
///   - 1 byte: content length
///   - N bytes: content (UTF-8)
///   - 1 byte: width
///   - 4 bytes: fg
///   - 4 bytes: bg
///   - 2 bytes: attrs
///   - 4 bytes: ul_color
///   - 1 byte: ul_style
///
/// Followed by:
/// - 4 bytes: cursor_x (i32 LE)
/// - 4 bytes: cursor_y (i32 LE)
/// - 1 byte: cursor_hidden (0 or 1)
/// - 1 byte: is_alt_screen (0 or 1)
pub fn serialize_diff(
    cells: &[DiffCell],
    cursor_x: i32,
    cursor_y: i32,
    cursor_hidden: bool,
    is_alt_screen: bool,
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(32 + cells.len() * 32);

    // Cell count.
    buf.extend_from_slice(&(cells.len() as u32).to_le_bytes());

    for c in cells {
        buf.extend_from_slice(&c.row.to_le_bytes());
        buf.extend_from_slice(&c.col.to_le_bytes());

        let content_bytes = c.content.as_bytes();
        let content_len = content_bytes.len().min(255) as u8;
        buf.push(content_len);
        buf.extend_from_slice(&content_bytes[..content_len as usize]);

        buf.push(c.width);
        buf.extend_from_slice(&c.fg.to_le_bytes());
        buf.extend_from_slice(&c.bg.to_le_bytes());
        buf.extend_from_slice(&c.attrs.to_le_bytes());
        buf.extend_from_slice(&c.ul_color.to_le_bytes());
        buf.push(c.ul_style);
    }

    // Cursor and screen state.
    buf.extend_from_slice(&cursor_x.to_le_bytes());
    buf.extend_from_slice(&cursor_y.to_le_bytes());
    buf.push(if cursor_hidden { 1 } else { 0 });
    buf.push(if is_alt_screen { 1 } else { 0 });

    buf
}

/// Deserialize a screen diff from the wire format produced by
/// [`serialize_diff`].
pub fn deserialize_diff(data: &[u8]) -> Option<(Vec<DiffCell>, i32, i32, bool, bool)> {
    if data.len() < 4 {
        return None;
    }
    let mut pos = 0;
    let cell_count = u32::from_le_bytes(data[pos..pos + 4].try_into().ok()?) as usize;
    pos += 4;

    let mut cells = Vec::with_capacity(cell_count);
    for _ in 0..cell_count {
        if pos + 8 > data.len() {
            return None;
        }
        let row = i32::from_le_bytes(data[pos..pos + 4].try_into().ok()?);
        pos += 4;
        let col = i32::from_le_bytes(data[pos..pos + 4].try_into().ok()?);
        pos += 4;

        if pos >= data.len() {
            return None;
        }
        let content_len = data[pos] as usize;
        pos += 1;
        if pos + content_len > data.len() {
            return None;
        }
        let content = String::from_utf8_lossy(&data[pos..pos + content_len]).into_owned();
        pos += content_len;

        if pos + 1 + 4 + 4 + 2 + 4 + 1 > data.len() {
            return None;
        }
        let width = data[pos];
        pos += 1;
        let fg = u32::from_le_bytes(data[pos..pos + 4].try_into().ok()?);
        pos += 4;
        let bg = u32::from_le_bytes(data[pos..pos + 4].try_into().ok()?);
        pos += 4;
        let attrs = u16::from_le_bytes(data[pos..pos + 2].try_into().ok()?);
        pos += 2;
        let ul_color = u32::from_le_bytes(data[pos..pos + 4].try_into().ok()?);
        pos += 4;
        let ul_style = data[pos];
        pos += 1;

        cells.push(DiffCell {
            row,
            col,
            content,
            width,
            fg,
            bg,
            attrs,
            ul_color,
            ul_style,
        });
    }

    // Cursor and screen state.
    if pos + 8 + 2 > data.len() {
        return None;
    }
    let cursor_x = i32::from_le_bytes(data[pos..pos + 4].try_into().ok()?);
    pos += 4;
    let cursor_y = i32::from_le_bytes(data[pos..pos + 4].try_into().ok()?);
    pos += 4;
    let cursor_hidden = data[pos] != 0;
    pos += 1;
    let is_alt_screen = data[pos] != 0;

    Some((cells, cursor_x, cursor_y, cursor_hidden, is_alt_screen))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::pty::WinSize;

    #[test]
    fn pack_unpack_color_default() {
        assert_eq!(pack_color(Color::Default), 0);
        assert_eq!(unpack_color(0), Color::Default);
    }

    #[test]
    fn pack_unpack_color_rgb() {
        let packed = pack_color(Color::Rgb(255, 128, 64));
        assert_ne!(packed, 0);
        let unpacked = unpack_color(packed);
        assert_eq!(unpacked, Color::Rgb(255, 128, 64));
    }

    #[test]
    fn pack_unpack_color_indexed() {
        let packed = pack_color(Color::Indexed(5));
        assert_ne!(packed, 0);
        let unpacked = unpack_color(packed);
        assert_eq!(unpacked, Color::Indexed(5));
    }

    #[test]
    fn pack_unpack_attrs_roundtrip() {
        let deco = Decoration {
            bold: true,
            italic: true,
            underline: true,
            reverse: true,
            ..Default::default()
        };
        let packed = pack_attrs(deco);
        let unpacked = unpack_attrs(packed);
        assert_eq!(unpacked, deco);
    }

    #[test]
    fn pack_attrs_all_clear() {
        let deco = Decoration::default();
        assert_eq!(pack_attrs(deco), 0);
    }

    #[test]
    fn diff_cell_to_cell_and_back() {
        let diff = DiffCell {
            row: 5,
            col: 10,
            content: "A".to_string(),
            width: 1,
            fg: pack_color(Color::Rgb(255, 0, 0)),
            bg: 0,
            attrs: pack_attrs(Decoration {
                bold: true,
                ..Default::default()
            }),
            ul_color: 0,
            ul_style: 0,
        };
        let cell = diff_cell_to_cell(&diff);
        let back = cell_to_diff_cell(&cell, 5, 10);
        assert_eq!(back, diff);
    }

    #[test]
    fn compute_diff_finds_changed_cells() {
        let old = vec![
            DiffCell {
                row: 0,
                col: 0,
                content: "A".to_string(),
                width: 1,
                ..Default::default()
            },
            DiffCell {
                row: 0,
                col: 1,
                content: "B".to_string(),
                width: 1,
                ..Default::default()
            },
        ];
        let new = vec![
            DiffCell {
                row: 0,
                col: 0,
                content: "A".to_string(),
                width: 1,
                ..Default::default()
            },
            DiffCell {
                row: 0,
                col: 1,
                content: "X".to_string(),
                width: 1,
                ..Default::default()
            },
        ];
        let diff = compute_diff(&old, &new);
        // "A" is unchanged, "B"→"X" is changed.
        assert_eq!(diff.len(), 1);
        assert_eq!(diff[0].content, "X");
    }

    #[test]
    fn compute_diff_handles_removed_cells() {
        let old = vec![DiffCell {
            row: 0,
            col: 0,
            content: "A".to_string(),
            width: 1,
            ..Default::default()
        }];
        let new: Vec<DiffCell> = vec![];
        let diff = compute_diff(&old, &new);
        // The removed cell should appear as a blank.
        assert_eq!(diff.len(), 1);
        assert!(diff[0].content.is_empty());
    }

    #[test]
    fn compute_diff_no_changes_is_empty() {
        let old = vec![DiffCell {
            row: 0,
            col: 0,
            content: "A".to_string(),
            width: 1,
            ..Default::default()
        }];
        let new = old.clone();
        let diff = compute_diff(&old, &new);
        assert!(diff.is_empty());
    }

    #[test]
    fn serialize_deserialize_roundtrip() {
        let cells = vec![
            DiffCell {
                row: 0,
                col: 0,
                content: "Hi".to_string(),
                width: 1,
                fg: pack_color(Color::Rgb(255, 0, 0)),
                bg: 0,
                attrs: pack_attrs(Decoration {
                    bold: true,
                    ..Default::default()
                }),
                ul_color: 0,
                ul_style: 0,
            },
            DiffCell {
                row: 1,
                col: 2,
                content: "World".to_string(),
                width: 1,
                ..Default::default()
            },
        ];
        let data = serialize_diff(&cells, 5, 10, true, false);
        let (decoded, cx, cy, ch, alt) = deserialize_diff(&data).unwrap();
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].content, "Hi");
        assert_eq!(decoded[1].content, "World");
        assert_eq!(cx, 5);
        assert_eq!(cy, 10);
        assert!(ch);
        assert!(!alt);
    }

    #[test]
    fn serialize_deserialize_empty_diff() {
        let data = serialize_diff(&[], 0, 0, false, false);
        let (decoded, cx, cy, ch, alt) = deserialize_diff(&data).unwrap();
        assert!(decoded.is_empty());
        assert_eq!(cx, 0);
        assert_eq!(cy, 0);
        assert!(!ch);
        assert!(!alt);
    }

    #[test]
    fn deserialize_garbage_returns_none() {
        assert!(deserialize_diff(&[]).is_none());
        assert!(deserialize_diff(&[1, 2]).is_none());
    }

    #[test]
    fn diff_cell_is_blank() {
        assert!(DiffCell::default().is_blank());
        assert!(!DiffCell {
            content: "X".to_string(),
            ..Default::default()
        }
        .is_blank());
    }

    #[test]
    fn apply_screen_diff_to_window() {
        let win = Window::without_pty(
            "test",
            "Test",
            WinSize {
                cols: 80,
                rows: 24,
            },
        );
        let cells = vec![DiffCell {
            row: 0,
            col: 0,
            content: "Z".to_string(),
            width: 1,
            ..Default::default()
        }];
        apply_screen_diff(&win, &cells, 0, 0, false, false);
        // Verify the cell was set.
        let emu = win.emulator.lock().unwrap();
        let cell = emu.screen().cell(0, 0).unwrap();
        assert_eq!(cell.content, "Z");
    }
}
