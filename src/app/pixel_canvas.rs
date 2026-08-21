//! Pixel canvas — a low-resolution framebuffer for GUI-like visual effects.
//!
//! Renders gradient backgrounds, soft shadows, anti-aliased shapes, and
//! gradient sparklines directly as ratatui RGB cell backgrounds. Each
//! terminal cell remains a 24-bit RGB pixel.
//!
//! Architecture: dual-layer rendering
//! - Layer 1 (this module): gradient/shadow/shape backgrounds
//! - Layer 2 (ratatui): text, borders, widgets with transparent backgrounds

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

/// Cache of computed drop-shadow intensity masks.
///
/// The Gaussian falloff at a cell depends only on its distance from the
/// shadow-casting rectangle's edges, so the intensity grid for a given
/// `(rect_w, rect_h, radius)` is identical for every frame and float
/// position.  Computing it once and sharing it across frames skips the
/// per-cell `exp()` (and the distance math) on every render.
///
/// Bounded: once the cache exceeds [`SHADOW_MASK_CACHE_MAX`] entries the
/// whole cache is cleared, so a session with many transient float sizes
/// cannot accumulate stale masks.
static SHADOW_MASK_CACHE: LazyLock<Mutex<HashMap<ShadowMaskKey, ShadowMask>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Maximum number of cached shadow masks before the cache is reset.
const SHADOW_MASK_CACHE_MAX: usize = 64;

/// The cached intensity grid for one `(rect_w, rect_h, radius)` key.
type ShadowMask = Arc<Vec<f32>>;
/// Mask lookup key: rectangle size plus the radius' raw bits.
type ShadowMaskKey = (usize, usize, u64);

/// A pixel canvas backed by an RGB framebuffer (3 bytes per cell).
///
/// The canvas is sized to the terminal area and rendered as colored cells.
/// Ratatui paints text on top with transparent backgrounds showing the canvas
/// through.
/// The colors that fully determine the canvas background layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackgroundKey {
    pub bg: (u8, u8, u8),
    pub accent_end: (u8, u8, u8),
    pub dock: (u8, u8, u8),
}

pub struct PixelCanvas {
    /// RGB framebuffer: `[R, G, B]` per cell, `width * height * 3` bytes.
    rgb: Vec<u8>,
    width: usize,
    height: usize,
    /// Cached background layer (solid fill + accent gradient + dock row) and
    /// the color key that produced it, so an unchanged background is a memcpy
    /// instead of recomputed gradients/lerps.
    bg_cache_key: Option<BackgroundKey>,
    bg_cache: Vec<u8>,
}

impl PixelCanvas {
    /// Create a new canvas for the given terminal dimensions.
    pub fn new(width: usize, height: usize) -> Self {
        let pixels = width * height;
        Self {
            rgb: vec![0u8; pixels * 3],
            width,
            height,
            bg_cache_key: None,
            bg_cache: Vec::new(),
        }
    }

    /// Fill the background layer: a solid fill, a one-row accent gradient
    /// above the dock, and a solid dock row. The computed RGB buffer is cached
    /// so a later call with the same colors is a memcpy rather than recomputed
    /// gradient/lerp work.
    pub fn fill_background(
        &mut self,
        bg: (u8, u8, u8),
        accent_end: (u8, u8, u8),
        dock: (u8, u8, u8),
        dock_position: &str,
    ) {
        let key = BackgroundKey {
            bg,
            accent_end,
            dock,
        };
        if self.bg_cache_key == Some(key) && self.bg_cache.len() == self.rgb.len() {
            self.rgb.copy_from_slice(&self.bg_cache);
            return;
        }
        self.clear(bg.0, bg.1, bg.2);
        match dock_position {
            "top" => {
                // Dock at top: accent bar at row 1, dock at row 0.
                if self.height >= 2 {
                    self.gradient_horizontal(0, 1, self.width, 1, bg, accent_end);
                }
                if self.height >= 1 {
                    for x in 0..self.width {
                        self.set_pixel(x, 0, dock.0, dock.1, dock.2);
                    }
                }
            }
            "hidden" => {
                // No dock — just background.
            }
            _ => {
                // "bottom" (default)
                if self.height >= 2 {
                    self.gradient_horizontal(0, self.height - 2, self.width, 1, bg, accent_end);
                }
                if self.height >= 1 {
                    for x in 0..self.width {
                        self.set_pixel(x, self.height - 1, dock.0, dock.1, dock.2);
                    }
                }
            }
        }
        self.bg_cache.clear();
        self.bg_cache.extend_from_slice(&self.rgb);
        self.bg_cache_key = Some(key);
    }

