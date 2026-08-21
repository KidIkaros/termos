#![allow(dead_code)]
//! `cargo run --bin compare`
//!
//! Side-by-side terminal comparison:
//!   Left  — PixelCanvas: RGB cell backgrounds + ratatui text overlay
//!   Right — asciline-style: half-block ▀ chars giving 2× vertical resolution
//!
//! Both panels render an iOS 26 Liquid Glass aesthetic:
//!   • Deep navy background gradient
//!   • Frosted-glass panels (luminance-tinted overlay)
//!   • Vivid indigo→violet→pink accent gradient
//!   • Specular highlight line + soft glow halo

use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::Span,
    widgets::Widget,
    Terminal,
};
use std::io;
use std::time::Duration;

// ─── Liquid Glass palette ────────────────────────────────────────────────────

const BG: (u8, u8, u8) = (8, 8, 22);          // deep navy
const BG2: (u8, u8, u8) = (18, 16, 40);        // slightly lighter bg
const PANEL: (u8, u8, u8) = (28, 26, 60);      // frosted panel fill
const PANEL_EDGE: (u8, u8, u8) = (60, 55, 110);// panel border highlight
const SPEC: (u8, u8, u8) = (180, 175, 255);    // specular top-edge highlight
const GLOW: (u8, u8, u8) = (80, 60, 160);      // purple glow halo
const ACCENT_A: (u8, u8, u8) = (99, 102, 241); // indigo  #6366f1
const ACCENT_B: (u8, u8, u8) = (168, 85, 247); // violet  #a855f7
const ACCENT_C: (u8, u8, u8) = (236, 72, 153); // pink    #ec4899
const FG_BRIGHT: (u8, u8, u8) = (240, 238, 255);
const FG_DIM: (u8, u8, u8) = (140, 135, 200);

// ─── Math helpers ────────────────────────────────────────────────────────────

fn lerp(a: u8, b: u8, t: f32) -> u8 {
    let t = t.clamp(0.0, 1.0);
    (a as f32 * (1.0 - t) + b as f32 * t).round() as u8
}

fn lerp3(a: (u8,u8,u8), b: (u8,u8,u8), t: f32) -> (u8,u8,u8) {
    (lerp(a.0, b.0, t), lerp(a.1, b.1, t), lerp(a.2, b.2, t))
}

/// Tri-stop gradient: A → B → C along t ∈ [0, 1].
fn accent_at(t: f32) -> (u8, u8, u8) {
    if t < 0.5 { lerp3(ACCENT_A, ACCENT_B, t * 2.0) }
    else        { lerp3(ACCENT_B, ACCENT_C, (t - 0.5) * 2.0) }
}

/// Gaussian falloff for glow/shadow.
fn gauss(dist2: f32, sigma: f32) -> f32 {
    (-dist2 / (2.0 * sigma * sigma)).exp()
}

/// Rec.601 luma — matches asciline's `Mapper::gray`.
fn luma(r: u8, g: u8, b: u8) -> u8 {
    ((77 * r as u32 + 150 * g as u32 + 29 * b as u32) >> 8) as u8
}

/// Map a luma value to one of 93 ASCII ramp characters (asciline DEFAULT_PALETTE).
fn ascii_char(luma: u8) -> char {
    const PALETTE: &str =
        " `.-':_,^=;><+!rc*/z?sLTv)J7(|Fi{C}fI31tlu[neoZ5Yxjya]2ESwqkP6h9d4VpOGbUAKXHm8RD#$Bg0MNWQ%&@";
    let chars: Vec<char> = PALETTE.chars().collect();
    let n = chars.len();
    let idx = (luma as usize * (n - 1)) / 255;
    chars[idx.min(n - 1)]
}

// ─── RGB framebuffer ─────────────────────────────────────────────────────────

struct Fb {
    data: Vec<(u8, u8, u8)>,
    w: usize,
    h: usize,
}

impl Fb {
    fn new(w: usize, h: usize) -> Self {
        Self { data: vec![(0, 0, 0); w * h], w, h }
    }

    fn set(&mut self, x: usize, y: usize, c: (u8, u8, u8)) {
        if x < self.w && y < self.h {
            self.data[y * self.w + x] = c;
        }
    }

