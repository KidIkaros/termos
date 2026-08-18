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
    /// Whitespace-delimited word forward (W).
    WordForwardBig,
    /// Whitespace-delimited word backward (B).
    WordBackwardBig,
    /// Whitespace-delimited end of word forward (E).
    WordEndBig,
    /// Whitespace-delimited end of word backward (gE).
    WordEndBackwardBig,
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
    // The "big" motions (W/B/E) split on whitespace only.
    let big = matches!(
        motion,
        WordMotion::WordForwardBig
            | WordMotion::WordBackwardBig
            | WordMotion::WordEndBig
            | WordMotion::WordEndBackwardBig
    );

    match motion {
        WordMotion::WordForward | WordMotion::WordForwardBig => {
            let mut i = start_col;
            // Skip current word
            let word_char = |c: char| {
                if big {
                    !c.is_whitespace()
                } else {
                    is_word_char(c)
                }
            };
            if i < len && word_char(chars[i]) {
                while i < len && word_char(chars[i]) {
                    i += 1;
                }
            } else if i < len && !chars[i].is_whitespace() {
                while i < len && !word_char(chars[i]) && !chars[i].is_whitespace() {
                    i += 1;
                }
            }
            // Skip whitespace
            while i < len && chars[i].is_whitespace() {
                i += 1;
            }
            i.min(len)
        }
        WordMotion::WordBackward | WordMotion::WordBackwardBig => {
            if start_col == 0 {
                return 0;
            }
            let mut i = start_col - 1;
            let word_char = |c: char| {
                if big {
                    !c.is_whitespace()
                } else {
                    is_word_char(c)
                }
            };
            // Skip whitespace
            while i > 0 && chars[i].is_whitespace() {
                i -= 1;
            }
            // Skip current word
            if word_char(chars[i]) {
                while i > 0 && word_char(chars[i - 1]) {
                    i -= 1;
                }
            } else if !chars[i].is_whitespace() {
                while i > 0 && !word_char(chars[i - 1]) && !chars[i - 1].is_whitespace() {
                    i -= 1;
                }
            }
            i
        }
        WordMotion::WordEnd | WordMotion::WordEndBig => {
            let mut i = start_col + 1;
            let word_char = |c: char| {
                if big {
                    !c.is_whitespace()
                } else {
                    is_word_char(c)
                }
            };
            // Skip whitespace
            while i < len && chars[i].is_whitespace() {
                i += 1;
            }
            if i >= len {
                return len.saturating_sub(1);
            }
            // Move to end of word
            if word_char(chars[i]) {
                while i + 1 < len && word_char(chars[i + 1]) {
                    i += 1;
                }
            } else {
                while i + 1 < len && !word_char(chars[i + 1]) && !chars[i + 1].is_whitespace() {
                    i += 1;
                }
            }
            i.min(len.saturating_sub(1))
        }
        WordMotion::WordEndBackward | WordMotion::WordEndBackwardBig => {
            if start_col == 0 {
                return 0;
            }
            let mut i = start_col - 1;
            let word_char = |c: char| {
                if big {
                    !c.is_whitespace()
                } else {
                    is_word_char(c)
                }
            };
            // Skip whitespace, then the current word, to its first char.
            while i > 0 && chars[i].is_whitespace() {
                i -= 1;
            }
            while i > 0 && word_char(chars[i - 1]) {
                i -= 1;
            }
            // One more left clears the current word; trailing whitespace
            // lands us on the last char of the previous word (ge/gE).
            if i > 0 {
                i = i.saturating_sub(1);
            }
            while i > 0 && chars[i].is_whitespace() {
                i -= 1;
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

// =========================================================================
// Mark system — vim-style marks (m{letter} to set, '{letter} / `{letter})
// =========================================================================

/// A stored mark position in absolute content coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Mark {
    /// Absolute content line.
    pub line: usize,
    /// Column (character index) within the line.
    pub col: usize,
}

/// In-memory store for vim-style marks.
///
/// Lowercase letters `a`–`z` are local marks (per window/session). Uppercase
/// `A`–`Z` are file/global marks (shared, but stored here for simplicity). The
/// digit marks `0`–`9` are not used by copy mode but the store accepts any
/// `char` key so callers can extend it.
#[derive(Debug, Clone, Default)]
pub struct MarkStore {
    marks: std::collections::HashMap<char, Mark>,
}

impl MarkStore {
    /// Create an empty mark store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set a mark at `(line, col)` for the given letter.
    pub fn set(&mut self, letter: char, line: usize, col: usize) {
        self.marks.insert(letter, Mark { line, col });
    }

    /// Get the mark for a letter, if set.
    pub fn get(&self, letter: char) -> Option<Mark> {
        self.marks.get(&letter).copied()
    }

    /// Remove a single mark.
    pub fn remove(&mut self, letter: char) -> Option<Mark> {
        self.marks.remove(&letter)
    }

    /// Clear all marks.
    pub fn clear(&mut self) {
        self.marks.clear();
    }

    /// Number of marks stored.
    pub fn len(&self) -> usize {
        self.marks.len()
    }

    /// Whether the store is empty.
    pub fn is_empty(&self) -> bool {
        self.marks.is_empty()
    }

    /// Iterate over all (letter, mark) pairs.
    pub fn iter(&self) -> impl Iterator<Item = (char, Mark)> + '_ {
        self.marks.iter().map(|(&c, &m)| (c, m))
    }
}

// =========================================================================
// Register system — named yank registers ("{letter}y / "{letter}p)
// =========================================================================

/// Whether a register's contents are line-wise or character-wise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RegisterKind {
    /// Character-wise selection (default).
    #[default]
    Char,
    /// Line-wise selection (whole lines).
    Line,
}

