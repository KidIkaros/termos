//! Per-window accent colours and the accent picker model.
//!
//! Ported from Go TUIOS `internal/app/sidebar_accent.go`. Each window can
//! carry an accent colour — either a named ANSI slot (8 bright + 7 normal) or
//! a literal RGB. Accents are inherited by windows that don't define their
//! own, and the picker supports HSL editing and nearest-slot calculation.

use crate::config::theme::Rgb;

use super::{Accent, ACCENT_BRIGHT_COUNT, ACCENT_SWATCH_COUNT};

/// The accent swatch row labels, in order.
pub const SWATCH_LABELS: &[&str] = &[
    "bright black", "bright red", "bright green", "bright yellow",
    "bright blue", "bright purple", "bright cyan", "bright white",
    "red", "green", "yellow", "blue", "purple", "cyan", "white",
];

/// Resolve an accent to an RGB colour against a 16-entry ANSI palette.
pub fn resolve(accent: &Accent, palette: &[Rgb; 16]) -> Rgb {
    accent.rgb(palette)
}

/// Find the nearest accent slot index to a target colour.
pub fn nearest_slot(target: Rgb, palette: &[Rgb; 16]) -> i32 {
    super::accent_nearest_slot(target, palette)
}

/// Convert an RGB colour to HSL (hue 0-360, saturation 0-1, lightness 0-1).
pub fn rgb_to_hsl(c: Rgb) -> (f64, f64, f64) {
    let r = c.0 as f64 / 255.0;
    let g = c.1 as f64 / 255.0;
    let b = c.2 as f64 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if (max - min).abs() < 1e-10 {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h * 60.0, s, l)
}