    fn get(&self, x: usize, y: usize) -> (u8, u8, u8) {
        if x < self.w && y < self.h { self.data[y * self.w + x] } else { (0, 0, 0) }
    }

    /// Fill a rectangle with a color.
    fn fill_rect(&mut self, x0: usize, y0: usize, w: usize, h: usize, c: (u8,u8,u8)) {
        for y in y0..(y0+h).min(self.h) {
            for x in x0..(x0+w).min(self.w) {
                self.set(x, y, c);
            }
        }
    }

    /// Horizontal gradient across a rect.
    fn grad_h(&mut self, x0: usize, y0: usize, w: usize, h: usize,
              a: (u8,u8,u8), b: (u8,u8,u8)) {
        for y in y0..(y0+h).min(self.h) {
            for dx in 0..w {
                let x = x0 + dx;
                if x >= self.w { break; }
                let t = if w <= 1 { 0.0 } else { dx as f32 / (w-1) as f32 };
                self.set(x, y, lerp3(a, b, t));
            }
        }
    }

    /// Vertical gradient.
    fn grad_v(&mut self, x0: usize, y0: usize, w: usize, h: usize,
              a: (u8,u8,u8), b: (u8,u8,u8)) {
        for dy in 0..h {
            let y = y0 + dy;
            if y >= self.h { break; }
            let t = if h <= 1 { 0.0 } else { dy as f32 / (h-1) as f32 };
            let c = lerp3(a, b, t);
            for x in x0..(x0+w).min(self.w) { self.set(x, y, c); }
        }
    }

    /// Blend a color onto existing pixel with alpha.
    fn blend(&mut self, x: usize, y: usize, c: (u8,u8,u8), alpha: f32) {
        if x >= self.w || y >= self.h { return; }
        let src = self.get(x, y);
        self.set(x, y, lerp3(src, c, alpha));
    }

    /// Soft rectangular glow halo (Gaussian on distance-to-rect).
    fn glow_rect(&mut self, x0: usize, y0: usize, w: usize, h: usize,
                 color: (u8,u8,u8), radius: f32, strength: f32) {
        let r = radius.ceil() as usize;
        let x1 = (x0 + w) as i32;
        let y1 = (y0 + h) as i32;
        let sx = (x0 as i32 - r as i32).max(0) as usize;
        let ex = (x1 + r as i32).min(self.w as i32) as usize;
        let sy = (y0 as i32 - r as i32).max(0) as usize;
        let ey = (y1 + r as i32).min(self.h as i32) as usize;
        for y in sy..ey {
            for x in sx..ex {
                let xi = x as i32; let yi = y as i32;
                if xi >= x0 as i32 && xi < x1 && yi >= y0 as i32 && yi < y1 { continue; }
                let dx = if xi < x0 as i32 { (x0 as i32 - xi) as f32 }
                         else if xi >= x1   { (xi - x1 + 1) as f32 }
                         else { 0.0 };
                let dy = if yi < y0 as i32 { (y0 as i32 - yi) as f32 }
                         else if yi >= y1   { (yi - y1 + 1) as f32 }
                         else { 0.0 };
                let d2 = dx * dx + dy * dy;
                let alpha = gauss(d2, radius / 2.0) * strength;
                if alpha > 0.01 { self.blend(x, y, color, alpha); }
            }
        }
    }
}

// ─── Scene builder ───────────────────────────────────────────────────────────