/// One yank register's contents.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Register {
    /// The yanked text.
    pub text: String,
    /// Whether the yank was line-wise.
    pub kind: RegisterKind,
}

impl Register {
    /// Create a character-wise register.
    pub fn char(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: RegisterKind::Char,
        }
    }

    /// Create a line-wise register.
    pub fn line(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: RegisterKind::Line,
        }
    }

    /// Whether the register is empty.
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}

/// In-memory store for vim-style yank registers.
///
/// The unnamed register (`"`) is always updated on yank. Named registers
/// `a`–`z` are set explicitly with `"{letter}y`. Uppercase `A`–`Z` append to
/// the corresponding lowercase register.
#[derive(Debug, Clone, Default)]
pub struct RegisterStore {
    registers: std::collections::HashMap<char, Register>,
}

impl RegisterStore {
    /// Create an empty register store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Yank text into the unnamed register (and optionally a named one).
    /// `register` is the letter after `"`, or `None` for the default yank.
    pub fn yank(&mut self, register: Option<char>, text: &str, kind: RegisterKind) {
        let reg = Register {
            text: text.to_string(),
            kind,
        };
        // Always update the unnamed register.
        self.registers.insert('"', reg.clone());
        if let Some(letter) = register {
            if letter.is_ascii_uppercase() {
                // Append to the lowercase equivalent.
                let lower = letter.to_ascii_lowercase();
                let entry = self.registers.entry(lower).or_default();
                if entry.kind != kind {
                    // Kind mismatch: replace rather than append.
                    *entry = reg;
                } else {
                    entry.text.push_str(&reg.text);
                }
            } else {
                self.registers.insert(letter, reg);
            }
        }
    }

    /// Get the contents of a register. `register` is the letter or `"` for the
    /// unnamed register.
    pub fn get(&self, register: char) -> Option<&Register> {
        self.registers.get(&register)
    }

    /// Get the contents of the unnamed register.
    pub fn unnamed(&self) -> Option<&Register> {
        self.registers.get(&'"')
    }

    /// Clear a single register.
    pub fn remove(&mut self, register: char) -> Option<Register> {
        self.registers.remove(&register)
    }

    /// Clear all registers.
    pub fn clear(&mut self) {
        self.registers.clear();
    }

    /// Number of registers stored.
    pub fn len(&self) -> usize {
        self.registers.len()
    }

    /// Whether the store is empty.
    pub fn is_empty(&self) -> bool {
        self.registers.is_empty()
    }
}

// =========================================================================
// Sentence motions — ( and )
// =========================================================================

/// Characters that terminate a sentence.
const SENTENCE_TERMINATORS: &[char] = &['.', '!', '?'];

