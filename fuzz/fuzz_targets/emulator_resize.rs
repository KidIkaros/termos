//! Fuzz target: interleave writes with resizes and verify the cursor stays
//! in bounds and the screen dimensions match the last resize.
//!
//! Mirrors the Go `FuzzEmulatorResize` fuzz target.

#![no_main]

use libfuzzer_sys::fuzz_target;
use termos::vt::Emulator;

fuzz_target!(|input: (Vec<u8>, u8, u8)| {
    let (data, w, h) = input;
    let data = if data.len() > 32768 { &data[..32768] } else { &data };

    // Fuzz the positive domain the emulator actually sees.
    let width = (w as i32 % 200).max(1);
    let height = (h as i32 % 200).max(1);

    let mut emu = Emulator::new(80, 24);
    let half = data.len() / 2;
    emu.write(&data[..half]);
    emu.resize(width, height);
    emu.write(&data[half..]);

    assert_eq!(emu.width(), width, "width after resize mismatch");
    assert_eq!(emu.height(), height, "height after resize mismatch");

    let pos = emu.cursor_position();
    assert!(
        pos.x >= 0 && pos.x <= width,
        "cursor ({},{}) outside {}x{} screen after resize",
        pos.x,
        pos.y,
        width,
        height
    );
    assert!(
        pos.y >= 0 && pos.y < height,
        "cursor ({},{}) outside {}x{} screen after resize",
        pos.x,
        pos.y,
        width,
        height
    );

    let _text = emu.render_text();
});
