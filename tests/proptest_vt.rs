//! Property-based tests using proptest — verifies invariants that must hold
//! for all inputs, not just hand-picked examples.
//!
//! These mirror the Go fuzz targets' invariant checks but run as part of
//! `cargo test` for CI integration.

use proptest::prelude::*;
use termos::vt::Emulator;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Feeding arbitrary bytes to the emulator must never panic, never change
    /// the screen dimensions, and always leave the cursor in bounds. The
    /// cursor may sit at `width` (the "phantom" position after printing at
    /// the last column); this is valid VT behaviour — the wrap happens on the
    /// next print.
    #[test]
    fn write_never_panics(ref data in prop::collection::vec(any::<u8>(), 0..65536)) {
        let mut emu = Emulator::new(80, 24);
        emu.write(data);

        let pos = emu.cursor_position();
        prop_assert!(pos.x >= 0 && pos.x <= 80, "cursor x out of bounds: {}", pos.x);
        prop_assert!(pos.y >= 0 && pos.y < 24, "cursor y out of bounds: {}", pos.y);
        prop_assert_eq!(emu.width(), 80, "screen width changed");
        prop_assert_eq!(emu.height(), 24, "screen height changed");
    }

    /// Feeding the same bytes whole vs. in chunks must produce the same screen
    /// contents. A PTY read boundary can land anywhere, so both paths must
    /// reach the same state.
    #[test]
    fn chunked_write_matches_whole(
        ref data in prop::collection::vec(any::<u8>(), 0..4096),
        chunk_size in 1u8..16
    ) {
        let mut whole = Emulator::new(80, 24);
        whole.write(data);

        let mut split = Emulator::new(80, 24);
        for chunk in data.chunks(chunk_size as usize) {
            split.write(chunk);
        }

        prop_assert_eq!(whole.render_text(), split.render_text());
    }

    /// Interleaving writes with resizes must never panic and must always leave
    /// the cursor inside the new screen bounds.
    #[test]
    fn resize_keeps_cursor_in_bounds(
        ref data in prop::collection::vec(any::<u8>(), 0..4096),
        w in 1u8..200,
        h in 1u8..200
    ) {
        let mut emu = Emulator::new(80, 24);
        let half = data.len() / 2;
        emu.write(&data[..half]);
        emu.resize(w as i32, h as i32);
        emu.write(&data[half..]);

        let pos = emu.cursor_position();
        prop_assert!(pos.x >= 0 && pos.x <= w as i32, "cursor x {} not in [0,{}]", pos.x, w);
        prop_assert!(pos.y >= 0 && pos.y < h as i32, "cursor y {} not in [0,{})", pos.y, h);
        prop_assert_eq!(emu.width(), w as i32);
        prop_assert_eq!(emu.height(), h as i32);
    }

    /// The tape parser must never panic on arbitrary input.
    #[test]
    fn tape_parse_never_panics(ref input in ".{0,10000}") {
        let (_commands, _errors) = termos::tape::parser::parse_file(input);
        // Should not panic on any input.
    }

    /// Structured token streams — realistic VT sequences built from a pool
    /// (prints, wide CJK, SGR, cursor moves, erases, OSC, alt-screen) rather
    /// than arbitrary bytes. This reaches parser branches in combination
    /// (e.g. SGR + wide char + erase at the right edge) that raw-byte fuzzing
    /// hits only by accident.
    #[test]
    fn structured_token_stream_keeps_grid_invariant(
        tokens in prop::collection::vec(any::<VtToken>(), 0..512)
    ) {
        let mut emu = Emulator::new(80, 24);
        let mut bytes: Vec<u8> = Vec::new();
        for t in &tokens {
            bytes.extend(t.bytes());
        }
        emu.write(&bytes);

        // Cursor stays in bounds on both screens.
        let pos = emu.cursor_position();
        prop_assert!(pos.x >= 0 && pos.x <= 80, "cursor x out of bounds: {}", pos.x);
        prop_assert!(pos.y >= 0 && pos.y < 24, "cursor y out of bounds: {}", pos.y);
        prop_assert_eq!(emu.width(), 80, "screen width changed");
        prop_assert_eq!(emu.height(), 24, "screen height changed");

        // Grid invariant: a cell with width 0 is always content-less, and an
        // occupied cell is never width 0. Check both the active screen and the
        // main screen (the alt-screen toggle may have left either active).
        check_grid_invariant(emu.main_screen());
        check_grid_invariant(emu.screen());
    }

    /// Structured CJK-heavy streams: the grid invariant must hold even when
    /// wide runes are interleaved with erases, insert/delete ops, and cursor
    /// moves (which legitimately orphan continuation cells).
    #[test]
    fn wide_char_streams_keep_grid_invariant(
        tokens in prop::collection::vec(any::<VtToken>(), 1..256)
    ) {
        let mut emu = Emulator::new(60, 20);
        let mut bytes: Vec<u8> = Vec::new();
        for t in &tokens {
            bytes.extend(t.bytes());
        }
        emu.write(&bytes);
        check_grid_invariant(emu.screen());
    }
}