/// Find the start of the next sentence from `(start_line, start_col)`.
///
/// A sentence ends at a `.`, `!`, or `?` followed by whitespace (or end of
/// line). The next sentence starts at the first non-whitespace character after
/// that. Returns `(line, col)` of the next sentence start, or the original
/// position if none is found.
///
/// `line_text` resolves a content line index to its text.
pub fn next_sentence(
    line_count: usize,
    start_line: usize,
    start_col: usize,
    line_text: impl Fn(usize) -> String,
) -> (usize, usize) {
    let mut line = start_line;
    let mut col = start_col;

    while line < line_count {
        let text = line_text(line);
        let chars: Vec<char> = text.chars().collect();

        // Search for a sentence terminator followed by whitespace.
        while col < chars.len() {
            if SENTENCE_TERMINATORS.contains(&chars[col]) {
                // Check the rest of the line for whitespace after the
                // terminator (possibly multiple terminators in a row like
                // "..." or "?!").
                let mut j = col;
                while j < chars.len() && SENTENCE_TERMINATORS.contains(&chars[j]) {
                    j += 1;
                }
                // Skip whitespace after the terminators.
                while j < chars.len() && chars[j].is_whitespace() {
                    j += 1;
                }
                if j < chars.len() {
                    return (line, j);
                }
                // Terminator at end of line — next sentence starts on the
                // next non-blank line.
                break;
            }
            col += 1;
        }

        // Move to the next line; skip blank lines.
        line += 1;
        col = 0;
        while line < line_count {
            let t = line_text(line);
            if !t.trim().is_empty() {
                // Find first non-whitespace.
                let first = t
                    .char_indices()
                    .skip_while(|(_, c)| c.is_whitespace())
                    .map(|(i, _)| i)
                    .next()
                    .unwrap_or(0);
                return (line, first);
            }
            line += 1;
        }
    }
    (start_line, start_col)
}

/// Find the start of the previous sentence from `(start_line, start_col)`.
///
/// Searches backward for a sentence boundary. Returns `(line, col)` of the
/// previous sentence start, or the original position if none is found.
///
/// A sentence starts at:
/// - The first non-whitespace character after a sentence terminator
///   (`.`, `!`, `?`) followed by whitespace or end-of-line.
/// - The first non-whitespace character after a blank line (paragraph
///   boundary).
/// - The first non-whitespace character of the buffer.
///
/// This function scans from line 0 up to `start_line`, collecting sentence
/// starts, and returns the last one strictly before `(start_line, start_col)`.
pub fn prev_sentence(
    start_line: usize,
    start_col: usize,
    line_text: impl Fn(usize) -> String,
) -> (usize, usize) {
    let mut last_start: Option<(usize, usize)> = None;
    let mut after_blank = true;

    for line in 0..=start_line {
        let text = line_text(line);
        let chars: Vec<char> = text.chars().collect();
        let is_blank = text.trim().is_empty();

        if is_blank {
            after_blank = true;
            continue;
        }

        let mut col = 0;
        // If this line follows a blank line, the first non-whitespace is a
        // sentence start.
        if after_blank {
            while col < chars.len() && chars[col].is_whitespace() {
                col += 1;
            }
            if col < chars.len() {
                let pos = (line, col);
                if pos < (start_line, start_col) {
                    last_start = Some(pos);
                } else {
                    break;
                }
            }
            after_blank = false;
        }

        // Scan the line for sentence terminators followed by whitespace.
        while col < chars.len() {
            if SENTENCE_TERMINATORS.contains(&chars[col]) {
                // Skip the terminator group.
                let mut j = col;
                while j < chars.len() && SENTENCE_TERMINATORS.contains(&chars[j]) {
                    j += 1;
                }
                // Skip whitespace after the terminators.
                while j < chars.len() && chars[j].is_whitespace() {
                    j += 1;
                }
                if j < chars.len() {
                    let pos = (line, j);
                    if pos < (start_line, start_col) {
                        last_start = Some(pos);
                    } else {
                        return last_start.unwrap_or((start_line, start_col));
                    }
                }
                // Terminator at end of line — next non-blank line's first
                // non-whitespace is a sentence start (handled by after_blank).
                col = j;
            } else {
                col += 1;
            }
        }
    }

    last_start.unwrap_or((start_line, start_col))
}

// =========================================================================
// Goto line — {count}G helper
// =========================================================================

