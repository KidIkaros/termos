//! Sixel command parsing — ported from Go TUIOS `internal/vt/sixel_parser.go`.
//!
//! Parses DCS sixel sequences and calculates image dimensions from the
//! sixel raster data.

/// A parsed Sixel graphics command.
/// DCS format: `ESC P <p1>;<p2>;<p3> q <sixel-data> ST`
#[derive(Debug, Clone)]
pub struct SixelCommand {
    /// Pixel aspect ratio (0,1=2:1, 2=5:1, 3,4=3:1, 5,6=2:1, 7,8,9=1:1).
    pub aspect_ratio: i32,
    /// Background mode (0=default, 1=transparent, 2=color 0).
    pub background_mode: i32,
    /// Horizontal grid (deprecated, ignored).
    pub horizontal_grid: i32,
    /// Calculated width in pixels.
    pub width: i32,
    /// Calculated height in pixels.
    pub height: i32,
    /// Raw sixel raster data (after the 'q' introducer).
    pub data: Vec<u8>,
    /// Complete DCS sequence for passthrough.
    pub raw_sequence: Vec<u8>,
}

/// Parse a DCS sixel sequence. The `data` parameter should contain everything
/// after the DCS introducer, including parameters, the 'q' introducer, and
/// sixel data.
pub fn parse_sixel_command(data: &[u8]) -> Option<SixelCommand> {
    if data.is_empty() {
        return None;
    }

    let mut cmd = SixelCommand {
        aspect_ratio: 2,
        background_mode: 0,
        horizontal_grid: 0,
        width: 0,
        height: 0,
        data: Vec::new(),
        raw_sequence: data.to_vec(),
    };

    // Find the 'q' introducer.
    let q_idx = data.iter().position(|&b| b == b'q')?;

    // Parse parameters before 'q' (if any).
    if q_idx > 0 {
        let param_str = &data[..q_idx];
        let params: Vec<&[u8]> = param_str.split(|&b| b == b';').collect();
        for (i, p) in params.iter().enumerate() {
            let trimmed = p.iter().filter(|b| !b.is_ascii_whitespace()).copied().collect::<Vec<_>>();
            if let Ok(val) = std::str::from_utf8(&trimmed) {
                if let Ok(n) = val.parse::<i32>() {
                    match i {
                        0 => cmd.aspect_ratio = n,
                        1 => cmd.background_mode = n,
                        2 => cmd.horizontal_grid = n,
                        _ => {}
                    }
                }
            }
        }
    }

    // Extract sixel data (everything after 'q').
    if q_idx + 1 < data.len() {
        cmd.data = data[q_idx + 1..].to_vec();
    }

    // Calculate dimensions from sixel data.
    let (w, h) = calculate_sixel_dimensions(&cmd.data);
    cmd.width = w;
    cmd.height = h;

    Some(cmd)
}

