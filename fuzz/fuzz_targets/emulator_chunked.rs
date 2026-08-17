//! Fuzz target: feed the same bytes whole vs. one byte at a time and verify
//! both paths produce the same screen contents. A PTY read boundary can land
//! anywhere, so chunked writes must reach the same state as whole writes.
//!
//! Mirrors the Go `FuzzEmulatorWriteChunked` fuzz target.

#![no_main]

use libfuzzer_sys::fuzz_target;
use termos::vt::Emulator;

fuzz_target!(|data: &[u8]| {
    let data = if data.len() > 32768 { &data[..32768] } else { data };

    let mut whole = Emulator::new(80, 24);
    whole.write(data);

    let mut split = Emulator::new(80, 24);
    for byte in data {
        split.write(std::slice::from_ref(byte));
    }

    assert_eq!(
        whole.render_text(),
        split.render_text(),
        "chunked write produced different screen than whole write"
    );
});
