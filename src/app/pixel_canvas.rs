//! Pixel canvas — a low-resolution framebuffer for GUI-like visual effects.
//!
//! Uses `asciline`'s pixel-mode mapper to render gradient backgrounds, soft
//! shadows, anti-aliased shapes, and gradient sparklines.  Each terminal cell
//! becomes a 24-bit RGB pixel via `\x1b[48;2;R;G;Bm `.
//!
//! Architecture: dual-layer rendering
//! - Layer 1 (this module): gradient/shadow/shape backgrounds
//! - Layer 2 (ratatui): text, borders, widgets with transparent backgrounds

use asciline::mapper::Mapper;

/// A pixel canvas backed by a BGR framebuffer (3 bytes per cell).
///
/// The canvas is sized to the terminal area and rendered as colored cells
/// each frame.  ratatui paints text on top with transparent backgrounds
/// showing the canvas through.
pub struct PixelCanvas {
    /// BGR framebuffer: `[B, G, R]` per cell, `width * height * 3` bytes.
    bgr: Vec<u8>,
    /// RGB framebuffer for gradient computation (fed to the mapper).
    rgb: Vec<u8>,
    width: usize,
    height: usize,
    mapper: Mapper,
}

impl PixelCanvas {
    /// Create a new canvas for the given terminal dimensions.
    pub fn new(width: usize, height: usize) -> Self {
        let pixels = width * height;
        Self {
            bgr: vec![0u8; pixels * 3],
            rgb: vec![0u8; pixels * 3],
            width,
            height,
            mapper: Mapper::new(&[' '], 0),
        }
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

    /// Flush the RGB framebuffer through the mapper to produce the BGR output.
    pub fn flush(&mut self) {
        self.mapper
            .map_pixel(&self.rgb, self.width, self.height, &mut self.bgr);
    }

    /// Get the BGR framebuffer for rendering.
    pub fn bgr(&self) -> &[u8] {
        &self.bgr
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
        if w == 0 {
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
                let t = dx as f64 / (w - 1) as f64;
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
        // Shadow region: the rect + offset, expanded by shadow_radius.
        let sx = (rect_x as i32 + offset_x - shadow_radius as i32).max(0) as usize;
        let sy = (rect_y as i32 + offset_y - shadow_radius as i32).max(0) as usize;
        let ex = (rect_x as i32 + offset_x + rect_w as i32 + shadow_radius as i32)
            .min(self.width as i32 - 1) as usize;
        let ey = (rect_y as i32 + offset_y + rect_h as i32 + shadow_radius as i32)
            .min(self.height as i32 - 1) as usize;

        // The shadow is only cast below and to the right of the rectangle.
        // Cells inside the rectangle itself are not shadowed.
        let rect_ex = rect_x + rect_w;
        let rect_ey = rect_y + rect_h;

        for y in sy..=ey {
            for x in sx..=ex {
                // Skip cells that are inside the source rectangle.
                if x >= rect_x && x < rect_ex && y >= rect_y && y < rect_ey {
                    continue;
                }

                // Distance from the nearest edge of the shadow-casting rect.
                let dx = if x < rect_x {
                    (rect_x - x) as f64
                } else if x >= rect_ex {
                    (x - rect_ex + 1) as f64
                } else {
                    0.0
                };
                let dy = if y < rect_y {
                    (rect_y - y) as f64
                } else if y >= rect_ey {
                    (y - rect_ey + 1) as f64
                } else {
                    0.0
                };
                let dist = (dx * dx + dy * dy).sqrt();

                if dist > shadow_radius {
                    continue;
                }

                // Gaussian falloff: exp(-dist² / (2 * σ²))
                let sigma = shadow_radius / 3.0; // 99.7% within radius
                let intensity = (-dist * dist / (2.0 * sigma * sigma)).exp();

                let r = lerp(bg.0, shadow_color.0, intensity);
                let g = lerp(bg.1, shadow_color.1, intensity);
                let b = lerp(bg.2, shadow_color.2, intensity);
                self.set_pixel(x, y, r, g, b);
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

// ─── Helpers ────────────────────────────────────────────────────────────

/// Linear interpolation between two u8 values.
fn lerp(a: u8, b: u8, t: f64) -> u8 {
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
        assert_eq!(c.bgr().len(), 10 * 5 * 3);
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
    fn gradient_vertical_fills_region() {
        let mut c = PixelCanvas::new(3, 10);
        c.gradient_vertical(0, 0, 3, 10, (0, 0, 255), (0, 0, 0));
        let (_, _, b_top) = c.get_pixel(1, 0);
        assert!(b_top > 240, "top should be blue, got b={b_top}");
        let (_, _, b_bot) = c.get_pixel(1, 9);
        assert!(b_bot < 10, "bottom should be black, got b={b_bot}");
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
}