    /// Clear the canvas to a solid color.
    pub fn clear(&mut self, r: u8, g: u8, b: u8) {
        for pixel in self.rgb.chunks_exact_mut(3) {
            pixel[0] = r;
            pixel[1] = g;
            pixel[2] = b;
        }
    }

    /// Set a single pixel (RGB) in the canvas.
    pub fn set_pixel(&mut self, x: usize, y: usize, r: u8, g: u8, b: u8) {
        if x < self.width && y < self.height {
            let idx = (y * self.width + x) * 3;
            self.rgb[idx] = r;
            self.rgb[idx + 1] = g;
            self.rgb[idx + 2] = b;
        }
    }

    /// Get a pixel's RGB values.
    pub fn get_pixel(&self, x: usize, y: usize) -> (u8, u8, u8) {
        if x < self.width && y < self.height {
            let idx = (y * self.width + x) * 3;
            (self.rgb[idx], self.rgb[idx + 1], self.rgb[idx + 2])
        } else {
            (0, 0, 0)
        }
    }

    /// Get the RGB framebuffer for rendering.
    pub fn rgb(&self) -> &[u8] {
        &self.rgb
    }

    /// Width in cells.
    pub fn width(&self) -> usize {
        self.width
    }

    /// Height in cells.
    pub fn height(&self) -> usize {
        self.height
    }

    // ─── Gradient primitives ───────────────────────────────────────────

    /// Draw a horizontal gradient across a rectangular region.
    pub fn gradient_horizontal(
        &mut self,
        x0: usize,
        y0: usize,
        w: usize,
        h: usize,
        start: (u8, u8, u8),
        end: (u8, u8, u8),
    ) {
        if w == 0 || h == 0 || self.width == 0 || self.height == 0 {
            return;
        }
        for dy in 0..h {
            let y = y0 + dy;
            if y >= self.height {
                break;
            }
            for dx in 0..w {
                let x = x0 + dx;
                if x >= self.width {
                    break;
                }
                let t = if w <= 1 {
                    0.0
                } else {
                    dx as f64 / (w - 1) as f64
                };
                let r = lerp(start.0, end.0, t);
                let g = lerp(start.1, end.1, t);
                let b = lerp(start.2, end.2, t);
                self.set_pixel(x, y, r, g, b);
            }
        }
    }

    /// Draw a vertical gradient across a rectangular region.
    pub fn gradient_vertical(
        &mut self,
        x0: usize,
        y0: usize,
        w: usize,
        h: usize,
        top: (u8, u8, u8),
        bottom: (u8, u8, u8),
    ) {
        if h == 0 {
            return;
        }
        for dy in 0..h {
            let y = y0 + dy;
            if y >= self.height {
                break;
            }
            let t = dy as f64 / (h - 1).max(1) as f64;
            let r = lerp(top.0, bottom.0, t);
            let g = lerp(top.1, bottom.1, t);
            let b = lerp(top.2, bottom.2, t);
            for dx in 0..w {
                let x = x0 + dx;
                if x >= self.width {
                    break;
                }
                self.set_pixel(x, y, r, g, b);
            }
        }
    }

