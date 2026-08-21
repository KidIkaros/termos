# Asciline-Rust Integration Research

*Generated: August 20, 2026; reviewed August 21, 2026 | Source: [asciline-rust](https://github.com/KidIkaros/asciline-rust) repository README, source, benchmarks, and docs*

## Executive Summary

**Yes, we can build a TermOS compositor on top of asciline-rust without requiring a GPU.** The repository's core is a CPU/Rust renderer: it decodes frames, maps pixels in parallel with Rayon, encodes them, and can write true-color ANSI directly to a terminal. Its measured throughput is high, but it is not GPU-backed and does not claim that a GPU is required. That is an advantage for TermOS: a CPU-only compositor can remain portable across SSH sessions, servers, containers, and machines without graphics hardware.

TermOS should reuse the asciline ideas and, where its public APIs are suitable, depend on the mapper/codec as a focused library. The compositor should remain separate from Ratatui's widget layer so we can replace the current `PixelCanvas` incrementally rather than rewrite the VT/session system.

## What Asciline-Rust Actually Is

A real-time CPU/Rust ASCII and pixel rendering engine with two output modes:

1. **ASCII mode**: Maps RGB pixels → palette characters + color (the classic "ASCII art" look)
2. **Pixel mode**: Maps RGB pixels → colored space characters (each cell = one pixel, `bg=#RRGGBB`)

The pixel mode is the key for TermOS. It renders with `\x1b[48;2;R;G;Bm ` — a space character with a 24-bit RGB background color. Every cell is an independent pixel.

### Relevant Capabilities

| Capability | Details | Relevance to TermOS |
|---|---|---|
| **Pixel-mode mapper** | `Mapper::map_pixel()` — RGB24 → BGR framebuffer, 3 bytes/cell | Core: render UI elements as colored cells |
| **Rayon-parallel mapping** | Row-parallel `par_chunks_exact` with no locks | High CPU throughput: measured ~3,600 fps map ceiling at 240×67 |
| **Palette system** | DEFAULT_PALETTE (93 levels), FLAT, BLOCK | Could power adaptive color schemes |
| **Codec/encoder** | ZLIB, DELTA, RLE_FULL adaptive compression | Could cache static UI regions |
| **Zero-dependency core** | Only needs rayon + flate2; no ffmpeg for the mapper | Lightweight dependency |
| **Quantization** | `quantize_bits` for color depth reduction | Performance/quality trade-off |

## How to Integrate with TermOS

### Proposed compositor seam

```text
Os + VT + layout state
        │
        ▼
TermOS scene/effects description
        │
        ├── Asciline CPU compositor → RGB cell framebuffer / ANSI
        └── Ratatui text compositor → text, widgets, input overlays
```

The first implementation should be a **cell compositor**, not a desktop window
compositor. It owns the background/effects framebuffer, damage tracking,
cache keys, and terminal output encoding. Ratatui remains responsible for
text and widgets until the scene model is stable.

A focused interface could look like:

```rust
pub trait CellCompositor {
    fn resize(&mut self, width: usize, height: usize);
    fn begin_frame(&mut self, background: Rgb);
    fn paint_surface(&mut self, surface: Surface);
    fn paint_shadow(&mut self, shadow: Shadow);
    fn finish_frame(&mut self) -> &[[u8; 3]];
}
```

The concrete implementation can use asciline's row-parallel mapping and
encoding strategy, while TermOS retains its own scene primitives and cache
policy. This avoids coupling the application to asciline's video pipeline,
ffmpeg requirements, WebSocket server, or container format.

### Strategy: Dual-Layer Rendering

```
┌─────────────────────────────────┐
│  Layer 2: ratatui text layer    │  ← text, borders, widgets (existing)
│  (transparent background cells) │
├─────────────────────────────────┤
│  Layer 1: asciline pixel layer  │  ← gradients, shadows, images
│  (colored background cells)     │
└─────────────────────────────────┘
```

**Layer 1 (asciline)** renders into a `Vec<u8>` framebuffer (BGR, 3 bytes/cell) covering the entire terminal area. This provides:
- Gradient backgrounds (horizontal, vertical, radial)
- Shadow effects (colored regions with soft edges)
- Anti-aliased shapes (rounded corners, circles, pill buttons)
- Image thumbnails (via Kitty/Sixel passthrough overlaid on top)

**Layer 2 (ratatui)** renders text and borders on top, with transparent/semi-transparent backgrounds where the gradient shows through.

### Concrete Integration Points

#### 1. Gradient Backgrounds (Easy — 1 day)

Use `Mapper::map_pixel()` to generate a gradient framebuffer, then render it as colored cells before ratatui paints text:

```rust
use asciline::mapper::Mapper;

// Generate a horizontal gradient (e.g., dock bar)
fn render_gradient(start: (u8,u8,u8), end: (u8,u8,u8), width: usize, height: usize) -> Vec<u8> {
    let mut rgb = vec![0u8; width * height * 3];
    for y in 0..height {
        for x in 0..width {
            let t = x as f64 / width as f64;
            let idx = (y * width + x) * 3;
            rgb[idx]     = (start.0 as f64 * (1.0-t) + end.0 as f64 * t) as u8;
            rgb[idx + 1] = (start.1 as f64 * (1.0-t) + end.1 as f64 * t) as u8;
            rgb[idx + 2] = (start.2 as f64 * (1.0-t) + end.2 as f64 * t) as u8;
        }
    }
    let mapper = Mapper::default(0);
    let mut bgr = vec![0u8; width * height * 3];
    mapper.map_pixel(&rgb, width, height, &mut bgr);
    bgr
}
```

Render with: `\x1b[48;2;{b};{g};{r}m \x1b[0m` per cell.

#### 2. Shadow Rendering (Medium — 2 days)

Compute a shadow buffer using Gaussian falloff, then blend it with the background:

```rust
fn shadow_buffer(width: usize, height: usize, offset_x: i32, offset_y: i32) -> Vec<u8> {
    // Each cell gets a darkness value based on distance from the shadow edge
    // Rendered as dark RGB with decreasing opacity (approaching bg color)
    // Uses the same pixel-mode mapper for color output
}
```

#### 3. Anti-Aliased Rounded Corners (Medium — 2 days)

Pre-compute a corner mask using signed distance fields (SDF), then apply it to block characters:

```rust
fn rounded_corner_mask(radius: f64, size: usize) -> Vec<f64> {
    // SDF: distance from the corner center, clamped to [0, 1]
    // Cells with SDF < 0.5 get the corner color, > 0.5 get the bg
    // This gives sub-cell anti-aliasing at cell boundaries
}
```

#### 4. Sparkline/Mini-Graph Rendering (Easy — 1 day)

Use the pixel mapper to render smooth sparklines for CPU/RAM widgets in the dock:

```rust
fn sparkline_rgb(data: &[f64], width: usize, height: usize, color: (u8,u8,u8)) -> Vec<u8> {
    // Map data values to colored cells with smooth interpolation
}
```

#### 5. Image Thumbnails via Kitty Passthrough (Already exists)

TermOS already passes through Kitty/Sixel. Asciline doesn't add new capability here — the passthrough handles it. But asciline could **generate** terminal-native image previews for non-Kitty terminals (by rendering images as colored blocks).

## CPU-only performance considerations

| Operation | Asciline Cost | Notes |
|---|---|---|
| Map 240×67 grid | ~276 µs | < 0.3ms, runs at ~3,600 fps |
| Map 480×135 grid | ~684 µs | < 1ms, runs at ~1,460 fps |
| ZLIB compress | ~50-200 µs | Only for caching static regions |
| Total render pipeline | ~2-3 ms | Well within 16ms budget at 60fps |

**Key**: the mapper is CPU-only but Rayon-parallelized. At typical terminal
sizes it can be fast enough for interactive rendering without a GPU. The
actual TermOS budget must still be measured end to end: scene construction,
VT text rendering, ANSI encoding, terminal transport, and terminal repaint
are outside the mapper benchmark.

## Dependency Cost

```toml
[dependencies]
asciline = { git = "https://github.com/KidIkaros/asciline-rust", default-features = false }
```

With `default-features = false`, this pulls in only:
- `rayon` (already common in Rust projects)
- `flate2` with `miniz_oxide` (pure Rust, no C deps)

**No ffmpeg, no tokio, no axum** — just the mapper and codec modules.

## Architecture Recommendation

```
src/render/
├── mod.rs           // orchestrator
├── gradient.rs      // gradient generation using asciline mapper
├── shadow.rs        // shadow computation + rendering
├── pixel_canvas.rs  // the BGR framebuffer + ratatui integration
└── effects.rs       // sparklines, anti-aliased shapes, SDF
```

The `PixelCanvas` holds a `Vec<u8>` (BGR) the size of the terminal area. Each frame:
1. Clear the canvas to the background color
2. Render gradients, shadows, effects into the canvas
3. Flush the canvas as colored cells to the ratatui `Buffer`
4. Ratatui paints text/borders on top (transparent background cells show the canvas)

## What This Actually Fixes

| Before (current TermOS) | After (with asciline layer) |
|---|---|
| Flat solid-color backgrounds | Smooth gradient backgrounds |
| Hard shadow borders (░▒▓) | Soft Gaussian shadows with color falloff |
| Blocky rounded corners (╭╮╰╯) | Anti-aliased rounded corners via SDF |
| Text-only sparklines | Smooth gradient sparklines |
| No depth perception | Elevation via shadows + brightness |
| "Early computing" feel | "Modern glass" feel |

## What This Doesn't Fix

- Anti-aliased **text** (still grid-based, still needs the terminal's font renderer)
- Variable-width fonts (terminal constraint)
- True desktop backdrop blur or transparency over windows outside TermOS
- GPU composition or subpixel geometry inside a terminal cell grid
- True transparency (terminal-dependent, not app-controlled)
- Drag-and-drop (no terminal standard)

## Delivery plan

The implementation is intentionally incremental. The central optimization is
retention and damage tracking, not blindly parallelizing every full frame:

```text
PTY/layout/input event
        │
        ▼
DamageSet (merged rectangles + reason)
        │
        ├── unchanged cached surfaces reused
        └── changed regions recomposited
                    │
                    ▼
           CPU CellCompositor
        scalar small regions / Rayon large regions
                    │
                    ▼
              Ratatui + ANSI
```


1. Benchmark the current direct-RGB `PixelCanvas` at representative terminal
   sizes and under one/default Rayon thread counts.
2. Add a `CellCompositor` interface and a no-op-behavior-change adapter around
   the current canvas.
3. Move gradients, shadows, rounded surfaces, sparklines, and damage tracking
   behind that interface.
4. Add an asciline-backed mapper/encoder prototype behind a feature or local
   adapter, using only the focused CPU rendering pieces.
5. Compare frame time, allocations, output bytes, determinism, and visual
   equivalence against direct RGB.
6. Add capability-tier fallbacks (true-color, 256-color, ANSI-only), then
   dogfood dock changes, floats, overlays, resize churn, wide/combining text,
   SSH, and web clients.
7. Keep whichever backend wins for each workload; asciline does not need to
   replace every path to be valuable.

## Verdict

**asciline-rust is a strong foundation for a CPU-only TermOS cell compositor.**
It can provide the terminal-side visual layer—gradients, shadows, colored
blocks, caching, and high-rate frame mapping—without requiring a GPU. The
best design is to reuse its focused mapper/codec techniques behind a TermOS
`CellCompositor` interface, not to embed the entire video server/player.

This materially improves the terminal experience, but it does not turn a
terminal into a desktop compositor. True Liquid Glass over arbitrary desktop
content still requires a graphical window surface and platform compositor.