/// Compute the target content line for a `gg` or `{count}G` motion.
///
/// - `count` of 0 means `gg` (go to first line).
/// - `count` > 0 means `{count}G` (go to line `count`, 1-indexed).
///
/// Returns the 0-indexed line, clamped to `[0, total_lines)`.
pub fn goto_line(count: usize, total_lines: usize) -> usize {
    if total_lines == 0 {
        return 0;
    }
    if count == 0 {
        return 0;
    }
    // 1-indexed to 0-indexed, clamped.
    (count - 1).min(total_lines.saturating_sub(1))
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

#[cfg(test)]
mod big_motion_tests {
    use super::*;

    #[test]
    fn w_forward_alnum() {
        let line = "hello world foo";
        assert_eq!(word_motion(line, 0, WordMotion::WordForward), 6);
    }

    #[test]
    fn w_big_forward_whitespace() {
        // "foo-bar" is one big word (hyphen not whitespace).
        let line = "foo-bar baz";
        assert_eq!(word_motion(line, 0, WordMotion::WordForwardBig), 8);
        // Plain w treats punctuation as its own word (established behavior).
        assert_eq!(word_motion(line, 0, WordMotion::WordForward), 3);
    }

    #[test]
    fn b_big_backward() {
        let line = "foo-bar baz";
        // From inside "baz", big-b goes to the start of "baz".
        assert_eq!(word_motion(line, 9, WordMotion::WordBackwardBig), 8);
    }

    #[test]
    fn e_big_end() {
        let line = "foo-bar baz";
        // From 0, big-e lands on the end of "foo-bar".
        assert_eq!(word_motion(line, 0, WordMotion::WordEndBig), 6);
        // Plain e lands on the end of "foo".
        assert_eq!(word_motion(line, 0, WordMotion::WordEnd), 2);
    }

    #[test]
    fn ge_big_end_backward() {
        let line = "foo bar baz";
        // From the end, ge lands on the last char of "bar".
        assert_eq!(word_motion(line, 10, WordMotion::WordEndBackwardBig), 6);
    }
}

#[cfg(test)]
mod mark_tests {
    use super::*;

    #[test]
    fn set_and_get_mark() {
        let mut store = MarkStore::new();
        assert!(store.get('a').is_none());
        store.set('a', 10, 5);
        let m = store.get('a').unwrap();
        assert_eq!((m.line, m.col), (10, 5));
    }

    #[test]
    fn overwrite_mark() {
        let mut store = MarkStore::new();
        store.set('a', 1, 0);
        store.set('a', 20, 3);
        let m = store.get('a').unwrap();
        assert_eq!((m.line, m.col), (20, 3));
    }

    #[test]
    fn remove_mark() {
        let mut store = MarkStore::new();
        store.set('b', 5, 2);
        assert!(store.remove('b').is_some());
        assert!(store.get('b').is_none());
    }

    #[test]
    fn clear_all_marks() {
        let mut store = MarkStore::new();
        store.set('a', 1, 0);
        store.set('z', 100, 10);
        assert_eq!(store.len(), 2);
        store.clear();
        assert!(store.is_empty());
    }

    #[test]
    fn uppercase_marks_stored_separately() {
        let mut store = MarkStore::new();
        store.set('a', 1, 0);
        store.set('A', 50, 5);
        assert_eq!(store.get('a').unwrap().line, 1);
        assert_eq!(store.get('A').unwrap().line, 50);
    }

    #[test]
    fn iter_marks() {
        let mut store = MarkStore::new();
        store.set('a', 1, 0);
        store.set('b', 2, 3);
        let collected: Vec<_> = store.iter().collect();
        assert_eq!(collected.len(), 2);
    }
}

#[cfg(test)]
mod register_tests {
    use super::*;

    #[test]
    fn yank_to_unnamed_only() {
        let mut store = RegisterStore::new();
        store.yank(None, "hello", RegisterKind::Char);
        let reg = store.unnamed().unwrap();
        assert_eq!(reg.text, "hello");
        assert_eq!(reg.kind, RegisterKind::Char);
    }

    #[test]
    fn yank_to_named_register() {
        let mut store = RegisterStore::new();
        store.yank(Some('a'), "world", RegisterKind::Char);
        assert_eq!(store.get('a').unwrap().text, "world");
        // Unnamed register is also updated.
        assert_eq!(store.unnamed().unwrap().text, "world");
    }

    #[test]
    fn uppercase_register_appends() {
        let mut store = RegisterStore::new();
        store.yank(Some('a'), "foo", RegisterKind::Char);
        store.yank(Some('A'), "bar", RegisterKind::Char);
        assert_eq!(store.get('a').unwrap().text, "foobar");
    }

    #[test]
    fn uppercase_register_kind_mismatch_replaces() {
        let mut store = RegisterStore::new();
        store.yank(Some('a'), "foo", RegisterKind::Char);
        store.yank(Some('A'), "bar\n", RegisterKind::Line);
        // Kind mismatch: replace, not append.
        assert_eq!(store.get('a').unwrap().text, "bar\n");
        assert_eq!(store.get('a').unwrap().kind, RegisterKind::Line);
    }

    #[test]
    fn line_wise_yank() {
        let mut store = RegisterStore::new();
        store.yank(Some('l'), "line1\nline2\n", RegisterKind::Line);
        let reg = store.get('l').unwrap();
        assert_eq!(reg.kind, RegisterKind::Line);
        assert!(reg.text.contains("line1"));
    }

    #[test]
    fn clear_registers() {
        let mut store = RegisterStore::new();
        store.yank(Some('a'), "x", RegisterKind::Char);
        store.yank(Some('b'), "y", RegisterKind::Char);
        store.clear();
        assert!(store.is_empty());
    }

    #[test]
    fn register_helpers() {
        let r = Register::char("abc");
        assert_eq!(r.text, "abc");
        assert_eq!(r.kind, RegisterKind::Char);
        assert!(!r.is_empty());

        let r2 = Register::line("hi\n");
        assert_eq!(r2.kind, RegisterKind::Line);

        let r3 = Register::default();
        assert!(r3.is_empty());
    }
}

#[cfg(test)]
mod sentence_tests {
    use super::*;

    fn lines() -> Vec<String> {
        vec![
            "First sentence. Second one!".into(),
            "".into(),
            "Third sentence? Fourth.".into(),
        ]
    }

    #[test]
    fn next_sentence_same_line() {
        let ls = lines();
        let text = |i: usize| ls[i].clone();
        let (line, col) = next_sentence(3, 0, 0, text);
        assert_eq!(line, 0);
        // "First sentence. " — second sentence starts at col 16.
        assert_eq!(col, 16);
    }

    #[test]
    fn next_sentence_across_blank_line() {
        let ls = lines();
        let text = |i: usize| ls[i].clone();
        // From end of line 0, next sentence is on line 2 (after blank line 1).
        let (line, _col) = next_sentence(3, 0, 100, text);
        assert_eq!(line, 2);
    }

    #[test]
    fn next_sentence_at_end_returns_original() {
        let ls = ["No terminator here".to_string()];
        let text = |i: usize| ls[i].clone();
        let (line, col) = next_sentence(1, 0, 0, text);
        assert_eq!((line, col), (0, 0));
    }

    #[test]
    fn prev_sentence_finds_terminator() {
        let ls = lines();
        let text = |i: usize| ls[i].clone();
        // From "Second one!" on line 0, prev sentence is "First sentence."
        let (line, col) = prev_sentence(0, 16, text);
        assert_eq!(line, 0);
        assert_eq!(col, 0);
    }

    #[test]
    fn prev_sentence_across_blank_line() {
        let ls = lines();
        let text = |i: usize| ls[i].clone();
        // From line 2, prev sentence should cross the blank line back to line 0.
        let (line, _col) = prev_sentence(2, 0, text);
        assert_eq!(line, 0);
    }

    #[test]
    fn multiple_terminators() {
        let ls = ["Wait... Is it?".to_string()];
        let text = |i: usize| ls[i].clone();
        // From col 0, "..." is a group of terminators; next sentence after
        // them starts at "Is".
        let (line, col) = next_sentence(1, 0, 0, text);
        assert_eq!(line, 0);
        // "Wait... " is 8 chars, "Is" starts at col 8.
        assert_eq!(col, 8);
    }
}

#[cfg(test)]
mod goto_line_tests {
    use super::*;

    #[test]
    fn gg_goes_to_first_line() {
        assert_eq!(goto_line(0, 100), 0);
    }

    #[test]
    fn count_g_goes_to_specific_line() {
        // 10G → line index 9.
        assert_eq!(goto_line(10, 100), 9);
    }

    #[test]
    fn count_g_clamps_to_last_line() {
        // 200G with only 50 lines → last line (index 49).
        assert_eq!(goto_line(200, 50), 49);
    }

    #[test]
    fn goto_line_empty_buffer() {
        assert_eq!(goto_line(0, 0), 0);
        assert_eq!(goto_line(5, 0), 0);
    }

    #[test]
    fn goto_line_one() {
        // 1G → line index 0.
        assert_eq!(goto_line(1, 100), 0);
    }
}