    /// Draw a radial gradient (circular falloff) centered at (cx, cy).
    pub fn gradient_radial(
        &mut self,
        cx: f64,
        cy: f64,
        radius: f64,
        center_color: (u8, u8, u8),
        edge_color: (u8, u8, u8),
    ) {
        if radius <= 0.0 || self.width == 0 || self.height == 0 {
            if radius <= 0.0 {
                self.set_pixel(cx.max(0.0) as usize, cy.max(0.0) as usize, center_color.0, center_color.1, center_color.2);
            }
            return;
        }
        let r2 = radius * radius;
        let min_x = (cx - radius).max(0.0) as usize;
        let max_x = (cx + radius).min(self.width as f64 - 1.0) as usize;
        let min_y = (cy - radius).max(0.0) as usize;
        let max_y = (cy + radius).min(self.height as f64 - 1.0) as usize;

        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let dx = x as f64 - cx;
                let dy = y as f64 - cy;
                let dist2 = dx * dx + dy * dy;
                let t = (dist2 / r2).min(1.0);
                let r = lerp(center_color.0, edge_color.0, t);
                let g = lerp(center_color.1, edge_color.1, t);
                let b = lerp(center_color.2, edge_color.2, t);
                self.set_pixel(x, y, r, g, b);
            }
        }
    }

    // ─── Shadow primitives ─────────────────────────────────────────────

    /// Draw a drop shadow offset from a rectangle.
    ///
    /// The shadow is a Gaussian falloff from dark (near the rectangle) to
    /// transparent (merging with the background).  `bg` is the background
    /// color the shadow fades into.
    #[allow(clippy::too_many_arguments)]
    pub fn drop_shadow(
        &mut self,
        rect_x: usize,
        rect_y: usize,
        rect_w: usize,
        rect_h: usize,
        offset_x: i32,
        offset_y: i32,
        shadow_radius: f64,
        shadow_color: (u8, u8, u8),
        bg: (u8, u8, u8),
    ) {
        if rect_w == 0 || rect_h == 0 || shadow_radius <= 0.0 || self.width == 0 || self.height == 0 {
            return;
        }

        // The intensity mask depends only on the rect size and the radius;
        // the offset and absolute position only shift where it is applied.
        let mask = shadow_mask(rect_w, rect_h, shadow_radius);
        let r = shadow_radius.ceil() as i32;
        let gw = rect_w as i32 + 2 * r; // grid columns (px ∈ [-r, rect_w + r))

        // Shadow region: the rect + offset, expanded by shadow_radius
        // (identical range to the original implementation).  The intensity
        // for a cell at canvas (x, y) is read from the mask at the cell's
        // position relative to the rect: px = x - rect_x, py = y - rect_y.
        let sx = (rect_x as i32 + offset_x - r).max(0) as usize;
        let sy = (rect_y as i32 + offset_y - r).max(0) as usize;
        let ex = (rect_x as i32 + offset_x + rect_w as i32 + r).min(self.width as i32 - 1) as usize;
        let ey = (rect_y as i32 + offset_y + rect_h as i32 + r).min(self.height as i32 - 1) as usize;

        for y in sy..=ey {
            for x in sx..=ex {
                let px = x as i32 - rect_x as i32;
                let py = y as i32 - rect_y as i32;

                // Cells inside the source rectangle are not shadowed.
                if px >= 0 && px < rect_w as i32 && py >= 0 && py < rect_h as i32 {
                    continue;
                }

                // Cells farther than the radius from the rect edges have
                // zero intensity; they are outside the mask grid entirely.
                let gi = (py + r) as isize;
                let gj = (px + r) as isize;
                if gi < 0 || gj < 0 || gi >= (rect_h as i32 + 2 * r) as isize || gj >= gw as isize {
                    continue;
                }

                let intensity = mask[gi as usize * gw as usize + gj as usize];
                if intensity > 0.0 {
                    let r = lerp(bg.0, shadow_color.0, intensity as f64);
                    let g = lerp(bg.1, shadow_color.1, intensity as f64);
                    let b = lerp(bg.2, shadow_color.2, intensity as f64);
                    self.set_pixel(x, y, r, g, b);
                }
            }
        }
    }

    // ─── SDF shapes ───────────────────────────────────────────────────

    /// Fill a rounded rectangle using a signed distance field.
    ///
    /// The SDF gives sub-cell anti-aliasing at the edges: cells near the
    /// boundary get a blended color based on their fractional distance.
    #[allow(clippy::too_many_arguments)]
    pub fn fill_rounded_rect(
        &mut self,
        x0: usize,
        y0: usize,
        w: usize,
        h: usize,
        radius: f64,
        fill: (u8, u8, u8),
    ) {
        let x1 = x0 + w;
        let y1 = y0 + h;
        for y in y0..y1.min(self.height) {
            for x in x0..x1.min(self.width) {
                let fx = x as f64;
                let fy = y as f64;
                let rx = x0 as f64 + radius;
                let ry = y0 as f64 + radius;
                let r1x = x1 as f64 - radius - 1.0;
                let r1y = y1 as f64 - radius - 1.0;

                // Distance from the rounded rect boundary.
                let cx = fx.max(rx).min(r1x);
                let cy = fy.max(ry).min(r1y);
                let dx = fx - cx;
                let dy = fy - cy;
                let dist = (dx * dx + dy * dy).sqrt() - radius;

                // Anti-alias: blend at the boundary.
                let alpha = 1.0 - smoothstep(-1.0, 0.0, dist);
                if alpha > 0.0 {
                    let bg = self.get_pixel(x, y);
                    let r = lerp(bg.0, fill.0, alpha);
                    let g = lerp(bg.1, fill.1, alpha);
                    let b = lerp(bg.2, fill.2, alpha);
                    self.set_pixel(x, y, r, g, b);
                }
            }
        }
    }

    /// Draw a rounded rectangle border (outline only).
    #[allow(clippy::too_many_arguments)]
    pub fn stroke_rounded_rect(
        &mut self,
        x0: usize,
        y0: usize,
        w: usize,
        h: usize,
        radius: f64,
        stroke: (u8, u8, u8),
        thickness: f64,
    ) {
        let x1 = x0 + w;
        let y1 = y0 + h;
        for y in y0..y1.min(self.height) {
            for x in x0..x1.min(self.width) {
                let fx = x as f64;
                let fy = y as f64;
                let rx = x0 as f64 + radius;
                let ry = y0 as f64 + radius;
                let r1x = x1 as f64 - radius - 1.0;
                let r1y = y1 as f64 - radius - 1.0;

                let cx = fx.max(rx).min(r1x);
                let cy = fy.max(ry).min(r1y);
                let dx = fx - cx;
                let dy = fy - cy;
                let dist = (dx * dx + dy * dy).sqrt() - radius;

                // Stroke: only the ring between -thickness and 0.
                let alpha = smoothstep(-thickness, 0.0, dist)
                    * (1.0 - smoothstep(0.0, 1.0, dist));
                if alpha > 0.0 {
                    let bg = self.get_pixel(x, y);
                    let r = lerp(bg.0, stroke.0, alpha);
                    let g = lerp(bg.1, stroke.1, alpha);
                    let b = lerp(bg.2, stroke.2, alpha);
                    self.set_pixel(x, y, r, g, b);
                }
            }
        }
    }

    // ─── Sparkline ────────────────────────────────────────────────────

    /// Draw a gradient sparkline (smooth colored bar graph).
    ///
    /// Each data point maps to a column.  The bar height is proportional to
    /// the value.  Color interpolates from `color_low` (bottom) to
    /// `color_high` (top).
    #[allow(clippy::too_many_arguments)]
    pub fn sparkline(
        &mut self,
        x0: usize,
        y0: usize,
        w: usize,
        h: usize,
        data: &[f64],
        color_low: (u8, u8, u8),
        color_high: (u8, u8, u8),
    ) {
        if data.is_empty() || w == 0 || h == 0 {
            return;
        }
        let max_val = data.iter().cloned().fold(f64::MIN, f64::max).max(1.0);
        let bar_w = (w as f64 / data.len() as f64).max(1.0);

        for (i, &val) in data.iter().enumerate() {
            let bar_h = ((val / max_val) * h as f64).round() as usize;
            let bx = x0 + (i as f64 * bar_w) as usize;
            let bw = bar_w.ceil() as usize;

            for dy in 0..bar_h.min(h) {
                let y = y0 + h - 1 - dy; // bottom-up
                let t = dy as f64 / h as f64;
                let r = lerp(color_low.0, color_high.0, t);
                let g = lerp(color_low.1, color_high.1, t);
                let b = lerp(color_low.2, color_high.2, t);
                for dx in 0..bw {
                    let x = bx + dx;
                    if x < x0 + w && x < self.width && y < self.height {
                        self.set_pixel(x, y, r, g, b);
                    }
                }
            }
        }
    }
}

