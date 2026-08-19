//! Line width calculation using `unicode-width`.
//!
//! Provides display-width calculation for strings containing wide characters
//! (CJK, emoji) and zero-width characters (combining marks, ZWJ). Used by
//! terminal rendering and copy mode to correctly position text.

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Return the display width of a string in terminal cells.
///
/// Wide characters (CJK, emoji) take 2 cells. Zero-width characters
/// (combining marks, ZWJ) take 0 cells. Regular characters take 1 cell.
pub fn width(s: &str) -> usize {
    s.width()
}

/// Return the display width of a single character.
pub fn char_width(c: char) -> usize {
    c.width().unwrap_or(0)
}

/// Truncate a string to fit within `max_width` display cells.
///
/// Returns the truncated string. If the string fits, it is returned as-is.
pub fn truncate(s: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if s.width() <= max_width {
        return s.to_string();
    }
    let mut result = String::new();
    let mut current_width = 0;
    for c in s.chars() {
        let w = char_width(c);
        if current_width + w > max_width {
            break;
        }
        result.push(c);
        current_width += w;
    }
    result
}

/// Truncate a string to fit within `max_width` display cells, appending an
/// ellipsis if truncation occurs.
pub fn truncate_with_ellipsis(s: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if s.width() <= max_width {
        return s.to_string();
    }
    let ellipsis = "…";
    let ellipsis_width = ellipsis.width();
    if max_width <= ellipsis_width {
        return ellipsis.chars().take(max_width).collect();
    }
    let target = max_width - ellipsis_width;
    let truncated = truncate(s, target);
    format!("{truncated}{ellipsis}")
}

/// Pad a string to exactly `width` display cells, padding with spaces on the
/// right.
pub fn pad_right(s: &str, width: usize) -> String {
    let current = s.width();
    if current >= width {
        s.to_string()
    } else {
        format!("{}{}", s, " ".repeat(width - current))
    }
}

/// Pad a string to exactly `width` display cells, padding with spaces on the
/// left.
pub fn pad_left(s: &str, width: usize) -> String {
    let current = s.width();
    if current >= width {
        s.to_string()
    } else {
        format!("{}{}", " ".repeat(width - current), s)
    }
}

/// Center a string within `width` display cells.
pub fn center(s: &str, width: usize) -> String {
    let current = s.width();
    if current >= width {
        return s.to_string();
    }
    let total_pad = width - current;
    let left = total_pad / 2;
    let right = total_pad - left;
    format!("{}{}{}", " ".repeat(left), s, " ".repeat(right))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_ascii() {
        assert_eq!(width("hello"), 5);
        assert_eq!(width(""), 0);
    }

    #[test]
    fn width_wide_char() {
        // CJK characters are 2 cells wide.
        assert_eq!(width("你好"), 4);
        assert_eq!(width("a你"), 3);
    }

    #[test]
    fn width_zero_width() {
        // Combining marks are 0 cells wide.
        assert_eq!(width("e\u{0301}"), 1); // é as e + combining acute
    }

    #[test]
    fn char_width_basic() {
        assert_eq!(char_width('a'), 1);
        assert_eq!(char_width('你'), 2);
        assert_eq!(char_width('\u{0301}'), 0); // combining acute
    }

    #[test]
    fn truncate_basic() {
        assert_eq!(truncate("hello world", 5), "hello");
        assert_eq!(truncate("hi", 10), "hi");
        assert_eq!(truncate("", 5), "");
        assert_eq!(truncate("hello", 0), "");
    }

    #[test]
    fn truncate_wide_char() {
        // Truncating in the middle of a wide char should not include it.
        assert_eq!(truncate("你好世界", 5), "你好"); // 4 cells, can't fit 6
    }

    #[test]
    fn truncate_with_ellipsis_basic() {
        // "hello world" is 11 cells. Max 8, ellipsis 1 cell, target 7.
        assert_eq!(truncate_with_ellipsis("hello world", 8), "hello w…");
        assert_eq!(truncate_with_ellipsis("hi", 10), "hi");
    }

    #[test]
    fn truncate_with_ellipsis_short() {
        // "hello" is 5 cells. Max 2, ellipsis 1 cell, target 1.
        assert_eq!(truncate_with_ellipsis("hello", 2), "h…");
        // Max 3, ellipsis 1 cell, target 2.
        assert_eq!(truncate_with_ellipsis("hello", 3), "he…");
    }

    #[test]
    fn pad_right_basic() {
        assert_eq!(pad_right("hi", 5), "hi   ");
        assert_eq!(pad_right("hello", 3), "hello");
    }

    #[test]
    fn pad_left_basic() {
        assert_eq!(pad_left("hi", 5), "   hi");
        assert_eq!(pad_left("hello", 3), "hello");
    }

    #[test]
    fn center_basic() {
        assert_eq!(center("hi", 6), "  hi  ");
        assert_eq!(center("hi", 5), " hi  ");
        assert_eq!(center("hello", 3), "hello");
    }

    #[test]
    fn pad_right_wide_char() {
        // 你 is 2 cells, so pad to 4 needs 2 spaces.
        assert_eq!(pad_right("你", 4), "你  ");
    }
}
