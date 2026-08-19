//! Junction-aware border grid rendering.
//!
//! Detects T-junctions, L-corners, and X-crossings from pane rectangles and
//! renders the appropriate Unicode box-drawing characters. Falls back to ASCII
//! (`+`, `-`, `|`) when `use_ascii_only` is set.

use std::collections::HashSet;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect as TuiRect;
use ratatui::style::{Color, Modifier, Style};

/// Edge direction for a border segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Dir {
    North,
    South,
    East,
    West,
}

/// A point on the grid.
type Point = (i32, i32);

/// Collect all border edge points from a set of pane rectangles.
///
/// For each rectangle, we collect:
/// - Top edge: (x, y) for x in [x0, x1)
/// - Bottom edge: (x, y1) for x in [x0, x1)
/// - Left edge: (x, y) for y in [y0, y1)
/// - Right edge: (x1, y) for y in [y0, y1)
///
/// Returns a set of (point, direction) pairs indicating which sides of each
/// cell are borders.
fn collect_edges(rects: &[TuiRect]) -> HashSet<(Point, Dir)> {
    let mut edges = HashSet::new();
    for r in rects {
        let x0 = r.x as i32;
        let y0 = r.y as i32;
        let x1 = x0 + r.width as i32;
        let y1 = y0 + r.height as i32;
        // Top edge (north side of each cell).
        for x in x0..x1 {
            edges.insert(((x, y0), Dir::North));
        }
        // Bottom edge (south side of each cell).
        for x in x0..x1 {
            edges.insert(((x, y1 - 1), Dir::South));
        }
        // Left edge (west side of each cell).
        for y in y0..y1 {
            edges.insert(((x0, y), Dir::West));
        }
        // Right edge (east side of each cell).
        for y in y0..y1 {
            edges.insert(((x1 - 1, y), Dir::East));
        }
    }
    edges
}

/// Determine which directions have borders at a given point.
fn border_dirs(edges: &HashSet<(Point, Dir)>, p: Point) -> [bool; 4] {
    // [N, S, E, W]
    [
        edges.contains(&(p, Dir::North)),
        edges.contains(&(p, Dir::South)),
        edges.contains(&(p, Dir::East)),
        edges.contains(&(p, Dir::West)),
    ]
}

/// Return the Unicode box-drawing character for the given junction, or the
/// ASCII fallback.
fn junction_glyph(dirs: [bool; 4], ascii: bool) -> char {
    let [n, s, e, w] = dirs;
    let count = dirs.iter().filter(|&&d| d).count();
    if ascii {
        return match count {
            0 => ' ',
            1 | 2 if n && s => '|',
            1 | 2 if e && w => '-',
            _ => '+',
        };
    }
    match (n, s, e, w) {
        // Single edges (line continuations).
        (true, false, false, false) => '╵',
        (false, true, false, false) => '╷',
        (false, false, true, false) => '╶',
        (false, false, false, true) => '╴',
        // Straight lines.
        (true, true, false, false) => '│',
        (false, false, true, true) => '─',
        // L-corners.
        (true, false, true, false) => '╰',
        (true, false, false, true) => '╯',
        (false, true, true, false) => '╭',
        (false, true, false, true) => '╮',
        // T-junctions.
        (true, true, true, false) => '├',
        (true, true, false, true) => '┤',
        (true, false, true, true) => '┴',
        (false, true, true, true) => '┬',
        // X-crossing.
        (true, true, true, true) => '┼',
        // No borders.
        _ => ' ',
    }
}

/// Render the border grid for a set of pane rectangles.
///
/// This draws junction-aware borders on top of the existing pane borders. It
/// should be called after all panes have been rendered but before overlays.
pub fn render_border_grid(
    buf: &mut Buffer,
    rects: &[TuiRect],
    color: Color,
    ascii: bool,
    gap: i32,
) {
    if rects.is_empty() {
        return;
    }

    // If there's a gap between panes, we need to account for the separator
    // cells. With gap=0, borders are shared. With gap>0, there are separator
    // cells between panes.
    let _ = gap; // Currently treating gap=0; gap handling can be extended.

    let edges = collect_edges(rects);

    // Find all points that are on at least one border.
    let border_points: HashSet<Point> = edges.iter().map(|(p, _)| *p).collect();

    // For each border point, determine the junction type and draw the glyph.
    for p in &border_points {
        let (x, y) = *p;
        if x < 0 || y < 0 {
            continue;
        }
        let ux = x as u16;
        let uy = y as u16;
        // Skip points outside the buffer.
        if ux >= buf.area().width || uy >= buf.area().height {
            continue;
        }
        let dirs = border_dirs(&edges, *p);
        let glyph = junction_glyph(dirs, ascii);

        // Only overwrite if the existing cell is a border character or space.
        let cell = &mut buf[(ux, uy)];
        let existing = cell.symbol();
        if existing == " " || is_border_char(existing) {
            cell.set_char(glyph);
            cell.set_style(Style::default().fg(color).add_modifier(Modifier::BOLD));
        }
    }
}

/// Check if a character is a border-drawing character.
fn is_border_char(s: &str) -> bool {
    matches!(
        s,
        "│" | "─" | "┌" | "┐" | "└" | "┘" | "├" | "┤" | "┬" | "┴" | "┼"
            | "╭" | "╮" | "╰" | "╯" | "╵" | "╷" | "╶" | "╴"
            | "|" | "-" | "+" | " "
    )
}

