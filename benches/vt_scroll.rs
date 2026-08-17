//! VT emulator scroll-path benchmarks — mirrors the Go
//! `BenchmarkEmulatorShortLineScroll` and `BenchmarkEmulatorScrollThroughput`.
//!
//! A short-line flood (e.g. `yes`, build logs) scrolls once every few
//! graphemes, so the per-scroll cost dominates. A long line amortises the
//! scroll over the printing. Both are measured with and without scrollback
//! retention to isolate the write cost from the retention cost.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use tuios::vt::Emulator;

const COLS: i32 = 207;
const ROWS: i32 = 55;

fn bench_short_line_scroll(c: &mut Criterion) {
    let line = b"tuiosflood\r\n";

    let mut group = c.benchmark_group("vt_short_line_scroll");

    group.throughput(Throughput::Bytes(line.len() as u64));
    group.bench_with_input(BenchmarkId::new("with-scrollback", "10k"), &(), |b, _| {
        b.iter(|| {
            let mut emu = Emulator::new(COLS, ROWS);
            emu.set_scrollback_max_lines(10000);
            emu.write(black_box(line));
        });
    });

    group.bench_with_input(
        BenchmarkId::new("alt-screen-no-scrollback", "alt"),
        &(),
        |b, _| {
            b.iter(|| {
                let mut emu = Emulator::new(COLS, ROWS);
                emu.write(b"\x1b[?1049h");
                emu.write(black_box(line));
            });
        },
    );

    group.finish();
}

fn bench_scroll_throughput(c: &mut Criterion) {
    let line = {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"output line with some length to it ".repeat(5).as_slice());
        buf.push(b'\r');
        buf.push(b'\n');
        buf
    };

    let mut group = c.benchmark_group("vt_scroll_throughput");

    group.throughput(Throughput::Bytes(line.len() as u64));
    group.bench_with_input(BenchmarkId::new("with-scrollback", "10k"), &(), |b, _| {
        b.iter(|| {
            let mut emu = Emulator::new(COLS, ROWS);
            emu.set_scrollback_max_lines(10000);
            emu.write(black_box(&line));
        });
    });

    group.bench_with_input(
        BenchmarkId::new("alt-screen-no-scrollback", "alt"),
        &(),
        |b, _| {
            b.iter(|| {
                let mut emu = Emulator::new(COLS, ROWS);
                emu.write(b"\x1b[?1049h");
                emu.write(black_box(&line));
            });
        },
    );

    group.finish();
}

criterion_group!(benches, bench_short_line_scroll, bench_scroll_throughput);
criterion_main!(benches);
