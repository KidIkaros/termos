//! Copy mode extensions — ported from Go TUIOS `internal/input/copymode_*.go`.
//!
//! Provides character search (f/F/t/T/;/,), visual mode effects, and
//! additional motion helpers that extend the basic copy mode in `input.rs`.

/// Character search direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharSearchDir {
    Forward,
    Backward,
}

/// Whether to land on the character or stop before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharSearchMode {
    /// Land on the character (f/F).
    On,
    /// Stop before the character (t/T).
    Till,
}

/// A pending character search operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CharSearch {
    pub dir: CharSearchDir,
    pub mode: CharSearchMode,
    pub ch: char,
}

impl CharSearch {
    /// Create a forward "find" search (f{char}).
    pub fn forward_find(ch: char) -> Self {
        Self {
            dir: CharSearchDir::Forward,
            mode: CharSearchMode::On,
            ch,
        }
    }

    /// Create a backward "find" search (F{char}).
    pub fn backward_find(ch: char) -> Self {
        Self {
            dir: CharSearchDir::Backward,
            mode: CharSearchMode::On,
            ch,
        }
    }

    /// Create a forward "till" search (t{char}).
    pub fn forward_till(ch: char) -> Self {
        Self {
            dir: CharSearchDir::Forward,
            mode: CharSearchMode::Till,
            ch,
        }
    }

    /// Create a backward "till" search (T{char}).
    pub fn backward_till(ch: char) -> Self {
        Self {
            dir: CharSearchDir::Backward,
            mode: CharSearchMode::Till,
            ch,
        }
    }
}

/// Find the next occurrence of a character on a line, starting from a column.
/// Returns the column where the character was found, or None.
///
/// - `line`: the text of the line to search
/// - `start_col`: the column to start searching from (exclusive)
/// - `search`: the search parameters
pub fn find_char_on_line(line: &str, start_col: usize, search: &CharSearch) -> Option<usize> {
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    // Convert start_col to a rune index
    let mut rune_start = 0;
    for (i, _) in chars.iter().enumerate() {
        if i >= start_col {
            break;
        }
        rune_start = i + 1;
    }

    match search.dir {
        CharSearchDir::Forward => {
            for (i, (_, c)) in chars.iter().enumerate().skip(rune_start + 1) {
                if *c == search.ch {
                    return Some(match search.mode {
                        CharSearchMode::On => i,
                        CharSearchMode::Till => i.saturating_sub(1),
                    });
                }
            }
        }
        CharSearchDir::Backward => {
            for (i, (_, c)) in chars.iter().enumerate().take(rune_start).rev() {
                if *c == search.ch {
                    return Some(match search.mode {
                        CharSearchMode::On => i,
                        CharSearchMode::Till => i + 1,
                    });
                }
            }
        }
    }
    None
}

/// Repeat the last character search in the same direction.
pub fn repeat_search(line: &str, start_col: usize, last: &CharSearch) -> Option<usize> {
    find_char_on_line(line, start_col, last)
}

/// Reverse the last character search direction.
pub fn reverse_search(line: &str, start_col: usize, last: &CharSearch) -> Option<usize> {
    let reversed = CharSearch {
        dir: match last.dir {
            CharSearchDir::Forward => CharSearchDir::Backward,
            CharSearchDir::Backward => CharSearchDir::Forward,
        },
        mode: last.mode,
        ch: last.ch,
    };
    find_char_on_line(line, start_col, &reversed)
}

/// Visual mode type for copy mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VisualMode {
    /// No visual selection.
    #[default]
    None,
    /// Character-wise visual (v).
    Char,
    /// Line-wise visual (V).
    Line,
    /// Block-wise visual (Ctrl+V).
    Block,
}

impl VisualMode {
    /// Whether visual mode is active.
    pub fn active(&self) -> bool {
        !matches!(self, Self::None)
    }
}

/// The start anchor of a visual selection.
#[derive(Debug, Clone, Copy, Default)]
pub struct VisualAnchor {
    pub line: usize,
    pub col: usize,
}

/// Compute the selection range given an anchor and current cursor position.
/// Returns (start_line, start_col, end_line, end_col) in inclusive coordinates.
pub fn selection_range(
    anchor: &VisualAnchor,
    cursor_line: usize,
    cursor_col: usize,
    mode: VisualMode,
) -> (usize, usize, usize, usize) {
    match mode {
        VisualMode::Line => {
            if cursor_line >= anchor.line {
                (anchor.line, 0, cursor_line, usize::MAX)
            } else {
                (cursor_line, 0, anchor.line, usize::MAX)
            }
        }
        _ => {
            if cursor_line > anchor.line || (cursor_line == anchor.line && cursor_col >= anchor.col)
            {
                (anchor.line, anchor.col, cursor_line, cursor_col)
            } else {
                (cursor_line, cursor_col, anchor.line, anchor.col)
            }
        }
    }
}