/// Parse sixel data to determine image dimensions (width, height in pixels).
fn calculate_sixel_dimensions(data: &[u8]) -> (i32, i32) {
    if data.is_empty() {
        return (0, 0);
    }

    let mut x: i32 = 0;
    let mut max_x: i32 = 0;
    let mut sixel_lines: i32 = 1;

    let mut i = 0;
    while i < data.len() {
        let c = data[i];
        match c {
            b'#' => {
                // Color introducer: skip until non-parameter character.
                i += 1;
                while i < data.len() && (data[i].is_ascii_digit() || data[i] == b';') {
                    i += 1;
                }
            }
            b'$' => {
                // Carriage return.
                if x > max_x {
                    max_x = x;
                }
                x = 0;
                i += 1;
            }
            b'-' => {
                // New sixel line (move down 6 pixels).
                if x > max_x {
                    max_x = x;
                }
                x = 0;
                sixel_lines += 1;
                i += 1;
            }
            b'!' => {
                // Repeat introducer: !<count><char>.
                i += 1;
                let count_start = i;
                while i < data.len() && data[i].is_ascii_digit() {
                    i += 1;
                }
                let count = if i > count_start {
                    std::str::from_utf8(&data[count_start..i])
                        .ok()
                        .and_then(|s| s.parse::<i32>().ok())
                        .unwrap_or(1)
                } else {
                    1
                };
                if i < data.len() && data[i] >= b'?' && data[i] <= b'~' {
                    x += count;
                    i += 1;
                }
            }
            b'"' => {
                // Raster attributes: "Pan;Pad;Ph;Pv
                i += 1;
                let mut params: Vec<i32> = Vec::with_capacity(4);
                while i < data.len() && params.len() < 4 {
                    let num_start = i;
                    while i < data.len() && data[i].is_ascii_digit() {
                        i += 1;
                    }
                    if i > num_start {
                        if let Ok(s) = std::str::from_utf8(&data[num_start..i]) {
                            if let Ok(n) = s.parse::<i32>() {
                                params.push(n);
                            }
                        }
                    }
                    if i < data.len() && data[i] == b';' {
                        i += 1;
                    } else {
                        break;
                    }
                }
                // If Ph and Pv are specified, use them directly.
                if params.len() >= 4 && params[2] > 0 && params[3] > 0 {
                    return (params[2], params[3]);
                }
            }
            c if (b'?'..=b'~').contains(&c) => {
                // Sixel data character (6 vertical pixels).
                x += 1;
                i += 1;
            }
            _ => {
                // Unknown character, skip.
                i += 1;
            }
        }
    }

    if x > max_x {
        max_x = x;
    }

    let height = sixel_lines * 6;
    (max_x, height)
}

impl SixelCommand {
    /// Number of terminal rows needed for this image.
    pub fn rows_for_height(&self, cell_height: i32) -> i32 {
        if cell_height <= 0 || self.height <= 0 {
            return 0;
        }
        (self.height + cell_height - 1) / cell_height
    }

    /// Number of terminal columns needed for this image.
    pub fn cols_for_width(&self, cell_width: i32) -> i32 {
        if cell_width <= 0 || self.width <= 0 {
            return 0;
        }
        (self.width + cell_width - 1) / cell_width
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sixel_basic() {
        // Minimal sixel: Pq 1:1 ratio, then one sixel char (~ = all 6 bits).
        let data = b"1;0;0q~";
        let cmd = parse_sixel_command(data).unwrap();
        assert_eq!(cmd.aspect_ratio, 1);
        assert_eq!(cmd.width, 1);
        assert_eq!(cmd.height, 6);
    }

    #[test]
    fn parse_sixel_with_newline() {
        // Two sixel lines: ~-~
        let data = b"q~-~";
        let cmd = parse_sixel_command(data).unwrap();
        assert_eq!(cmd.width, 1);
        assert_eq!(cmd.height, 12);
    }

    #[test]
    fn parse_sixel_with_repeat() {
        // Repeat: !5~ = 5 sixel chars
        let data = b"q!5~";
        let cmd = parse_sixel_command(data).unwrap();
        assert_eq!(cmd.width, 5);
        assert_eq!(cmd.height, 6);
    }

    #[test]
    fn parse_sixel_raster_attributes() {
        // Raster attributes specify Ph=100, Pv=200
        let data = b"q\"1;1;100;200~";
        let cmd = parse_sixel_command(data).unwrap();
        assert_eq!(cmd.width, 100);
        assert_eq!(cmd.height, 200);
    }

    #[test]
    fn rows_and_cols_calculation() {
        let cmd = SixelCommand {
            aspect_ratio: 1,
            background_mode: 0,
            horizontal_grid: 0,
            width: 100,
            height: 48,
            data: vec![],
            raw_sequence: vec![],
        };
        assert_eq!(cmd.rows_for_height(16), 3); // ceil(48/16) = 3
        assert_eq!(cmd.cols_for_width(10), 10); // ceil(100/10) = 10
    }
}
