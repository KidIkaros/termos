//! Property-based tests using proptest — verifies invariants that must hold
//! for all inputs, not just hand-picked examples.
//!
//! These mirror the Go fuzz targets' invariant checks but run as part of
//! `cargo test` for CI integration.

use proptest::prelude::*;
use tuios::vt::Emulator;

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
        let (_commands, _errors) = tuios::tape::parser::parse_file(input);
        // Should not panic on any input.
    }
}
