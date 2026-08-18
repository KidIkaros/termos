//! Collapsed rail (glyph strip) rendering.
//!
//! Ported from Go TUIOS `internal/app/sidebar_strip.go`. When the sidebar is
//! collapsed to its glyph strip (3 columns wide), it shows a badge, session
//! marks, agent marks, a "more" marker, a new-session control, and an
//! expand/toggle control. Each mark is one cell wide.

use crate::config::theme::Rgb;

use super::{Accent, SidebarRow, RowKind};

/// The width of the collapsed strip.
pub const STRIP_WIDTH: i32 = 3;

/// The maximum number of session marks in the strip.
pub const MAX_SESSION_MARKS: usize = 8;

/// The maximum number of agent marks in the strip.
pub const MAX_AGENT_MARKS: usize = 8;

/// A strip row: one cell of content with its colour and target.
#[derive(Debug, Clone)]
pub struct StripCell {
    /// The glyph to show.
    pub glyph: &'static str,
    /// The ANSI colour slot (-1 = default).
    pub color_slot: i32,
    /// The session ID this cell targets (empty = none).
    pub session_id: String,
    /// The window ID this cell targets (empty = none).
    pub window_id: String,
    /// The window index this cell targets (-1 = none).
    pub window_index: i32,
    /// Whether this cell is the current session.
    pub current: bool,
}

impl StripCell {
    /// An empty cell (blank space).
    pub fn blank() -> Self {
        Self {
            glyph: " ",
            color_slot: -1,
            session_id: String::new(),
            window_id: String::new(),
            window_index: -1,
            current: false,
        }
    }
}

/// A strip row: the cells for one terminal row of the strip.
#[derive(Debug, Clone)]
pub struct StripRow {
    pub cells: Vec<StripCell>,
}

/// Build the strip rows for the given sidebar state.
///
/// The strip has a fixed height: one row for the badge, one for session marks,
/// one for agent marks, one for the "more" marker, and one for controls.
pub fn build_strip_rows(
    rows: &[SidebarRow],
    current_session: &str,
    _palette: &[Rgb; 16],
) -> Vec<StripRow> {
    let mut strip_rows = Vec::new();

    // Row 0: badge (the sidebar icon).
    strip_rows.push(StripRow {
        cells: vec![StripCell {
            glyph: "\u{2588}", // █
            color_slot: 4,     // blue
            ..StripCell::blank()
        }],
    });

    // Row 1: session marks (one per session, up to MAX_SESSION_MARKS).
    let mut session_cells = Vec::new();
    let mut session_count = 0;
    for row in rows {
        if row.kind == RowKind::Session {
            if session_count >= MAX_SESSION_MARKS {
                break;
            }
            let glyph = if row.session.as_deref() == Some(current_session) {
                "\u{25cf}" // ●
            } else {
                "\u{25cb}" // ○
            };
            session_cells.push(StripCell {
                glyph,
                color_slot: -1,
                session_id: row.session.clone().unwrap_or_default(),
                current: row.session.as_deref() == Some(current_session),
                ..StripCell::blank()
            });
            session_count += 1;
        }
    }
    strip_rows.push(StripRow { cells: session_cells });

    // Row 2: agent marks (one per agent pane, up to MAX_AGENT_MARKS).
    let mut agent_cells = Vec::new();
    let mut agent_count = 0;
    for row in rows {
        if row.kind == RowKind::Window && !row.agent_state.is_empty() && row.agent_state != "none" {
            if agent_count >= MAX_AGENT_MARKS {
                break;
            }
            let glyph = super::agent_glyph(&row.agent_state);
            let color_slot = super::agent_glyph_color_slot(&row.agent_state);
            agent_cells.push(StripCell {
                glyph,
                color_slot,
                window_id: row.window_id.clone().unwrap_or_default(),
                window_index: row.window.map(|i| i as i32).unwrap_or(-1),
                ..StripCell::blank()
            });
            agent_count += 1;
        }
    }
    strip_rows.push(StripRow { cells: agent_cells });

    // Row 3: "more" marker (if there are more sessions/agents than shown).
    let more_needed = rows.iter().filter(|r| r.kind == RowKind::Session).count() > MAX_SESSION_MARKS
        || rows.iter().filter(|r| r.kind == RowKind::Window && !r.agent_state.is_empty() && r.agent_state != "none").count() > MAX_AGENT_MARKS;
    if more_needed {
        strip_rows.push(StripRow {
            cells: vec![StripCell {
                glyph: "\u{2026}", // …
                color_slot: 8,
                ..StripCell::blank()
            }],
        });
    }

    // Row 4: controls (new session + expand).
    strip_rows.push(StripRow {
        cells: vec![
            StripCell {
                glyph: "+",
                color_slot: 2,
                ..StripCell::blank()
            },
            StripCell {
                glyph: "\u{25c0}", // ◀
                color_slot: 8,
                ..StripCell::blank()
            },
        ],
    });

    strip_rows
}

