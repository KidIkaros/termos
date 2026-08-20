//! VT emulator render-path benchmarks — mirrors the Go
//! `BenchmarkEmulatorRenderReal`.
//!
//! The emulator's `render_view_lines` is the single most expensive thing
//! the unfocused render path does. Measured at 207x55 (maintainer's real
//! terminal) and 80x24 (common default).

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use termos::vt::Emulator;

fn fill_screen(emu: &mut Emulator, cols: i32, rows: i32) {
    for y in 1..=rows {
        emu.write(
            format!(
                "\x1b[{};1H\x1b[38;5;{}m{}\x1b[m",
                y,
                16 + ((y as u32) % 200),
                "content ".repeat(cols as usize / 8)
            )
            .as_bytes(),
        );
    }
}

fn bench_render(c: &mut Criterion) {
    let mut group = c.benchmark_group("vt_render");

    for (name, cols, rows) in &[("207x55", 207i32, 55i32), ("80x24", 80i32, 24i32)] {
        group.bench_with_input(
            BenchmarkId::from_parameter(name),
            &(*cols, *rows),
            |b, &(c, r)| {
                let mut emu = Emulator::new(c, r);
                fill_screen(&mut emu, c, r);
                b.iter(|| {
                    let lines = emu.render_view_lines();
                    black_box(lines);
                });
            },
        );
    }

    group.finish();
}

/// Per-cell style resolution through the per-frame `StylePalette`. This is
/// the cost the palette approach replaced: previously each cell re-resolved
/// `Color::Default`/`Color::Indexed` through an `Option<&Theme>` branch.
/// Raw ingest throughput for PTY output. `plain` is a single 1 MiB run of
/// printable ASCII (the fast path: batched straight into the screen).
/// `escaped` interleaves a CSI sequence every 80 chars, exercising the
/// parser path plus the fast-path runs between escapes.
fn bench_write(c: &mut Criterion) {
    let mut group = c.benchmark_group("vt_write");

    let plain: Vec<u8> = (0..1_048_576u32).map(|i| b'a' + (i % 26) as u8).collect();
    let mut escaped = Vec::with_capacity(1_100_000);
    for i in 0..1_048_576u32 {
        if i % 80 == 0 {
            escaped.extend_from_slice(b"\x1b[38;5;1m");
        }
        escaped.push(b'a' + (i % 26) as u8);
    }

    group.bench_function("plain_1MiB", |b| {
        b.iter(|| {
            let mut emu = Emulator::new(200, 50);
            emu.write(black_box(&plain));
            black_box(emu.screen().cursor.pos);
        });
    });

    group.bench_function("escaped_1MiB", |b| {
        b.iter(|| {
            let mut emu = Emulator::new(200, 50);
            emu.write(black_box(&escaped));
            black_box(emu.screen().cursor.pos);
        });
    });

    group.finish();
}

fn bench_style_resolution(c: &mut Criterion) {
    let mut group = c.benchmark_group("vt_style");

    for (name, cols, rows) in &[("207x55", 207i32, 55i32), ("80x24", 80i32, 24i32)] {
        group.bench_with_input(
            BenchmarkId::from_parameter(name),
            &(*cols, *rows),
            |b, &(c, r)| {
                let mut emu = Emulator::new(c, r);
                fill_screen(&mut emu, c, r);
                let lines = emu.render_view_lines();
                let theme = termos::config::theme::Theme::catppuccin_mocha();
                let palette = termos::ui::StylePalette::new(Some(&theme));
                b.iter(|| {
                    let mut acc = 0u64;
                    for row in &lines {
                        for sc in row {
                            acc = acc.wrapping_add(palette.style(sc.style).add_modifier.bits() as u64);
                        }
                    }
                    black_box(acc);
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_render,
    bench_style_resolution,
    bench_write
);
criterion_main!(benches);
