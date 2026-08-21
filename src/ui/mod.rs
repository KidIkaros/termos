//! Shared rendering helpers — borders, the dock bar, and style conversion.

pub mod animation;
pub mod overlay;
pub mod perf;

use ratatui::style::{Color as TuiColor, Modifier, Style as TuiStyle};

use crate::config::theme::{Rgb, Theme};

/// A precomputed color mapping from the VT color space to ratatui colors.
///
/// Built once per render frame from the active theme. This turns per-cell
/// style conversion into array lookups instead of re-resolving
/// `Color::Default`/`Color::Indexed` through an `Option<&Theme>` for every
/// cell on screen.
#[derive(Debug, Clone)]
pub struct StylePalette {
    /// What `Color::Default` foreground resolves to.
    fg_default: TuiColor,
    /// The 256 indexed-color slots: theme ANSI palette for 0–15, standard
    /// xterm 256-color cube/grayscale for 16–255.
    indexed: [TuiColor; 256],
}

impl StylePalette {
    /// Build a palette from the active theme (or the xterm fallback when
    /// `theme` is `None`).
    pub fn new(theme: Option<&Theme>) -> Self {
        let mut indexed = [TuiColor::Reset; 256];
        for (i, slot) in indexed.iter_mut().enumerate() {
            *slot = xterm_256_color(i as u8);
        }
        let fg_default = match theme {
            Some(t) => TuiColor::Rgb(t.foreground.0, t.foreground.1, t.foreground.2),
            None => TuiColor::Reset,
        };
        if let Some(t) = theme {
            for (i, rgb) in t.ansi.iter().enumerate() {
                indexed[i] = TuiColor::Rgb(rgb.0, rgb.1, rgb.2);
            }
        }
        Self { fg_default, indexed }
    }

    /// Resolve a `vt::Color` to its ratatui color. O(1), branch-minimal.
    #[inline]
    pub fn resolve(&self, color: crate::vt::Color) -> TuiColor {
        match color {
            crate::vt::Color::Default => self.fg_default,
            crate::vt::Color::Indexed(i) => self.indexed[i as usize],
            crate::vt::Color::Rgb(r, g, b) => TuiColor::Rgb(r, g, b),
        }
    }

    /// Convert a `vt::Style` into a ratatui style.
    #[inline]
    pub fn style(&self, style: crate::vt::Style) -> TuiStyle {
        let mut s = TuiStyle::default().fg(self.resolve(style.fg));
        if !matches!(style.bg, crate::vt::Color::Default) {
            s = s.bg(self.resolve(style.bg));
        }
        let d = style.decoration;
        if d.bold {
            s = s.add_modifier(Modifier::BOLD);
        }
        if d.dim {
            s = s.add_modifier(Modifier::DIM);
        }
        if d.italic {
            s = s.add_modifier(Modifier::ITALIC);
        }
        if d.underline || d.double_underline {
            s = s.add_modifier(Modifier::UNDERLINED);
        }
        if d.reverse {
            s = s.add_modifier(Modifier::REVERSED);
        }
        if d.hidden {
            s = s.add_modifier(Modifier::HIDDEN);
        }
        if d.strikethrough {
            s = s.add_modifier(Modifier::CROSSED_OUT);
        }
        s
    }
}

/// The standard xterm 256-color palette: 16 basic colors, a 6×6×6 color cube
/// (16–231), and a grayscale ramp (232–255).
fn xterm_256_color(i: u8) -> TuiColor {
    const BASIC: [(u8, u8, u8); 16] = [
        (0, 0, 0),
        (205, 49, 49),
        (13, 188, 121),
        (229, 229, 16),
        (36, 114, 200),
        (188, 63, 188),
        (17, 168, 205),
        (229, 229, 229),
        (102, 102, 102),
        (241, 76, 76),
        (35, 209, 139),
        (245, 245, 67),
        (59, 142, 234),
        (214, 112, 214),
        (41, 184, 219),
        (255, 255, 255),
    ];
    let i = i as usize;
    if i < 16 {
        let (r, g, b) = BASIC[i];
        return TuiColor::Rgb(r, g, b);
    }
    if i < 232 {
        let idx = i - 16;
        let r = idx / 36;
        let g = (idx % 36) / 6;
        let b = idx % 6;
        let level = |v: usize| if v == 0 { 0u8 } else { (55 + 40 * v) as u8 };
        return TuiColor::Rgb(level(r), level(g), level(b));
    }
    let g = (8 + 10 * (i - 232)) as u8;
    TuiColor::Rgb(g, g, g)
}

