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
}
