//! Half-block mapper: converts RGB24 frames to terminal cells using
//! `▀` (U+2580) for 2× vertical resolution.
//!
//! Each cell encodes TWO vertical pixels:
//! - The foreground color = top pixel
//! - The background color = bottom pixel
//! - The character is always `▀` (upper half block)
//!
//! This means a frame of W×H pixels renders in W×(H/2) terminal cells.

/// A single half-block cell ready for ratatui rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HalfBlockCell {
    /// The character: always `▀`.
    pub ch: char,
    /// Top pixel color (RGB).
    pub fg: (u8, u8, u8),
    /// Bottom pixel color (RGB).
    pub bg: (u8, u8, u8),
}

/// Maps RGB24 frame buffers to half-block cell grids.
pub struct HalfBlockMapper {
    /// Quantization bits per channel (default 6 = 262144 colors).
    quantize_bits: u8,
}

impl HalfBlockMapper {
    /// Create a mapper with default quantization (6 bits).
    pub fn new() -> Self {
        Self { quantize_bits: 6 }
    }

    /// Create a mapper with custom quantization.
    pub fn with_quantize(quantize_bits: u8) -> Self {
        Self { quantize_bits: quantize_bits.min(8) }
    }

    /// Map an RGB24 frame to half-block cells.
    ///
    /// `rgb` is a flat `[R, G, B, R, G, B, ...]` buffer of width × height pixels.
    /// Returns a Vec of `width × (height / 2)` cells, row-major.
    /// If height is odd, the last row is padded with black.
    pub fn map_frame(&self, rgb: &[u8], width: usize, height: usize) -> Vec<HalfBlockCell> {
        let half_h = height / 2;
        let mut cells = Vec::with_capacity(width * half_h);

        for row in 0..half_h {
            for col in 0..width {
                let top_idx = (row * 2) * width * 3 + col * 3;
                let bot_idx = ((row * 2) + 1) * width * 3 + col * 3;

                let fg = Self::read_pixel(rgb, top_idx);
                let bg = if (row * 2 + 1) < height {
                    Self::read_pixel(rgb, bot_idx)
                } else {
                    (0, 0, 0)
                };

                cells.push(HalfBlockCell {
                    ch: '\u{2580}',
                    fg: self.quantize(fg),
                    bg: self.quantize(bg),
                });
            }
        }

        cells
    }

    /// Read a pixel from the RGB24 buffer, returning (r, g, b).
    /// Returns black if the index is out of bounds.
    fn read_pixel(rgb: &[u8], idx: usize) -> (u8, u8, u8) {
        if idx + 2 < rgb.len() {
            (rgb[idx], rgb[idx + 1], rgb[idx + 2])
        } else {
            (0, 0, 0)
        }
    }

    /// Quantize an RGB triple to reduce color precision (for fewer distinct
    /// cell styles, which reduces ratatui buffer writes).
    fn quantize(&self, (r, g, b): (u8, u8, u8)) -> (u8, u8, u8) {
        if self.quantize_bits >= 8 {
            return (r, g, b);
        }
        let shift = 8 - self.quantize_bits;
        let mask = !((1u8 << shift) - 1);
        (r & mask, g & mask, b & mask)
    }

    /// Convert half-block cells to a flat RGB buffer (useful for caching).
    pub fn cells_to_rgb(cells: &[HalfBlockCell]) -> Vec<u8> {
        let mut rgb = Vec::with_capacity(cells.len() * 6);
        for cell in cells {
            rgb.extend_from_slice(&[cell.fg.0, cell.fg.1, cell.fg.2]);
            rgb.extend_from_slice(&[cell.bg.0, cell.bg.1, cell.bg.2]);
        }
        rgb
    }
}

impl Default for HalfBlockMapper {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a simple test frame: red top half, blue bottom half.
    fn test_frame(w: usize, h: usize) -> Vec<u8> {
        let mut rgb = vec![0u8; w * h * 3];
        for row in 0..h {
            for col in 0..w {
                let idx = (row * w + col) * 3;
                if row < h / 2 {
                    // Red
                    rgb[idx] = 255;
                    rgb[idx + 1] = 0;
                    rgb[idx + 2] = 0;
                } else {
                    // Blue
                    rgb[idx] = 0;
                    rgb[idx + 1] = 0;
                    rgb[idx + 2] = 255;
                }
            }
        }
        rgb
    }

    #[test]
    fn halfblock_basic_mapping() {
        let mapper = HalfBlockMapper::with_quantize(8); // no quantization
        let rgb = test_frame(4, 4);
        let cells = mapper.map_frame(&rgb, 4, 4);
        // 4 wide × 2 high = 8 cells
        assert_eq!(cells.len(), 8);

        // Top-left cell: top=red, bottom=red
        assert_eq!(cells[0].ch, '\u{2580}');
        assert_eq!(cells[0].fg, (255, 0, 0));
        assert_eq!(cells[0].bg, (255, 0, 0));

        // Bottom-left cell: top=blue, bottom=blue
        assert_eq!(cells[4].fg, (0, 0, 255));
        assert_eq!(cells[4].bg, (0, 0, 255));
    }

    #[test]
    fn halfblock_odd_height() {
        let mapper = HalfBlockMapper::with_quantize(8);
        let rgb = test_frame(2, 3);
        let cells = mapper.map_frame(&rgb, 2, 3);
        // 2 wide × 1 high (3/2=1) = 2 cells
        assert_eq!(cells.len(), 2);
        // h=3: row 0 is red, rows 1-2 are blue
        // Cell: top=red (row 0), bottom=blue (row 1)
        assert_eq!(cells[0].fg, (255, 0, 0));
        assert_eq!(cells[0].bg, (0, 0, 255));
    }

    #[test]
    fn halfblock_quantize() {
        let mapper = HalfBlockMapper::with_quantize(4);
        let rgb = test_frame(1, 2);
        let cells = mapper.map_frame(&rgb, 1, 2);
        // 255 quantized to 4 bits: 255 & 0xF0 = 240
        assert_eq!(cells[0].fg, (240, 0, 0));
    }

    #[test]
    fn halfblock_empty_frame() {
        let mapper = HalfBlockMapper::new();
        let cells = mapper.map_frame(&[], 0, 0);
        assert!(cells.is_empty());
    }

    #[test]
    fn halfblock_single_pixel_pair() {
        let mapper = HalfBlockMapper::with_quantize(8);
        let rgb = vec![255, 128, 0, 0, 128, 255];
        let cells = mapper.map_frame(&rgb, 1, 2);
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].fg, (255, 128, 0));
        assert_eq!(cells[0].bg, (0, 128, 255));
    }
}
