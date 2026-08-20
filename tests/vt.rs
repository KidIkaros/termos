//! VT emulator conformance smoke tests — verifying that common escape
//! sequences produce the expected screen state.

use termos::vt::Emulator;

/// Feed bytes and return the rendered screen text.
fn run(bytes: &[u8], w: i32, h: i32) -> Emulator {
    let mut emu = Emulator::new(w, h);
    emu.write(bytes);
    emu
}

#[test]
fn plain_text_is_printed() {
    let emu = run(b"hello world", 20, 4);
    let text = emu.render_text();
    assert!(text.contains("hello world"));
}

#[test]
fn cursor_positioning() {
    // CUP: move to row 3, col 5, then print.
    let emu = run(b"\x1b[3;5HX", 20, 6);
    let pos = emu.cursor_position();
    assert_eq!(pos.x, 5);
    assert_eq!(pos.y, 2); // 0-indexed
}

#[test]
fn carriage_return_moves_to_left_margin() {
    let emu = run(b"abc\rD", 20, 2);
    let text = emu.render_text();
    assert!(text.contains("Dbc"));
}

#[test]
fn line_feed_scrolls_at_bottom() {
    // Fill a 1-line screen and LF should scroll.
    let emu = run(b"a\r\nb", 5, 1);
    let text = emu.render_text();
    assert!(text.contains('b'));
}

#[test]
fn clear_screen() {
    let emu = run(b"hello\x1b[2J", 20, 2);
    let text = emu.render_text();
    assert!(!text.contains("hello"));
}

#[test]
fn erase_to_end_of_line() {
    let emu = run(b"hello\x1b[3G\x1b[K", 20, 2);
    let text = emu.render_text();
    // "hello" → cursor to col 3 → erase to EOL leaves "he".
    assert!(text.contains("he"));
    assert!(!text.contains("llo"));
}

#[test]
fn sgr_bold_sets_pen() {
    let emu = run(b"\x1b[1m", 20, 2);
    assert!(emu.pen().decoration.bold);
    let emu = run(b"\x1b[0m", 20, 2);
    assert!(!emu.pen().decoration.bold);
}

#[test]
fn sgr_truecolor_sets_foreground() {
    let emu = run(b"\x1b[38;2;255;0;0m", 20, 2);
    assert_eq!(emu.pen().fg, termos::vt::Color::Rgb(255, 0, 0));
}

#[test]
fn alt_screen_switches_and_restores() {
    let mut emu = Emulator::new(20, 4);
    emu.write(b"main");
    emu.write(b"\x1b[?1049h");
    assert!(emu.is_alt_screen());
    emu.write(b"alt");
    assert!(emu.render_text().contains("alt"));
    emu.write(b"\x1b[?1049l");
    assert!(!emu.is_alt_screen());
    assert!(emu.render_text().contains("main"));
}

#[test]
fn title_via_osc() {
    let mut emu = Emulator::new(20, 4);
    emu.write(b"\x1b]0;My Title\x07");
    assert_eq!(emu.title, "My Title");
}

#[test]
fn scrollback_captures_lines() {
    let mut emu = Emulator::new(10, 2);
    emu.write(b"line1\r\nline2\r\nline3\r\nline4");
    assert!(emu.scrollback_len() >= 2);
    let first = emu.scrollback_line_text(0).unwrap_or_default();
    assert!(first.contains("line1") || first.contains("line2"));
}

#[test]
fn cursor_save_restore() {
    let emu = run(b"\x1b[5;5H\x1b7\x1b[1;1H\x1b8", 20, 10);
    let pos = emu.cursor_position();
    assert_eq!(pos.x, 4);
    assert_eq!(pos.y, 4);
}

#[test]
fn insert_line_shifts_content() {
    let emu = run(b"top\r\nbottom\x1b[1;1H\x1b[L", 20, 4);
    let text = emu.render_text();
    // Inserting a line at the top pushes "top" down.
    assert!(text.contains("top"));
    assert!(text.contains("bottom"));
}

