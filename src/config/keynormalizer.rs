//! Key normalization — ported from Go TUIOS `internal/config/keynormalizer.go`.
//!
//! Normalizes `opt+` to `alt+` on macOS and provides platform-aware key
//! expansion. The `normalize_key` function is the primary entry point for
//! converting a single key string to its canonical form.

use std::collections::HashMap;

/// Returns `true` if `s` is exactly one Unicode rune and that rune is a letter.
/// Single-letter keys must preserve case (m and M are distinct keys); compound
/// keys are lowercased.
fn is_single_rune_letter(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(r) = chars.next() else {
        return false;
    };
    chars.next().is_none() && r.is_alphabetic()
}

/// Detect whether the current platform is macOS.
fn detect_macos() -> bool {
    cfg!(target_os = "macos")
}

/// macOS Option+number → composed unicode glyph.
fn mac_option_number_map() -> HashMap<&'static str, &'static str> {
    let mut m = HashMap::new();
    m.insert("opt+1", "\u{00A1}");
    m.insert("option+1", "\u{00A1}");
    m.insert("opt+2", "\u{2122}");
    m.insert("option+2", "\u{2122}");
    m.insert("opt+3", "\u{00A3}");
    m.insert("option+3", "\u{00A3}");
    m.insert("opt+4", "\u{00A2}");
    m.insert("option+4", "\u{00A2}");
    m.insert("opt+5", "\u{221E}");
    m.insert("option+5", "\u{221E}");
    m.insert("opt+6", "\u{00A7}");
    m.insert("option+6", "\u{00A7}");
    m.insert("opt+7", "\u{00B6}");
    m.insert("option+7", "\u{00B6}");
    m.insert("opt+8", "\u{2022}");
    m.insert("option+8", "\u{2022}");
    m.insert("opt+9", "\u{00AA}");
    m.insert("option+9", "\u{00AA}");
    m
}

/// macOS Option+Shift+number → composed unicode glyph.
fn mac_option_shift_number_map() -> HashMap<&'static str, &'static str> {
    let mut m = HashMap::new();
    m.insert("opt+shift+1", "\u{2044}");
    m.insert("option+shift+1", "\u{2044}");
    m.insert("opt+shift+2", "\u{20AC}");
    m.insert("option+shift+2", "\u{20AC}");
    m.insert("opt+shift+3", "\u{2039}");
    m.insert("option+shift+3", "\u{2039}");
    m.insert("opt+shift+4", "\u{203A}");
    m.insert("option+shift+4", "\u{203A}");
    m.insert("opt+shift+5", "\u{FB01}");
    m.insert("option+shift+5", "\u{FB01}");
    m.insert("opt+shift+6", "\u{FB02}");
    m.insert("option+shift+6", "\u{FB02}");
    m.insert("opt+shift+7", "\u{2021}");
    m.insert("option+shift+7", "\u{2021}");
    m.insert("opt+shift+8", "\u{00B0}");
    m.insert("option+shift+8", "\u{00B0}");
    m.insert("opt+shift+9", "\u{00B7}");
    m.insert("option+shift+9", "\u{00B7}");
    m
}

/// macOS Option+Tab → composed unicode glyph.
fn mac_option_tab_map() -> HashMap<&'static str, &'static str> {
    let mut m = HashMap::new();
    m.insert("opt+tab", "\u{21E5}");
    m.insert("option+tab", "\u{21E5}");
    m.insert("opt+shift+tab", "\u{21E4}");
    m.insert("option+shift+tab", "\u{21E4}");
    m
}

/// macOS Option+letter → composed unicode glyph (US layout).
fn mac_option_letters() -> HashMap<&'static str, &'static str> {
    let mut m = HashMap::new();
    m.insert("a", "\u{00E5}");
    m.insert("b", "\u{222B}");
    m.insert("c", "\u{00E7}");
    m.insert("d", "\u{2202}");
    m.insert("e", "\u{00B4}");
    m.insert("f", "\u{0192}");
    m.insert("g", "\u{00A9}");
    m.insert("h", "\u{02D9}");
    m.insert("i", "\u{02C6}");
    m.insert("j", "\u{2206}");
    m.insert("k", "\u{02DA}");
    m.insert("l", "\u{00AC}");
    m.insert("m", "\u{00B5}");
    m.insert("n", "\u{02DC}");
    m.insert("o", "\u{00F8}");
    m.insert("p", "\u{03C0}");
    m.insert("q", "\u{0153}");
    m.insert("r", "\u{00AE}");
    m.insert("s", "\u{00DF}");
    m.insert("t", "\u{2020}");
    m.insert("u", "\u{00A8}");
    m.insert("v", "\u{221A}");
    m.insert("w", "\u{2211}");
    m.insert("x", "\u{2248}");
    m.insert("y", "\u{00A5}");
    m.insert("z", "\u{03A9}");
    m
}

