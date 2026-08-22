//! Buffer rendering primitives for dashboard widgets.
//!
//! These helpers draw ratatui-style widgets directly into a `Buffer`
//! without needing a `Frame`, so the dashboard overlay can render real
//! gauges, bar charts, and sparklines.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Widget};

/// Draw a block border + title around an area, returning the inner area.
pub fn draw_block(block: &Block, area: Rect, buf: &mut Buffer) -> Rect {
    block.render(area, buf);
    block.inner(area)
}

/// Draw a horizontal gauge bar.
///
/// `ratio` is 0.0–1.0. The bar fills from left with `color`, remainder is dim.
pub fn draw_gauge(
    area: Rect,
    ratio: f64,
    color: Color,
    label: &str,
    buf: &mut Buffer,
) {
    if area.height < 1 || area.width < 2 {
        return;
    }
    let fill = ((area.width as f64 * ratio) as u16).min(area.width);
    for x in 0..area.width {
        let cell = buf.cell_mut((area.x + x, area.y));
        if let Some(cell) = cell {
            if x < fill {
                cell.set_char('█');
                cell.set_fg(color);
            } else {
                cell.set_char('░');
                cell.set_fg(Color::DarkGray);
            }
        }
    }
    // Draw label centered
    if !label.is_empty() && area.width > label.len() as u16 {
        let offset = (area.width - label.len() as u16) / 2;
        for (i, ch) in label.chars().enumerate() {
            let cell = buf.cell_mut((area.x + offset + i as u16, area.y));
            if let Some(cell) = cell {
                cell.set_char(ch);
                cell.set_fg(Color::White);

            }
        }
    }
}

/// Draw a horizontal bar chart (stacked: used/available).
pub fn draw_bar_chart(
    area: Rect,
    segments: &[(f64, Color)],
    buf: &mut Buffer,
) {
    if area.height < 1 || area.width < 2 {
        return;
    }
    let total: f64 = segments.iter().map(|(v, _)| v).sum();
    if total <= 0.0 {
        return;
    }
    let mut x = 0u16;
    for (value, color) in segments {
        let width = ((area.width as f64 * value / total) as u16).min(area.width - x);
        for dx in 0..width {
            let cell = buf.cell_mut((area.x + x + dx, area.y));
            if let Some(cell) = cell {
                cell.set_char('█');
                cell.set_fg(*color);
            }
        }
        x += width;
    }
    // Fill remainder with dim
    while x < area.width {
        let cell = buf.cell_mut((area.x + x, area.y));
        if let Some(cell) = cell {
            cell.set_char('░');
            cell.set_fg(Color::DarkGray);
        }
        x += 1;
    }
}

/// Draw a sparkline (vertical bars from a history slice).
pub fn draw_sparkline(
    area: Rect,
    data: &[f64],
    color: Color,
    buf: &mut Buffer,
) {
    if area.height < 1 || area.width < 1 || data.is_empty() {
        return;
    }
    let max_val = data.iter().copied().fold(0.0f64, f64::max).max(1.0);
    let bar_chars = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

    let step = data.len() as f64 / area.width as f64;
    for x in 0..area.width {
        let idx = (x as f64 * step) as usize;
        let val = data[idx.min(data.len() - 1)];
        let level = ((val / max_val) * 7.0) as usize;
        let ch = bar_chars[level.min(7)];
        let cell = buf.cell_mut((area.x + x, area.y));
        if let Some(cell) = cell {
            cell.set_char(ch);
            cell.set_fg(color);
        }
    }
}

/// Draw a text line at a specific position.
pub fn draw_text(x: u16, y: u16, text: &str, style: Style, buf: &mut Buffer) {
    for (i, ch) in text.chars().enumerate() {
        let cell = buf.cell_mut((x + i as u16, y));
        if let Some(cell) = cell {
            cell.set_char(ch);
            cell.set_fg(style.fg.unwrap_or(Color::White));
        }
    }
}

/// Draw a colored status indicator (● OK / ● FAIL).
pub fn draw_status_dot(x: u16, y: u16, ok: bool, buf: &mut Buffer) {
    let cell = buf.cell_mut((x, y));
    if let Some(cell) = cell {
        cell.set_char('●');
        cell.set_fg(if ok { Color::Green } else { Color::Red });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draw_gauge_renders() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 1));
        draw_gauge(Rect::new(0, 0, 20, 1), 0.5, Color::Green, "50%", &mut buf);
        // First 10 chars should be █, rest ░
        assert_eq!(buf[(0, 0)].symbol(), "█");
        assert_eq!(buf[(10, 0)].symbol(), "░");
    }

    #[test]
    fn draw_sparkline_renders() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 1));
        let data: Vec<f64> = (0..10).map(|i| i as f64).collect();
        draw_sparkline(Rect::new(0, 0, 10, 1), &data, Color::Cyan, &mut buf);
        // Should have bar characters
        let ch = buf[(5, 0)].symbol().chars().next().unwrap();
        assert!('▁'..='█').contains(&ch);
    }

    #[test]
    fn draw_block_returns_inner() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 5));
        let block = Block::default().title(" Test ").borders(Borders::ALL);
        let inner = draw_block(&block, Rect::new(0, 0, 10, 5), &mut buf);
        assert_eq!(inner.x, 1);
        assert_eq!(inner.y, 1);
        assert_eq!(inner.width, 8);
        assert_eq!(inner.height, 3);
    }
}
