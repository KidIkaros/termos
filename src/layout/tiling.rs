//! Master-stack tiling layout — a port of TUIOS `internal/layout/tiling.go`.
//!
//! One master pane holds the left (or top) region at `master_ratio`; the rest
//! stack beside it. The split axis follows the on-screen shape of the area.

/// How many times taller a character cell is than it is wide.
const CELL_ASPECT: i32 = 2;

/// The position and size for a tiled window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TileLayout {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Return optimal positions for `n` windows. `master_ratio` controls the width
/// ratio of the master (left) pane (0.3-0.7). `gap` is the cells reserved
/// between neighbours for the drawn divider.
pub fn calculate_tiling_layout(
    n: i32,
    screen_width: i32,
    usable_height: i32,
    top_margin: i32,
    master_ratio: f64,
    gap: i32,
) -> Vec<TileLayout> {
    if n == 0 {
        return Vec::new();
    }

    let mut layouts = Vec::with_capacity(n as usize);

    // Clamp master ratio to reasonable bounds (30%-70%).
    let master_ratio = if master_ratio < 0.3 {
        0.3
    } else if master_ratio > 0.7 {
        0.7
    } else {
        master_ratio
    };

    match n {
        1 => {
            layouts.push(TileLayout {
                x: 0,
                y: top_margin,
                width: screen_width,
                height: usable_height,
            });
        }
        2 => {
            // Split along whichever axis the screen is longer on as it is drawn.
            if screen_width >= usable_height * CELL_ASPECT {
                let master_width = (screen_width as f64 * master_ratio) as i32;
                layouts.push(TileLayout {
                    x: 0,
                    y: top_margin,
                    width: master_width,
                    height: usable_height,
                });
                layouts.push(TileLayout {
                    x: master_width + gap,
                    y: top_margin,
                    width: screen_width - master_width - gap,
                    height: usable_height,
                });
            } else {
                let master_height = (usable_height as f64 * master_ratio) as i32;
                layouts.push(TileLayout {
                    x: 0,
                    y: top_margin,
                    width: screen_width,
                    height: master_height,
                });
                layouts.push(TileLayout {
                    x: 0,
                    y: top_margin + master_height + gap,
                    width: screen_width,
                    height: usable_height - master_height - gap,
                });
            }
        }
        _ => {
            // Master-stack with n-1 stacked.
            let stack_height = usable_height;
            let master_width = (screen_width as f64 * master_ratio) as i32;
            let stack_width = screen_width - master_width - gap;

            // Master on the left.
            layouts.push(TileLayout {
                x: 0,
                y: top_margin,
                width: master_width,
                height: stack_height,
            });

            // Stack the rest vertically on the right.
            let stack_count = n - 1;
            let each_height = (stack_height - (stack_count - 1) * gap) / stack_count;
            for i in 0..stack_count {
                layouts.push(TileLayout {
                    x: master_width + gap,
                    y: top_margin + i * (each_height + gap),
                    width: stack_width,
                    height: each_height,
                });
            }
        }
    }

    layouts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_window_fills_screen() {
        let layouts = calculate_tiling_layout(1, 80, 24, 0, 0.5, 1);
        assert_eq!(layouts.len(), 1);
        assert_eq!(layouts[0].width, 80);
        assert_eq!(layouts[0].height, 24);
    }

    #[test]
    fn two_windows_split_side_by_side_on_wide_screen() {
        // 120 cols vs 24 rows*2=48 → wide, split vertically.
        let layouts = calculate_tiling_layout(2, 120, 24, 0, 0.5, 1);
        assert_eq!(layouts.len(), 2);
        assert_eq!(layouts[0].width, 60);
        assert_eq!(layouts[1].x, 61);
        assert_eq!(layouts[0].width + layouts[1].width + 1, 120);
    }

    #[test]
    fn two_windows_stack_on_tall_screen() {
        // 51 cols vs 37 rows*2=74 → tall, stack horizontally.
        let layouts = calculate_tiling_layout(2, 51, 37, 0, 0.5, 1);
        assert_eq!(layouts.len(), 2);
        assert_eq!(layouts[0].height, 18);
        assert_eq!(layouts[1].y, 19);
        assert_eq!(layouts[0].height + layouts[1].height + 1, 37);
    }

    #[test]
    fn master_ratio_clamped() {
        let layouts = calculate_tiling_layout(2, 120, 24, 0, 0.9, 1);
        // Clamped to 0.7.
        assert_eq!(layouts[0].width, 84);
    }
}