/// The SDF coverage of a `w`×`h` rounded rectangle at cell `(x, y)`.
///
/// Mirrors the math inside [`PixelCanvas::fill_rounded_rect`]: `alpha` is 1
/// deep inside the rect, 0 at/beyond the rounded boundary, and smooth in
/// between, so the caller can blend a fill toward the underlying content to
/// fake rounded corners at cell resolution.
pub(crate) fn rounded_corner_alpha(x: usize, y: usize, w: usize, h: usize, radius: f64) -> f64 {
    let fx = x as f64;
    let fy = y as f64;
    let rx = radius;
    let ry = radius;
    let r1x = w as f64 - radius - 1.0;
    let r1y = h as f64 - radius - 1.0;
    let cx = fx.max(rx).min(r1x);
    let cy = fy.max(ry).min(r1y);
    let dx = fx - cx;
    let dy = fy - cy;
    let dist = (dx * dx + dy * dy).sqrt() - radius;
    1.0 - smoothstep(-1.0, 0.0, dist)
}

// ─── Helpers ────────────────────────────────────────────────────────────

/// Compute (or fetch from the cache) the shadow intensity mask for a
/// rectangle of `rect_w` × `rect_h` with the given Gaussian `radius`.
///
/// The returned grid is row-major over `py` then `px`, with `px`/`py`
/// measured relative to the rectangle origin, covering `px ∈ [-r, rect_w + r)`
/// and `py ∈ [-r, rect_h + r)` where `r = ceil(radius)`.  Cells farther than
/// `radius` from the rectangle edges store `0.0` (no shadow).
fn shadow_mask(rect_w: usize, rect_h: usize, radius: f64) -> ShadowMask {
    let key = (rect_w, rect_h, radius.to_bits());
    if let Some(mask) = SHADOW_MASK_CACHE.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return Arc::clone(mask);
    }

    let r = radius.ceil() as i32;
    let gw = rect_w as i32 + 2 * r;
    let gh = rect_h as i32 + 2 * r;
    let radius2 = radius * radius;
    let sigma2 = 2.0 * (radius / 3.0) * (radius / 3.0);

    let mut mask = vec![0f32; (gw * gh) as usize];
    for py in -r..(rect_h as i32 + r) {
        for px in -r..(rect_w as i32 + r) {
            let dx = if px < 0 {
                (-px) as f64
            } else if px >= rect_w as i32 {
                (px - rect_w as i32 + 1) as f64
            } else {
                0.0
            };
            let dy = if py < 0 {
                (-py) as f64
            } else if py >= rect_h as i32 {
                (py - rect_h as i32 + 1) as f64
            } else {
                0.0
            };
            let dist2 = dx * dx + dy * dy;
            if dist2 <= radius2 {
                let gi = (py + r) as usize;
                let gj = (px + r) as usize;
                mask[gi * gw as usize + gj] = (-dist2 / sigma2).exp() as f32;
            }
        }
    }

    let mask = Arc::new(mask);
    let mut cache = SHADOW_MASK_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if cache.len() >= SHADOW_MASK_CACHE_MAX {
        cache.clear();
    }
    cache.insert(key, Arc::clone(&mask));
    mask
}