/// Word motion type for w/b/e/ge motions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WordMotion {
    /// Word forward (w).
    WordForward,
    /// Word backward (b).
    WordBackward,
    /// End of word forward (e).
    WordEnd,
    /// End of word backward (ge).
    WordEndBackward,
}

/// Find the next word boundary on a line.
/// Returns the column of the word boundary.
pub fn word_motion(line: &str, start_col: usize, motion: WordMotion) -> usize {
    let chars: Vec<char> = line.chars().collect();
    let len = chars.len();
    if len == 0 {
        return 0;
    }

    let is_word_char = |c: char| c.is_alphanumeric() || c == '_';

    match motion {
        WordMotion::WordForward => {
            let mut i = start_col;
            // Skip current word
            if i < len && is_word_char(chars[i]) {
                while i < len && is_word_char(chars[i]) {
                    i += 1;
                }
            } else if i < len && !chars[i].is_whitespace() {
                while i < len && !is_word_char(chars[i]) && !chars[i].is_whitespace() {
                    i += 1;
                }
            }
            // Skip whitespace
            while i < len && chars[i].is_whitespace() {
                i += 1;
            }
            i.min(len)
        }
        WordMotion::WordBackward => {
            if start_col == 0 {
                return 0;
            }
            let mut i = start_col - 1;
            // Skip whitespace
            while i > 0 && chars[i].is_whitespace() {
                i -= 1;
            }
            // Skip current word
            if is_word_char(chars[i]) {
                while i > 0 && is_word_char(chars[i - 1]) {
                    i -= 1;
                }
            } else if !chars[i].is_whitespace() {
                while i > 0 && !is_word_char(chars[i - 1]) && !chars[i - 1].is_whitespace() {
                    i -= 1;
                }
            }
            i
        }
        WordMotion::WordEnd => {
            let mut i = start_col + 1;
            // Skip whitespace
            while i < len && chars[i].is_whitespace() {
                i += 1;
            }
            if i >= len {
                return len.saturating_sub(1);
            }
            // Move to end of word
            if is_word_char(chars[i]) {
                while i + 1 < len && is_word_char(chars[i + 1]) {
                    i += 1;
                }
            } else {
                while i + 1 < len && !is_word_char(chars[i + 1]) && !chars[i + 1].is_whitespace() {
                    i += 1;
                }
            }
            i.min(len.saturating_sub(1))
        }
        WordMotion::WordEndBackward => {
            if start_col == 0 {
                return 0;
            }
            let mut i = start_col - 1;
            // Skip whitespace
            while i > 0 && chars[i].is_whitespace() {
                i -= 1;
            }
            if i == 0 {
                return 0;
            }
            // Move to end of previous word
            if is_word_char(chars[i]) {
                while i > 0 && is_word_char(chars[i - 1]) {
                    i -= 1;
                }
                if i > 0 {
                    i = i.saturating_sub(1);
                }
            } else {
                while i > 0 && !is_word_char(chars[i - 1]) && !chars[i - 1].is_whitespace() {
                    i -= 1;
                }
                if i > 0 {
                    i = i.saturating_sub(1);
                }
            }
            i
        }
    }
}

/// Bracket matching pairs for % motion.
pub const BRACKET_PAIRS: &[(char, char)] = &[('(', ')'), ('[', ']'), ('{', '}')];

