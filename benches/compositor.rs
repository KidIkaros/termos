//! CPU cell-compositor benchmarks.
//!
//! Run with:
//!   cargo bench --features asciline-compositor --bench compositor
//!   cargo bench --bench compositor               (without asciline)

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use termos::app::damage::{DamageReason, DamageRect};
use termos::app::pixel_canvas::{CanvasCacheKey, CellCompositor, PixelCanvas, Rgb};
use termos::layout::Rect as LRect;

const SIZES: &[(&str, usize, usize)] = &[
    ("80x24", 80, 24),
    ("120x40", 120, 40),
    ("240x67", 240, 67),
    ("480x135", 480, 135),
];

/// Damage ratios to benchmark: (label, fraction of cells damaged).
const DAMAGE_RATIOS: &[(&str, f64)] = &[
    ("1pct", 0.01),
    ("10pct", 0.10),
    ("50pct", 0.50),
    ("100pct", 1.00),
];

fn rgb_frame(width: usize, height: usize) -> Vec<u8> {
    (0..width * height * 3)
        .map(|i| ((i * 31 + i / 7) % 256) as u8)
        .collect()
}

fn copy_frame(dst: &mut [u8], src: &[u8]) {
    dst.copy_from_slice(src);
}

/// Replicate the damage-aware flush logic from render.rs.
fn flush_damage_aware(
    rgb: &[u8],
    width: usize,
    height: usize,
    buf: &mut Buffer,
    damage: &[DamageRect],
) {
    if damage.is_empty() {
        // Full-frame flush.
        for y in 0..height {
            for x in 0..width {
                let idx = (y * width + x) * 3;
                let cell = &mut buf[(x as u16, y as u16)];
                cell.set_char(' ');
                cell.set_bg(Color::Rgb(rgb[idx], rgb[idx + 1], rgb[idx + 2]));
            }
        }
        return;
    }
    for d in damage {
        let x0 = d.rect.x.max(0) as usize;
        let y0 = d.rect.y.max(0) as usize;
        let x1 = (d.rect.x + d.rect.w).min(width as i32) as usize;
        let y1 = (d.rect.y + d.rect.h).min(height as i32) as usize;
        for y in y0..y1 {
            for x in x0..x1 {
                let idx = (y * width + x) * 3;
                let cell = &mut buf[(x as u16, y as u16)];
                cell.set_char(' ');
                cell.set_bg(Color::Rgb(rgb[idx], rgb[idx + 1], rgb[idx + 2]));
            }
        }
    }
}

fn flush_rgb(rgb: &[u8], width: usize, height: usize, buffer: &mut Buffer) {
    flush_damage_aware(rgb, width, height, buffer, &[]);
}



/// Build damage rects for a given fraction of the canvas.
/// Returns a single rect covering `frac` of the total cells (top-left corner).
fn damage_for_ratio(width: usize, height: usize, frac: f64) -> Vec<DamageRect> {
    let total = width * height;
    let damaged_cells = (total as f64 * frac).ceil() as usize;
    let cols = (damaged_cells as f64).sqrt().ceil() as usize;
    let cols = cols.min(width);
    let rows = (damaged_cells as f64 / cols as f64).ceil() as usize;
    let rows = rows.min(height);
    vec![DamageRect::new(
        LRect { x: 0, y: 0, w: cols as i32, h: rows as i32 },
        DamageReason::Output,
    )]
}

// ---------------------------------------------------------------------------
// Benchmarks
// ---------------------------------------------------------------------------

/// Full pipeline: effects + flush (no cache, no damage).
fn bench_direct_rgb(c: &mut Criterion) {
    let mut group = c.benchmark_group("cell_compositor/direct_rgb");
    for &(name, width, height) in SIZES {
        let frame = rgb_frame(width, height);
        group.bench_with_input(
            BenchmarkId::new("effects_and_flush", name),
            &frame,
            |b, frame| {
                let mut canvas = PixelCanvas::new(width, height);
                let mut buffer = Buffer::empty(Rect::new(0, 0, width as u16, height as u16));
                b.iter(|| {
                    canvas.begin_frame(Rgb(0, 0, 0));
                    canvas.gradient_horizontal(
                        0,
                        height.saturating_sub(2),
                        width,
                        1,
                        (0, 0, 0),
                        (32, 64, 128),
                    );
                    copy_frame(canvas.rgb_mut(), frame);
                    flush_rgb(canvas.finish_frame(), width, height, &mut buffer);
                    black_box(buffer[(0, 0)].bg);
                });
            },
        );
    }
    group.finish();
}