/// Parse a border style name into a ratatui border type.
pub fn border_type(name: &str) -> ratatui::widgets::BorderType {
    match name {
        "rounded" => ratatui::widgets::BorderType::Rounded,
        "thick" => ratatui::widgets::BorderType::Thick,
        "double" => ratatui::widgets::BorderType::Double,
        "plain" | "normal" | "single" => ratatui::widgets::BorderType::Plain,
        "block" | "outer-half-block" => ratatui::widgets::BorderType::QuadrantOutside,
        "inner-half-block" => ratatui::widgets::BorderType::QuadrantInside,
        // "hidden" and "none" suppress all border glyphs entirely.
        // `draw_pane_border` calls `border_is_hidden` and early-returns, so
        // the type value here is never actually used for rendering.
        "hidden" | "none" => ratatui::widgets::BorderType::Plain,
        _ => ratatui::widgets::BorderType::Rounded,
    }
}

/// Returns true when the configured border style suppresses all border glyphs.
/// Used by `draw_pane_border` and `paint_scrollbar` to skip rendering.
#[inline]
pub fn border_is_hidden(name: &str) -> bool {
    matches!(name, "hidden" | "none")
}

/// A dock bar renderer. The dock sits at the bottom (or top) and shows the
/// workspace, session, mode, and notification status.
#[derive(Debug, Clone, Default)]
pub struct DockState {
    pub message: Option<String>,
    pub message_kind: String,
}