/// Paint the iOS 26 Liquid Glass scene into `fb`.
/// `fb` may be 1× height (PixelCanvas mode) or 2× height (asciline half-block mode).
fn paint_scene(fb: &mut Fb) {
    let w = fb.w;
    let h = fb.h;

    // Background: deep navy vertical gradient
    fb.grad_v(0, 0, w, h, BG, BG2);

    // Background diagonal shimmer (liquid effect)
    for y in 0..h {
        for x in 0..w {
            let t = ((x as f32 * 0.8 + y as f32 * 0.4) / (w + h) as f32 * 6.0).sin();
            let shimmer = (t * 0.5 + 0.5) * 0.04;
            fb.blend(x, y, (120, 100, 220), shimmer);
        }
    }

    // Accent gradient bar at bottom
    if h >= 3 {
        for x in 0..w {
            let t = x as f32 / w as f32;
            let c = accent_at(t);
            fb.set(x, h - 1, c);
            let dim = lerp3(BG2, c, 0.3);
            fb.set(x, h - 2, dim);
        }
    }

    // ── Main frosted glass panel ──────────────────────────────────────────
    let pw = (w as f32 * 0.7) as usize;
    let ph = (h as f32 * 0.55) as usize;
    let px = (w - pw) / 2;
    let py = (h as f32 * 0.12) as usize;

    // Glow halo behind panel
    fb.glow_rect(px, py, pw, ph, GLOW, 4.0, 0.7);

    // Panel body: frosted glass (semi-transparent tinted fill)
    for y in py..(py+ph).min(h) {
        for x in px..(px+pw).min(w) {
            let base = fb.get(x, y);
            // "Frosting": lighten and tint toward panel color
            let frosted = lerp3(base, PANEL, 0.72);
            // Add subtle noise texture (deterministic)
            let noise = (((x * 37 + y * 53) % 8) as f32 - 4.0) / 255.0;
            let frosted = (
                (frosted.0 as f32 + noise * 8.0).clamp(0.0, 255.0) as u8,
                (frosted.1 as f32 + noise * 6.0).clamp(0.0, 255.0) as u8,
                (frosted.2 as f32 + noise * 10.0).clamp(0.0, 255.0) as u8,
            );
            fb.set(x, y, frosted);
        }
    }

    // Panel border (edge glow, not box-drawing — those are added in the ratatui pass)
    for x in px..(px+pw).min(w) {
        fb.blend(x, py, PANEL_EDGE, 0.9);
        fb.blend(x, (py+ph-1).min(h-1), PANEL_EDGE, 0.5);
    }
    for y in py..(py+ph).min(h) {
        fb.blend(px, y, PANEL_EDGE, 0.8);
        fb.blend((px+pw-1).min(w-1), y, PANEL_EDGE, 0.8);
    }

    // Specular highlight — thin bright line at top of panel
    for x in px..(px+pw).min(w) {
        let t = (x - px) as f32 / pw as f32;
        let spec_a = if t < 0.5 { t * 2.0 } else { (1.0 - t) * 2.0 };
        let spec_a = spec_a * 0.85;
        fb.blend(x, py, SPEC, spec_a);
    }

    // ── Smaller secondary panel (lower-left) ─────────────────────────────
    let sw = (w as f32 * 0.28) as usize;
    let sh = (h as f32 * 0.22) as usize;
    let sx0 = (w as f32 * 0.06) as usize;
    let sy0 = (h as f32 * 0.62) as usize;

    fb.glow_rect(sx0, sy0, sw, sh, ACCENT_A, 3.0, 0.5);
    for y in sy0..(sy0+sh).min(h) {
        for x in sx0..(sx0+sw).min(w) {
            let base = fb.get(x, y);
            fb.set(x, y, lerp3(base, (22, 20, 55), 0.75));
        }
    }
    // Accent gradient top edge of secondary panel
    for x in sx0..(sx0+sw).min(w) {
        let t = (x - sx0) as f32 / sw as f32;
        fb.set(x, sy0, accent_at(t));
    }
    for x in sx0..(sx0+sw).min(w) {
        fb.blend(x, sy0, SPEC, 0.4);
    }

    // ── Third panel (lower-right) ─────────────────────────────────────────
    let tw = (w as f32 * 0.28) as usize;
    let th = (h as f32 * 0.22) as usize;
    let tx0 = (w as f32 * 0.66) as usize;
    let ty0 = (h as f32 * 0.62) as usize;

    fb.glow_rect(tx0, ty0, tw, th, ACCENT_C, 3.0, 0.4);
    for y in ty0..(ty0+th).min(h) {
        for x in tx0..(tx0+tw).min(w) {
            let base = fb.get(x, y);
            fb.set(x, y, lerp3(base, (40, 18, 40), 0.75));
        }
    }
    for x in tx0..(tx0+tw).min(w) {
        let t = (x - tx0) as f32 / tw as f32;
        fb.set(x, ty0, accent_at(0.5 + t * 0.5));
    }
    for x in tx0..(tx0+tw).min(w) {
        fb.blend(x, ty0, SPEC, 0.35);
    }
}

