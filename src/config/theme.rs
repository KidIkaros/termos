//! Themes — built-in color themes and custom theme JSON, ported from TUIOS
//! `internal/theme`.

use std::path::PathBuf;

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

    /// WCAG-style relative luminance in `[0, 1]`.
    pub fn luminance(&self) -> f64 {
        let lin = |c: u8| {
            let c = f64::from(c) / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(self.0) + 0.7152 * lin(self.1) + 0.0722 * lin(self.2)
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

/// Macro to define a theme with minimal boilerplate.
macro_rules! theme {
    ($name:expr, $fg:expr, $bg:expr, $cursor:expr,
     $a0:expr, $a1:expr, $a2:expr, $a3:expr,
     $a4:expr, $a5:expr, $a6:expr, $a7:expr,
     $a8:expr, $a9:expr, $a10:expr, $a11:expr,
     $a12:expr, $a13:expr, $a14:expr, $a15:expr) => {
        Theme {
            name: $name.into(),
            foreground: $fg,
            background: $bg,
            cursor: $cursor,
            ansi: [
                $a0, $a1, $a2, $a3, $a4, $a5, $a6, $a7,
                $a8, $a9, $a10, $a11, $a12, $a13, $a14, $a15,
            ],
        }
    };
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

    /// 8 representative colors for the theme picker swatch.
    pub fn swatch(&self) -> [Rgb; 8] {
        [
            self.ansi[9],  // bright red
            self.ansi[11], // bright yellow
            self.ansi[10], // bright green
            self.ansi[14], // bright cyan
            self.ansi[12], // bright blue
            self.ansi[13], // bright magenta
            self.foreground,
            self.background,
        ]
    }

    pub fn catppuccin_mocha() -> Self {
        theme!(
            "catppuccin-mocha",
            Rgb::new(0xcd, 0xd6, 0xf4), Rgb::new(0x1e, 0x1e, 0x2e), Rgb::new(0xf5, 0xe0, 0xdc),
            Rgb::new(0x1e, 0x1e, 0x2e), Rgb::new(0xf3, 0x8b, 0xa8), Rgb::new(0xa6, 0xe3, 0xa1), Rgb::new(0xf9, 0xe2, 0xaf),
            Rgb::new(0x89, 0xb4, 0xfa), Rgb::new(0xf5, 0xc2, 0xe7), Rgb::new(0x94, 0xe2, 0xd5), Rgb::new(0xba, 0xc2, 0xde),
            Rgb::new(0x58, 0x5b, 0x70), Rgb::new(0xf3, 0x8b, 0xa8), Rgb::new(0xa6, 0xe3, 0xa1), Rgb::new(0xf9, 0xe2, 0xaf),
            Rgb::new(0x89, 0xb4, 0xfa), Rgb::new(0xf5, 0xc2, 0xe7), Rgb::new(0x94, 0xe2, 0xd5), Rgb::new(0xcd, 0xd6, 0xf4)
        )
    }

    pub fn catppuccin_frappe() -> Self {
        theme!(
            "catppuccin-frappe",
            Rgb::new(0xc6, 0xd0, 0xf5), Rgb::new(0x30, 0x34, 0x4a), Rgb::new(0xf2, 0xd5, 0xcf),
            Rgb::new(0x30, 0x34, 0x4a), Rgb::new(0xe7, 0x82, 0x84), Rgb::new(0xa6, 0xd1, 0x8e), Rgb::new(0xe5, 0xc8, 0x90),
            Rgb::new(0x81, 0xa1, 0xc1), Rgb::new(0xca, 0x9e, 0xe6), Rgb::new(0x8b, 0xc2, 0xbf), Rgb::new(0xb5, 0xbf, 0xea),
            Rgb::new(0x62, 0x68, 0x80), Rgb::new(0xe7, 0x82, 0x84), Rgb::new(0xa6, 0xd1, 0x8e), Rgb::new(0xe5, 0xc8, 0x90),
            Rgb::new(0x81, 0xa1, 0xc1), Rgb::new(0xca, 0x9e, 0xe6), Rgb::new(0x8b, 0xc2, 0xbf), Rgb::new(0xa5, 0xad, 0xce)
        )
    }

    pub fn catppuccin_macchiato() -> Self {
        theme!(
            "catppuccin-macchiato",
            Rgb::new(0xca, 0xd3, 0xf5), Rgb::new(0x24, 0x27, 0x3a), Rgb::new(0xf4, 0xdb, 0xd6),
            Rgb::new(0x24, 0x27, 0x3a), Rgb::new(0xed, 0x87, 0x96), Rgb::new(0xa6, 0xda, 0x95), Rgb::new(0xe5, 0xc8, 0x90),
            Rgb::new(0x7d, 0x91, 0xda), Rgb::new(0xc6, 0x78, 0xdd), Rgb::new(0x8b, 0xd5, 0xca), Rgb::new(0xb8, 0xc0, 0xe0),
            Rgb::new(0x5b, 0x60, 0x78), Rgb::new(0xed, 0x87, 0x96), Rgb::new(0xa6, 0xda, 0x95), Rgb::new(0xe5, 0xc8, 0x90),
            Rgb::new(0x7d, 0x91, 0xda), Rgb::new(0xc6, 0x78, 0xdd), Rgb::new(0x8b, 0xd5, 0xca), Rgb::new(0xb7, 0xc1, 0xe3)
        )
    }

    pub fn catppuccin_latte() -> Self {
        theme!(
            "catppuccin-latte",
            Rgb::new(0x4c, 0x4f, 0x69), Rgb::new(0xef, 0xe9, 0xd9), Rgb::new(0xdc, 0x8a, 0x78),
            Rgb::new(0xdc, 0xe0, 0xe8), Rgb::new(0xd2, 0x0f, 0x39), Rgb::new(0x40, 0xa0, 0x2b), Rgb::new(0xdf, 0x8e, 0x1d),
            Rgb::new(0x1e, 0x66, 0xf5), Rgb::new(0x88, 0x39, 0xbf), Rgb::new(0x17, 0x9b, 0x99), Rgb::new(0x5c, 0x5f, 0x77),
            Rgb::new(0x6c, 0x6f, 0x85), Rgb::new(0xd2, 0x0f, 0x39), Rgb::new(0x40, 0xa0, 0x2b), Rgb::new(0xdf, 0x8e, 0x1d),
            Rgb::new(0x1e, 0x66, 0xf5), Rgb::new(0x88, 0x39, 0xbf), Rgb::new(0x17, 0x9b, 0x99), Rgb::new(0xac, 0xbe, 0xe0)
        )
    }

    pub fn dracula() -> Self {
        theme!(
            "dracula",
            Rgb::new(0xf8, 0xf8, 0xf2), Rgb::new(0x28, 0x28, 0x36), Rgb::new(0xf8, 0xf8, 0xf0),
            Rgb::new(0x28, 0x28, 0x36), Rgb::new(0xff, 0x55, 0x55), Rgb::new(0x50, 0xfa, 0x7b), Rgb::new(0xf1, 0xfa, 0x8c),
            Rgb::new(0xbd, 0x93, 0xf9), Rgb::new(0xff, 0x79, 0xc6), Rgb::new(0x8b, 0xe9, 0xfd), Rgb::new(0xf8, 0xf8, 0xf2),
            Rgb::new(0x62, 0x72, 0xa4), Rgb::new(0xff, 0x6e, 0x6e), Rgb::new(0x69, 0xff, 0x94), Rgb::new(0xff, 0xff, 0xa5),
            Rgb::new(0xd6, 0xac, 0xff), Rgb::new(0xff, 0x92, 0xdf), Rgb::new(0xa4, 0xff, 0xff), Rgb::new(0xff, 0xff, 0xff)
        )
    }

    pub fn gruvbox_dark() -> Self {
        theme!(
            "gruvbox-dark",
            Rgb::new(0xeb, 0xdb, 0xb2), Rgb::new(0x28, 0x28, 0x28), Rgb::new(0xeb, 0xdb, 0xb2),
            Rgb::new(0x28, 0x28, 0x28), Rgb::new(0xcc, 0x24, 0x1d), Rgb::new(0x98, 0x97, 0x1a), Rgb::new(0xd7, 0x99, 0x21),
            Rgb::new(0x45, 0x85, 0x88), Rgb::new(0xb1, 0x62, 0x86), Rgb::new(0x68, 0x9d, 0x6a), Rgb::new(0xa8, 0x99, 0x84),
            Rgb::new(0x92, 0x83, 0x74), Rgb::new(0xfb, 0x49, 0x34), Rgb::new(0xb8, 0xbb, 0x26), Rgb::new(0xfa, 0xbd, 0x2f),
            Rgb::new(0x83, 0xa5, 0x98), Rgb::new(0xd3, 0x86, 0x9b), Rgb::new(0x8e, 0xc0, 0x7c), Rgb::new(0xeb, 0xdb, 0xb2)
        )
    }

    pub fn gruvbox_light() -> Self {
        theme!(
            "gruvbox-light",
            Rgb::new(0x3c, 0x38, 0x36), Rgb::new(0xfb, 0xf1, 0xc7), Rgb::new(0x3c, 0x38, 0x36),
            Rgb::new(0xfb, 0xf1, 0xc7), Rgb::new(0xcc, 0x24, 0x1d), Rgb::new(0x98, 0x97, 0x1a), Rgb::new(0xd7, 0x99, 0x21),
            Rgb::new(0x45, 0x85, 0x88), Rgb::new(0xb1, 0x62, 0x86), Rgb::new(0x68, 0x9d, 0x6a), Rgb::new(0x7c, 0x6f, 0x64),
            Rgb::new(0x92, 0x83, 0x74), Rgb::new(0x9d, 0x00, 0x06), Rgb::new(0x79, 0x74, 0x0e), Rgb::new(0xb5, 0x76, 0x14),
            Rgb::new(0x07, 0x66, 0x78), Rgb::new(0x8f, 0x3f, 0x71), Rgb::new(0x42, 0x7b, 0x58), Rgb::new(0x3c, 0x38, 0x36)
        )
    }

    pub fn nord() -> Self {
        theme!(
            "nord",
            Rgb::new(0xd8, 0xde, 0xe9), Rgb::new(0x2e, 0x34, 0x40), Rgb::new(0xd8, 0xde, 0xe9),
            Rgb::new(0x2e, 0x34, 0x40), Rgb::new(0xbf, 0x61, 0x6a), Rgb::new(0xa3, 0xbe, 0x8c), Rgb::new(0xeb, 0xcb, 0x8b),
            Rgb::new(0x81, 0xa1, 0xc1), Rgb::new(0xb4, 0x8e, 0xad), Rgb::new(0x88, 0xc0, 0xd0), Rgb::new(0xe5, 0xe9, 0xf0),
            Rgb::new(0x4c, 0x56, 0x6a), Rgb::new(0xbf, 0x61, 0x6a), Rgb::new(0xa3, 0xbe, 0x8c), Rgb::new(0xeb, 0xcb, 0x8b),
            Rgb::new(0x81, 0xa1, 0xc1), Rgb::new(0xb4, 0x8e, 0xad), Rgb::new(0x8f, 0xbc, 0xbb), Rgb::new(0xec, 0xef, 0xf4)
        )
    }

    pub fn tokyo_night() -> Self {
        theme!(
            "tokyo-night",
            Rgb::new(0xa9, 0xb1, 0xd6), Rgb::new(0x1a, 0x1b, 0x26), Rgb::new(0xc0, 0xca, 0xf5),
            Rgb::new(0x1a, 0x1b, 0x26), Rgb::new(0xf7, 0x76, 0x8e), Rgb::new(0x9e, 0xce, 0x6a), Rgb::new(0xe0, 0xaf, 0x68),
            Rgb::new(0x7a, 0xa2, 0xf7), Rgb::new(0xbb, 0x9a, 0xf7), Rgb::new(0x7d, 0xcf, 0xff), Rgb::new(0xac, 0xb0, 0xd0),
            Rgb::new(0x56, 0x5f, 0x89), Rgb::new(0xf7, 0x76, 0x8e), Rgb::new(0x9e, 0xce, 0x6a), Rgb::new(0xe0, 0xaf, 0x68),
            Rgb::new(0x7a, 0xa2, 0xf7), Rgb::new(0xbb, 0x9a, 0xf7), Rgb::new(0x7d, 0xcf, 0xff), Rgb::new(0xc0, 0xca, 0xf5)
        )
    }

    pub fn tokyo_night_storm() -> Self {
        theme!(
            "tokyo-night-storm",
            Rgb::new(0xa9, 0xb1, 0xd6), Rgb::new(0x24, 0x28, 0x3b), Rgb::new(0xc0, 0xca, 0xf5),
            Rgb::new(0x3b, 0x4e, 0x92), Rgb::new(0xf7, 0x76, 0x8e), Rgb::new(0x9e, 0xce, 0x6a), Rgb::new(0xe0, 0xaf, 0x68),
            Rgb::new(0x7a, 0xa2, 0xf7), Rgb::new(0xbb, 0x9a, 0xf7), Rgb::new(0x7d, 0xcf, 0xff), Rgb::new(0xac, 0xb0, 0xd0),
            Rgb::new(0x41, 0x46, 0x5e), Rgb::new(0xf7, 0x76, 0x8e), Rgb::new(0x9e, 0xce, 0x6a), Rgb::new(0xe0, 0xaf, 0x68),
            Rgb::new(0x7a, 0xa2, 0xf7), Rgb::new(0xbb, 0x9a, 0xf7), Rgb::new(0x7d, 0xcf, 0xff), Rgb::new(0xc0, 0xca, 0xf5)
        )
    }

    pub fn solarized_dark() -> Self {
        theme!(
            "solarized-dark",
            Rgb::new(0x93, 0xa1, 0xa1), Rgb::new(0x00, 0x2b, 0x36), Rgb::new(0x93, 0xa1, 0xa1),
            Rgb::new(0x07, 0x36, 0x42), Rgb::new(0xdc, 0x32, 0x2f), Rgb::new(0x85, 0x99, 0x00), Rgb::new(0xb5, 0x89, 0x00),
            Rgb::new(0x26, 0x8b, 0xd2), Rgb::new(0xd3, 0x36, 0x82), Rgb::new(0x2a, 0xa1, 0x98), Rgb::new(0xee, 0xe8, 0xd5),
            Rgb::new(0x00, 0x29, 0x4f), Rgb::new(0xcb, 0x4b, 0x16), Rgb::new(0x58, 0x6e, 0x75), Rgb::new(0x65, 0x7b, 0x83),
            Rgb::new(0x83, 0x94, 0x96), Rgb::new(0x6c, 0x71, 0xc4), Rgb::new(0x8a, 0xb1, 0xb1), Rgb::new(0xfd, 0xf6, 0xe3)
        )
    }

    pub fn solarized_light() -> Self {
        theme!(
            "solarized-light",
            Rgb::new(0x58, 0x6e, 0x75), Rgb::new(0xfd, 0xf6, 0xe3), Rgb::new(0x58, 0x6e, 0x75),
            Rgb::new(0xee, 0xe8, 0xd5), Rgb::new(0xdc, 0x32, 0x2f), Rgb::new(0x85, 0x99, 0x00), Rgb::new(0xb5, 0x89, 0x00),
            Rgb::new(0x26, 0x8b, 0xd2), Rgb::new(0xd3, 0x36, 0x82), Rgb::new(0x2a, 0xa1, 0x98), Rgb::new(0x07, 0x36, 0x42),
            Rgb::new(0x93, 0xa1, 0xa1), Rgb::new(0xcb, 0x4b, 0x16), Rgb::new(0x58, 0x6e, 0x75), Rgb::new(0x65, 0x7b, 0x83),
            Rgb::new(0x83, 0x94, 0x96), Rgb::new(0x6c, 0x71, 0xc4), Rgb::new(0x8a, 0xb1, 0xb1), Rgb::new(0xfd, 0xf6, 0xe3)
        )
    }

    pub fn one_dark() -> Self {
        theme!(
            "one-dark",
            Rgb::new(0xab, 0xb2, 0xbf), Rgb::new(0x28, 0x2c, 0x34), Rgb::new(0x52, 0x8b, 0xff),
            Rgb::new(0x28, 0x2c, 0x34), Rgb::new(0xe0, 0x6c, 0x75), Rgb::new(0x98, 0xc3, 0x79), Rgb::new(0xe5, 0xc0, 0x7b),
            Rgb::new(0x61, 0xaf, 0xef), Rgb::new(0xc6, 0x78, 0xdd), Rgb::new(0x56, 0xb6, 0xc2), Rgb::new(0xab, 0xb2, 0xbf),
            Rgb::new(0x5c, 0x63, 0x70), Rgb::new(0xe0, 0x6c, 0x75), Rgb::new(0x98, 0xc3, 0x79), Rgb::new(0xe5, 0xc0, 0x7b),
            Rgb::new(0x61, 0xaf, 0xef), Rgb::new(0xc6, 0x78, 0xdd), Rgb::new(0x56, 0xb6, 0xc2), Rgb::new(0xff, 0xff, 0xff)
        )
    }

    pub fn rose_pine() -> Self {
        theme!(
            "rose-pine",
            Rgb::new(0xe0, 0xdf, 0xf5), Rgb::new(0x19, 0x17, 0x24), Rgb::new(0xe0, 0xdf, 0xf5),
            Rgb::new(0x26, 0x23, 0x33), Rgb::new(0xeb, 0xbc, 0xba), Rgb::new(0x31, 0x73, 0x97), Rgb::new(0xf6, 0xc1, 0x77),
            Rgb::new(0x9c, 0xcf, 0xd8), Rgb::new(0xeb, 0xbc, 0xba), Rgb::new(0x31, 0x73, 0x97), Rgb::new(0xe0, 0xdf, 0xf5),
            Rgb::new(0x55, 0x51, 0x69), Rgb::new(0xeb, 0xbc, 0xba), Rgb::new(0x3e, 0x8f, 0xb0), Rgb::new(0xf6, 0xc1, 0x77),
            Rgb::new(0x9c, 0xcf, 0xd8), Rgb::new(0xeb, 0xbc, 0xba), Rgb::new(0x3e, 0x8f, 0xb0), Rgb::new(0xe0, 0xdf, 0xf5)
        )
    }

    pub fn rose_pine_dawn() -> Self {
        theme!(
            "rose-pine-dawn",
            Rgb::new(0x57, 0x62, 0x73), Rgb::new(0xfa, 0xf4, 0xed), Rgb::new(0x57, 0x62, 0x73),
            Rgb::new(0xf2, 0xe9, 0xe4), Rgb::new(0xb4, 0x63, 0x7a), Rgb::new(0x28, 0x69, 0x8b), Rgb::new(0xea, 0x9d, 0x34),
            Rgb::new(0x56, 0x94, 0x9f), Rgb::new(0xb4, 0x63, 0x7a), Rgb::new(0x28, 0x69, 0x8b), Rgb::new(0x57, 0x62, 0x73),
            Rgb::new(0xe5, 0xc9, 0xa3), Rgb::new(0xd9, 0x82, 0x7c), Rgb::new(0x2a, 0x7b, 0x9c), Rgb::new(0xee, 0xa7, 0x4f),
            Rgb::new(0x6a, 0xb3, 0xc3), Rgb::new(0xd9, 0x82, 0x7c), Rgb::new(0x2a, 0x7b, 0x9c), Rgb::new(0x57, 0x62, 0x73)
        )
    }

    pub fn monokai() -> Self {
        theme!(
            "monokai",
            Rgb::new(0xf8, 0xf8, 0xf2), Rgb::new(0x27, 0x28, 0x22), Rgb::new(0xf8, 0xf8, 0xf0),
            Rgb::new(0x27, 0x28, 0x22), Rgb::new(0xf9, 0x26, 0x72), Rgb::new(0xa6, 0xe2, 0x2e), Rgb::new(0xf4, 0xbf, 0x75),
            Rgb::new(0x66, 0xd9, 0xef), Rgb::new(0xae, 0x81, 0xff), Rgb::new(0xa1, 0xef, 0xe4), Rgb::new(0xf8, 0xf8, 0xf2),
            Rgb::new(0x75, 0x71, 0x5e), Rgb::new(0xf9, 0x26, 0x72), Rgb::new(0xa6, 0xe2, 0x2e), Rgb::new(0xf4, 0xbf, 0x75),
            Rgb::new(0x66, 0xd9, 0xef), Rgb::new(0xae, 0x81, 0xff), Rgb::new(0xa1, 0xef, 0xe4), Rgb::new(0xf9, 0xf8, 0xf5)
        )
    }

    pub fn everforest() -> Self {
        theme!(
            "everforest",
            Rgb::new(0xd3, 0xc6, 0xaa), Rgb::new(0x2d, 0x35, 0x3b), Rgb::new(0xd3, 0xc6, 0xaa),
            Rgb::new(0x2d, 0x35, 0x3b), Rgb::new(0xe6, 0x7e, 0x80), Rgb::new(0xa7, 0xc0, 0x80), Rgb::new(0xdb, 0xbc, 0x7f),
            Rgb::new(0x7f, 0xbb, 0xb3), Rgb::new(0xd6, 0x99, 0xb6), Rgb::new(0x83, 0xc0, 0x9c), Rgb::new(0xd3, 0xc6, 0xaa),
            Rgb::new(0x7a, 0x84, 0x7f), Rgb::new(0xe6, 0x7e, 0x80), Rgb::new(0xa7, 0xc0, 0x80), Rgb::new(0xdb, 0xbc, 0x7f),
            Rgb::new(0x7f, 0xbb, 0xb3), Rgb::new(0xd6, 0x99, 0xb6), Rgb::new(0x83, 0xc0, 0x9c), Rgb::new(0xed, 0xf0, 0xe0)
        )
    }

    pub fn kanagawa() -> Self {
        theme!(
            "kanagawa",
            Rgb::new(0xdc, 0xd7, 0xba), Rgb::new(0x1f, 0x1f, 0x28), Rgb::new(0xc8, 0xc0, 0x93),
            Rgb::new(0x0f, 0x0f, 0x14), Rgb::new(0xc3, 0x40, 0x43), Rgb::new(0x76, 0x94, 0x6a), Rgb::new(0xc0, 0xa3, 0x5e),
            Rgb::new(0x7e, 0x9c, 0xd8), Rgb::new(0x95, 0x7f, 0xb8), Rgb::new(0x6a, 0x9d, 0x89), Rgb::new(0xc8, 0xc0, 0x93),
            Rgb::new(0x52, 0x53, 0x5f), Rgb::new(0xe7, 0x57, 0x5a), Rgb::new(0x98, 0xbb, 0x6c), Rgb::new(0xe6, 0xc3, 0x84),
            Rgb::new(0x7e, 0xa9, 0xc0), Rgb::new(0xd5, 0x8f, 0xc0), Rgb::new(0x7d, 0xb8, 0xa3), Rgb::new(0xdc, 0xd7, 0xba)
        )
    }

    pub fn ayu_dark() -> Self {
        theme!(
            "ayu-dark",
            Rgb::new(0xb9, 0xbf, 0xca), Rgb::new(0x0b, 0x0e, 0x14), Rgb::new(0xff, 0xcc, 0x66),
            Rgb::new(0x01, 0x03, 0x06), Rgb::new(0xff, 0x33, 0x33), Rgb::new(0xba, 0xe6, 0x7e), Rgb::new(0xff, 0x80, 0x80),
            Rgb::new(0x59, 0xc2, 0xff), Rgb::new(0xff, 0x80, 0x80), Rgb::new(0x95, 0xe6, 0xcb), Rgb::new(0xb9, 0xbf, 0xca),
            Rgb::new(0x68, 0x71, 0x80), Rgb::new(0xf0, 0x73, 0x73), Rgb::new(0xba, 0xe6, 0x7e), Rgb::new(0xff, 0xa8, 0x59),
            Rgb::new(0x73, 0xb8, 0xff), Rgb::new(0xff, 0xa8, 0x59), Rgb::new(0x95, 0xe6, 0xcb), Rgb::new(0xff, 0xff, 0xff)
        )
    }

    pub fn ayu_light() -> Self {
        theme!(
            "ayu-light",
            Rgb::new(0x5c, 0x6f, 0x73), Rgb::new(0xfa, 0xfa, 0xfa), Rgb::new(0xff, 0x99, 0x40),
            Rgb::new(0xf0, 0xee, 0xe6), Rgb::new(0xf0, 0x73, 0x73), Rgb::new(0x86, 0xb3, 0x00), Rgb::new(0xff, 0xaa, 0x11),
            Rgb::new(0x4d, 0x9b, 0xff), Rgb::new(0xff, 0x80, 0x80), Rgb::new(0x4d, 0xb6, 0x99), Rgb::new(0x5c, 0x6f, 0x73),
            Rgb::new(0xab, 0xb0, 0xb6), Rgb::new(0xf0, 0x73, 0x73), Rgb::new(0x86, 0xb3, 0x00), Rgb::new(0xff, 0xaa, 0x11),
            Rgb::new(0x4d, 0x9b, 0xff), Rgb::new(0xff, 0x80, 0x80), Rgb::new(0x4d, 0xb6, 0x99), Rgb::new(0x5c, 0x6f, 0x73)
        )
    }

    pub fn atom_one_light() -> Self {
        theme!(
            "atom-one-light",
            Rgb::new(0x38, 0x3a, 0x42), Rgb::new(0xfa, 0xfa, 0xfa), Rgb::new(0x55, 0x55, 0x55),
            Rgb::new(0xfa, 0xfa, 0xfa), Rgb::new(0xe4, 0x5c, 0x57), Rgb::new(0x50, 0xa1, 0x4f), Rgb::new(0xc1, 0x8b, 0x1f),
            Rgb::new(0x40, 0x78, 0xf2), Rgb::new(0xa6, 0x26, 0xa4), Rgb::new(0x01, 0x87, 0x9a), Rgb::new(0x38, 0x3a, 0x42),
            Rgb::new(0xc0, 0xc0, 0xc0), Rgb::new(0xe0, 0x6c, 0x75), Rgb::new(0x98, 0xc3, 0x79), Rgb::new(0xe5, 0xc0, 0x7b),
            Rgb::new(0x61, 0xaf, 0xef), Rgb::new(0xc6, 0x78, 0xdd), Rgb::new(0x56, 0xb6, 0xc2), Rgb::new(0x20, 0x20, 0x20)
        )
    }

    /// The built-in themes by name.
    pub fn built_in(name: &str) -> Option<Self> {
        match name {
            "catppuccin-mocha" | "catppuccin_mocha" => Some(Self::catppuccin_mocha()),
            "catppuccin-frappe" | "catppuccin_frappe" => Some(Self::catppuccin_frappe()),
            "catppuccin-macchiato" | "catppuccin_macchiato" => Some(Self::catppuccin_macchiato()),
            "catppuccin-latte" | "catppuccin_latte" => Some(Self::catppuccin_latte()),
            "dracula" => Some(Self::dracula()),
            "gruvbox-dark" | "gruvbox_dark" => Some(Self::gruvbox_dark()),
            "gruvbox-light" | "gruvbox_light" => Some(Self::gruvbox_light()),
            "nord" => Some(Self::nord()),
            "tokyo-night" | "tokyo_night" => Some(Self::tokyo_night()),
            "tokyo-night-storm" | "tokyo_night_storm" => Some(Self::tokyo_night_storm()),
            "solarized-dark" | "solarized_dark" => Some(Self::solarized_dark()),
            "solarized-light" | "solarized_light" => Some(Self::solarized_light()),
            "one-dark" | "one_dark" => Some(Self::one_dark()),
            "rose-pine" | "rose_pine" => Some(Self::rose_pine()),
            "rose-pine-dawn" | "rose_pine_dawn" => Some(Self::rose_pine_dawn()),
            "monokai" => Some(Self::monokai()),
            "everforest" => Some(Self::everforest()),
            "kanagawa" => Some(Self::kanagawa()),
            "ayu-dark" | "ayu_dark" => Some(Self::ayu_dark()),
            "ayu-light" | "ayu_light" => Some(Self::ayu_light()),
            "atom-one-light" | "atom_one_light" => Some(Self::atom_one_light()),
            _ => None,
        }
    }

    /// The names of the built-in themes, sorted alphabetically.
    pub fn built_in_names() -> Vec<&'static str> {
        let mut names = vec![
            "atom-one-light",
            "ayu-dark",
            "ayu-light",
            "catppuccin-frappe",
            "catppuccin-latte",
            "catppuccin-macchiato",
            "catppuccin-mocha",
            "dracula",
            "everforest",
            "gruvbox-dark",
            "gruvbox-light",
            "kanagawa",
            "monokai",
            "nord",
            "one-dark",
            "rose-pine",
            "rose-pine-dawn",
            "solarized-dark",
            "solarized-light",
            "tokyo-night",
            "tokyo-night-storm",
        ];
        names.sort();
        names
    }

    /// Whether this theme's background reads as light (for `theme = "auto"`
    /// pairing and picker annotations).
    pub fn is_light(&self) -> bool {
        self.background.luminance() > 0.5
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

/// Theme-aware color accessor trait.
///
/// Every method returns a `TuiColor` (ratatui `Color`) suitable for direct use
/// in `Style::default().fg(...)`. When a theme is active, colors are derived
/// from the theme's ANSI palette. When no theme is active, hardcoded defaults
/// are returned. Border overrides from config take precedence over theme
/// colors.
pub trait ThemeColors {
    // --- Borders ---
    fn border_focused_window(&self) -> ratatui::style::Color;
    fn border_focused_terminal(&self) -> ratatui::style::Color;
    fn border_unfocused(&self) -> ratatui::style::Color;

    // --- Dock ---
    fn dock_bg(&self) -> ratatui::style::Color;
    fn dock_fg(&self) -> ratatui::style::Color;
    fn dock_accent(&self) -> ratatui::style::Color;
    fn dock_dimmed(&self) -> ratatui::style::Color;
    fn dock_separator(&self) -> ratatui::style::Color;
    fn dock_highlight_bg(&self) -> ratatui::style::Color;
    fn dock_highlight_fg(&self) -> ratatui::style::Color;

    // --- Notifications ---
    fn notification_error(&self) -> ratatui::style::Color;
    fn notification_warning(&self) -> ratatui::style::Color;
    fn notification_success(&self) -> ratatui::style::Color;
    fn notification_info(&self) -> ratatui::style::Color;
    fn notification_bg(&self) -> ratatui::style::Color;
    fn notification_fg(&self) -> ratatui::style::Color;

    // --- Copy mode ---
    fn copy_mode_cursor_bg(&self) -> ratatui::style::Color;
    fn copy_mode_cursor_fg(&self) -> ratatui::style::Color;
    fn copy_mode_visual_bg(&self) -> ratatui::style::Color;
    fn copy_mode_visual_fg(&self) -> ratatui::style::Color;
    fn copy_mode_search_current_bg(&self) -> ratatui::style::Color;
    fn copy_mode_search_current_fg(&self) -> ratatui::style::Color;
    fn copy_mode_search_other_bg(&self) -> ratatui::style::Color;
    fn copy_mode_search_other_fg(&self) -> ratatui::style::Color;

    // --- Help / overlays ---
    fn help_key_badge(&self) -> ratatui::style::Color;
    fn help_gray(&self) -> ratatui::style::Color;
    fn help_border(&self) -> ratatui::style::Color;
    fn help_tab_active(&self) -> ratatui::style::Color;
    fn overlay_title(&self) -> ratatui::style::Color;
    fn overlay_border(&self) -> ratatui::style::Color;
    fn overlay_bg(&self) -> ratatui::style::Color;
    fn overlay_fg(&self) -> ratatui::style::Color;

    // --- Sidebar ---
    fn sidebar_accent(&self) -> ratatui::style::Color;
    fn sidebar_dimmed(&self) -> ratatui::style::Color;

    // --- Scrollbar ---
    fn scrollbar_thumb(&self) -> ratatui::style::Color;

    // --- Terminal ---
    fn terminal_cursor(&self) -> ratatui::style::Color;
}

impl ThemeColors for Option<Theme> {
    #[inline]
    fn border_focused_window(&self) -> ratatui::style::Color {
        if let Some(t) = self {
            ratatui::style::Color::Rgb(t.ansi[12].0, t.ansi[12].1, t.ansi[12].2)
        } else {
            ratatui::style::Color::Blue
        }
    }

    #[inline]
    fn border_focused_terminal(&self) -> ratatui::style::Color {
        if let Some(t) = self {
            ratatui::style::Color::Rgb(t.ansi[4].0, t.ansi[4].1, t.ansi[4].2)
        } else {
            ratatui::style::Color::Blue
        }
    }

    #[inline]
    fn border_unfocused(&self) -> ratatui::style::Color {
        if let Some(t) = self {
            ratatui::style::Color::Rgb(t.ansi[8].0, t.ansi[8].1, t.ansi[8].2)
        } else {
            ratatui::style::Color::DarkGray
        }
    }

    #[inline]
    fn dock_bg(&self) -> ratatui::style::Color {
        if let Some(t) = self {
            ratatui::style::Color::Rgb(t.ansi[0].0, t.ansi[0].1, t.ansi[0].2)
        } else {
            ratatui::style::Color::DarkGray
        }
    }

    #[inline]
    fn dock_fg(&self) -> ratatui::style::Color {
        if let Some(t) = self {
            ratatui::style::Color::Rgb(t.foreground.0, t.foreground.1, t.foreground.2)
        } else {
            ratatui::style::Color::White
        }
    }

    #[inline]
    fn dock_accent(&self) -> ratatui::style::Color {
        if let Some(t) = self {
            ratatui::style::Color::Rgb(t.ansi[4].0, t.ansi[4].1, t.ansi[4].2)
        } else {
            ratatui::style::Color::Cyan
        }
    }

    #[inline]
    fn dock_dimmed(&self) -> ratatui::style::Color {
        if let Some(t) = self {
            ratatui::style::Color::Rgb(t.ansi[8].0, t.ansi[8].1, t.ansi[8].2)
        } else {
            ratatui::style::Color::DarkGray
        }
    }

    #[inline]
    fn dock_separator(&self) -> ratatui::style::Color {
        if let Some(t) = self {
            ratatui::style::Color::Rgb(t.ansi[7].0, t.ansi[7].1, t.ansi[7].2)
        } else {
            ratatui::style::Color::Gray
        }
    }

    #[inline]
    fn dock_highlight_bg(&self) -> ratatui::style::Color {
        if let Some(t) = self {
            ratatui::style::Color::Rgb(t.ansi[4].0, t.ansi[4].1, t.ansi[4].2)
        } else {
            ratatui::style::Color::Blue
        }
    }

    #[inline]
    fn dock_highlight_fg(&self) -> ratatui::style::Color {
        if let Some(t) = self {
            ratatui::style::Color::Rgb(t.background.0, t.background.1, t.background.2)
        } else {
            ratatui::style::Color::White
        }
    }

    #[inline]
    fn notification_error(&self) -> ratatui::style::Color {
        if let Some(t) = self {
            ratatui::style::Color::Rgb(t.ansi[1].0, t.ansi[1].1, t.ansi[1].2)
        } else {
            ratatui::style::Color::Red
        }
    }

    #[inline]
    fn notification_warning(&self) -> ratatui::style::Color {
        if let Some(t) = self {
            ratatui::style::Color::Rgb(t.ansi[3].0, t.ansi[3].1, t.ansi[3].2)
        } else {
            ratatui::style::Color::Yellow
        }
    }

    #[inline]
    fn notification_success(&self) -> ratatui::style::Color {
        if let Some(t) = self {
            ratatui::style::Color::Rgb(t.ansi[2].0, t.ansi[2].1, t.ansi[2].2)
        } else {
            ratatui::style::Color::Green
        }
    }

    #[inline]
    fn notification_info(&self) -> ratatui::style::Color {
        if let Some(t) = self {
            ratatui::style::Color::Rgb(t.ansi[6].0, t.ansi[6].1, t.ansi[6].2)
        } else {
            ratatui::style::Color::Cyan
        }
    }

    #[inline]
    fn notification_bg(&self) -> ratatui::style::Color {
        if let Some(t) = self {
            ratatui::style::Color::Rgb(t.ansi[0].0, t.ansi[0].1, t.ansi[0].2)
        } else {
            ratatui::style::Color::Black
        }
    }

    #[inline]
    fn notification_fg(&self) -> ratatui::style::Color {
        if let Some(t) = self {
            ratatui::style::Color::Rgb(t.foreground.0, t.foreground.1, t.foreground.2)
        } else {
            ratatui::style::Color::White
        }
    }

    #[inline]
    fn copy_mode_cursor_bg(&self) -> ratatui::style::Color {
        if let Some(t) = self {
            ratatui::style::Color::Rgb(t.ansi[6].0, t.ansi[6].1, t.ansi[6].2)
        } else {
            ratatui::style::Color::Cyan
        }
    }

    #[inline]
    fn copy_mode_cursor_fg(&self) -> ratatui::style::Color {
        if let Some(t) = self {
            ratatui::style::Color::Rgb(t.background.0, t.background.1, t.background.2)
        } else {
            ratatui::style::Color::Black
        }
    }

    #[inline]
    fn copy_mode_visual_bg(&self) -> ratatui::style::Color {
        if let Some(t) = self {
            ratatui::style::Color::Rgb(t.ansi[5].0, t.ansi[5].1, t.ansi[5].2)
        } else {
            ratatui::style::Color::Magenta
        }
    }

    #[inline]
    fn copy_mode_visual_fg(&self) -> ratatui::style::Color {
        if let Some(t) = self {
            ratatui::style::Color::Rgb(t.foreground.0, t.foreground.1, t.foreground.2)
        } else {
            ratatui::style::Color::White
        }
    }

    #[inline]
    fn copy_mode_search_current_bg(&self) -> ratatui::style::Color {
        if let Some(t) = self {
            ratatui::style::Color::Rgb(t.ansi[5].0, t.ansi[5].1, t.ansi[5].2)
        } else {
            ratatui::style::Color::Magenta
        }
    }

    #[inline]
    fn copy_mode_search_current_fg(&self) -> ratatui::style::Color {
        if let Some(t) = self {
            ratatui::style::Color::Rgb(t.background.0, t.background.1, t.background.2)
        } else {
            ratatui::style::Color::Black
        }
    }

    #[inline]
    fn copy_mode_search_other_bg(&self) -> ratatui::style::Color {
        if let Some(t) = self {
            ratatui::style::Color::Rgb(t.ansi[3].0, t.ansi[3].1, t.ansi[3].2)
        } else {
            ratatui::style::Color::Yellow
        }
    }

    #[inline]
    fn copy_mode_search_other_fg(&self) -> ratatui::style::Color {
        if let Some(t) = self {
            ratatui::style::Color::Rgb(t.background.0, t.background.1, t.background.2)
        } else {
            ratatui::style::Color::Black
        }
    }

    #[inline]
    fn help_key_badge(&self) -> ratatui::style::Color {
        if let Some(t) = self {
            ratatui::style::Color::Rgb(t.ansi[4].0, t.ansi[4].1, t.ansi[4].2)
        } else {
            ratatui::style::Color::Blue
        }
    }

    #[inline]
    fn help_gray(&self) -> ratatui::style::Color {
        if let Some(t) = self {
            ratatui::style::Color::Rgb(t.ansi[8].0, t.ansi[8].1, t.ansi[8].2)
        } else {
            ratatui::style::Color::DarkGray
        }
    }

    #[inline]
    fn help_border(&self) -> ratatui::style::Color {
        if let Some(t) = self {
            ratatui::style::Color::Rgb(t.ansi[8].0, t.ansi[8].1, t.ansi[8].2)
        } else {
            ratatui::style::Color::DarkGray
        }
    }

    #[inline]
    fn help_tab_active(&self) -> ratatui::style::Color {
        if let Some(t) = self {
            ratatui::style::Color::Rgb(t.ansi[4].0, t.ansi[4].1, t.ansi[4].2)
        } else {
            ratatui::style::Color::Cyan
        }
    }

    #[inline]
    fn overlay_title(&self) -> ratatui::style::Color {
        if let Some(t) = self {
            ratatui::style::Color::Rgb(t.ansi[4].0, t.ansi[4].1, t.ansi[4].2)
        } else {
            ratatui::style::Color::Cyan
        }
    }

    #[inline]
    fn overlay_border(&self) -> ratatui::style::Color {
        if let Some(t) = self {
            ratatui::style::Color::Rgb(t.ansi[8].0, t.ansi[8].1, t.ansi[8].2)
        } else {
            ratatui::style::Color::DarkGray
        }
    }

    #[inline]
    fn overlay_bg(&self) -> ratatui::style::Color {
        if let Some(t) = self {
            ratatui::style::Color::Rgb(t.ansi[0].0, t.ansi[0].1, t.ansi[0].2)
        } else {
            ratatui::style::Color::Black
        }
    }

    #[inline]
    fn overlay_fg(&self) -> ratatui::style::Color {
        if let Some(t) = self {
            ratatui::style::Color::Rgb(t.foreground.0, t.foreground.1, t.foreground.2)
        } else {
            ratatui::style::Color::White
        }
    }

    #[inline]
    fn sidebar_accent(&self) -> ratatui::style::Color {
        if let Some(t) = self {
            ratatui::style::Color::Rgb(t.ansi[4].0, t.ansi[4].1, t.ansi[4].2)
        } else {
            ratatui::style::Color::Cyan
        }
    }

    #[inline]
    fn sidebar_dimmed(&self) -> ratatui::style::Color {
        if let Some(t) = self {
            ratatui::style::Color::Rgb(t.ansi[8].0, t.ansi[8].1, t.ansi[8].2)
        } else {
            ratatui::style::Color::DarkGray
        }
    }

    #[inline]
    fn scrollbar_thumb(&self) -> ratatui::style::Color {
        if let Some(t) = self {
            ratatui::style::Color::Rgb(t.ansi[7].0, t.ansi[7].1, t.ansi[7].2)
        } else {
            ratatui::style::Color::Gray
        }
    }

    #[inline]
    fn terminal_cursor(&self) -> ratatui::style::Color {
        if let Some(t) = self {
            ratatui::style::Color::Rgb(t.cursor.0, t.cursor.1, t.cursor.2)
        } else {
            ratatui::style::Color::White
        }
    }
}

/// Parse a hex color string like `#rrggbb` into an `Rgb`.
pub fn parse_hex(s: &str) -> Option<Rgb> {
    Rgb::parse(s)
}

/// The directory where custom theme JSON files live:
/// `~/.config/termos/themes/`.
pub fn themes_dir() -> Option<PathBuf> {
    let config = dirs::config_dir()?;
    Some(config.join("termos").join("themes"))
}

/// Load every custom theme from `~/.config/termos/themes/*.json`.
///
/// Each file must match the JSON shape: an object with `name` (string),
/// `foreground`, `background`, `cursor` (hex color strings like `#rrggbb`),
/// and `ansi` (an array of exactly 16 hex color strings).
///
/// Invalid files log a warning and are skipped; the function never fails.
pub fn load_custom_themes() -> Vec<Theme> {
    let Some(dir) = themes_dir() else {
        return Vec::new();
    };
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    let mut themes = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        match load_theme_file(&path) {
            Ok(theme) => themes.push(theme),
            Err(e) => {
                log::warn!("theme: failed to load {}: {e}", path.display());
            }
        }
    }
    themes
}

/// Read and parse a single theme JSON file.
fn load_theme_file(path: &PathBuf) -> Result<Theme, Box<dyn std::error::Error>> {
    let data = std::fs::read_to_string(path)?;
    let raw: ThemeJson = serde_json::from_str(&data)?;
    raw.into_theme()
}

/// The on-disk JSON representation of a theme.
#[derive(Debug, Deserialize)]
struct ThemeJson {
    name: String,
    foreground: String,
    background: String,
    cursor: String,
    ansi: Vec<String>,
}

impl ThemeJson {
    fn into_theme(self) -> Result<Theme, Box<dyn std::error::Error>> {
        let foreground = parse_hex(&self.foreground)
            .ok_or_else(|| format!("invalid foreground: {}", self.foreground))?;
        let background = parse_hex(&self.background)
            .ok_or_else(|| format!("invalid background: {}", self.background))?;
        let cursor =
            parse_hex(&self.cursor).ok_or_else(|| format!("invalid cursor: {}", self.cursor))?;
        if self.ansi.len() != 16 {
            return Err(format!("ansi must have 16 entries, got {}", self.ansi.len()).into());
        }
        let mut ansi = [Rgb::new(0, 0, 0); 16];
        for (i, hex) in self.ansi.iter().enumerate() {
            ansi[i] = parse_hex(hex).ok_or_else(|| format!("invalid ansi[{}]: {}", i, hex))?;
        }
        Ok(Theme {
            name: self.name,
            foreground,
            background,
            cursor,
            ansi,
        })
    }
}

/// All available themes: built-ins first, then custom themes from disk.
pub fn all_themes() -> Vec<Theme> {
    let mut themes: Vec<Theme> = Theme::built_in_names()
        .iter()
        .filter_map(|name| Theme::built_in(name))
        .collect();
    themes.extend(load_custom_themes());
    themes
}

/// List the names of all available themes (built-in + custom).
pub fn list_theme_names() -> Vec<String> {
    all_themes().into_iter().map(|t| t.name).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb_parse() {
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

    #[test]
    fn parse_hex_helper() {
        assert_eq!(parse_hex("#ff0000"), Some(Rgb::new(0xff, 0x00, 0x00)));
        assert_eq!(parse_hex("00ff00"), Some(Rgb::new(0x00, 0xff, 0x00)));
        assert_eq!(parse_hex("#gggggg"), None);
    }

    #[test]
    fn theme_json_roundtrip() {
        let json = r##"{
            "name": "test-custom",
            "foreground": "#cdd6f4",
            "background": "#1e1e2e",
            "cursor": "#f5e0dc",
            "ansi": [
                "#1e1e2e","#f38ba8","#a6e3a1","#f9e2af",
                "#89b4fa","#f5c2e7","#94e2d5","#bac2de",
                "#585b70","#f38ba8","#a6e3a1","#f9e2af",
                "#89b4fa","#f5c2e7","#94e2d5","#cdd6f4"
            ]
        }"##;
        let raw: ThemeJson = serde_json::from_str(json).unwrap();
        let theme = raw.into_theme().unwrap();
        assert_eq!(theme.name, "test-custom");
        assert_eq!(theme.foreground, Rgb::new(0xcd, 0xd6, 0xf4));
        assert_eq!(theme.background, Rgb::new(0x1e, 0x1e, 0x2e));
        assert_eq!(theme.ansi.len(), 16);
        assert_eq!(theme.ansi[1], Rgb::new(0xf3, 0x8b, 0xa8));
    }

    #[test]
    fn theme_json_bad_ansi_count() {
        let json = r##"{
            "name": "bad",
            "foreground": "#000000",
            "background": "#000000",
            "cursor": "#000000",
            "ansi": ["#000000","#000000"]
        }"##;
        let raw: ThemeJson = serde_json::from_str(json).unwrap();
        assert!(raw.into_theme().is_err());
    }

    #[test]
    fn all_themes_includes_builtins() {
        let themes = all_themes();
        let names: Vec<&str> = themes.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"catppuccin-mocha"));
        assert!(names.contains(&"dracula"));
    }

    #[test]
    fn all_built_in_themes_load() {
        for name in Theme::built_in_names() {
            assert!(Theme::built_in(name).is_some(), "missing theme: {name}");
        }
    }

    #[test]
    fn built_in_names_sorted() {
        let names = Theme::built_in_names();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
    }

    #[test]
    fn built_in_names_has_20_plus() {
        assert!(Theme::built_in_names().len() >= 20, "expected 20+ themes");
    }

    #[test]
    fn swatch_returns_8_colors() {
        let theme = Theme::dracula();
        let swatch = theme.swatch();
        assert_eq!(swatch.len(), 8);
    }

    #[test]
    fn theme_colors_trait_with_theme() {
        let theme: Option<Theme> = Some(Theme::dracula());
        // Should return Rgb colors from the theme.
        let bg = theme.dock_bg();
        assert!(matches!(bg, ratatui::style::Color::Rgb(_, _, _)));
        let fg = theme.dock_fg();
        assert!(matches!(fg, ratatui::style::Color::Rgb(_, _, _)));
    }

    #[test]
    fn theme_colors_trait_without_theme() {
        let theme: Option<Theme> = None;
        // Should return hardcoded defaults.
        assert_eq!(theme.dock_bg(), ratatui::style::Color::DarkGray);
        assert_eq!(theme.dock_fg(), ratatui::style::Color::White);
        assert_eq!(theme.dock_accent(), ratatui::style::Color::Cyan);
        assert_eq!(theme.border_unfocused(), ratatui::style::Color::DarkGray);
        assert_eq!(theme.notification_error(), ratatui::style::Color::Red);
    }

    #[test]
    fn theme_colors_copy_mode() {
        let theme: Option<Theme> = Some(Theme::catppuccin_mocha());
        let cursor_bg = theme.copy_mode_cursor_bg();
        assert!(matches!(cursor_bg, ratatui::style::Color::Rgb(_, _, _)));
        let visual_bg = theme.copy_mode_visual_bg();
        assert!(matches!(visual_bg, ratatui::style::Color::Rgb(_, _, _)));
    }

    #[test]
    fn catppuccin_variants_distinct() {
        let mocha = Theme::catppuccin_mocha();
        let frappe = Theme::catppuccin_frappe();
        let macchiato = Theme::catppuccin_macchiato();
        let latte = Theme::catppuccin_latte();
        assert_ne!(mocha.background, frappe.background);
        assert_ne!(frappe.background, macchiato.background);
        assert_ne!(macchiato.background, latte.background);
        assert_ne!(mocha.background, latte.background);
    }

    #[test]
    fn gruvbox_dark_and_light_distinct() {
        let dark = Theme::gruvbox_dark();
        let light = Theme::gruvbox_light();
        assert_ne!(dark.background, light.background);
        assert_ne!(dark.foreground, light.foreground);
    }

    #[test]
    fn tokyo_night_variants_distinct() {
        let night = Theme::tokyo_night();
        let storm = Theme::tokyo_night_storm();
        assert_ne!(night.background, storm.background);
    }
}