/// Convert HSL to RGB.
pub fn hsl_to_rgb(h: f64, s: f64, l: f64) -> Rgb {
    if s.abs() < 1e-10 {
        let v = (l * 255.0).round() as u8;
        return Rgb::new(v, v, v);
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let hue_to_rgb = |p: f64, q: f64, t: f64| {
        let mut t = t;
        if t < 0.0 { t += 1.0; }
        if t > 1.0 { t -= 1.0; }
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 1.0 / 2.0 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    let h_norm = h / 360.0;
    let r = hue_to_rgb(p, q, h_norm + 1.0 / 3.0);
    let g = hue_to_rgb(p, q, h_norm);
    let b = hue_to_rgb(p, q, h_norm - 1.0 / 3.0);
    Rgb::new(
        (r * 255.0).round() as u8,
        (g * 255.0).round() as u8,
        (b * 255.0).round() as u8,
    )
}

/// Generate harmony colours (complementary, analogous, triadic) from a base
/// HSL hue.
pub fn harmony_colors(h: f64, s: f64, l: f64) -> [Rgb; 5] {
    let base = hsl_to_rgb(h, s, l);
    let comp = hsl_to_rgb((h + 180.0).rem_euclid(360.0), s, l);
    let analog1 = hsl_to_rgb((h + 30.0).rem_euclid(360.0), s, l);
    let analog2 = hsl_to_rgb((h + 330.0).rem_euclid(360.0), s, l);
    let triad = hsl_to_rgb((h + 120.0).rem_euclid(360.0), s, l);
    [base, comp, analog1, analog2, triad]
}

/// Relative luminance (for contrast checks), per WCAG 2.x.
pub fn relative_luminance(c: Rgb) -> f64 {
    let channel = |v: u8| {
        let v = v as f64 / 255.0;
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(c.0) + 0.7152 * channel(c.1) + 0.0722 * channel(c.2)
}

/// Contrast ratio between two colours (1.0 = identical, 21.0 = max).
pub fn contrast_ratio(a: Rgb, b: Rgb) -> f64 {
    let la = relative_luminance(a);
    let lb = relative_luminance(b);
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// Whether white or black text reads better on the given background.
pub fn best_text_color(bg: Rgb) -> Rgb {
    let white = Rgb::new(255, 255, 255);
    let black = Rgb::new(0, 0, 0);
    if contrast_ratio(bg, white) >= contrast_ratio(bg, black) {
        white
    } else {
        black
    }
}

/// The accent picker state.
#[derive(Debug, Clone, Default)]
pub struct AccentPicker {
    /// The window ID being edited.
    pub window_id: String,
    /// The current HSL hue (0-360).
    pub hue: f64,
    /// The current HSL saturation (0-1).
    pub saturation: f64,
    /// The current HSL lightness (0-1).
    pub lightness: f64,
    /// Whether the picker is in hex-edit mode.
    pub hex_edit: bool,
    /// The hex string being typed.
    pub hex_input: String,
    /// The currently selected harmony index (0 = base).
    pub harmony_index: usize,
}

impl AccentPicker {
    /// Create a picker for a window, initialised from an existing accent.
    pub fn new(window_id: impl Into<String>, accent: Option<&Accent>, palette: &[Rgb; 16]) -> Self {
        let (h, s, l) = match accent {
            Some(a) => rgb_to_hsl(a.rgb(palette)),
            None => rgb_to_hsl(palette[11]), // default blue
        };
        Self {
            window_id: window_id.into(),
            hue: h,
            saturation: s,
            lightness: l,
            hex_edit: false,
            hex_input: String::new(),
            harmony_index: 0,
        }
    }

    /// The current colour.
    pub fn rgb(&self) -> Rgb {
        hsl_to_rgb(self.hue, self.saturation, self.lightness)
    }

    /// The current accent.
    pub fn accent(&self) -> Accent {
        Accent::Rgb(self.rgb())
    }

    /// The nearest slot to the current colour.
    pub fn nearest_slot(&self, palette: &[Rgb; 16]) -> i32 {
        nearest_slot(self.rgb(), palette)
    }

    /// Move the hue by `delta` degrees (wrapping 0-360).
    pub fn shift_hue(&mut self, delta: f64) {
        self.harmony_index = 0;
        self.hue = (self.hue + delta).rem_euclid(360.0);
    }

    /// Adjust saturation by `delta` (clamped 0-1).
    pub fn shift_saturation(&mut self, delta: f64) {
        self.harmony_index = 0;
        self.saturation = (self.saturation + delta).clamp(0.0, 1.0);
    }

    /// Adjust lightness by `delta` (clamped 0-1).
    pub fn shift_lightness(&mut self, delta: f64) {
        self.harmony_index = 0;
        self.lightness = (self.lightness + delta).clamp(0.0, 1.0);
    }

    /// Snap to the nearest named slot.
    pub fn snap_to_nearest(&mut self, palette: &[Rgb; 16]) {
        let slot = self.nearest_slot(palette);
        let c = Accent::Slot(slot).rgb(palette);
        let (h, s, l) = rgb_to_hsl(c);
        self.hue = h;
        self.saturation = s;
        self.lightness = l;
    }

    /// Select a harmony colour.
    pub fn select_harmony(&mut self, index: usize) {
        self.harmony_index = index;
        let colors = harmony_colors(self.hue, self.saturation, self.lightness);
        if let Some(&c) = colors.get(index) {
            let (h, s, l) = rgb_to_hsl(c);
            self.hue = h;
            self.saturation = s;
            self.lightness = l;
        }
    }

    /// Begin hex editing with the current colour.
    pub fn begin_hex_edit(&mut self) {
        self.hex_edit = true;
        self.hex_input = format!(
            "#{:02x}{:02x}{:02x}",
            self.rgb().0,
            self.rgb().1,
            self.rgb().2
        );
    }

    /// Commit the hex edit, returning the parsed accent if valid.
    pub fn commit_hex_edit(&mut self) -> Option<Accent> {
        self.hex_edit = false;
        let accent = super::parse_accent(&self.hex_input);
        if let Some(ref a) = accent {
            let c = a.rgb(&[Rgb::new(0,0,0); 16]); // palette not needed for Rgb
            if let Accent::Rgb(rgb) = a {
                let (h, s, l) = rgb_to_hsl(*rgb);
                self.hue = h;
                self.saturation = s;
                self.lightness = l;
            }
            let _ = c;
        }
        accent
    }

    /// Cancel hex editing.
    pub fn cancel_hex_edit(&mut self) {
        self.hex_edit = false;
        self.hex_input.clear();
    }

    /// Type a character into the hex input.
    pub fn type_hex_char(&mut self, ch: char) {
        if self.hex_edit && self.hex_input.len() < 7 {
            let lower = ch.to_ascii_lowercase();
            if lower.is_ascii_hexdigit() || (lower == '#' && self.hex_input.is_empty()) {
                self.hex_input.push(lower);
            }
        }
    }

    /// Backspace in the hex input.
    pub fn hex_backspace(&mut self) {
        if self.hex_edit {
            self.hex_input.pop();
        }
    }
}

/// The accent payload: what gets saved per window.
#[derive(Debug, Clone, Default)]
pub struct AccentPayload {
    /// The slot index, if a named slot.
    pub slot: Option<i32>,
    /// The custom RGB, if a literal colour.
    pub rgb: Option<Rgb>,
}

impl AccentPayload {
    /// Build a payload from an accent.
    pub fn from_accent(accent: &Accent) -> Self {
        match accent {
            Accent::Slot(i) => Self {
                slot: Some(*i),
                rgb: None,
            },
            Accent::Rgb(c) => Self {
                slot: None,
                rgb: Some(*c),
            },
        }
    }

    /// Convert back to an accent.
    pub fn to_accent(&self) -> Option<Accent> {
        self.slot.map(Accent::Slot).or_else(|| self.rgb.map(Accent::Rgb))
    }
}

/// Effective accent for a window: its own, or inherited from its session.
pub fn effective_accent(
    window_accent: Option<&Accent>,
    session_accent: Option<&Accent>,
) -> Option<Accent> {
    window_accent
        .or(session_accent)
        .cloned()
}

/// Migrate an accent from one slot to another when a session is renamed.
pub fn migrate_accent(old_key: &str, new_key: &str, accents: &mut std::collections::HashMap<String, Accent>) {
    if let Some(a) = accents.remove(old_key) {
        accents.insert(new_key.to_string(), a);
    }
}

/// The number of swatches available in the picker.
pub fn swatch_count() -> i32 {
    ACCENT_SWATCH_COUNT
}

/// The number of bright swatches.
pub fn bright_count() -> i32 {
    ACCENT_BRIGHT_COUNT
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::theme::Theme;

    #[test]
    fn rgb_to_hsl_black() {
        let (h, s, l) = rgb_to_hsl(Rgb::new(0, 0, 0));
        assert_eq!(h, 0.0);
        assert_eq!(s, 0.0);
        assert_eq!(l, 0.0);
    }

    #[test]
    fn rgb_to_hsl_white() {
        let (h, s, l) = rgb_to_hsl(Rgb::new(255, 255, 255));
        assert_eq!(h, 0.0);
        assert_eq!(s, 0.0);
        assert_eq!(l, 1.0);
    }

    #[test]
    fn rgb_to_hsl_red() {
        let (h, _s, l) = rgb_to_hsl(Rgb::new(255, 0, 0));
        assert!((h - 0.0).abs() < 1e-6 || (h - 360.0).abs() < 1e-6);
        assert!((l - 0.5).abs() < 1e-6);
    }

    #[test]
    fn hsl_to_rgb_roundtrip() {
        let original = Rgb::new(100, 150, 200);
        let (h, s, l) = rgb_to_hsl(original);
        let result = hsl_to_rgb(h, s, l);
        assert!((result.0 as i32 - 100).abs() <= 1);
        assert!((result.1 as i32 - 150).abs() <= 1);
        assert!((result.2 as i32 - 200).abs() <= 1);
    }

    #[test]
    fn contrast_ratio_identical() {
        let c = Rgb::new(128, 128, 128);
        let r = contrast_ratio(c, c);
        assert!((r - 1.0).abs() < 1e-6);
    }

    #[test]
    fn contrast_ratio_black_white() {
        let r = contrast_ratio(Rgb::new(0, 0, 0), Rgb::new(255, 255, 255));
        assert!((r - 21.0).abs() < 0.1);
    }

    #[test]
    fn best_text_color_dark_bg() {
        let text = best_text_color(Rgb::new(30, 30, 30));
        assert_eq!(text, Rgb::new(255, 255, 255));
    }

    #[test]
    fn best_text_color_light_bg() {
        let text = best_text_color(Rgb::new(240, 240, 240));
        assert_eq!(text, Rgb::new(0, 0, 0));
    }

    #[test]
    fn accent_picker_new_from_accent() {
        let palette = Theme::default_ansi();
        let accent = Accent::Slot(4);
        let picker = AccentPicker::new("w1", Some(&accent), &palette);
        assert_eq!(picker.window_id, "w1");
    }

    #[test]
    fn accent_picker_shift_hue_wraps() {
        let palette = Theme::default_ansi();
        let mut picker = AccentPicker::new("w1", None, &palette);
        picker.hue = 350.0;
        picker.shift_hue(20.0);
        assert!((picker.hue - 10.0).abs() < 1e-6);
    }

    #[test]
    fn accent_picker_snap_to_nearest() {
        let palette = Theme::default_ansi();
        let mut picker = AccentPicker::new("w1", None, &palette);
        picker.snap_to_nearest(&palette);
        let slot = picker.nearest_slot(&palette);
        let slot_rgb = Accent::Slot(slot).rgb(&palette);
        assert_eq!(picker.rgb(), slot_rgb);
    }

    #[test]
    fn accent_payload_roundtrip_slot() {
        let accent = Accent::Slot(3);
        let payload = AccentPayload::from_accent(&accent);
        assert_eq!(payload.to_accent(), Some(accent));
    }

    #[test]
    fn accent_payload_roundtrip_rgb() {
        let accent = Accent::Rgb(Rgb::new(100, 200, 50));
        let payload = AccentPayload::from_accent(&accent);
        assert_eq!(payload.to_accent(), Some(accent));
    }

    #[test]
    fn effective_accent_inherits() {
        let session = Accent::Slot(2);
        assert_eq!(
            effective_accent(None, Some(&session)),
            Some(Accent::Slot(2))
        );
    }

    #[test]
    fn effective_accent_prefers_window() {
        let window = Accent::Slot(1);
        let session = Accent::Slot(2);
        assert_eq!(
            effective_accent(Some(&window), Some(&session)),
            Some(Accent::Slot(1))
        );
    }

    #[test]
    fn migrate_accent_moves_entry() {
        let mut accents = std::collections::HashMap::new();
        accents.insert("old".to_string(), Accent::Slot(1));
        migrate_accent("old", "new", &mut accents);
        assert!(!accents.contains_key("old"));
        assert_eq!(accents.get("new"), Some(&Accent::Slot(1)));
    }

    #[test]
    fn harmony_colors_produces_five() {
        let colors = harmony_colors(180.0, 0.5, 0.5);
        assert_eq!(colors.len(), 5);
    }

    #[test]
    fn nearest_slot_finds_bright_blue() {
        let palette = Theme::default_ansi();
        let slot = nearest_slot(palette[12], &palette); // bright blue
        assert_eq!(slot, 4);
    }
}
