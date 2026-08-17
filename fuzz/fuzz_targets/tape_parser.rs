//! Fuzz target: feed arbitrary strings to the tape parser and verify it
//! never panics.
//!
//! Mirrors the Go tape fuzz target.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &str| {
    let _ = termos::tape::parser::parse_file(input);
});
