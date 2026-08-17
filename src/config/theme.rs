//! Themes — built-in color themes and custom theme JSON, ported from TUIOS
//! `internal/theme`.

use serde::{Deserialize, Serialize};

/// A color as a 24-bit RGB triple.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Rgb(r, g, b)
    }

    /// Parse a hex color like `#RRGGBB` or `RRGGBB`.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().trim_start_matches('#');
        if s.len() != 6 {
            return None;
        }
        let r = u8::from_str_radix(&s[0..2], 16).ok()?;
        let g = u8::from_str_radix(&s[2..4], 16).ok()?;
        let b = u8::from_str_radix(&s[4..6], 16).ok()?;
        Some(Rgb(r, g, b))
    }
}

/// A terminal theme: 16 ANSI palette colors plus fg/bg/cursor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Theme {
    pub name: String,
    pub foreground: Rgb,
    pub background: Rgb,
    pub cursor: Rgb,
    /// The 16 ANSI palette slots (black, red, ..., bright white).
    pub ansi: [Rgb; 16],
}

impl Theme {
    pub fn default_ansi() -> [Rgb; 16] {
        [
            Rgb::new(0x1e, 0x1e, 0x2e), // black
            Rgb::new(0xf3, 0x8b, 0xa8), // red
            Rgb::new(0xa6, 0xe3, 0xa1), // green
            Rgb::new(0xf9, 0xe2, 0xaf), // yellow
            Rgb::new(0x89, 0xb4, 0xfa), // blue
            Rgb::new(0xf5, 0xc2, 0xe7), // magenta
            Rgb::new(0x94, 0xe2, 0xd5), // cyan
            Rgb::new(0xa6, 0xad, 0xc8), // white
            Rgb::new(0x58, 0x5b, 0x70), // bright black
            Rgb::new(0xf3, 0x8b, 0xa8), // bright red
            Rgb::new(0xa6, 0xe3, 0xa1), // bright green
            Rgb::new(0xf9, 0xe2, 0xaf), // bright yellow
            Rgb::new(0x89, 0xb4, 0xfa), // bright blue
            Rgb::new(0xf5, 0xc2, 0xe7), // bright magenta
            Rgb::new(0x94, 0xe2, 0xd5), // bright cyan
            Rgb::new(0xcd, 0xd6, 0xf4), // bright white
        ]
    }

    pub fn catppuccin_mocha() -> Self {
        let mut ansi = Self::default_ansi();
        ansi[0] = Rgb::new(0x1e, 0x1e, 0x2e);
        ansi[1] = Rgb::new(0xf3, 0x8b, 0xa8);
        ansi[2] = Rgb::new(0xa6, 0xe3, 0xa1);
        ansi[3] = Rgb::new(0xf9, 0xe2, 0xaf);
        ansi[4] = Rgb::new(0x89, 0xb4, 0xfa);
        ansi[5] = Rgb::new(0xf5, 0xc2, 0xe7);
        ansi[6] = Rgb::new(0x94, 0xe2, 0xd5);
        ansi[7] = Rgb::new(0xba, 0xc2, 0xde);
        ansi[8] = Rgb::new(0x58, 0x5b, 0x70);
        ansi[9] = Rgb::new(0xf3, 0x8b, 0xa8);
        ansi[10] = Rgb::new(0xa6, 0xe3, 0xa1);
        ansi[11] = Rgb::new(0xf9, 0xe2, 0xaf);
        ansi[12] = Rgb::new(0x89, 0xb4, 0xfa);
        ansi[13] = Rgb::new(0xf5, 0xc2, 0xe7);
        ansi[14] = Rgb::new(0x94, 0xe2, 0xd5);
        ansi[15] = Rgb::new(0xcd, 0xd6, 0xf4);
        Self {
            name: "catppuccin-mocha".into(),
            foreground: Rgb::new(0xcd, 0xd6, 0xf4),
            background: Rgb::new(0x1e, 0x1e, 0x2e),
            cursor: Rgb::new(0xf5, 0xe0, 0xdc),
            ansi,
        }
    }

    pub fn dracula() -> Self {
        let mut ansi = Self::default_ansi();
        ansi[0] = Rgb::new(0x28, 0x28, 0x36);
        ansi[1] = Rgb::new(0xff, 0x55, 0x55);
        ansi[2] = Rgb::new(0x50, 0xfa, 0x7b);
        ansi[3] = Rgb::new(0xf1, 0xfa, 0x8c);
        ansi[4] = Rgb::new(0xbd, 0x93, 0xf9);
        ansi[5] = Rgb::new(0xff, 0x79, 0xc6);
        ansi[6] = Rgb::new(0x8b, 0xe9, 0xfd);
        ansi[7] = Rgb::new(0xf8, 0xf8, 0xf2);
        ansi[8] = Rgb::new(0x62, 0x72, 0xa4);
        ansi[9] = Rgb::new(0xff, 0x6e, 0x6e);
        ansi[10] = Rgb::new(0x69, 0xff, 0x94);
        ansi[11] = Rgb::new(0xff, 0xff, 0xa5);
        ansi[12] = Rgb::new(0xd6, 0xac, 0xff);
        ansi[13] = Rgb::new(0xff, 0x92, 0xdf);
        ansi[14] = Rgb::new(0xa4, 0xff, 0xff);
        ansi[15] = Rgb::new(0xff, 0xff, 0xff);
        Self {
            name: "dracula".into(),
            foreground: Rgb::new(0xf8, 0xf8, 0xf2),
            background: Rgb::new(0x28, 0x28, 0x36),
            cursor: Rgb::new(0xf8, 0xf8, 0xf0),
            ansi,
        }
    }

    /// The built-in themes by name.
    pub fn built_in(name: &str) -> Option<Self> {
        match name {
            "catppuccin-mocha" | "catppuccin_mocha" => Some(Self::catppuccin_mocha()),
            "dracula" => Some(Self::dracula()),
            _ => None,
        }
    }

    /// The names of the built-in themes.
    pub fn built_in_names() -> Vec<&'static str> {
        vec!["catppuccin-mocha", "dracula"]
    }

    /// Convert this theme's ANSI palette to `vt::Color` values.
    pub fn to_vt_colors(&self) -> [crate::vt::Color; 16] {
        let mut out = [crate::vt::Color::Default; 16];
        for (i, c) in self.ansi.iter().enumerate() {
            out[i] = crate::vt::Color::Rgb(c.0, c.1, c.2);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex() {
        assert_eq!(Rgb::parse("#1e1e2e"), Some(Rgb::new(0x1e, 0x1e, 0x2e)));
        assert_eq!(Rgb::parse("1E1E2E"), Some(Rgb::new(0x1e, 0x1e, 0x2e)));
        assert_eq!(Rgb::parse("xyz"), None);
    }

    #[test]
    fn builtin_themes_exist() {
        assert!(Theme::built_in("dracula").is_some());
        assert!(Theme::built_in("catppuccin-mocha").is_some());
        assert!(Theme::built_in("nonexistent").is_none());
    }
}
