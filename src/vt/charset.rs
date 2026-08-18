//! DEC character set translation (SCS).
//!
//! Applications select the DEC Special Graphics set (ESC (0) to draw box
//! characters as plain letters. This maps those letters to their drawing
//! equivalents, and provides the UK set (ESC (A) for £).

/// Translate a byte through the DEC Special Graphics character set.
/// Letters a-z map to line-drawing characters; other bytes pass through.
pub fn special_graphics(b: u8) -> char {
    match b {
        b'`' => '◆',
        b'a' => '▒',
        b'b' => '␉',
        b'c' => '␌',
        b'd' => '␍',
        b'e' => '␊',
        b'f' => '°',
        b'g' => '±',
        b'h' => '␤',
        b'i' => '␋',
        b'j' => '┘',
        b'k' => '┐',
        b'l' => '┌',
        b'm' => '└',
        b'n' => '┼',
        b'o' => '⎺',
        b'p' => '⎻',
        b'q' => '─',
        b'r' => '⎼',
        b's' => '⎽',
        b't' => '├',
        b'u' => '┤',
        b'v' => '┴',
        b'w' => '┬',
        b'x' => '│',
        b'y' => '≤',
        b'z' => '≥',
        b'{' => 'π',
        b'|' => '≠',
        b'}' => '£',
        b'~' => '·',
        _ => b as char,
    }
}

/// Translate a byte through the UK character set (only `#` → `£` differs from
/// ASCII).
pub fn uk(b: u8) -> char {
    match b {
        b'#' => '£',
        _ => b as char,
    }
}

/// A character set selected into one of the G0-G3 slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharSet {
    /// US ASCII (the default).
    Ascii,
    /// DEC Special Graphics (line drawing).
    SpecialGraphics,
    /// UK set.
    Uk,
}

impl CharSet {
    /// Translate a byte through this set.
    pub fn translate(self, b: u8) -> char {
        match self {
            CharSet::Ascii => b as char,
            CharSet::SpecialGraphics => special_graphics(b),
            CharSet::Uk => uk(b),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn special_graphics_box_drawing() {
        assert_eq!(special_graphics(b'j'), '┘');
        assert_eq!(special_graphics(b'k'), '┐');
        assert_eq!(special_graphics(b'l'), '┌');
        assert_eq!(special_graphics(b'm'), '└');
        assert_eq!(special_graphics(b'n'), '┼');
        assert_eq!(special_graphics(b'q'), '─');
        assert_eq!(special_graphics(b'x'), '│');
        assert_eq!(special_graphics(b't'), '├');
        assert_eq!(special_graphics(b'u'), '┤');
        assert_eq!(special_graphics(b'v'), '┴');
        assert_eq!(special_graphics(b'w'), '┬');
    }

    #[test]
    fn special_graphics_passthrough() {
        assert_eq!(special_graphics(b'A'), 'A');
        assert_eq!(special_graphics(b'0'), '0');
        assert_eq!(special_graphics(b' '), ' ');
    }

    #[test]
    fn special_graphics_special_chars() {
        assert_eq!(special_graphics(b'`'), '◆');
        assert_eq!(special_graphics(b'a'), '▒');
        assert_eq!(special_graphics(b'f'), '°');
        assert_eq!(special_graphics(b'g'), '±');
        assert_eq!(special_graphics(b'~'), '·');
    }

    #[test]
    fn uk_hash_to_pound() {
        assert_eq!(uk(b'#'), '£');
    }

    #[test]
    fn uk_passthrough() {
        assert_eq!(uk(b'A'), 'A');
        assert_eq!(uk(b' '), ' ');
    }

    #[test]
    fn charset_ascii_translate() {
        assert_eq!(CharSet::Ascii.translate(b'A'), 'A');
        assert_eq!(CharSet::Ascii.translate(b'z'), 'z');
    }

    #[test]
    fn charset_special_graphics_translate() {
        assert_eq!(CharSet::SpecialGraphics.translate(b'j'), '┘');
    }

    #[test]
    fn charset_uk_translate() {
        assert_eq!(CharSet::Uk.translate(b'#'), '£');
    }

    #[test]
    fn charset_variants() {
        let ascii = CharSet::Ascii;
        let sg = CharSet::SpecialGraphics;
        let uk = CharSet::Uk;
        assert_ne!(ascii, sg);
        assert_ne!(ascii, uk);
        assert_ne!(sg, uk);
    }
}
