//! Fuzz target: feed arbitrary byte streams to the VT emulator and verify
//! invariants that matter to a multiplexer:
//! - parsing never panics
//! - screen dimensions never change
//! - cursor stays in bounds (or at the phantom position)
//! - rendered output is valid (no panic on render)
//!
//! Mirrors the Go `FuzzEmulatorWrite` fuzz target.

#![no_main]

use libfuzzer_sys::fuzz_target;
use termos::vt::Emulator;

const WIDTH: i32 = 80;
const HEIGHT: i32 = 24;

fuzz_target!(|data: &[u8]| {
    // Cap input size to what a PTY read could plausibly deliver.
    let data = if data.len() > 65536 { &data[..65536] } else { data };

    let mut emu = Emulator::new(WIDTH, HEIGHT);
    emu.write(data);

    // Screen dimensions must not change.
    assert_eq!(emu.width(), WIDTH, "parsing changed screen width");
    assert_eq!(emu.height(), HEIGHT, "parsing changed screen height");

    // Cursor must stay in bounds (or at the phantom position == width).
    let pos = emu.cursor_position();
    assert!(
        pos.x >= 0 && pos.x <= WIDTH,
        "cursor X out of bounds: {} not in [0,{}]",
        pos.x,
        WIDTH
    );
    assert!(
        pos.y >= 0 && pos.y < HEIGHT,
        "cursor Y out of bounds: {} not in [0,{})",
        pos.y,
        HEIGHT
    );

    // Rendering must succeed.
    let _text = emu.render_text();
});
