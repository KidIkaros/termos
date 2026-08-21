//! CPU cell-compositor benchmarks.
//!
//! Run with:
//!   cargo bench --features asciline-compositor --bench compositor

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use termos::app::pixel_canvas::{CellCompositor, PixelCanvas, Rgb};

const SIZES: &[(&str, usize, usize)] = &[
    ("80x24", 80, 24),
    ("120x40", 120, 40),
    ("240x67", 240, 67),
    ("480x135", 480, 135),
];

fn rgb_frame(width: usize, height: usize) -> Vec<u8> {
    (0..width * height * 3)
        .map(|i| ((i * 31 + i / 7) % 256) as u8)
        .collect()
}

fn copy_frame(dst: &mut [u8], src: &[u8]) {
    dst.copy_from_slice(src);
}

fn flush_rgb(rgb: &[u8], width: usize, height: usize, buffer: &mut Buffer) {
    for y in 0..height {
        for x in 0..width {
            let idx = (y * width + x) * 3;
            let cell = &mut buffer[(x as u16, y as u16)];
            cell.set_char(' ');
            cell.set_bg(Color::Rgb(rgb[idx], rgb[idx + 1], rgb[idx + 2]));
        }
    }
}

fn bgr_to_rgb(bgr: &[u8], rgb: &mut [u8]) {
    for (src, dst) in bgr.chunks_exact(3).zip(rgb.chunks_exact_mut(3)) {
        dst[0] = src[2];
        dst[1] = src[1];
        dst[2] = src[0];
    }
}

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

#[cfg(feature = "asciline-compositor")]
fn bench_asciline_pipeline(c: &mut Criterion) {
    let mut group = c.benchmark_group("cell_compositor/asciline");
    let mapper = asciline::mapper::Mapper::default(0);
    for &(name, width, height) in SIZES {
        let frame = rgb_frame(width, height);
        let mut bgr = vec![0u8; frame.len()];
        let mut rgb = vec![0u8; frame.len()];
        let mut buffer = Buffer::empty(Rect::new(0, 0, width as u16, height as u16));
        group.bench_with_input(
            BenchmarkId::new("map_convert_flush", name),
            &frame,
            |b, frame| {
                b.iter(|| {
                    mapper.map_pixel(black_box(frame), width, height, &mut bgr);
                    bgr_to_rgb(&bgr, &mut rgb);
                    flush_rgb(&rgb, width, height, &mut buffer);
                    black_box(buffer[(0, 0)].bg);
                });
            },
        );
    }
    group.finish();
}

#[cfg(feature = "asciline-compositor")]
criterion_group!(benches, bench_direct_rgb, bench_asciline_pipeline);

#[cfg(not(feature = "asciline-compositor"))]
criterion_group!(benches, bench_direct_rgb);

criterion_main!(benches);