// ─── PixelCanvas renderer ────────────────────────────────────────────────────
// Approach: 1× framebuffer → RGB cell backgrounds in ratatui

struct PixelCanvasWidget {
    fb: Fb,
}

impl PixelCanvasWidget {
    fn new(w: usize, h: usize) -> Self {
        let mut fb = Fb::new(w, h);
        paint_scene(&mut fb);
        Self { fb }
    }
}

impl Widget for PixelCanvasWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        for y in 0..area.height {
            for x in 0..area.width {
                let (r, g, b) = self.fb.get(x as usize, y as usize);
                let cell = &mut buf[(area.x + x, area.y + y)];
                cell.set_char(' ');
                cell.set_bg(Color::Rgb(r, g, b));
            }
        }
        // Overlay text / labels via ratatui (transparent fg over colored bg)
        overlay_text(area, buf, false);
    }
}

// ─── Asciline half-block renderer ────────────────────────────────────────────
// Approach: 2× height framebuffer → half-block ▀ chars (fg=top, bg=bottom)
// Each terminal row encodes TWO pixel rows → 2× vertical resolution.
// ASCII brightness char is blended in for the "character art" effect.

struct AscilineWidget {
    fb: Fb,   // 2× height
}

impl AscilineWidget {
    fn new(w: usize, h: usize) -> Self {
        // Paint at 2× vertical resolution
        let mut fb = Fb::new(w, h * 2);
        paint_scene(&mut fb);
        Self { fb }
    }
}

impl Widget for AscilineWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Fold 2× buffer into half-block cells
        for row in 0..area.height {
            for col in 0..area.width {
                let top = self.fb.get(col as usize, row as usize * 2);
                let bot = self.fb.get(col as usize, row as usize * 2 + 1);

                // Compute luma of the top pixel to choose ASCII char.
                // At brightness extremes, use half-block; mid-range gets
                // a density char from the ASCII ramp — this is the
                // "character art" effect that distinguishes asciline.
                let l = luma(top.0, top.1, top.2);
                let ch = if l < 12 || l > 245 {
                    '▀'
                } else {
                    // ASCII ramp char chosen by brightness
                    ascii_char(l)
                };

                let cell = &mut buf[(area.x + col, area.y + row)];
                cell.set_char(ch);
                cell.set_fg(Color::Rgb(top.0, top.1, top.2));
                cell.set_bg(Color::Rgb(bot.0, bot.1, bot.2));
            }
        }
        // Same text overlay
        overlay_text(area, buf, true);
    }
}

// ─── Shared text overlay ─────────────────────────────────────────────────────
// Draws the same UI chrome (labels, simulated controls) on both panels.