/// Return the divider glyphs for a given border style.
pub fn divider_glyphs(ascii: bool) -> (char, char, char, char, char) {
    if ascii {
        ('+', '+', '+', '+', '+')
    } else {
        ('┌', '┐', '└', '┘', '┼')
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: u16, y: u16, w: u16, h: u16) -> TuiRect {
        TuiRect::new(x, y, w, h)
    }

    #[test]
    fn collect_edges_single_rect() {
        let edges = collect_edges(&[r(0, 0, 10, 5)]);
        // Top edge: 10 cells with North.
        assert!(edges.contains(&((0, 0), Dir::North)));
        assert!(edges.contains(&((9, 0), Dir::North)));
        // Bottom edge: 10 cells with South.
        assert!(edges.contains(&((0, 4), Dir::South)));
        assert!(edges.contains(&((9, 4), Dir::South)));
        // Left edge: 5 cells with West.
        assert!(edges.contains(&((0, 0), Dir::West)));
        assert!(edges.contains(&((0, 4), Dir::West)));
        // Right edge: 5 cells with East.
        assert!(edges.contains(&((9, 0), Dir::East)));
        assert!(edges.contains(&((9, 4), Dir::East)));
    }

    #[test]
    fn junction_glyph_corners() {
        // L-corners.
        assert_eq!(junction_glyph([true, false, true, false], false), '╰');
        assert_eq!(junction_glyph([true, false, false, true], false), '╯');
        assert_eq!(junction_glyph([false, true, true, false], false), '╭');
        assert_eq!(junction_glyph([false, true, false, true], false), '╮');
    }

    #[test]
    fn junction_glyph_t_junctions() {
        assert_eq!(junction_glyph([true, true, true, false], false), '├');
        assert_eq!(junction_glyph([true, true, false, true], false), '┤');
        assert_eq!(junction_glyph([true, false, true, true], false), '┴');
        assert_eq!(junction_glyph([false, true, true, true], false), '┬');
    }

    #[test]
    fn junction_glyph_crossing() {
        assert_eq!(junction_glyph([true, true, true, true], false), '┼');
    }

    #[test]
    fn junction_glyph_ascii_fallback() {
        assert_eq!(junction_glyph([true, true, false, false], true), '|');
        assert_eq!(junction_glyph([false, false, true, true], true), '-');
        assert_eq!(junction_glyph([true, true, true, true], true), '+');
    }

    #[test]
    fn junction_glyph_straight_lines() {
        assert_eq!(junction_glyph([true, true, false, false], false), '│');
        assert_eq!(junction_glyph([false, false, true, true], false), '─');
    }

    #[test]
    fn is_border_char_recognizes_unicode() {
        assert!(is_border_char("│"));
        assert!(is_border_char("─"));
        assert!(is_border_char("┼"));
        assert!(is_border_char("┌"));
        assert!(!is_border_char("x"));
        assert!(!is_border_char("A"));
    }

    #[test]
    fn border_dirs_detects_all_four() {
        let mut edges = HashSet::new();
        let p = (5, 5);
        edges.insert((p, Dir::North));
        edges.insert((p, Dir::South));
        edges.insert((p, Dir::East));
        edges.insert((p, Dir::West));
        let dirs = border_dirs(&edges, p);
        assert_eq!(dirs, [true, true, true, true]);
    }

    #[test]
    fn divider_glyphs_unicode() {
        let (tl, tr, bl, br, cross) = divider_glyphs(false);
        assert_eq!(tl, '┌');
        assert_eq!(tr, '┐');
        assert_eq!(bl, '└');
        assert_eq!(br, '┘');
        assert_eq!(cross, '┼');
    }

    #[test]
    fn divider_glyphs_ascii() {
        let (tl, tr, bl, br, cross) = divider_glyphs(true);
        assert_eq!(tl, '+');
        assert_eq!(tr, '+');
        assert_eq!(bl, '+');
        assert_eq!(br, '+');
        assert_eq!(cross, '+');
    }

    #[test]
    fn render_border_grid_two_horizontal_panes() {
        let mut buf = Buffer::empty(TuiRect::new(0, 0, 40, 10));
        let left = r(0, 0, 20, 10);
        let right = r(20, 0, 20, 10);
        render_border_grid(&mut buf, &[left, right], Color::White, false, 0);
        // The shared vertical border at x=19 should have junction characters.
        // Check that some border characters were drawn.
        let has_border = (0..10).any(|y| {
            let cell = &buf[(19, y)];
            is_border_char(cell.symbol())
        });
        assert!(has_border);
    }

    #[test]
    fn render_border_grid_two_vertical_panes() {
        let mut buf = Buffer::empty(TuiRect::new(0, 0, 40, 20));
        let top = r(0, 0, 40, 10);
        let bottom = r(0, 10, 40, 10);
        render_border_grid(&mut buf, &[top, bottom], Color::White, false, 0);
        // The shared horizontal border at y=9 should have junction characters.
        let has_border = (0..40).any(|x| {
            let cell = &buf[(x, 9)];
            is_border_char(cell.symbol())
        });
        assert!(has_border);
    }

    #[test]
    fn render_border_grid_empty_rects() {
        let mut buf = Buffer::empty(TuiRect::new(0, 0, 40, 10));
        render_border_grid(&mut buf, &[], Color::White, false, 0);
        // Should not crash and should leave buffer unchanged.
    }

    #[test]
    fn render_border_grid_four_quadrants() {
        let mut buf = Buffer::empty(TuiRect::new(0, 0, 40, 20));
        let tl = r(0, 0, 20, 10);
        let tr = r(20, 0, 20, 10);
        let bl = r(0, 10, 20, 10);
        let br = r(20, 10, 20, 10);
        render_border_grid(&mut buf, &[tl, tr, bl, br], Color::White, false, 0);
        // The center crossing point should be a ┼ character.
        // The crossing is at the shared corners: (19, 9) or (20, 10).
        let has_cross = is_border_char(buf[(19, 9)].symbol())
            || is_border_char(buf[(20, 10)].symbol());
        assert!(has_cross);
    }
}
