//! VT emulator render-path benchmarks — mirrors the Go
//! `BenchmarkEmulatorRenderReal`.
//!
//! The emulator's `render_view_lines` is the single most expensive thing
//! the unfocused render path does. Measured at 207x55 (maintainer's real
//! terminal) and 80x24 (common default).

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use tuios::vt::Emulator;

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

criterion_group!(benches, bench_render);
criterion_main!(benches);