/// Retained surface: cache hit vs cache miss.
fn bench_cache_hit_vs_miss(c: &mut Criterion) {
    let mut group = c.benchmark_group("cell_compositor/cache");
    for &(name, width, height) in SIZES {
        let key = CanvasCacheKey {
            bg: (10, 10, 10),
            accent: (5, 5, 5),
            dock: (20, 20, 20),
            dock_position: 0,
            width: width as u16,
            height: height as u16,
            floats_hash: 0,
        };

        // Cache miss: full effects + commit.
        group.bench_with_input(
            BenchmarkId::new("miss", name),
            &key,
            |b, key| {
                let mut canvas = PixelCanvas::new(width, height);
                b.iter(|| {
                    canvas.fill_background(
                        key.bg, key.accent, key.dock, "bottom",
                    );
                    canvas.commit_cache(CanvasCacheKey::clone(key));
                    black_box(canvas.finish_frame()[0]);
                });
            },
        );

        // Cache hit: restore from cache (memcpy only).
        group.bench_with_input(
            BenchmarkId::new("hit", name),
            &key,
            |b, key| {
                let mut canvas = PixelCanvas::new(width, height);
                canvas.fill_background(
                    key.bg, key.accent, key.dock, "bottom",
                );
                canvas.commit_cache(CanvasCacheKey::clone(key));
                b.iter(|| {
                    canvas.restore_cache();
                    black_box(canvas.finish_frame()[0]);
                });
            },
        );
    }
    group.finish();
}

/// Damage-aware flush at different damage ratios.
fn bench_damage_flush(c: &mut Criterion) {
    let mut group = c.benchmark_group("cell_compositor/damage_flush");
    for &(name, width, height) in SIZES {
        let frame = rgb_frame(width, height);
        for &(ratio_name, ratio) in DAMAGE_RATIOS {
            let damage = damage_for_ratio(width, height, ratio);
            group.bench_with_input(
                BenchmarkId::new(format!("{name}/{ratio_name}"), ""),
                &(&frame, &damage),
                |b, (frame, damage)| {
                    let mut buffer = Buffer::empty(Rect::new(0, 0, width as u16, height as u16));
                    b.iter(|| {
                        flush_damage_aware(
                            black_box(frame),
                            width,
                            height,
                            &mut buffer,
                            damage,
                        );
                        black_box(buffer[(0, 0)].bg);
                    });
                },
            );
        }
    }
    group.finish();
}

/// Full pipeline: effects + damage-aware flush at different damage levels.
fn bench_full_pipeline(c: &mut Criterion) {
    let mut group = c.benchmark_group("cell_compositor/full_pipeline");
    for &(name, width, height) in SIZES {
        let key = CanvasCacheKey {
            bg: (10, 10, 10),
            accent: (5, 5, 5),
            dock: (20, 20, 20),
            dock_position: 0,
            width: width as u16,
            height: height as u16,
            floats_hash: 0,
        };

        for &(ratio_name, ratio) in DAMAGE_RATIOS {
            let damage = damage_for_ratio(width, height, ratio);

            // Cache miss: effects + damage flush.
            group.bench_with_input(
                BenchmarkId::new(format!("{name}/{ratio_name}/miss"), ""),
                &(&key, &damage),
                |b, (key, damage)| {
                    let mut canvas = PixelCanvas::new(width, height);
                    let mut buffer = Buffer::empty(Rect::new(0, 0, width as u16, height as u16));
                    b.iter(|| {
                        canvas.fill_background(
                            key.bg, key.accent, key.dock, "bottom",
                        );
                        canvas.commit_cache(CanvasCacheKey::clone(key));
                        flush_damage_aware(
                            canvas.finish_frame(),
                            width,
                            height,
                            &mut buffer,
                            damage,
                        );
                        black_box(buffer[(0, 0)].bg);
                    });
                },
            );

            // Cache hit: restore + damage flush.
            group.bench_with_input(
                BenchmarkId::new(format!("{name}/{ratio_name}/hit"), ""),
                &(&key, &damage),
                |b, (key, damage)| {
                    let mut canvas = PixelCanvas::new(width, height);
                    let mut buffer = Buffer::empty(Rect::new(0, 0, width as u16, height as u16));
                    canvas.fill_background(
                        key.bg, key.accent, key.dock, "bottom",
                    );
                    canvas.commit_cache(CanvasCacheKey::clone(key));
                    b.iter(|| {
                        canvas.restore_cache();
                        flush_damage_aware(
                            canvas.finish_frame(),
                            width,
                            height,
                            &mut buffer,
                            damage,
                        );
                        black_box(buffer[(0, 0)].bg);
                    });
                },
            );
        }
    }
    group.finish();
}