/// The ASCII fallback glyph for a strip cell (when Unicode is unavailable).
pub fn ascii_fallback(glyph: &str) -> &'static str {
    match glyph {
        "\u{25cf}" => "*",  // ●
        "\u{25cb}" => "o",  // ○
        "\u{2588}" => "#",  // █
        "\u{2026}" => "...", // …
        "\u{25c0}" => "<",  // ◀
        "\u{25d0}" => "%",  // ◐
        "\u{270b}" => "!",  // ✋
        "\u{2713}" => "v",  // ✓
        "\u{2715}" => "x",  // ✕
        _ => " ",
    }
}

/// Whether a glyph has an ASCII fallback.
pub fn has_ascii_fallback(glyph: &str) -> bool {
    ascii_fallback(glyph) != " " || glyph == " "
}

/// The accent mark for a strip cell, if the window has an accent.
pub fn strip_accent_mark(accent: Option<&Accent>, palette: &[Rgb; 16]) -> Option<Rgb> {
    accent.map(|a| a.rgb(palette))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session_row(name: &str) -> SidebarRow {
        SidebarRow::session_row(name, "detail", name)
    }

    fn window_row(id: &str, agent: &str) -> SidebarRow {
        SidebarRow::window_row("title", "detail", 0, 1, agent, id)
    }

    #[test]
    fn build_strip_has_badge_and_controls() {
        let rows = vec![session_row("work"), window_row("w0", "working")];
        let strip = build_strip_rows(&rows, "work", &crate::config::theme::Theme::default_ansi());
        // At least 4 rows: badge, sessions, agents, controls.
        assert!(strip.len() >= 4);
        // First cell of first row is the badge.
        assert_eq!(strip[0].cells[0].glyph, "\u{2588}");
    }

    #[test]
    fn current_session_gets_filled_circle() {
        let rows = vec![session_row("work"), session_row("play")];
        let strip = build_strip_rows(&rows, "work", &crate::config::theme::Theme::default_ansi());
        let session_row = &strip[1];
        assert_eq!(session_row.cells[0].glyph, "\u{25cf}"); // ●
        assert!(session_row.cells[0].current);
        assert_eq!(session_row.cells[1].glyph, "\u{25cb}"); // ○
        assert!(!session_row.cells[1].current);
    }

    #[test]
    fn agent_marks_use_agent_glyph() {
        let rows = vec![window_row("w0", "working"), window_row("w1", "errored")];
        let strip = build_strip_rows(&rows, "", &crate::config::theme::Theme::default_ansi());
        let agent_row = &strip[2];
        assert_eq!(agent_row.cells[0].glyph, "\u{25d0}"); // ◐
        assert_eq!(agent_row.cells[1].glyph, "\u{2715}"); // ✕
    }

    #[test]
    fn more_marker_when_overflow() {
        let mut rows = Vec::new();
        for i in 0..(MAX_SESSION_MARKS + 1) {
            rows.push(session_row(&format!("s{i}")));
        }
        let strip = build_strip_rows(&rows, "s0", &crate::config::theme::Theme::default_ansi());
        // Should have a "more" row (row 3).
        assert!(strip.len() >= 5);
        assert_eq!(strip[3].cells[0].glyph, "\u{2026}"); // …
    }

    #[test]
    fn ascii_fallbacks() {
        assert_eq!(ascii_fallback("\u{25cf}"), "*");
        assert_eq!(ascii_fallback("\u{25cb}"), "o");
        assert_eq!(ascii_fallback("\u{2715}"), "x");
        assert_eq!(ascii_fallback("\u{2713}"), "v");
    }

    #[test]
    fn blank_cell_is_space() {
        let cell = StripCell::blank();
        assert_eq!(cell.glyph, " ");
        assert_eq!(cell.color_slot, -1);
    }
}