/// A structured VT token from a realistic sequence pool.
#[derive(Debug, Clone, Copy)]
enum VtToken {
    Print(char),
    Ctrl(u8),
    Csi(&'static str),
    Osc(&'static str),
    Esc(&'static str),
}

impl VtToken {
    /// The raw bytes this token emits, UTF-8 encoded.
    fn bytes(&self) -> Vec<u8> {
        match *self {
            VtToken::Print(c) => {
                let mut b = [0u8; 4];
                let n = c.encode_utf8(&mut b).len();
                b[..n].to_vec()
            }
            VtToken::Ctrl(c) => vec![c],
            VtToken::Csi(s) | VtToken::Osc(s) | VtToken::Esc(s) => s.as_bytes().to_vec(),
        }
    }
}

impl Arbitrary for VtToken {
    type Parameters = ();
    type Strategy = BoxedStrategy<Self>;

    fn arbitrary_with(_: ()) -> Self::Strategy {
        prop_oneof![
            // Printable runs — the hot path.
            any::<char>().prop_filter("printable", |c| {
                *c != '\x1b' && *c != '\x00' && (c.is_ascii() || c.is_alphanumeric())
            }).prop_map(VtToken::Print),
            // Control chars: CR, LF, TAB, BS, BEL, HT.
            prop_oneof![
                Just(VtToken::Ctrl(b'\r')),
                Just(VtToken::Ctrl(b'\n')),
                Just(VtToken::Ctrl(b'\t')),
                Just(VtToken::Ctrl(0x08)),
                Just(VtToken::Ctrl(0x07)),
            ],
            // CSI sequences: SGR, cursor moves, erases, modes, pos.
            prop_oneof![
                Just(VtToken::Csi("\x1b[31m")),
                Just(VtToken::Csi("\x1b[1m")),
                Just(VtToken::Csi("\x1b[0m")),
                Just(VtToken::Csi("\x1b[2J")),
                Just(VtToken::Csi("\x1b[K")),
                Just(VtToken::Csi("\x1b[J")),
                Just(VtToken::Csi("\x1b[2K")),
                Just(VtToken::Csi("\x1b[A")),
                Just(VtToken::Csi("\x1b[B")),
                Just(VtToken::Csi("\x1b[C")),
                Just(VtToken::Csi("\x1b[D")),
                Just(VtToken::Csi("\x1b[10;5H")),
                Just(VtToken::Csi("\x1b[?25l")),
                Just(VtToken::Csi("\x1b[?25h")),
                Just(VtToken::Csi("\x1b[?1049h")),
                Just(VtToken::Csi("\x1b[?1049l")),
                Just(VtToken::Csi("\x1b[3J")),
                Just(VtToken::Csi("\x1b[38;5;196m")),
                Just(VtToken::Csi("\x1b[48;2;10;20;30m")),
                Just(VtToken::Csi("\x1b[@")),
                Just(VtToken::Csi("\x1b[P")),
                Just(VtToken::Csi("\x1b[L")),
                Just(VtToken::Csi("\x1b[M")),
                Just(VtToken::Csi("\x1b[X")),
                Just(VtToken::Csi("\x1b[1G")),
                Just(VtToken::Csi("\x1b[1;1H")),
            ],
            // OSC sequences: hyperlinks, prompt markers, titles, kitty.
            prop_oneof![
                Just(VtToken::Osc("\x1b]8;;https://example.com\x1b\\")),
                Just(VtToken::Osc("\x1b]8;;\x1b\\")),
                Just(VtToken::Osc("\x1b]133;A\x1b\\")),
                Just(VtToken::Osc("\x1b]133;B\x1b\\")),
                Just(VtToken::Osc("\x1b]0;termos\x07")),
                Just(VtToken::Osc("\x1b]7;file:///home/user\x07")),
                Just(VtToken::Osc("\x1b]52;c;aGVsbG8=\x07")),
                Just(VtToken::Osc("\x1b]99;{\"type\":\"progress\"}\x07")),
            ],
            // ESC sequences: charsets, save/restore cursor, DEC, wide CJK.
            prop_oneof![
                Just(VtToken::Esc("\x1b(0")),
                Just(VtToken::Esc("\x1b(B")),
                Just(VtToken::Esc("\x1b7")),
                Just(VtToken::Esc("\x1b8")),
                Just(VtToken::Esc("\x1b=")),
                Just(VtToken::Esc("\x1b>")),
                Just(VtToken::Esc("\x1b c")),
                Just(VtToken::Esc("\x1b#8")),
                // Wide CJK runes (3-byte UTF-8) exercising the wide path.
                Just(VtToken::Print('你')),
                Just(VtToken::Print('界')),
                Just(VtToken::Print('漢')),
                Just(VtToken::Print('字')),
            ],
        ]
        .boxed()
    }
}

fn check_grid_invariant(scr: &termos::vt::ScreenBuffer) {
    let w = scr.width();
    let h = scr.height();
    for y in 0..h {
        for x in 0..w {
            let cell = scr.cell(x, y).unwrap();
            if cell.width == 0 {
                assert!(
                    cell.content.is_none(),
                    "occupied cell with width 0 at ({},{}): {:?}",
                    x, y, cell.content
                );
            } else {
                assert!(
                    cell.width <= 2,
                    "cell at ({},{}) has width {}",
                    x, y, cell.width
                );
            }
            // Combining marks only ever ride on an occupied base cell.
            assert!(
                cell.combining_len == 0 || cell.content.is_some(),
                "combining run without base at ({},{})",
                x, y
            );
            // Total marks = inline + spill; the spill only exists past the
            // inline budget, and its buffer holds exactly the spilled marks.
            let capacity = cell.combining.len()
                + cell.combining_overflow.as_ref().map_or(0, |v| v.len());
            assert!(
                (cell.combining_len as usize) <= capacity,
                "combining len {} exceeds capacity {} at ({},{})",
                cell.combining_len, capacity, x, y
            );
            assert!(
                cell.combining_overflow.is_none()
                    || (cell.combining_len as usize) > cell.combining.len(),
                "overflow present without inline budget exhausted at ({},{})",
                x, y
            );
        }
    }
}

/// selection_text must never contain phantom spaces from wide-char
/// continuation cells, regardless of what content was written.
#[test]
fn selection_text_never_has_phantom_wide_spaces() {
    let mut emu = Emulator::new(80, 24);
    // Write wide CJK, ASCII, and combining marks.
    emu.write(b"\x1b[2J\x1b[H");
    emu.write("\u{4f60}\u{4f60}XX e\u{301}Z\n".as_bytes());
    emu.write(b"hello world\n");
    // Select across both lines.
    let text = emu.selection_text(0, 0, 1, 10);
    // Must not contain phantom spaces between wide chars.
    assert!(!text.contains("\u{4f60} \u{4f60}"), "phantom space between wide chars: {text:?}");
    // Must contain the combining mark attached to its base.
    assert!(text.contains("e\u{301}Z"), "combining mark lost: {text:?}");
    // Must not contain a space before the combining mark.
    assert!(!text.contains("e \u{301}"), "phantom space before combining: {text:?}");
}