/// Find the matching bracket from the given position.
/// Returns the column of the matching bracket, or None.
pub fn find_matching_bracket(line: &str, start_col: usize) -> Option<usize> {
    let chars: Vec<char> = line.chars().collect();
    if start_col >= chars.len() {
        return None;
    }
    let ch = chars[start_col];

    // Find which pair and direction
    let (open, close, forward) = BRACKET_PAIRS.iter().find_map(|(o, c)| {
        if ch == *o {
            Some((*o, *c, true))
        } else if ch == *c {
            Some((*o, *c, false))
        } else {
            None
        }
    })?;

    let mut depth = 0;
    if forward {
        for (i, &ch) in chars.iter().enumerate().skip(start_col) {
            if ch == open {
                depth += 1;
            } else if ch == close {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
        }
    } else {
        for (i, &ch) in chars.iter().enumerate().take(start_col + 1).rev() {
            if ch == close {
                depth += 1;
            } else if ch == open {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
        }
    }
    None
}

/// Scroll the copy-mode view by `delta` lines. Positive = down, negative = up.
/// Returns the new scroll offset, clamped to valid range.
pub fn scroll_copy_mode(
    current_offset: usize,
    delta: i32,
    scrollback_len: usize,
    _screen_height: usize,
) -> usize {
    if scrollback_len == 0 {
        return 0;
    }
    let max_offset = scrollback_len;
    let new_offset = current_offset as i32 + delta;
    if new_offset <= 0 {
        0
    } else if new_offset as usize > max_offset {
        max_offset
    } else {
        new_offset as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn char_search_forward_find() {
        let line = "hello world";
        let search = CharSearch::forward_find('o');
        let result = find_char_on_line(line, 0, &search);
        assert_eq!(result, Some(4));
    }

    #[test]
    fn char_search_forward_till() {
        let line = "hello world";
        let search = CharSearch::forward_till('o');
        let result = find_char_on_line(line, 0, &search);
        assert_eq!(result, Some(3));
    }

    #[test]
    fn char_search_backward_find() {
        let line = "hello world";
        let search = CharSearch::backward_find('l');
        let result = find_char_on_line(line, 9, &search);
        assert_eq!(result, Some(3));
    }

    #[test]
    fn char_search_not_found() {
        let line = "hello world";
        let search = CharSearch::forward_find('z');
        let result = find_char_on_line(line, 0, &search);
        assert_eq!(result, None);
    }

    #[test]
    fn repeat_search_same_direction() {
        let line = "hello world";
        let search = CharSearch::forward_find('l');
        let first = find_char_on_line(line, 0, &search);
        assert_eq!(first, Some(2));
        let second = repeat_search(line, 2, &search);
        assert_eq!(second, Some(3));
    }

    #[test]
    fn reverse_search_flips_direction() {
        let line = "hello world";
        let search = CharSearch::forward_find('l');
        let result = reverse_search(line, 3, &search);
        assert_eq!(result, Some(2));
    }

    #[test]
    fn visual_mode_active() {
        assert!(!VisualMode::None.active());
        assert!(VisualMode::Char.active());
        assert!(VisualMode::Line.active());
        assert!(VisualMode::Block.active());
    }

    #[test]
    fn selection_range_char_mode() {
        let anchor = VisualAnchor { line: 5, col: 10 };
        let (sl, sc, el, ec) = selection_range(&anchor, 5, 20, VisualMode::Char);
        assert_eq!((sl, sc, el, ec), (5, 10, 5, 20));

        let (sl, sc, el, ec) = selection_range(&anchor, 5, 5, VisualMode::Char);
        assert_eq!((sl, sc, el, ec), (5, 5, 5, 10));
    }

    #[test]
    fn selection_range_line_mode() {
        let anchor = VisualAnchor { line: 3, col: 0 };
        let (sl, sc, el, _) = selection_range(&anchor, 7, 10, VisualMode::Line);
        assert_eq!((sl, sc, el), (3, 0, 7));
    }

    #[test]
    fn word_motion_forward() {
        let line = "hello world foo";
        assert_eq!(word_motion(line, 0, WordMotion::WordForward), 6);
        assert_eq!(word_motion(line, 6, WordMotion::WordForward), 12);
    }

    #[test]
    fn word_motion_backward() {
        let line = "hello world foo";
        assert_eq!(word_motion(line, 12, WordMotion::WordBackward), 6);
        assert_eq!(word_motion(line, 6, WordMotion::WordBackward), 0);
    }

    #[test]
    fn word_motion_end() {
        let line = "hello world";
        assert_eq!(word_motion(line, 0, WordMotion::WordEnd), 4);
        assert_eq!(word_motion(line, 4, WordMotion::WordEnd), 10);
    }

    #[test]
    fn find_matching_bracket_forward() {
        let line = "fn foo(bar) {";
        let result = find_matching_bracket(line, 6);
        assert_eq!(result, Some(10));
    }

    #[test]
    fn find_matching_bracket_backward() {
        let line = "fn foo(bar) {";
        let result = find_matching_bracket(line, 10);
        assert_eq!(result, Some(6));
    }

    #[test]
    fn find_matching_bracket_nested() {
        let line = "((a+b))";
        let result = find_matching_bracket(line, 0);
        assert_eq!(result, Some(6));
    }

    #[test]
    fn find_matching_bracket_no_bracket() {
        let line = "hello world";
        let result = find_matching_bracket(line, 0);
        assert_eq!(result, None);
    }

    #[test]
    fn scroll_copy_mode_clamps() {
        assert_eq!(scroll_copy_mode(100, -200, 500, 24), 0);
        assert_eq!(scroll_copy_mode(100, 600, 500, 24), 500);
        assert_eq!(scroll_copy_mode(100, 50, 500, 24), 150);
    }

    #[test]
    fn scroll_copy_mode_no_scrollback() {
        assert_eq!(scroll_copy_mode(0, 10, 0, 24), 0);
    }
}
