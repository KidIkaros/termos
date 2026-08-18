//! Shared rendering helpers — borders, the dock bar, and style conversion.

pub mod animation;
pub mod overlay;
pub mod style_cache;

use ratatui::style::{Color as TuiColor, Modifier, Style as TuiStyle};

use crate::config::theme::Rgb;

/// Convert a `vt::Color` into a ratatui color. Default colors resolve to the
/// theme's foreground/background when provided.
pub fn to_tui_color(
    color: crate::vt::Color,
    theme: Option<&crate::config::theme::Theme>,
) -> TuiColor {
    match color {
        crate::vt::Color::Default => {
            if let Some(theme) = theme {
                TuiColor::Rgb(theme.foreground.0, theme.foreground.1, theme.foreground.2)
            } else {
                TuiColor::Reset
            }
        }
        crate::vt::Color::Indexed(i) => {
            if let Some(theme) = theme {
                TuiColor::Rgb(
                    theme.ansi[i as usize].0,
                    theme.ansi[i as usize].1,
                    theme.ansi[i as usize].2,
                )
            } else {
                // Fall back to the standard xterm 256 palette for 0-15.
                xterm_256(i)
            }
        }
        crate::vt::Color::Rgb(r, g, b) => TuiColor::Rgb(r, g, b),
    }
}

/// Convert a `vt::Style` into a ratatui style.
pub fn to_tui_style(
    style: crate::vt::Style,
    theme: Option<&crate::config::theme::Theme>,
) -> TuiStyle {
    let mut s = TuiStyle::default().fg(to_tui_color(style.fg, theme));
    if !matches!(style.bg, crate::vt::Color::Default) {
        s = s.bg(to_tui_color(style.bg, theme));
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

/// A minimal xterm-256 palette entry for the basic 16 colors.
fn xterm_256(i: u8) -> TuiColor {
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
    let (r, g, b) = BASIC[i as usize];
    TuiColor::Rgb(r, g, b)
}

/// Parse a border style name into a ratatui border type.
pub fn border_type(name: &str) -> ratatui::widgets::BorderType {
    match name {
        "rounded" => ratatui::widgets::BorderType::Rounded,
        "thick" => ratatui::widgets::BorderType::Thick,
        "double" => ratatui::widgets::BorderType::Double,
        "plain" | "normal" => ratatui::widgets::BorderType::Plain,
        _ => ratatui::widgets::BorderType::Rounded,
    }
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
    use crate::vt::cell::{Color, Style};
    use crate::config::theme::{Theme, Rgb};
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
    fn to_tui_color_default_no_theme() {
        let c = to_tui_color(Color::Default, None);
        assert_eq!(c, TuiColor::Reset);
    }

    #[test]
    fn to_tui_color_default_with_theme() {
        let theme = test_theme();
        let c = to_tui_color(Color::Default, Some(&theme));
        match c {
            TuiColor::Rgb(_, _, _) => {}
            _ => panic!("expected Rgb"),
        }
    }

    #[test]
    fn to_tui_color_indexed_no_theme() {
        let c = to_tui_color(Color::Indexed(0), None);
        match c {
            TuiColor::Rgb(r, g, b) => assert_eq!((r, g, b), (0, 0, 0)),
            _ => panic!("expected Rgb"),
        }
    }

    #[test]
    fn to_tui_color_indexed_with_theme() {
        let theme = test_theme();
        let c = to_tui_color(Color::Indexed(1), Some(&theme));
        match c {
            TuiColor::Rgb(_, _, _) => {}
            _ => panic!("expected Rgb"),
        }
    }

    #[test]
    fn to_tui_color_rgb() {
        let c = to_tui_color(Color::Rgb(100, 200, 50), None);
        assert_eq!(c, TuiColor::Rgb(100, 200, 50));
    }

    #[test]
    fn to_tui_style_default() {
        let style = Style::new();
        let s = to_tui_style(style, None);
        assert_eq!(s.fg, Some(TuiColor::Reset));
    }

    #[test]
    fn to_tui_style_bold() {
        let mut style = Style::new();
        style.decoration.bold = true;
        let s = to_tui_style(style, None);
        assert!(s.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn to_tui_style_dim() {
        let mut style = Style::new();
        style.decoration.dim = true;
        let s = to_tui_style(style, None);
        assert!(s.add_modifier.contains(Modifier::DIM));
    }

    #[test]
    fn to_tui_style_italic() {
        let mut style = Style::new();
        style.decoration.italic = true;
        let s = to_tui_style(style, None);
        assert!(s.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn to_tui_style_underline() {
        let mut style = Style::new();
        style.decoration.underline = true;
        let s = to_tui_style(style, None);
        assert!(s.add_modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn to_tui_style_double_underline() {
        let mut style = Style::new();
        style.decoration.double_underline = true;
        let s = to_tui_style(style, None);
        assert!(s.add_modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn to_tui_style_reverse() {
        let mut style = Style::new();
        style.decoration.reverse = true;
        let s = to_tui_style(style, None);
        assert!(s.add_modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn to_tui_style_hidden() {
        let mut style = Style::new();
        style.decoration.hidden = true;
        let s = to_tui_style(style, None);
        assert!(s.add_modifier.contains(Modifier::HIDDEN));
    }

    #[test]
    fn to_tui_style_strikethrough() {
        let mut style = Style::new();
        style.decoration.strikethrough = true;
        let s = to_tui_style(style, None);
        assert!(s.add_modifier.contains(Modifier::CROSSED_OUT));
    }

    #[test]
    fn to_tui_style_with_bg() {
        let mut style = Style::new();
        style.bg = Color::Rgb(10, 20, 30);
        let s = to_tui_style(style, None);
        assert!(s.bg.is_some());
    }

    #[test]
    fn to_tui_style_default_bg_omitted() {
        let style = Style::new();
        let s = to_tui_style(style, None);
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
    fn border_type_thick() {
        assert_eq!(
            border_type("thick"),
            ratatui::widgets::BorderType::Thick
        );
    }

    #[test]
    fn border_type_double() {
        assert_eq!(
            border_type("double"),
            ratatui::widgets::BorderType::Double
        );
    }

    #[test]
    fn border_type_plain() {
        assert_eq!(
            border_type("plain"),
            ratatui::widgets::BorderType::Plain
        );
    }

    #[test]
    fn border_type_normal() {
        assert_eq!(
            border_type("normal"),
            ratatui::widgets::BorderType::Plain
        );
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