/// Flush asciline's 4-byte `[char, R, G, B]` output into a Ratatui buffer.
/// Extracts RGB from the interleaved format.
#[cfg(feature = "asciline-compositor")]
fn flush_asciline_cells(cells: &[u8], width: usize, height: usize, buf: &mut Buffer) {
    for y in 0..height {
        for x in 0..width {
            let idx = (y * width + x) * 4;
            let char_byte = cells[idx];
            let r = cells[idx + 1];
            let g = cells[idx + 2];
            let b = cells[idx + 3];
            let cell = &mut buf[(x as u16, y as u16)];
            // Use the palette character if it's a visible glyph, else space.
            if char_byte > 0x20 {
                cell.set_char(char_byte as char);
            } else {
                cell.set_char(' ');
            }
            cell.set_bg(Color::Rgb(r, g, b));
        }
    }
}

/// Asciline `map_ascii`: RGB → [char, R, G, B] cells with palette + Rayon.
/// This is the REAL compositor — not the pixel format converter.
#[cfg(feature = "asciline-compositor")]
fn bench_asciline_map_ascii(c: &mut Criterion) {
    let mut group = c.benchmark_group("cell_compositor/asciline_map_ascii");
    let mapper = asciline::mapper::Mapper::default(0);
    for &(name, width, height) in SIZES {
        let frame = rgb_frame(width, height);
        let mut cells = vec![0u8; width * height * 4];
        let mut buffer = Buffer::empty(Rect::new(0, 0, width as u16, height as u16));
        group.bench_with_input(
            BenchmarkId::new("map_and_flush", name),
            &frame,
            |b, frame| {
                b.iter(|| {
                    mapper.map_ascii(black_box(frame), width, height, &mut cells);
                    flush_asciline_cells(&cells, width, height, &mut buffer);
                    black_box(buffer[(0, 0)].bg);
                });
            },
        );
    }
    group.finish();
}

/// Head-to-head: PixelCanvas effects + flush vs Asciline map_ascii + flush.
#[cfg(feature = "asciline-compositor")]
fn bench_head_to_head(c: &mut Criterion) {
    let mut group = c.benchmark_group("cell_compositor/head_to_head");
    let mapper = asciline::mapper::Mapper::default(0);
    for &(name, width, height) in SIZES {
        let frame = rgb_frame(width, height);
        let mut cells = vec![0u8; width * height * 4];

        // PixelCanvas: fill_background + gradient + flush.
        group.bench_with_input(
            BenchmarkId::new("pixel_canvas", name),
            &frame,
            |b, frame| {
                let mut canvas = PixelCanvas::new(width, height);
                let mut buffer = Buffer::empty(Rect::new(0, 0, width as u16, height as u16));
                b.iter(|| {
                    canvas.fill_background((10, 10, 10), (5, 5, 5), (20, 20, 20), "bottom");
                    canvas.gradient_horizontal(
                        0, height.saturating_sub(2), width, 1,
                        (10, 10, 10), (32, 64, 128),
                    );
                    copy_frame(canvas.rgb_mut(), frame);
                    flush_rgb(canvas.finish_frame(), width, height, &mut buffer);
                    black_box(buffer[(0, 0)].bg);
                });
            },
        );

        // Asciline: map_ascii + flush with palette characters.
        group.bench_with_input(
            BenchmarkId::new("asciline_map_ascii", name),
            &frame,
            |b, frame| {
                let mut buffer = Buffer::empty(Rect::new(0, 0, width as u16, height as u16));
                b.iter(|| {
                    mapper.map_ascii(black_box(frame), width, height, &mut cells);
                    flush_asciline_cells(&cells, width, height, &mut buffer);
                    black_box(buffer[(0, 0)].bg);
                });
            },
        );
    }
    group.finish();
}

#[cfg(feature = "asciline-compositor")]
criterion_group!(
    benches,
    bench_direct_rgb,
    bench_cache_hit_vs_miss,
    bench_damage_flush,
    bench_full_pipeline,
    bench_asciline_map_ascii,
    bench_head_to_head,
);

#[cfg(not(feature = "asciline-compositor"))]
criterion_group!(
    benches,
    bench_direct_rgb,
    bench_cache_hit_vs_miss,
    bench_damage_flush,
    bench_full_pipeline,
);

criterion_main!(benches);