/// Linear interpolation between two u8 values.
pub(crate) fn lerp(a: u8, b: u8, t: f64) -> u8 {
    let t = t.clamp(0.0, 1.0);
    let result = a as f64 * (1.0 - t) + b as f64 * t;
    result.round().min(255.0) as u8
}

/// Smooth hermite interpolation (0 at x<=edge0, 1 at x>=edge1).
fn smoothstep(edge0: f64, edge1: f64, x: f64) -> f64 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canvas_creation() {
        let c = PixelCanvas::new(10, 5);
        assert_eq!(c.width(), 10);
        assert_eq!(c.height(), 5);
        assert_eq!(c.rgb().len(), 10 * 5 * 3);
    }

    #[test]
    fn clear_fills_all_pixels() {
        let mut c = PixelCanvas::new(4, 3);
        c.clear(255, 128, 0);
        for y in 0..3 {
            for x in 0..4 {
                let (r, g, b) = c.get_pixel(x, y);
                assert_eq!((r, g, b), (255, 128, 0));
            }
        }
    }

    #[test]
    fn set_get_pixel_roundtrip() {
        let mut c = PixelCanvas::new(10, 10);
        c.set_pixel(3, 7, 100, 200, 50);
        assert_eq!(c.get_pixel(3, 7), (100, 200, 50));
        // Out of bounds returns (0,0,0).
        assert_eq!(c.get_pixel(100, 100), (0, 0, 0));
    }

    #[test]
    fn gradient_horizontal_fills_region() {
        let mut c = PixelCanvas::new(10, 3);
        c.gradient_horizontal(0, 0, 10, 3, (0, 0, 0), (255, 0, 0));
        // Leftmost pixel should be black.
        let (r, _, _) = c.get_pixel(0, 0);
        assert!(r < 10, "left edge should be near black, got r={r}");
        // Rightmost pixel should be red.
        let (r, g, b) = c.get_pixel(9, 1);
        assert!(r > 240, "right edge should be red, got r={r}");
        assert_eq!(g, 0);
        assert_eq!(b, 0);
    }

    #[test]
    fn single_pixel_gradient_is_well_defined() {
        let mut c = PixelCanvas::new(1, 1);
        c.gradient_horizontal(0, 0, 1, 1, (10, 20, 30), (200, 210, 220));
        assert_eq!(c.get_pixel(0, 0), (10, 20, 30));
    }

    #[test]
    fn zero_radius_radial_gradient_is_safe() {
        let mut c = PixelCanvas::new(4, 4);
        c.gradient_radial(2.0, 2.0, 0.0, (10, 20, 30), (200, 210, 220));
        assert_eq!(c.get_pixel(2, 2), (10, 20, 30));
    }

    #[test]
    fn gradient_vertical_fills_region() {
        let mut c = PixelCanvas::new(3, 10);
        c.gradient_vertical(0, 0, 3, 10, (0, 0, 255), (0, 0, 0));
        let (_, _, b_top) = c.get_pixel(1, 0);
        assert!(b_top > 240, "top should be blue, got b={b_top}");
        let (_, _, b_bot) = c.get_pixel(1, 9);
        assert!(b_bot < 10, "bottom should be black, got b={b_bot}");
    }

    #[test]
    fn rounded_corner_alpha_edges() {
        // Deep inside a rounded rect the cell is fully covered.
        assert!((rounded_corner_alpha(5, 5, 20, 10, 2.0) - 1.0).abs() < 0.001);
        // The corner cell fades out entirely (radius 1 → the corner is cut).
        let corner = rounded_corner_alpha(0, 0, 10, 10, 1.0);
        assert!(corner < 0.1, "corner cell should be cut, got {corner}");
        // The cell one in from the corner sits on the SDF boundary (cut).
        assert!(rounded_corner_alpha(1, 0, 10, 10, 1.0) < 0.1);
        // The next cell in is fully covered.
        assert!((rounded_corner_alpha(1, 1, 10, 10, 1.0) - 1.0).abs() < 0.001);
        // A larger radius leaves a partial-coverage blend zone at the corner.
        let blended = rounded_corner_alpha(0, 0, 10, 10, 2.0);
        assert!(
            (0.0..1.0).contains(&blended),
            "radius-2 corner should be a partial blend, got {blended}"
        );
    }

    #[test]
    fn rounded_rect_fills_center() {
        let mut c = PixelCanvas::new(20, 20);
        c.clear(0, 0, 0);
        c.fill_rounded_rect(2, 2, 16, 16, 3.0, (100, 100, 100));
        // Center should be filled.
        let (r, _, _) = c.get_pixel(10, 10);
        assert!(r > 90, "center should be filled, got r={r}");
        // Far corner should be empty.
        let (r, _, _) = c.get_pixel(0, 0);
        assert!(r < 10, "corner should be empty, got r={r}");
    }

    #[test]
    fn drop_shadow_darkens_outside_rect() {
        let mut c = PixelCanvas::new(20, 20);
        c.clear(200, 200, 200); // light gray background
        c.drop_shadow(5, 5, 6, 4, 1, 1, 3.0, (0, 0, 0), (200, 200, 200));
        // Inside the rect should be unchanged.
        let (r, g, b) = c.get_pixel(7, 6);
        assert_eq!((r, g, b), (200, 200, 200), "inside rect should be unchanged");
        // Just outside should be darker.
        let (r, _, _) = c.get_pixel(12, 6);
        assert!(r < 200, "shadow should darken, got r={r}");
    }

    /// Serializes tests that assert on the global `SHADOW_MASK_CACHE`
    /// contents, so parallel runs can't interleave inserts.
    static CACHE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn shadow_mask_is_cached_and_position_independent() {
        // The mask cache is global; clear it so the test measures its own work.
        let _guard = CACHE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        SHADOW_MASK_CACHE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();

        let key_a = (6usize, 4usize, 3.0f64.to_bits());
        let mask_a = shadow_mask(6, 4, 3.0);
        assert!(!mask_a.is_empty());

        // A second request for the same size returns the cached Arc (same ptr).
        let mask_b = shadow_mask(6, 4, 3.0);
        assert!(Arc::ptr_eq(&mask_a, &mask_b), "mask should be cached and shared");

        // A different size produces a distinct mask, and the cache holds both.
        let mask_c = shadow_mask(10, 2, 3.0);
        assert!(!Arc::ptr_eq(&mask_a, &mask_c));
        let cache = SHADOW_MASK_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(cache.len(), 2);
        assert!(cache.contains_key(&key_a));
        drop(cache);

        // Applying the same cached mask at different positions yields the
        // same relative shadow (identical pixel values at equal offsets).
        let mut c1 = PixelCanvas::new(30, 20);
        c1.clear(200, 200, 200);
        c1.drop_shadow(5, 5, 6, 4, 1, 1, 3.0, (0, 0, 0), (200, 200, 200));
        let mut c2 = PixelCanvas::new(30, 20);
        c2.clear(200, 200, 200);
        c2.drop_shadow(15, 8, 6, 4, 1, 1, 3.0, (0, 0, 0), (200, 200, 200));
        // Cell at rect_x+10, rect_y+1 sits right of both rects.
        assert_eq!(
            c1.get_pixel(5 + 10, 5 + 1),
            c2.get_pixel(15 + 10, 8 + 1),
            "shadow shape must not depend on position"
        );
    }

    #[test]
    fn shadow_mask_cache_is_bounded() {
        let _guard = CACHE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        SHADOW_MASK_CACHE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        // Exceed the cap; the cache must reset rather than grow unbounded.
        for w in 1..=(SHADOW_MASK_CACHE_MAX + 4) {
            shadow_mask(w, 3, 3.0);
        }
        let len = SHADOW_MASK_CACHE.lock().unwrap_or_else(|e| e.into_inner()).len();
        assert!(len <= SHADOW_MASK_CACHE_MAX, "cache must stay bounded, len={len}");
    }

    #[test]
    fn lerp_edges() {
        assert_eq!(lerp(0, 100, 0.0), 0);
        assert_eq!(lerp(0, 100, 1.0), 100);
        assert_eq!(lerp(0, 100, 0.5), 50);
    }

    #[test]
    fn smoothstep_edges() {
        assert!((smoothstep(0.0, 1.0, -1.0) - 0.0).abs() < 0.01);
        assert!((smoothstep(0.0, 1.0, 0.5) - 0.5).abs() < 0.01);
        assert!((smoothstep(0.0, 1.0, 2.0) - 1.0).abs() < 0.01);
    }

    #[test]
    fn fill_background_sets_accent_gradient_and_dock() {
        let mut c = PixelCanvas::new(10, 4);
        c.fill_background((0, 0, 0), (255, 0, 0), (0, 0, 255), "bottom");
        // Dock row (y = 3) is solid blue.
        assert_eq!(c.get_pixel(0, 3), (0, 0, 255));
        assert_eq!(c.get_pixel(9, 3), (0, 0, 255));
        // Accent row (y = 2) fades black → red left to right.
        let (r0, _, _) = c.get_pixel(0, 2);
        assert!(r0 < 30, "left edge near black, got {r0}");
        let (r9, _, _) = c.get_pixel(9, 2);
        assert!(r9 > 240, "right edge near red, got {r9}");
        // Content rows (y < 2) are solid background.
        assert_eq!(c.get_pixel(4, 0), (0, 0, 0));
    }

    #[test]
    fn fill_background_updates_with_different_key() {
        let mut c = PixelCanvas::new(10, 4);
        c.fill_background((0, 0, 0), (255, 0, 0), (0, 0, 255), "bottom");
        c.fill_background((0, 0, 0), (255, 0, 0), (0, 255, 0), "bottom");
        // The dock row reflects the new key rather than the cached one.
        assert_eq!(c.get_pixel(5, 3), (0, 255, 0));
    }

    #[test]
    fn fill_background_single_row_is_safe() {
        // A 1-row canvas has no accent row; only the dock row is painted.
        let mut c = PixelCanvas::new(4, 1);
        c.fill_background((10, 20, 30), (1, 2, 3), (200, 100, 50), "bottom");
        assert_eq!(c.get_pixel(0, 0), (200, 100, 50));
        assert_eq!(c.get_pixel(3, 0), (200, 100, 50));
    }
}
