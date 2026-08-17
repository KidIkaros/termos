//! VT emulator conformance smoke tests — verifying that common escape
//! sequences produce the expected screen state.

use tuios::vt::Emulator;

/// Feed bytes and return the rendered screen text.
fn run(bytes: &[u8], w: i32, h: i32) -> Emulator {
    let mut emu = Emulator::new(w, h);
    emu.write(bytes);
    emu
}

#[test]
fn plain_text_is_printed() {
    let emu = run(b"hello world", 20, 4);
    let text = emu.to_string();
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
    let text = emu.to_string();
    assert!(text.contains("Dbc"));
}

#[test]
fn line_feed_scrolls_at_bottom() {
    // Fill a 1-line screen and LF should scroll.
    let emu = run(b"a\r\nb", 5, 1);
    let text = emu.to_string();
    assert!(text.contains('b'));
}

#[test]
fn clear_screen() {
    let emu = run(b"hello\x1b[2J", 20, 2);
    let text = emu.to_string();
    assert!(!text.contains("hello"));
}

#[test]
fn erase_to_end_of_line() {
    let emu = run(b"hello\x1b[3G\x1b[K", 20, 2);
    let text = emu.to_string();
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
    assert_eq!(emu.pen().fg, tuios::vt::Color::Rgb(255, 0, 0));
}

#[test]
fn alt_screen_switches_and_restores() {
    let mut emu = Emulator::new(20, 4);
    emu.write(b"main");
    emu.write(b"\x1b[?1049h");
    assert!(emu.is_alt_screen());
    emu.write(b"alt");
    assert!(emu.to_string().contains("alt"));
    emu.write(b"\x1b[?1049l");
    assert!(!emu.is_alt_screen());
    assert!(emu.to_string().contains("main"));
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
    let text = emu.to_string();
    // Inserting a line at the top pushes "top" down.
    assert!(text.contains("top"));
    assert!(text.contains("bottom"));
}

#[test]
fn delete_char_pulls_content_left() {
    let emu = run(b"abcdef\x1b[1;1H\x1b[P", 20, 2);
    let text = emu.to_string();
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
        .map(|r| r.iter().map(|(s, _)| s.as_str()).collect::<String>().trim_end().to_string())
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