fn overlay_text(area: Rect, buf: &mut Buffer, is_asciline: bool) {
    let w = area.width as usize;
    let h = area.height as usize;

    // Renderer label pill at top
    let label = if is_asciline {
        " asciline  ▀ half-block  2× vres "
    } else {
        " PixelCanvas  RGB bg  1× vres "
    };
    let label_x = area.x + (w as u16).saturating_sub(label.len() as u16) / 2;
    let label_y = area.y + 1;
    let accent = if is_asciline { Color::Rgb(236, 72, 153) } else { Color::Rgb(99, 102, 241) };
    let pill_style = Style::default()
        .fg(Color::Rgb(240, 238, 255))
        .bg(accent)
        .add_modifier(Modifier::BOLD);
    let span = Span::styled(label, pill_style);
    buf.set_span(label_x, label_y, &span, label.len() as u16);

    // Panel title in the main frosted panel area
    let py = (h as f32 * 0.12) as u16 + 2;
    let title = "✦  Liquid Glass";
    let tx = area.x + (w as u16).saturating_sub(title.len() as u16) / 2;
    buf.set_span(tx, area.y + py, &Span::styled(title, Style::default()
        .fg(Color::Rgb(FG_BRIGHT.0, FG_BRIGHT.1, FG_BRIGHT.2))
        .add_modifier(Modifier::BOLD)), title.len() as u16);

    let sub = "iOS 26 inspired TUI compositor";
    let sx = area.x + (w as u16).saturating_sub(sub.len() as u16) / 2;
    buf.set_span(sx, area.y + py + 1, &Span::styled(sub, Style::default()
        .fg(Color::Rgb(FG_DIM.0, FG_DIM.1, FG_DIM.2))), sub.len() as u16);

    // Fake stat pills inside panel
    let stats = ["  99% CPU  ", "  2.1 GB  ", "  ↑ 1.2 GB/s  "];
    let colors = [ACCENT_A, ACCENT_B, ACCENT_C];
    let panel_center = area.x + w as u16 / 2;
    let total_w: u16 = stats.iter().map(|s| s.len() as u16 + 2).sum();
    let mut sx2 = panel_center.saturating_sub(total_w / 2);
    let stat_y = area.y + py + 3;
    for (stat, color) in stats.iter().zip(colors.iter()) {
        let s = Span::styled(*stat, Style::default()
            .fg(Color::Rgb(240, 238, 255))
            .bg(Color::Rgb(color.0, color.1, color.2)));
        buf.set_span(sx2, stat_y, &s, stat.len() as u16);
        sx2 += stat.len() as u16 + 2;
    }

    // Bottom secondary panel label
    let lp_y = (h as f32 * 0.63) as u16;
    let lp_x = area.x + (w as f32 * 0.08) as u16;
    buf.set_span(lp_x, area.y + lp_y + 1,
        &Span::styled("Sessions", Style::default().fg(Color::Rgb(FG_BRIGHT.0, FG_BRIGHT.1, FG_BRIGHT.2))
            .add_modifier(Modifier::BOLD)), 8);
    buf.set_span(lp_x, area.y + lp_y + 2,
        &Span::styled("3 active", Style::default().fg(Color::Rgb(FG_DIM.0, FG_DIM.1, FG_DIM.2))), 8);

    // Right secondary panel label
    let rp_x = area.x + (w as f32 * 0.68) as u16;
    buf.set_span(rp_x, area.y + lp_y + 1,
        &Span::styled("Workspaces", Style::default().fg(Color::Rgb(FG_BRIGHT.0, FG_BRIGHT.1, FG_BRIGHT.2))
            .add_modifier(Modifier::BOLD)), 10);
    buf.set_span(rp_x, area.y + lp_y + 2,
        &Span::styled("7 windows", Style::default().fg(Color::Rgb(FG_DIM.0, FG_DIM.1, FG_DIM.2))), 9);

    // Divider line between panels (only drawn once, handled by caller)
    // Footer hint
    if h > 3 {
        let hint = " q/Esc — quit ";
        let hx = area.x + (w as u16).saturating_sub(hint.len() as u16) / 2;
        let hy = area.y + area.height - 2;
        buf.set_span(hx, hy, &Span::styled(hint, Style::default()
            .fg(Color::Rgb(100, 95, 160))), hint.len() as u16);
    }
}

// ─── Screenshot renderer ──────────────────────────────────────────────────────
// Renders both panels at a fixed size into a PPM image file without needing
// an interactive terminal. Each terminal "cell" becomes a CHAR_W×CHAR_H block
// of pixels — left panel uses solid color fill, right uses half-block split.

const CHAR_W: usize = 8;
const CHAR_H: usize = 16;

