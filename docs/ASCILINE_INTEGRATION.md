# Asciline-Rust Integration Research

*Generated: August 20, 2026 | Sources: [asciline-rust](https://github.com/KidIkaros/asciline-rust) source code and docs*

## Executive Summary

**Yes, asciline-rust can bridge the gap** — not by replacing ratatui, but by providing a **pixel-level rendering layer** that works alongside it. The key insight: in asciline's pixel mode, every terminal cell becomes a true-color pixel with its own 24-bit RGB background. This gives you what amounts to a **low-resolution framebuffer** inside the terminal — enough for smooth gradients, anti-aliased edges, and shadow effects that pure text-cell rendering can't achieve.

## What Asciline-Rust Actually Is

A real-time ASCII video rendering engine with two output modes:

1. **ASCII mode**: Maps RGB pixels → palette characters + color (the classic "ASCII art" look)
2. **Pixel mode**: Maps RGB pixels → colored space characters (each cell = one pixel, `bg=#RRGGBB`)

The pixel mode is the key for TermOS. It renders with `\x1b[48;2;R;G;Bm ` — a space character with a 24-bit RGB background color. Every cell is an independent pixel.

### Relevant Capabilities

| Capability | Details | Relevance to TermOS |
|---|---|---|
| **Pixel-mode mapper** | `Mapper::map_pixel()` — RGB24 → BGR framebuffer, 3 bytes/cell | Core: render UI elements as colored cells |
| **Rayon-parallel mapping** | Row-parallel `par_chunks_exact` with no locks | Fast: 3,600 fps ceiling at 240×67 grid |
| **Palette system** | DEFAULT_PALETTE (93 levels), FLAT, BLOCK | Could power adaptive color schemes |
| **Codec/encoder** | ZLIB, DELTA, RLE_FULL adaptive compression | Could cache static UI regions |
| **Zero-dependency core** | Only needs rayon + flate2; no ffmpeg for the mapper | Lightweight dependency |
| **Quantization** | `quantize_bits` for color depth reduction | Performance/quality trade-off |

## How to Integrate with TermOS

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

## Performance Considerations

| Operation | Asciline Cost | Notes |
|---|---|---|
| Map 240×67 grid | ~276 µs | < 0.3ms, runs at ~3,600 fps |
| Map 480×135 grid | ~684 µs | < 1ms, runs at ~1,460 fps |
| ZLIB compress | ~50-200 µs | Only for caching static regions |
| Total render pipeline | ~2-3 ms | Well within 16ms budget at 60fps |

**Key**: The mapper is the hot path, and it's rayon-parallelized. At typical terminal sizes (120×40 to 240×67), the gradient/shadow computation is sub-millisecond.

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
- True transparency (terminal-dependent, not app-controlled)
- Drag-and-drop (no terminal standard)

## Verdict

**asciline-rust is the right tool for this job.** Its pixel-mode mapper is exactly what TermOS needs to bridge the visual gap — it turns the terminal into a low-res framebuffer where every cell is a colored pixel. Combined with ratatui for text, this gives you the "modern GUI" feel (gradients, shadows, smooth shapes) without leaving the terminal.

The integration is lightweight (just the mapper module, ~200 lines of code), fast (sub-millisecond at typical terminal sizes), and zero-C-dependency (pure Rust via rayon + miniz_oxide).
