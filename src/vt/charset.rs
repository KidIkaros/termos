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