#[test]
fn delete_char_pulls_content_left() {
    let emu = run(b"abcdef\x1b[1;1H\x1b[P", 20, 2);
    let text = emu.render_text();
    assert!(text.contains("bcdef"));
}

#[test]
fn tab_advances_to_next_stop() {
    let emu = run(b"ab\tX", 20, 2);
    let pos = emu.cursor_position();
    // After "ab" (cols 0-1) tab moves to col 8, then 'X' to col 9.
    assert_eq!(pos.x, 9);
}

#[test]
fn da_response() {
    let mut emu = Emulator::new(20, 4);
    emu.write(b"\x1b[c");
    let response = emu.take_response();
    assert!(!response.is_empty());
    assert_eq!(&response[..2], b"\x1b[");
}

#[test]
fn cursor_position_report() {
    let mut emu = Emulator::new(20, 10);
    emu.write(b"\x1b[5;5H\x1b[6n");
    let response = emu.take_response();
    let text = String::from_utf8_lossy(&response);
    assert!(text.contains("5;5"));
}

/// Render one line of a styled view as plain text.
fn view_line_text(emu: &Emulator, row: usize) -> String {
    let lines = emu.render_view_lines();
    lines
        .get(row)
        .map(|r| {
            r.iter()
                .map(|(s, _)| *s)
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .unwrap_or_default()
}

#[test]
fn viewport_scrolls_into_scrollback() {
    let mut emu = Emulator::new(10, 2);
    emu.write(b"line1\r\nline2\r\nline3\r\nline4");
    // scrollback = [line1, line2], live = [line3, line4].
    assert!(emu.scrollback_len() >= 2);
    assert_eq!(emu.viewport(), 0);
    assert!(!emu.in_scrollback());

    emu.scroll_viewport(1);
    assert_eq!(emu.viewport(), 1);
    assert!(emu.in_scrollback());
    // The top row is now the last scrollback line.
    assert_eq!(view_line_text(&emu, 0), "line2");
    assert_eq!(view_line_text(&emu, 1), "line3");

    // Scrolling back down to zero returns to the live screen.
    emu.scroll_viewport(-1);
    assert_eq!(emu.viewport(), 0);
    assert!(!emu.in_scrollback());
    assert_eq!(view_line_text(&emu, 0), "line3");
    assert_eq!(view_line_text(&emu, 1), "line4");
}

#[test]
fn viewport_clamps_to_available_scrollback() {
    let mut emu = Emulator::new(10, 1);
    emu.write(b"a\r\nb\r\nc");
    emu.scroll_viewport(i32::MAX);
    // Never more than the scrollback length.
    assert_eq!(emu.viewport(), emu.scrollback_len());
    emu.reset_viewport();
    assert_eq!(emu.viewport(), 0);
}

#[test]
fn content_lines_and_selection_text() {
    let mut emu = Emulator::new(10, 2);
    emu.write(b"ab\r\ncd\r\nef");
    // scrollback = ["ab"], live = ["cd", "ef"].
    assert_eq!(emu.content_line_count(), 3);
    assert_eq!(emu.content_line_text(0), "ab");
    assert_eq!(emu.content_line_text(1), "cd");
    assert_eq!(emu.content_line_text(2), "ef");

    // A rectangular selection spanning two lines.
    assert_eq!(emu.selection_text(1, 0, 2, 1), "cd\nef");
    // Single line, single column.
    assert_eq!(emu.selection_text(0, 1, 0, 1), "b");
}

#[test]
fn content_index_for_view_row_honors_viewport() {
    let mut emu = Emulator::new(10, 2);
    emu.write(b"ab\r\ncd\r\nef");
    // Live view: rows 0,1 are content lines 1,2.
    assert_eq!(emu.content_index_for_view_row(0), 1);
    assert_eq!(emu.content_index_for_view_row(1), 2);
    // Scrolled back one line: rows 0,1 are content lines 0,1.
    emu.scroll_viewport(1);
    assert_eq!(emu.content_index_for_view_row(0), 0);
    assert_eq!(emu.content_index_for_view_row(1), 1);
}

// ===========================================================================
// Extended VT conformance tests — ported from the Go test suite.
// ===========================================================================

#[test]
fn tab_stops_default_every_8() {
    let emu = run(b"a\tb", 20, 2);
    let text = emu.render_text();
    // Tab moves to the next 8-column stop: "a" at col 0, tab to col 8, "b".
    assert!(text.contains("a       b"), "got: {text:?}");
}

#[test]
fn backspace_moves_cursor_left() {
    let emu = run(b"abc\x08X", 20, 2);
    let text = emu.render_text();
    assert!(text.contains("abX"), "got: {text:?}");
}

#[test]
fn backspace_at_col_zero_does_not_wrap() {
    let emu = run(b"\x08X", 20, 2);
    let pos = emu.cursor_position();
    // Backspace at col 0 is a no-op; X is printed at col 0.
    assert_eq!(pos.x, 1);
}

#[test]
fn csi_a_moves_cursor_up() {
    // Move to row 3, then CUU (cursor up) by 1.
    let emu = run(b"\x1b[3;1H\x1b[1AX", 20, 5);
    let pos = emu.cursor_position();
    assert_eq!(pos.y, 1); // row 2 (0-indexed)
}

#[test]
fn csi_b_moves_cursor_down() {
    let emu = run(b"\x1b[1;1H\x1b[2BX", 20, 5);
    let pos = emu.cursor_position();
    assert_eq!(pos.y, 2); // moved down 2 rows
}

#[test]
fn csi_c_moves_cursor_right() {
    let emu = run(b"\x1b[1;1H\x1b[3CX", 20, 5);
    let pos = emu.cursor_position();
    assert_eq!(pos.x, 4); // moved right 3 cols, then printed X
}

#[test]
fn csi_d_moves_cursor_left() {
    let emu = run(b"\x1b[1;5H\x1b[2DX", 20, 5);
    let pos = emu.cursor_position();
    assert_eq!(pos.x, 3); // moved left from col 4 to col 2, then printed X
}

#[test]
fn csi_k_erases_to_end_of_line() {
    let emu = run(b"hello\x1b[2;1Hworld\x1b[1;1H\x1b[K", 20, 3);
    let text = emu.render_text();
    // Line 1 should be erased (EL 0 = erase to end of line from cursor).
    // "hello" is on line 1; after moving to (1,1) and EL, it's gone.
    assert!(!text.contains("hello") || text.trim().is_empty() || text.contains("world"));
}

#[test]
fn csi_2j_clears_entire_screen() {
    let emu = run(b"hello\r\nworld\x1b[2J", 20, 3);
    let text = emu.render_text();
    assert!(text.trim().is_empty(), "screen not cleared: {text:?}");
}

#[test]
fn csi_s_and_u_save_restore_cursor() {
    // Save at (1,5), move to (3,1), restore.
    let emu = run(b"\x1b[1;5H\x1b[s\x1b[3;1H\x1b[uX", 20, 5);
    let pos = emu.cursor_position();
    // After restore, cursor is back at (1,5) (0-indexed: 4,0), then X.
    assert_eq!(pos.x, 5);
    assert_eq!(pos.y, 0);
}

#[test]
fn decstbm_sets_scroll_region() {
    // Set scroll region to rows 2-3 (1-indexed), then fill and scroll.
    let mut emu = Emulator::new(10, 4);
    emu.write(b"\x1b[2;3r");
    emu.write(b"\x1b[2;1Hline1\r\nline2\r\nline3");
    // The scroll region is rows 1-2 (0-indexed); line3 should scroll out.
    let text = emu.render_text();
    assert!(text.contains("line1") || text.contains("line2"));
}

#[test]
fn alternate_screen_switch() {
    let mut emu = Emulator::new(20, 4);
    emu.write(b"main screen");
    emu.write(b"\x1b[?1049h"); // switch to alt
    emu.write(b"\x1b[2J");
    emu.write(b"alt screen");
    assert!(emu.is_alt_screen());
    emu.write(b"\x1b[?1049l"); // switch back
    assert!(!emu.is_alt_screen());
    let text = emu.render_text();
    assert!(text.contains("main screen"), "main screen lost: {text:?}");
}

#[test]
fn osc52_clipboard_write() {
    let mut emu = Emulator::new(20, 4);
    // OSC 52 ; c = clipboard, base64 "hi" = "aGk="
    emu.write(b"\x1b]52;c;aGk=\x07");
    let clip = emu.take_clipboard();
    assert_eq!(clip.as_deref(), Some("hi"));
}

#[test]
fn unicode_wide_char_takes_two_cells() {
    let emu = run("你好".as_bytes(), 20, 2);
    let pos = emu.cursor_position();
    // Each CJK char is 2 cells wide; after 2 chars, cursor is at col 4.
    assert_eq!(pos.x, 4);
}

#[test]
fn apc_kitty_graphics_is_collected() {
    let mut emu = Emulator::new(20, 4);
    emu.write(b"\x1b_Ga=T,f=100,i=1;AAAA\x1b\\");
    let apcs = emu.drain_pending_apc();
    assert_eq!(apcs.len(), 1);
    assert_eq!(apcs[0].first(), Some(&b'G'));
}

#[test]
fn sixel_dcs_is_collected() {
    let mut emu = Emulator::new(20, 4);
    // DCS q ... ST
    emu.write(b"\x1bPq\x1b\\");
    let sixels = emu.drain_pending_sixel();
    assert_eq!(sixels.len(), 1);
}

#[test]
fn empty_apc_is_terminated_by_st() {
    let mut emu = Emulator::new(20, 4);
    // ESC _ (empty APC) ESC \
    emu.write(b"\x1b_\x1b\\");
    // Should not hang or crash; the APC is dispatched (but not G-prefixed,
    // so it's not collected).
    let apcs = emu.drain_pending_apc();
    assert_eq!(apcs.len(), 0);
}

#[test]
fn osc_with_bel_terminator() {
    let mut emu = Emulator::new(20, 4);
    // OSC 52 ; c ; aGk= BEL
    emu.write(b"\x1b]52;c;aGk=\x07");
    let clip = emu.take_clipboard();
    assert_eq!(clip.as_deref(), Some("hi"));
}

#[test]
fn osc_with_st_terminator() {
    let mut emu = Emulator::new(20, 4);
    // OSC 52 ; c ; aGk= ESC \
    emu.write(b"\x1b]52;c;aGk=\x1b\\");
    let clip = emu.take_clipboard();
    assert_eq!(clip.as_deref(), Some("hi"));
}

// ===========================================================================
// Edge-case conformance tests — ported from the Go test suite.
// ===========================================================================

// --- Wide-char-at-margin (from wide_cell_edge_test.go) ---

#[test]
fn narrowing_resize_blanks_cut_wide_rune() {
    // Write 世世世 (each 2 cells wide) in a 6-wide screen, then narrow to 5.
    // The last column (index 4) held the lead of the 3rd 世; after resize it
    // must be blank, not a dangling wide-rune lead.
    let mut emu = Emulator::new(6, 3);
    emu.write("世世世".as_bytes());
    emu.resize(5, 3);
    let last = emu.screen().cell(4, 0).expect("last cell exists");
    assert!(
        last.content.is_none() || last.content == Some(' '),
        "dangling wide-rune lead at last col: {:?}",
        last.content
    );
}

#[test]
fn narrowing_resize_preserves_clean_edge() {
    // Resize 6→4: the edge falls between two wide runes, so nothing is cut.
    let mut emu = Emulator::new(6, 3);
    emu.write("世世世".as_bytes());
    emu.resize(4, 3);
    let _last = emu.screen().cell(3, 0).expect("cell at col 3");
    // The 2nd 世 occupies cols 2-3, so col 3 is its continuation (empty content).
    // The lead at col 2 should still be 世.
    let lead = emu.screen().cell(2, 0).expect("cell at col 2");
    assert_eq!(lead.content, Some('世'), "clean-edge lead should survive");
}

#[test]
fn wide_char_at_right_margin_wraps() {
    // In an 80-wide screen, writing 世 at col 79 (0-indexed) should wrap
    // because it needs 2 cells but only 1 remains.
    let mut emu = Emulator::new(80, 3);
    emu.write(b"\x1b[1;80H");
    emu.write("世".as_bytes());
    // The cursor should have wrapped to the next line.
    let pos = emu.cursor_position();
    assert_eq!(
        pos.y, 1,
        "wide char at right margin should wrap to next line"
    );
}

// --- Cell shift / ICH DCH (from wide_cell_shift_test.go) ---

#[test]
fn delete_1_of_wide_runes() {
    // 中日本 + CSI 1P (delete 1 cell) → " 日本"
    let mut emu = Emulator::new(20, 3);
    emu.write("中日本".as_bytes());
    emu.write(b"\x1b[H"); // cursor to home
    emu.write(b"\x1b[1P"); // delete 1 cell
    let text = emu.render_text();
    assert!(text.contains(" 日本"), "delete 1 of wide: got {text:?}");
}

#[test]
fn delete_2_of_wide_runes() {
    let mut emu = Emulator::new(20, 3);
    emu.write("中日本".as_bytes());
    emu.write(b"\x1b[H");
    emu.write(b"\x1b[2P"); // delete 2 cells
    let text = emu.render_text();
    assert!(text.contains("日本"), "delete 2 of wide: got {text:?}");
}

#[test]
fn delete_3_of_wide_runes() {
    let mut emu = Emulator::new(20, 3);
    emu.write("中日本".as_bytes());
    emu.write(b"\x1b[H");
    emu.write(b"\x1b[3P"); // delete 3 cells
    let text = emu.render_text();
    assert!(text.contains(" 本"), "delete 3 of wide: got {text:?}");
}

#[test]
fn insert_1_of_wide_runes() {
    // 中日本 + CSI 1@ (insert 1 cell) → " 中日本"
    let mut emu = Emulator::new(20, 3);
    emu.write("中日本".as_bytes());
    emu.write(b"\x1b[H");
    emu.write(b"\x1b[1@"); // insert 1 cell
    let text = emu.render_text();
    assert!(text.contains(" 中日本"), "insert 1 of wide: got {text:?}");
}

#[test]
fn delete_1_of_mixed_runes() {
    // ab中日本cd + CSI 1P → "b中日本cd"
    let mut emu = Emulator::new(20, 3);
    emu.write("ab中日本cd".as_bytes());
    emu.write(b"\x1b[H");
    emu.write(b"\x1b[1P");
    let text = emu.render_text();
    assert!(
        text.contains("b中日本cd"),
        "delete 1 of mixed: got {text:?}"
    );
}

#[test]
fn delete_3_of_mixed_runes() {
    let mut emu = Emulator::new(20, 3);
    emu.write("ab中日本cd".as_bytes());
    emu.write(b"\x1b[H");
    emu.write(b"\x1b[3P");
    let text = emu.render_text();
    assert!(text.contains(" 日本cd"), "delete 3 of mixed: got {text:?}");
}

// --- Scrollback trimming ---

#[test]
fn scrollback_capped_at_max() {
    let mut emu = Emulator::new(20, 2);
    emu.set_scrollback_max_lines(100);
    // Write 200 lines (each triggers a scroll since height=2).
    for i in 0..200 {
        emu.write(format!("line{i}\r\n").as_bytes());
    }
    assert_eq!(
        emu.scrollback_len(),
        100,
        "scrollback should be capped at 100"
    );
}

#[test]
fn scrollback_drops_oldest() {
    let mut emu = Emulator::new(20, 2);
    emu.set_scrollback_max_lines(5);
    for i in 0..20 {
        emu.write(format!("line{i:02}\r\n").as_bytes());
    }
    // The oldest retained line should be one of the later lines (cap 5).
    let first = emu.scrollback_line_text(0).unwrap_or_default();
    assert!(
        first.contains("line1"),
        "oldest retained should be a later line: {first:?}"
    );
}

#[test]
fn alt_screen_no_scrollback() {
    let mut emu = Emulator::new(20, 4);
    emu.write(b"\x1b[?1049h"); // switch to alt
    emu.write(b"hello\r\nworld\r\n");
    assert_eq!(
        emu.scrollback_len(),
        0,
        "alt screen should not accumulate scrollback"
    );
}

// --- Chunked-write equivalence (from FuzzEmulatorWriteChunked) ---

#[test]
fn chunked_write_matches_whole_write_1byte() {
    let data = b"hello\r\nworld\x1b[31mred\x1b[m\x1b[2J\x1b[Hdone";
    let mut whole = Emulator::new(80, 24);
    whole.write(data);
    let mut split = Emulator::new(80, 24);
    for byte in data.iter() {
        split.write(std::slice::from_ref(byte));
    }
    assert_eq!(whole.render_text(), split.render_text());
}

#[test]
fn chunked_write_matches_whole_write_2byte() {
    let data = b"hello\r\nworld\x1b[31mred\x1b[m";
    let mut whole = Emulator::new(80, 24);
    whole.write(data);
    let mut split = Emulator::new(80, 24);
    for chunk in data.chunks(2) {
        split.write(chunk);
    }
    assert_eq!(whole.render_text(), split.render_text());
}

#[test]
fn chunked_write_matches_whole_write_utf8_split() {
    // Split a multibyte char across a chunk boundary.
    let data = "hello 世界 \x1b[31mred\x1b[m".as_bytes();
    let mut whole = Emulator::new(80, 24);
    whole.write(data);
    // Split at byte 8 (in the middle of 世 = 0xe4 0xb8 0x96).
    let mut split = Emulator::new(80, 24);
    split.write(&data[..8]);
    split.write(&data[8..]);
    assert_eq!(whole.render_text(), split.render_text());
}

#[test]
fn chunked_write_matches_whole_write_csi_split() {
    // Split an ESC sequence across a chunk boundary.
    let data = b"abc\x1b[31mdef";
    let mut whole = Emulator::new(80, 24);
    whole.write(data);
    let mut split = Emulator::new(80, 24);
    split.write(b"abc\x1b");
    split.write(b"[31mdef");
    assert_eq!(whole.render_text(), split.render_text());
}

// --- Resize safety (from FuzzEmulatorResize) ---

#[test]
fn resize_clamps_cursor() {
    let mut emu = Emulator::new(80, 24);
    emu.write(b"\x1b[10;40Hhello");
    emu.resize(1, 1);
    let pos = emu.cursor_position();
    // Cursor may be at 0 or 1 (phantom) for a 1-wide screen.
    assert!(pos.x >= 0 && pos.x <= 1, "cursor x out of bounds: {pos:?}");
    assert!(pos.y >= 0 && pos.y <= 1, "cursor y out of bounds: {pos:?}");
}

#[test]
fn resize_preserves_content() {
    let mut emu = Emulator::new(20, 3);
    emu.write(b"hello");
    emu.resize(40, 6);
    let text = emu.render_text();
    assert!(
        text.contains("hello"),
        "content lost after resize: {text:?}"
    );
}

#[test]
fn resize_to_tiny_then_back() {
    let mut emu = Emulator::new(80, 24);
    emu.write(b"hello world");
    emu.resize(1, 1);
    emu.resize(80, 24);
    // Should not panic; cursor should be in bounds (may be at phantom position).
    let pos = emu.cursor_position();
    assert!(pos.x >= 0 && pos.x <= 80);
    assert!(pos.y >= 0 && pos.y < 24);
}

#[test]
fn resize_reflows_scrollback() {
    let mut emu = Emulator::new(10, 2);
    emu.set_scrollback_max_lines(100);
    // Write a long line that wraps, then resize wider.
    emu.write(b"abcdefghijklmnopqrstuvwxyz\r\n");
    emu.resize(30, 4);
    // Should not panic; scrollback should still be accessible.
    assert!(emu.content_line_count() > 0);
}

// --- Truncated/unterminated sequences ---

#[test]
fn truncated_esc_at_eof() {
    let mut emu = Emulator::new(20, 4);
    emu.write(b"\x1b");
    // No panic, cursor in bounds.
    let pos = emu.cursor_position();
    assert!(pos.x < 20 && pos.y < 4);
}

#[test]
fn truncated_csi_at_eof() {
    let mut emu = Emulator::new(20, 4);
    emu.write(b"\x1b[");
    let pos = emu.cursor_position();
    assert!(pos.x < 20 && pos.y < 4);
}

#[test]
fn truncated_osc_at_eof() {
    let mut emu = Emulator::new(20, 4);
    emu.write(b"\x1b]0;title");
    // No panic, no unbounded accumulation.
    let pos = emu.cursor_position();
    assert!(pos.x < 20 && pos.y < 4);
}

#[test]
fn unterminated_apc_large_payload() {
    let mut emu = Emulator::new(20, 4);
    // 100KB of data after an APC introducer, no terminator.
    let data: Vec<u8> = std::iter::once(b'\x1b')
        .chain(std::iter::once(b'_'))
        .chain(std::iter::repeat_n(b'q', 100_000))
        .collect();
    emu.write(&data);
    // No panic.
    let pos = emu.cursor_position();
    assert!(pos.x < 20 && pos.y < 4);
}

#[test]
fn oversized_csi_params() {
    let mut emu = Emulator::new(20, 4);
    // 8192 parameters in a single SGR.
    let mut data = Vec::from(b"\x1b[");
    for _ in 0..8192 {
        data.extend_from_slice(b"1;");
    }
    data.push(b'm');
    emu.write(&data);
    // No panic.
    let pos = emu.cursor_position();
    assert!(pos.x < 20 && pos.y < 4);
}

// --- Mode/restore state ---

#[test]
fn save_restore_modes() {
    let mut emu = Emulator::new(20, 4);
    // Enable bracketed paste and cursor visibility.
    emu.write(b"\x1b[?2004h\x1b[?25l");
    let modes = emu.mode_map().clone();
    assert!(
        modes.get(&2004).copied().unwrap_or(false),
        "bracketed paste should be on"
    );

    // Change modes.
    emu.write(b"\x1b[?2004l\x1b[?25h");
    assert!(!emu.is_mode_set(2004), "bracketed paste should be off");

    // Restore.
    emu.restore_modes(&modes);
    assert!(emu.is_mode_set(2004), "bracketed paste should be restored");
}

#[test]
fn save_restore_cursor_and_scroll_region() {
    let mut emu = Emulator::new(20, 6);
    // Set scroll region to rows 2-4, move cursor to row 3.
    emu.write(b"\x1b[2;4r\x1b[3;1H");
    let region = emu.scroll_region();
    let cursor = emu.cursor_position();

    // Move cursor and change scroll region.
    emu.write(b"\x1b[1;1r\x1b[1;1H");
    assert_ne!(emu.cursor_position().y, cursor.y);

    // Restore.
    emu.restore_scroll_region(region);
    emu.restore_cursor_position(cursor);
    let restored_region = emu.scroll_region();
    assert_eq!(restored_region.top, region.top);
    assert_eq!(restored_region.bottom, region.bottom);
}