fn render_to_ppm(path: &str) {
    let cols: usize = 200;
    let rows: usize = 48;
    let img_w = cols * CHAR_W;
    let img_h = rows * CHAR_H;

    // Render both panels into ratatui buffers at fixed size
    let half_cols = cols / 2;
    let right_cols = cols - half_cols - 1;

    let left_area  = Rect { x: 0, y: 0, width: half_cols as u16, height: rows as u16 };
    let right_area = Rect { x: 0, y: 0, width: right_cols as u16, height: rows as u16 };

    let mut left_buf  = Buffer::empty(left_area);
    let mut right_buf = Buffer::empty(right_area);

    PixelCanvasWidget::new(half_cols, rows).render(left_area, &mut left_buf);
    AscilineWidget::new(right_cols, rows).render(right_area, &mut right_buf);

    // Build pixel image
    let mut pixels = vec![0u8; img_w * img_h * 3];

    // Helper: write a solid color block for one cell
    let set_block = |pixels: &mut Vec<u8>, cx: usize, cy: usize, top: (u8,u8,u8), bot: (u8,u8,u8)| {
        for dy in 0..CHAR_H {
            let color = if dy < CHAR_H / 2 { top } else { bot };
            for dx in 0..CHAR_W {
                let px = cx * CHAR_W + dx;
                let py = cy * CHAR_H + dy;
                let idx = (py * img_w + px) * 3;
                pixels[idx]     = color.0;
                pixels[idx + 1] = color.1;
                pixels[idx + 2] = color.2;
            }
        }
    };

    // Render left panel (PixelCanvas — solid bg color per cell)
    for row in 0..rows {
        for col in 0..half_cols {
            let cell = left_buf.cell((col as u16, row as u16)).unwrap();
            let bg = match cell.style().bg {
                Some(Color::Rgb(r, g, b)) => (r, g, b),
                _ => BG,
            };
            set_block(&mut pixels, col, row, bg, bg);
        }
    }

    // Render centre divider
    for row in 0..rows {
        set_block(&mut pixels, half_cols, row, (80, 60, 160), (80, 60, 160));
    }

    // Render right panel (asciline — fg=top half, bg=bottom half per cell)
    for row in 0..rows {
        for col in 0..right_cols {
            let cell = right_buf.cell((col as u16, row as u16)).unwrap();
            let fg = match cell.style().fg {
                Some(Color::Rgb(r, g, b)) => (r, g, b),
                _ => BG2,
            };
            let bg = match cell.style().bg {
                Some(Color::Rgb(r, g, b)) => (r, g, b),
                _ => BG,
            };
            set_block(&mut pixels, half_cols + 1 + col, row, fg, bg);
        }
    }

    // Write PPM
    let header = format!("P6\n{img_w} {img_h}\n255\n");
    let mut data = header.into_bytes();
    data.extend_from_slice(&pixels);
    std::fs::write(path, &data).expect("Failed to write PPM");
    eprintln!("Written: {path}  ({img_w}×{img_h} px)");
}

// ─── Main ────────────────────────────────────────────────────────────────────

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(|s| s.as_str()) == Some("--screenshot") {
        let out = args.get(2).map(|s| s.as_str()).unwrap_or("/tmp/compare.ppm");
        render_to_ppm(out);
        return Ok(());
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    loop {
        terminal.draw(|frame| {
            let area = frame.area();
            let buf = frame.buffer_mut();
            let half_w = area.width / 2;

            // Left half — PixelCanvas (1× resolution RGB bg)
            let left = Rect { x: area.x, y: area.y, width: half_w, height: area.height };
            PixelCanvasWidget::new(half_w as usize, area.height as usize).render(left, buf);

            // Centre divider
            let div_x = area.x + half_w;
            for y in area.y..area.y + area.height {
                let cell = &mut buf[(div_x, y)];
                cell.set_char('│');
                cell.set_fg(Color::Rgb(GLOW.0, GLOW.1, GLOW.2));
                cell.set_bg(Color::Rgb(BG.0, BG.1, BG.2));
            }

            // Right half — asciline (2× resolution half-block)
            let right_x = div_x + 1;
            let right_w = area.width.saturating_sub(half_w + 1);
            let right = Rect { x: right_x, y: area.y, width: right_w, height: area.height };
            AscilineWidget::new(right_w as usize, area.height as usize).render(right, buf);

            // Top header bar spanning full width
            let header = " PixelCanvas  │  asciline half-block — iOS 26 Liquid Glass TUI compositor ";
            let hx = area.x + area.width.saturating_sub(header.len() as u16) / 2;
            buf.set_span(hx, area.y, &Span::styled(header, Style::default()
                .fg(Color::Rgb(SPEC.0, SPEC.1, SPEC.2))
                .add_modifier(Modifier::BOLD)), header.len() as u16);
        })?;

        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(k) = event::read()? {
                if matches!(k.code, KeyCode::Char('q') | KeyCode::Esc) {
                    break;
                }
            }
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}