/// macOS Option+Shift+letter → composed unicode glyph (US layout).
fn mac_option_shift_letters() -> HashMap<&'static str, &'static str> {
    let mut m = HashMap::new();
    m.insert("a", "\u{00C5}");
    m.insert("b", "\u{0131}");
    m.insert("c", "\u{00C7}");
    m.insert("d", "\u{00CE}");
    m.insert("f", "\u{00CF}");
    m.insert("g", "\u{02DD}");
    m.insert("h", "\u{00D3}");
    m.insert("j", "\u{00D4}");
    m.insert("l", "\u{00D2}");
    m.insert("m", "\u{00C2}");
    m.insert("o", "\u{00D8}");
    m.insert("p", "\u{220F}");
    m.insert("q", "\u{0152}");
    m.insert("r", "\u{2030}");
    m.insert("s", "\u{00CD}");
    m.insert("t", "\u{02C7}");
    m.insert("v", "\u{25CA}");
    m.insert("w", "\u{201E}");
    m.insert("x", "\u{02DB}");
    m.insert("y", "\u{00C1}");
    m.insert("z", "\u{00B8}");
    m
}

/// Shifted digits on a US layout: digit → symbol.
fn shifted_digits() -> HashMap<&'static str, &'static str> {
    let mut m = HashMap::new();
    m.insert("1", "!");
    m.insert("2", "@");
    m.insert("3", "#");
    m.insert("4", "$");
    m.insert("5", "%");
    m.insert("6", "^");
    m.insert("7", "&");
    m.insert("8", "*");
    m.insert("9", "(");
    m.insert("0", ")");
    m
}

/// Strip a leading `alt+`/`opt+`/`option+` prefix and return the base key.
/// Only a single modifier counts: `ctrl+alt+n` is not a chord macOS composes.
fn cut_option_prefix(key_lower: &str) -> Option<&str> {
    for prefix in &["alt+", "opt+", "option+"] {
        if let Some(base) = key_lower.strip_prefix(prefix) {
            return Some(base);
        }
    }
    None
}

/// Return the composed glyph for an `opt+x`/`alt+x` chord spelled in
/// lowercase, or `None` when the chord is not an Option+letter one.
fn mac_option_letter_glyph(key_lower: &str) -> Option<&'static str> {
    let base = cut_option_prefix(key_lower)?;
    if let Some(shifted) = base.strip_prefix("shift+") {
        return mac_option_shift_letters().get(shifted).copied();
    }
    mac_option_letters().get(base).copied()
}

/// Return the alternate spellings of a shifted key: the shifted character for
/// a `shift+x` chord and the `shift+x` chord for a shifted character.
fn shift_aliases(key: &str, key_lower: &str) -> Vec<String> {
    if let Some(after) = key_lower.strip_prefix("shift+") {
        let base = after;
        if let Some(symbol) = shifted_digits().get(base) {
            return vec![symbol.to_string()];
        }
        if is_single_rune_letter(base) {
            return vec![base.to_uppercase()];
        }
        return vec![];
    }

    // Inverted shifted digits: "!" → "shift+1"
    let sd = shifted_digits();
    for (digit, symbol) in &sd {
        if key == *symbol {
            return vec![format!("shift+{}", digit)];
        }
    }

    // An uppercase letter is the shifted spelling of its lowercase self.
    if is_single_rune_letter(key) && key != key_lower {
        return vec![format!("shift+{}", key_lower)];
    }

    modified_shift_aliases(key_lower)
}

/// Shift aliases for a chord carrying other modifiers (e.g. `alt+shift+1`).
fn modified_shift_aliases(key_lower: &str) -> Vec<String> {
    let i = match key_lower.rfind('+') {
        Some(i) if i > 0 => i,
        _ => return vec![],
    };
    let mods = &key_lower[..=i];
    let base = &key_lower[i + 1..];
    let shifted = mods.ends_with("shift+");
    let mods_trimmed = mods.strip_suffix("shift+").unwrap_or(mods);
    if mods_trimmed.is_empty() {
        return vec![];
    }

    let sd = shifted_digits();
    // Inverted lookup: symbol → digit
    for digit in sd.keys() {
        if base == *digit {
            return vec![
                format!("{}{}", mods_trimmed, base),
                format!("{}shift+{}", mods_trimmed, base),
                format!("{}shift+{}", mods_trimmed, digit),
            ];
        }
    }
    // Forward lookup: digit → symbol
    if shifted {
        for symbol in sd.values() {
            if base == *symbol {
                return vec![
                    format!("{}{}", mods_trimmed, symbol),
                    format!("{}shift+{}", mods_trimmed, symbol),
                ];
            }
        }
    }
    vec![]
}

/// Key normalizer with platform detection.
pub struct KeyNormalizer {
    is_macos: bool,
}