impl DockState {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Convert an RGB triple to a ratatui color.
pub fn rgb_tui(rgb: Rgb) -> TuiColor {
    TuiColor::Rgb(rgb.0, rgb.1, rgb.2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::theme::{Rgb, Theme};
    use crate::vt::cell::{Color, Style};
    use ratatui::style::Modifier;

    fn test_theme() -> Theme {
        Theme {
            name: "test".to_string(),
            foreground: Rgb::new(200, 200, 200),
            background: Rgb::new(30, 30, 30),
            cursor: Rgb::new(200, 200, 200),
            ansi: Theme::default_ansi(),
        }
    }

    #[test]
    fn palette_default_no_theme() {
        let p = StylePalette::new(None);
        assert_eq!(p.resolve(Color::Default), TuiColor::Reset);
    }

    #[test]
    fn palette_default_with_theme() {
        let theme = test_theme();
        let p = StylePalette::new(Some(&theme));
        match p.resolve(Color::Default) {
            TuiColor::Rgb(_, _, _) => {}
            _ => panic!("expected Rgb"),
        }
    }

    #[test]
    fn palette_indexed_no_theme() {
        let p = StylePalette::new(None);
        match p.resolve(Color::Indexed(0)) {
            TuiColor::Rgb(r, g, b) => assert_eq!((r, g, b), (0, 0, 0)),
            _ => panic!("expected Rgb"),
        }
    }

    #[test]
    fn palette_indexed_with_theme() {
        let theme = test_theme();
        let p = StylePalette::new(Some(&theme));
        match p.resolve(Color::Indexed(1)) {
            TuiColor::Rgb(_, _, _) => {}
            _ => panic!("expected Rgb"),
        }
    }

    #[test]
    fn palette_indexed_beyond_ansi_range() {
        // Indices 16–255 previously indexed a 16-entry theme array and would
        // panic; they must resolve to a concrete cube/grayscale color.
        let theme = test_theme();
        let p = StylePalette::new(Some(&theme));
        for i in [16u8, 196, 232, 255] {
            match p.resolve(Color::Indexed(i)) {
                TuiColor::Rgb(_, _, _) => {}
                other => panic!("expected Rgb for index {i}, got {other:?}"),
            }
        }
    }

    #[test]
    fn palette_rgb() {
        let p = StylePalette::new(None);
        assert_eq!(
            p.resolve(Color::Rgb(100, 200, 50)),
            TuiColor::Rgb(100, 200, 50)
        );
    }

    #[test]
    fn palette_style_default() {
        let p = StylePalette::new(None);
        let s = p.style(Style::new());
        assert_eq!(s.fg, Some(TuiColor::Reset));
    }

    #[test]
    fn palette_style_bold() {
        let mut style = Style::new();
        style.decoration.bold = true;
        let s = StylePalette::new(None).style(style);
        assert!(s.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn palette_style_dim() {
        let mut style = Style::new();
        style.decoration.dim = true;
        let s = StylePalette::new(None).style(style);
        assert!(s.add_modifier.contains(Modifier::DIM));
    }

    #[test]
    fn palette_style_italic() {
        let mut style = Style::new();
        style.decoration.italic = true;
        let s = StylePalette::new(None).style(style);
        assert!(s.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn palette_style_underline() {
        let mut style = Style::new();
        style.decoration.underline = true;
        let s = StylePalette::new(None).style(style);
        assert!(s.add_modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn palette_style_double_underline() {
        let mut style = Style::new();
        style.decoration.double_underline = true;
        let s = StylePalette::new(None).style(style);
        assert!(s.add_modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn palette_style_reverse() {
        let mut style = Style::new();
        style.decoration.reverse = true;
        let s = StylePalette::new(None).style(style);
        assert!(s.add_modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn palette_style_hidden() {
        let mut style = Style::new();
        style.decoration.hidden = true;
        let s = StylePalette::new(None).style(style);
        assert!(s.add_modifier.contains(Modifier::HIDDEN));
    }

    #[test]
    fn palette_style_strikethrough() {
        let mut style = Style::new();
        style.decoration.strikethrough = true;
        let s = StylePalette::new(None).style(style);
        assert!(s.add_modifier.contains(Modifier::CROSSED_OUT));
    }

    #[test]
    fn palette_style_with_bg() {
        let mut style = Style::new();
        style.bg = Color::Rgb(10, 20, 30);
        let s = StylePalette::new(None).style(style);
        assert!(s.bg.is_some());
    }

    #[test]
    fn palette_style_default_bg_omitted() {
        let p = StylePalette::new(None);
        let s = p.style(Style::new());
        assert!(s.bg.is_none());
    }

    #[test]
    fn border_type_rounded() {
        assert_eq!(
            border_type("rounded"),
            ratatui::widgets::BorderType::Rounded
        );
    }

    #[test]
    fn border_type_single_aliases_plain() {
        assert_eq!(border_type("single"), ratatui::widgets::BorderType::Plain);
    }

    #[test]
    fn border_type_thick() {
        assert_eq!(border_type("thick"), ratatui::widgets::BorderType::Thick);
    }

    #[test]
    fn border_type_double() {
        assert_eq!(border_type("double"), ratatui::widgets::BorderType::Double);
    }

    #[test]
    fn border_type_plain() {
        assert_eq!(border_type("plain"), ratatui::widgets::BorderType::Plain);
    }

    #[test]
    fn border_type_normal() {
        assert_eq!(border_type("normal"), ratatui::widgets::BorderType::Plain);
    }

    #[test]
    fn border_type_half_block_styles() {
        assert_eq!(
            border_type("outer-half-block"),
            ratatui::widgets::BorderType::QuadrantOutside
        );
        assert_eq!(
            border_type("inner-half-block"),
            ratatui::widgets::BorderType::QuadrantInside
        );
        assert_eq!(border_type("block"), ratatui::widgets::BorderType::QuadrantOutside);
    }

    #[test]
    fn border_type_hidden_aliases_plain() {
        assert_eq!(border_type("hidden"), ratatui::widgets::BorderType::Plain);
        assert_eq!(border_type("none"), ratatui::widgets::BorderType::Plain);
        assert!(border_is_hidden("hidden"));
        assert!(border_is_hidden("none"));
    }

    #[test]
    fn border_type_unknown_falls_back_to_rounded() {
        assert_eq!(
            border_type("unknown"),
            ratatui::widgets::BorderType::Rounded
        );
    }

    #[test]
    fn dock_state_new() {
        let ds = DockState::new();
        assert!(ds.message.is_none());
    }

    #[test]
    fn rgb_tui_converts() {
        let c = rgb_tui(Rgb::new(100, 200, 50));
        assert_eq!(c, TuiColor::Rgb(100, 200, 50));
    }
}
