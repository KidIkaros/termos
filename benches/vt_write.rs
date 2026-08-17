//! VT emulator write-path benchmarks — mirrors the Go
//! `BenchmarkEmulatorWriteHeavyOutput` at the maintainer's real terminal
//! size (207x55).
//!
//! Shapes measured:
//! - `plain-log`: a compiler/test-runner scrolling past (no SGR)
//! - `colored-log`: the same volume with per-line 256-color SGR
//! - `fullscreen-repaint`: an editor/dashboard repainting every row

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use tuios::vt::Emulator;

const COLS: i32 = 207;
const ROWS: i32 = 55;

fn plain_log() -> Vec<u8> {
    let line = b"compiling package github.com/example/project/internal/thing\r\n";
    let mut buf = Vec::new();
    for _ in 0..32 {
        buf.extend_from_slice(line);
    }
    buf
}

fn colored_log() -> Vec<u8> {
    let mut buf = Vec::new();
    for i in 0..32u32 {
        buf.extend_from_slice(
            format!(
                "\x1b[38;5;{}mok   github.com/example/project/pkg{:02}\t0.0{:02}s\x1b[m\r\n",
                32 + (i % 6),
                i,
                i % 10
            )
            .as_bytes(),
        );
    }
    buf
}

fn fullscreen_repaint() -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(b"\x1b[H");
    for y in 1..=ROWS {
        buf.extend_from_slice(
            format!(
                "\x1b[{};1H\x1b[48;5;{}m\x1b[38;5;15m{}\x1b[m",
                y,
                16 + ((y as u32) % 200),
                "x".repeat((COLS - 1) as usize)
            )
            .as_bytes(),
        );
    }
    buf
}

fn bench_write(c: &mut Criterion) {
    let cases: &[(&str, Vec<u8>)] = &[
        ("plain-log", plain_log()),
        ("colored-log", colored_log()),
        ("fullscreen-repaint", fullscreen_repaint()),
    ];

    let mut group = c.benchmark_group("vt_write");
    for (name, data) in cases {
        group.throughput(Throughput::Bytes(data.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(name), data, |b, data| {
            b.iter(|| {
                let mut emu = Emulator::new(COLS, ROWS);
                emu.write(black_box(data));
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_write);
criterion_main!(benches);