impl KeyNormalizer {
    /// Create a new key normalizer with platform detection.
    pub fn new() -> Self {
        Self {
            is_macos: detect_macos(),
        }
    }

    /// Returns whether the current platform is macOS.
    pub fn is_macos(&self) -> bool {
        self.is_macos
    }

    /// Convert a key string to all its canonical forms for the current
    /// platform. The first element is always the primary normalized form.
    pub fn normalize_key(&self, key: &str) -> Vec<String> {
        let key = key.trim();

        // For single letters, preserve case (M and m are different keys).
        // For everything else, normalize to lowercase.
        let normalized = if is_single_rune_letter(key) {
            key.to_string()
        } else {
            key.to_lowercase()
        };

        let key_lower = key.to_lowercase();

        // Always include the normalized version.
        let mut result = vec![normalized.clone()];

        // Accept both spellings of a shifted key, on every platform.
        result.extend(shift_aliases(key, &key_lower));

        // On macOS, expand opt+N and option+N to unicode and alt+N.
        if self.is_macos {
            let opt_shift = mac_option_shift_number_map();
            let opt_num = mac_option_number_map();
            let opt_tab = mac_option_tab_map();

            if let Some(glyph) = opt_shift.get(key_lower.as_str()) {
                result.push(glyph.to_lowercase());
                result.push(key_lower.replace("opt+", "alt+").replace("option+", "alt+"));
            } else if let Some(glyph) = opt_num.get(key_lower.as_str()) {
                result.push(glyph.to_lowercase());
                result.push(key_lower.replace("opt+", "alt+").replace("option+", "alt+"));
            } else if let Some(glyph) = opt_tab.get(key_lower.as_str()) {
                result.push((*glyph).to_string());
                result.push(key_lower.replace("opt+", "alt+").replace("option+", "alt+"));
            } else if let Some(glyph) = mac_option_letter_glyph(&key_lower) {
                result.push(glyph.to_string());
                result.push(key_lower.replace("opt+", "alt+").replace("option+", "alt+"));
            }

            // If the key starts with "alt+", also accept "opt+" and "option+".
            if key_lower.starts_with("alt+") {
                result.push(key_lower.replace("alt+", "opt+"));
                result.push(key_lower.replace("alt+", "option+"));
            }
        }

        // Remove duplicates, preserving order.
        let mut seen = std::collections::HashSet::new();
        result.retain(|k| seen.insert(k.clone()));
        result
    }

    /// Expand a slice of user-provided keys to all platform-specific variants.
    pub fn expand_keys(&self, keys: &[String]) -> Vec<String> {
        let mut expanded = Vec::new();
        for key in keys {
            expanded.extend(self.normalize_key(key));
        }
        let mut seen = std::collections::HashSet::new();
        expanded.retain(|k| seen.insert(k.clone()));
        expanded
    }

    /// Check if a key string is valid for the current platform.
    pub fn validate_key(&self, key: &str) -> (bool, String) {
        let key = key.trim();
        let key_lower = key.to_lowercase();

        if key_lower.is_empty() {
            return (false, "key cannot be empty".into());
        }

        // On non-macOS systems, error on opt/option keys.
        if !self.is_macos && (key_lower.contains("opt+") || key_lower.contains("option+")) {
            return (
                false,
                "opt/option keys are only valid on macOS, use alt+ instead".into(),
            );
        }

        // Check for invalid modifier combinations.
        let parts: Vec<&str> = key_lower.split('+').collect();
        if parts.len() > 1 {
            let modifiers = &parts[..parts.len() - 1];
            let actual_key = parts[parts.len() - 1];

            if actual_key.is_empty() {
                return (
                    false,
                    "key combination incomplete (ends with +)".into(),
                );
            }

            let valid_modifiers: HashMap<&str, bool> = [
                ("ctrl", true),
                ("alt", true),
                ("shift", true),
                ("opt", self.is_macos),
                ("option", self.is_macos),
            ]
            .into_iter()
            .collect();

            for &mod_ in modifiers {
                if !*valid_modifiers.get(mod_).unwrap_or(&false) {
                    if mod_ == "opt" || mod_ == "option" {
                        return (
                            false,
                            "opt/option modifiers are only valid on macOS".into(),
                        );
                    }
                    return (false, format!("invalid modifier: {}", mod_));
                }
            }

            // Check for duplicate modifiers.
            let mut mod_set = std::collections::HashSet::new();
            for &mod_ in modifiers {
                if !mod_set.insert(mod_) {
                    return (false, format!("duplicate modifier: {}", mod_));
                }
            }
        }

        // Single-rune keys are always valid.
        let actual_key = parts[parts.len() - 1];
        if actual_key.chars().count() == 1 {
            return (true, String::new());
        }

        // Check if it's a valid special key.
        let valid_special_keys: &[&str] = &[
            "leftalt", "rightalt", "leftctrl", "rightctrl", "leftshift", "rightshift",
            "leftsuper", "rightsuper", "leftmeta", "rightmeta", "lefthyper", "righthyper",
            "enter", "return", "esc", "escape", "tab", "space", "backspace", "delete",
            "up", "down", "left", "right", "home", "end", "pgup", "pageup", "pgdown",
            "pagedown", "f1", "f2", "f3", "f4", "f5", "f6", "f7", "f8", "f9", "f10",
            "f11", "f12",
        ];
        if !valid_special_keys.contains(&actual_key) {
            return (false, format!("unknown special key: {}", actual_key));
        }

        (true, String::new())
    }
}

impl Default for KeyNormalizer {
    fn default() -> Self {
        Self::new()
    }
}

/// Normalize a key string to its canonical form.
///
/// On macOS, `opt+` and `option+` are converted to `alt+`. On all platforms,
/// `cmd+` is converted to `super+` and `control+` to `ctrl+`. Single-letter
/// keys preserve case; compound keys are lowercased.
pub fn normalize_key(key: &str) -> String {
    let key = key.trim();
    if is_single_rune_letter(key) {
        return key.to_string();
    }
    let key = key.to_lowercase();
    let key = key.replace("cmd+", "super+");
    let key = key.replace("opt+", "alt+");
    let key = key.replace("option+", "alt+");
    key.replace("control+", "ctrl+")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_key_single_letter_preserves_case() {
        assert_eq!(normalize_key("M"), "M");
        assert_eq!(normalize_key("m"), "m");
    }

    #[test]
    fn normalize_key_compound_lowercases() {
        assert_eq!(normalize_key("Ctrl+B"), "ctrl+b");
        assert_eq!(normalize_key("ALT+x"), "alt+x");
    }

    #[test]
    fn normalize_key_opt_to_alt() {
        assert_eq!(normalize_key("opt+x"), "alt+x");
        assert_eq!(normalize_key("option+x"), "alt+x");
    }

    #[test]
    fn normalize_key_cmd_to_super() {
        assert_eq!(normalize_key("cmd+c"), "super+c");
    }

    #[test]
    fn normalize_key_control_to_ctrl() {
        assert_eq!(normalize_key("control+a"), "ctrl+a");
    }

    #[test]
    fn normalize_key_whitespace_trimmed() {
        assert_eq!(normalize_key("  ctrl+b  "), "ctrl+b");
    }

    #[test]
    fn key_normalizer_normalize_single_letter() {
        let kn = KeyNormalizer::new();
        let result = kn.normalize_key("m");
        assert!(result.contains(&"m".to_string()));
    }

    #[test]
    fn key_normalizer_normalize_compound() {
        let kn = KeyNormalizer::new();
        let result = kn.normalize_key("ctrl+b");
        assert!(result.contains(&"ctrl+b".to_string()));
    }

    #[test]
    fn key_normalizer_validate_empty() {
        let kn = KeyNormalizer::new();
        let (valid, msg) = kn.validate_key("");
        assert!(!valid);
        assert!(msg.contains("empty"));
    }

    #[test]
    fn key_normalizer_validate_single_rune() {
        let kn = KeyNormalizer::new();
        let (valid, _) = kn.validate_key("a");
        assert!(valid);
    }

    #[test]
    fn key_normalizer_validate_special_key() {
        let kn = KeyNormalizer::new();
        let (valid, _) = kn.validate_key("enter");
        assert!(valid);
    }

    #[test]
    fn key_normalizer_validate_unknown_special() {
        let kn = KeyNormalizer::new();
        let (valid, msg) = kn.validate_key("ctrl+unknownkey");
        assert!(!valid);
        assert!(msg.contains("unknown special key"));
    }

    #[test]
    fn key_normalizer_validate_duplicate_modifier() {
        let kn = KeyNormalizer::new();
        let (valid, msg) = kn.validate_key("ctrl+ctrl+a");
        assert!(!valid);
        assert!(msg.contains("duplicate modifier"));
    }

    #[test]
    fn key_normalizer_expand_keys_dedupes() {
        let kn = KeyNormalizer::new();
        let keys = vec!["ctrl+b".to_string(), "ctrl+b".to_string()];
        let expanded = kn.expand_keys(&keys);
        let count = expanded.iter().filter(|k| k == &"ctrl+b").count();
        assert_eq!(count, 1);
    }

    #[test]
    fn is_single_rune_letter_basic() {
        assert!(is_single_rune_letter("a"));
        assert!(is_single_rune_letter("M"));
        assert!(!is_single_rune_letter("ab"));
        assert!(!is_single_rune_letter("ctrl+b"));
        assert!(!is_single_rune_letter(""));
    }
}
