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

// =========================================================================
// Search match tracking — SearchMatch, SearchState
// =========================================================================

/// A single search match position in content coordinates.
///
/// Ported from Go's `terminal.SearchMatch` and `scrollback.VimSearchMatch`.
/// `start` and `end` are character (rune) indices within the line, with
/// `end` exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchMatch {
    /// The content line index.
    pub line: usize,
    /// Start character index (inclusive).
    pub start: usize,
    /// End character index (exclusive).
    pub end: usize,
}

impl SearchMatch {
    /// Create a match at `(line, start..end)`.
    pub fn new(line: usize, start: usize, end: usize) -> Self {
        Self { line, start, end }
    }

    /// Whether this match is at or after `(line, col)`.
    pub fn at_or_after(&self, line: usize, col: usize) -> bool {
        self.line > line || (self.line == line && self.start >= col)
    }

    /// Whether this match is at or before `(line, col)`.
    pub fn at_or_before(&self, line: usize, col: usize) -> bool {
        self.line < line || (self.line == line && self.start <= col)
    }

    /// Whether this match is strictly before `(line, col)` (excludes exact
    /// position). Used for backward search initial jump so the cursor's
    /// current match is skipped.
    pub fn strictly_before(&self, line: usize, col: usize) -> bool {
        self.line < line || (self.line == line && self.start < col)
    }
}

/// Consolidated search state for copy mode — query, matches, current index,
/// and direction. Ported from Go's `CopyMode` search fields and
/// `scrollback.VimState` search fields.
#[derive(Debug, Clone, Default)]
pub struct SearchState {
    /// The active search query (empty = no search).
    pub query: String,
    /// All matches found in the content.
    pub matches: Vec<SearchMatch>,
    /// Index into `matches` of the current match.
    pub current: usize,
    /// Whether the search is forward (`true` for `/`) or backward (`false`
    /// for `?`).
    pub forward: bool,
    /// Whether the search is case-sensitive.
    pub case_sensitive: bool,
}

impl SearchState {
    /// Create empty search state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether there are any matches.
    pub fn has_matches(&self) -> bool {
        !self.matches.is_empty()
    }

    /// Number of matches.
    pub fn match_count(&self) -> usize {
        self.matches.len()
    }

    /// The current match, if any.
    pub fn current_match(&self) -> Option<&SearchMatch> {
        if self.matches.is_empty() {
            None
        } else {
            self.matches.get(self.current)
        }
    }

    /// Clear all search state.
    pub fn clear(&mut self) {
        self.query.clear();
        self.matches.clear();
        self.current = 0;
    }

    /// Advance to the next match (wraps around). Returns the new current match.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<SearchMatch> {
        if self.matches.is_empty() {
            return None;
        }
        self.current = (self.current + 1) % self.matches.len();
        self.matches.get(self.current).copied()
    }

    /// Advance to the previous match (wraps around). Returns the new current match.
    pub fn prev(&mut self) -> Option<SearchMatch> {
        if self.matches.is_empty() {
            return None;
        }
        if self.current == 0 {
            self.current = self.matches.len() - 1;
        } else {
            self.current -= 1;
        }
        self.matches.get(self.current).copied()
    }

    /// Find all matches of `query` across `line_count` lines, using
    /// `line_text` to resolve each line. Case-insensitive unless
    /// `case_sensitive` is set. Stores results in `self.matches`.
    pub fn execute(
        &mut self,
        line_count: usize,
        line_text: impl Fn(usize) -> String,
    ) {
        self.matches.clear();
        if self.query.is_empty() {
            self.current = 0;
            return;
        }
        let query = if self.case_sensitive {
            self.query.clone()
        } else {
            self.query.to_lowercase()
        };
        let q_len = query.chars().count();
        if q_len == 0 {
            self.current = 0;
            return;
        }
        for line in 0..line_count {
            let raw = line_text(line);
            let hay = if self.case_sensitive {
                raw.clone()
            } else {
                raw.to_lowercase()
            };
            let hay_chars: Vec<char> = hay.chars().collect();
            if hay_chars.len() < q_len {
                continue;
            }
            for start in 0..=(hay_chars.len() - q_len) {
                if hay_chars[start..start + q_len]
                    .iter()
                    .collect::<String>()
                    == query
                {
                    self.matches
                        .push(SearchMatch::new(line, start, start + q_len));
                    // Limit to 1000 matches like Go.
                    if self.matches.len() >= 1000 {
                        break;
                    }
                }
            }
            if self.matches.len() >= 1000 {
                break;
            }
        }
        self.current = 0;
    }

    /// After `execute`, jump to the first match at or after `(line, col)` for
    /// forward search, or the closest match at or before for backward search.
    /// Returns the match jumped to.
    pub fn jump_initial(&mut self, line: usize, col: usize) -> Option<SearchMatch> {
        if self.matches.is_empty() {
            return None;
        }
        if self.forward {
            // Find first match at or after cursor.
            let idx = self
                .matches
                .iter()
                .position(|m| m.at_or_after(line, col))
                .unwrap_or(0);
            self.current = idx;
        } else {
            // Find last match strictly before cursor (skip current match).
            let idx = self
                .matches
                .iter()
                .rposition(|m| m.strictly_before(line, col))
                .unwrap_or(self.matches.len() - 1);
            self.current = idx;
        }
        self.current_match().copied()
    }

    /// Matches on a specific line (for highlighting).
    pub fn matches_on_line(&self, line: usize) -> Vec<SearchMatch> {
        self.matches
            .iter()
            .filter(|m| m.line == line)
            .copied()
            .collect()
    }
}

// =========================================================================
// Count prefix state — vim-style {count}motion
// =========================================================================

/// Accumulates a vim-style count prefix (digits typed before a command).
///
/// Ported from Go's `CopyMode.PendingCount`. A count of 0 means no count has
/// been entered; `consume()` returns the accumulated count or 1 (the default
/// vim count) when none was entered.
#[derive(Debug, Clone, Default)]
pub struct CountState {
    count: usize,
}

impl CountState {
    /// Create empty count state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a count is being accumulated.
    pub fn active(&self) -> bool {
        self.count > 0
    }

    /// The current accumulated count (0 if none).
    pub fn value(&self) -> usize {
        self.count
    }

    /// Feed a digit to the count. Returns `true` if the digit was consumed.
    /// A leading `0` is only part of a count if a count is already active
    /// (e.g., `10`, `20`); a standalone `0` is the "start of line" command.
    pub fn feed(&mut self, digit: u8) -> bool {
        let d = (digit - b'0') as usize;
        if d == 0 && !self.active() {
            return false;
        }
        self.count = self.count.saturating_mul(10).saturating_add(d);
        true
    }

    /// Consume the count, resetting to 0. Returns the count, or 1 (default).
    pub fn consume(&mut self) -> usize {
        let c = self.count;
        self.count = 0;
        if c == 0 {
            1
        } else {
            c
        }
    }

    /// Reset the count to 0 without consuming.
    pub fn reset(&mut self) {
        self.count = 0;
    }
}

// =========================================================================
// Paragraph motion — skip blank lines to next paragraph start
// =========================================================================

/// Find the start of the next paragraph from `start_line`.
///
/// A paragraph boundary is a blank line. This skips the current paragraph's
/// non-blank lines, then skips blank lines, landing on the first non-blank
/// line of the next paragraph. Ported from Go's `moveParagraphDown` and
/// `scrollback.VimState.ParagraphDown`.
///
/// Returns the target line, or `start_line` if none found.
pub fn paragraph_forward(
    line_count: usize,
    start_line: usize,
    line_text: impl Fn(usize) -> String,
) -> usize {
    if line_count == 0 {
        return start_line;
    }
    let mut i = start_line;
    // Skip non-blank lines (current paragraph).
    while i < line_count && !line_text(i).trim().is_empty() {
        i += 1;
    }
    // Skip blank lines (separator).
    while i < line_count && line_text(i).trim().is_empty() {
        i += 1;
    }
    if i >= line_count {
        line_count - 1
    } else {
        i
    }
}

/// Find the start of the previous paragraph from `start_line`.
///
/// Skips blank lines backward, then non-blank lines, then blank lines again,
/// landing on the first non-blank line of the previous paragraph. Ported from
/// Go's `moveParagraphUp` and `scrollback.VimState.ParagraphUp`.
///
/// Returns the target line, or `start_line` if none found.
pub fn paragraph_backward(
    start_line: usize,
    line_text: impl Fn(usize) -> String,
) -> usize {
    if start_line == 0 {
        return 0;
    }
    let mut i = start_line;
    // If current line is non-blank, find start of current paragraph.
    if !line_text(i).trim().is_empty() {
        let start = i;
        while i > 0 && !line_text(i - 1).trim().is_empty() {
            i -= 1;
        }
        // If we moved, we found the start of the current paragraph.
        if i != start {
            return i;
        }
        // Already at start of current paragraph — step back to find previous.
        i = i.saturating_sub(1);
    }
    // Skip blank lines backward.
    while i > 0 && line_text(i).trim().is_empty() {
        i -= 1;
    }
    // Skip non-blank lines backward to find start of previous paragraph.
    while i > 0 && !line_text(i - 1).trim().is_empty() {
        i -= 1;
    }
    i
}

// =========================================================================
// Styled output extraction — extract styled text from cells
// =========================================================================

/// A styled text fragment: content plus its style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyledFragment {
    /// The text content.
    pub text: String,
    /// The style (foreground, background, decoration).
    pub style: crate::vt::cell::Style,
}

/// Extract styled text from a slice of cells, skipping continuation cells
/// (width=0) of wide characters. Empty cells become spaces with default style.
///
/// Ported from Go's `extractLineTextFromCells` and `extractVisualText`.
pub fn extract_styled_line(cells: &[crate::vt::cell::Cell]) -> Vec<StyledFragment> {
    let mut out = Vec::new();
    for cell in cells {
        // Skip continuation cells of wide characters.
        if cell.width == 0 {
            continue;
        }
        if cell.content.is_none() {
            out.push(StyledFragment {
                text: " ".into(),
                style: cell.style,
            });
        } else {
            out.push(StyledFragment {
                text: cell.content.map(|c| c.to_string()).unwrap_or_default(),
                style: cell.style,
            });
        }
    }
    out
}

/// Extract styled text from a range of cells (column range), for selection
/// extraction. Filters out continuation cells and empty trailing cells.
///
/// `start_col` and `end_col` are inclusive cell-column indices.
pub fn extract_styled_range(
    cells: &[crate::vt::cell::Cell],
    start_col: usize,
    end_col: usize,
) -> Vec<StyledFragment> {
    let mut out = Vec::new();
    let lo = start_col.min(cells.len().saturating_sub(1));
    let hi = end_col.min(cells.len().saturating_sub(1));
    for cell in &cells[lo..=hi] {
        if cell.width == 0 {
            continue;
        }
        if cell.content.is_none() {
            // Preserve internal spaces but not trailing empty cells.
            out.push(StyledFragment {
                text: " ".into(),
                style: cell.style,
            });
        } else {
            out.push(StyledFragment {
                text: cell.content.map(|c| c.to_string()).unwrap_or_default(),
                style: cell.style,
            });
        }
    }
    // Trim trailing space fragments.
    while out.last().map(|f| f.text == " ").unwrap_or(false) {
        out.pop();
    }
    out
}

/// Convert styled fragments to plain text.
pub fn styled_to_plain(fragments: &[StyledFragment]) -> String {
    let mut out = String::new();
    for f in fragments {
        out.push_str(&f.text);
    }
    out
}

// =========================================================================
// Garbage detection — filter non-text garbage from selections
// =========================================================================

/// Characters considered garbage (control characters, zero-width, etc.).
const GARBAGE_CHARS: &[char] = &[
    '\0', '\x07', '\x08', '\x0b', '\x0c', '\x1b', '\x7f', '\u{200b}', '\u{200d}',
];

/// Whether a character is "garbage" — a control character or zero-width space
/// that should be filtered from selections.
pub fn is_garbage(c: char) -> bool {
    GARBAGE_CHARS.contains(&c) || (c.is_control() && c != '\n' && c != '\t')
}

/// Clean selection text by removing garbage characters and trimming.
///
/// Ported from Go's `extractVisualText` which filters empty cells and
/// preserves internal spaces. This also strips control characters that
/// leaked into the cell buffer.
pub fn clean_selection_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if is_garbage(c) {
            continue;
        }
        out.push(c);
    }
    out.trim().to_string()
}

// =========================================================================
// Command text cleaning — clean up command text before extraction
// =========================================================================

/// Clean up command text extracted from a prompt line.
///
/// Strips leading prompt sigils (`$`, `#`, `>`, `%`, `❯`), trailing
/// whitespace, and control characters. Ported from Go's prompt-based block
/// parsing which trims the command line.
pub fn clean_command_text(text: &str) -> String {
    // First strip ANSI CSI/OSC escape sequences (e.g. \x1b[0m).
    let stripped = strip_ansi_escapes(text);
    let mut cleaned = String::with_capacity(stripped.len());
    for c in stripped.chars() {
        if is_garbage(c) {
            continue;
        }
        cleaned.push(c);
    }
    // Strip leading prompt sigils and whitespace.
    let trimmed = cleaned.trim_start();
    let trimmed = trimmed
        .strip_prefix("$ ")
        .or_else(|| trimmed.strip_prefix("# "))
        .or_else(|| trimmed.strip_prefix("> "))
        .or_else(|| trimmed.strip_prefix("% "))
        .unwrap_or(trimmed);
    trimmed.trim().to_string()
}

/// Remove ANSI CSI (`ESC [ ... letter`) and OSC (`ESC ] ... BEL/ST`) sequences.
fn strip_ansi_escapes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            // Match the actual ESC character (U+001B).
            out.push(c);
            continue;
        }
        // ESC sequence
        match chars.peek() {
            Some('[') => {
                chars.next();
                // Consume until we hit a final byte (0x40..=0x7E).
                while let Some(&p) = chars.peek() {
                    chars.next();
                    if p as u32 >= 0x40 && p as u32 <= 0x7E {
                        break;
                    }
                }
            }
            Some(']') => {
                chars.next();
                // OSC: consume until BEL (\x07) or ST (ESC \\).
                while let Some(p) = chars.next() {
                    if p == '\x07' {
                        break;
                    }
                    if p == '\x1b' {
                        if chars.peek() == Some(&'\\') {
                            chars.next();
                        }
                        break;
                    }
                }
            }
            _ => {
                // Other escape sequences: skip the next char if present.
                chars.next();
            }
        }
    }
    out
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

#[cfg(test)]
mod search_match_tests {
    use super::*;

    #[test]
    fn search_match_basic() {
        let m = SearchMatch::new(5, 10, 14);
        assert_eq!(m.line, 5);
        assert_eq!(m.start, 10);
        assert_eq!(m.end, 14);
    }

    #[test]
    fn search_match_at_or_after() {
        let m = SearchMatch::new(5, 10, 14);
        assert!(m.at_or_after(5, 10));
        assert!(m.at_or_after(4, 0));
        assert!(!m.at_or_after(5, 11));
        assert!(!m.at_or_after(6, 0));
    }

    #[test]
    fn search_match_at_or_before() {
        let m = SearchMatch::new(5, 10, 14);
        assert!(m.at_or_before(5, 10));
        assert!(m.at_or_before(6, 0));
        assert!(m.at_or_before(5, 11));
        assert!(!m.at_or_before(4, 0));
    }
}

#[cfg(test)]
mod search_state_tests {
    use super::*;

    fn sample_lines() -> Vec<String> {
        [
            "hello world".to_string(),
            "foo bar".to_string(),
            "hello again".to_string(),
        ]
        .into_iter()
        .collect()
    }

    #[test]
    fn execute_finds_all_matches() {
        let lines = sample_lines();
        let text = |i: usize| lines[i].clone();
        let mut s = SearchState::new();
        s.query = "hello".into();
        s.execute(3, text);
        assert_eq!(s.match_count(), 2);
        assert_eq!(s.matches[0].line, 0);
        assert_eq!(s.matches[1].line, 2);
    }

    #[test]
    fn execute_case_insensitive() {
        let lines = ["Hello World".to_string(), "HELLO there".to_string()];
        let text = |i: usize| lines[i].clone();
        let mut s = SearchState::new();
        s.query = "hello".into();
        s.execute(2, text);
        assert_eq!(s.match_count(), 2);
    }

    #[test]
    fn execute_case_sensitive() {
        let lines = ["Hello World".to_string(), "hello there".to_string()];
        let text = |i: usize| lines[i].clone();
        let mut s = SearchState::new();
        s.query = "hello".into();
        s.case_sensitive = true;
        s.execute(2, text);
        assert_eq!(s.match_count(), 1);
        assert_eq!(s.matches[0].line, 1);
    }

    #[test]
    fn execute_empty_query_clears() {
        let lines = sample_lines();
        let text = |i: usize| lines[i].clone();
        let mut s = SearchState::new();
        s.query = "hello".into();
        s.execute(3, text);
        assert!(s.has_matches());
        s.query.clear();
        s.execute(3, text);
        assert!(!s.has_matches());
    }

    #[test]
    fn next_wraps_around() {
        let lines = sample_lines();
        let text = |i: usize| lines[i].clone();
        let mut s = SearchState::new();
        s.query = "hello".into();
        s.execute(3, text);
        s.current = 0;
        s.next();
        assert_eq!(s.current, 1);
        s.next();
        assert_eq!(s.current, 0); // wraps
    }

    #[test]
    fn prev_wraps_around() {
        let lines = sample_lines();
        let text = |i: usize| lines[i].clone();
        let mut s = SearchState::new();
        s.query = "hello".into();
        s.execute(3, text);
        s.current = 0;
        s.prev();
        assert_eq!(s.current, 1); // wraps to last
    }

    #[test]
    fn jump_initial_forward() {
        let lines = sample_lines();
        let text = |i: usize| lines[i].clone();
        let mut s = SearchState::new();
        s.query = "hello".into();
        s.forward = true;
        s.execute(3, text);
        // From line 0 col 5, first match at or after is line 2.
        let m = s.jump_initial(0, 5);
        assert_eq!(m.unwrap().line, 2);
        assert_eq!(s.current, 1);
    }

    #[test]
    fn jump_initial_backward() {
        let lines = sample_lines();
        let text = |i: usize| lines[i].clone();
        let mut s = SearchState::new();
        s.query = "hello".into();
        s.forward = false;
        s.execute(3, text);
        // From line 2 col 0, last match at or before is line 0.
        let m = s.jump_initial(2, 0);
        assert_eq!(m.unwrap().line, 0);
        assert_eq!(s.current, 0);
    }

    #[test]
    fn matches_on_line() {
        let lines = ["ab ab ab".to_string()];
        let text = |i: usize| lines[i].clone();
        let mut s = SearchState::new();
        s.query = "ab".into();
        s.execute(1, text);
        let on_line = s.matches_on_line(0);
        assert_eq!(on_line.len(), 3);
    }

    #[test]
    fn clear_resets_state() {
        let mut s = SearchState::new();
        s.query = "test".into();
        s.matches.push(SearchMatch::new(0, 0, 4));
        s.current = 5;
        s.clear();
        assert!(s.query.is_empty());
        assert!(s.matches.is_empty());
        assert_eq!(s.current, 0);
    }

    #[test]
    fn current_match_returns_none_when_empty() {
        let s = SearchState::new();
        assert!(s.current_match().is_none());
    }

    #[test]
    fn execute_limits_to_1000_matches() {
        let line = "a".repeat(2000);
        let lines = [line];
        let text = |i: usize| lines[i].clone();
        let mut s = SearchState::new();
        s.query = "a".into();
        s.execute(1, text);
        assert_eq!(s.match_count(), 1000);
    }
}

#[cfg(test)]
mod count_state_tests {
    use super::*;

    #[test]
    fn default_is_inactive() {
        let cs = CountState::new();
        assert!(!cs.active());
        assert_eq!(cs.value(), 0);
    }

    #[test]
    fn feed_digit_accumulates() {
        let mut cs = CountState::new();
        assert!(cs.feed(b'1'));
        assert_eq!(cs.value(), 1);
        assert!(cs.feed(b'0'));
        assert_eq!(cs.value(), 10);
        assert!(cs.feed(b'5'));
        assert_eq!(cs.value(), 105);
    }

    #[test]
    fn leading_zero_not_consumed() {
        let mut cs = CountState::new();
        assert!(!cs.feed(b'0'));
        assert!(!cs.active());
    }

    #[test]
    fn zero_after_count_is_consumed() {
        let mut cs = CountState::new();
        assert!(cs.feed(b'1'));
        assert!(cs.feed(b'0'));
        assert_eq!(cs.value(), 10);
    }

    #[test]
    fn consume_returns_count_or_default() {
        let mut cs = CountState::new();
        assert_eq!(cs.consume(), 1); // default
        cs.feed(b'5');
        assert_eq!(cs.consume(), 5);
        assert!(!cs.active());
    }

    #[test]
    fn reset_clears() {
        let mut cs = CountState::new();
        cs.feed(b'3');
        cs.reset();
        assert!(!cs.active());
        assert_eq!(cs.value(), 0);
    }
}

#[cfg(test)]
mod paragraph_motion_tests {
    use super::*;

    fn lines() -> Vec<String> {
        [
            "first paragraph".to_string(),
            "second line".to_string(),
            "".to_string(),
            "".to_string(),
            "second paragraph".to_string(),
            "its line".to_string(),
            "".to_string(),
            "third paragraph".to_string(),
        ]
        .into_iter()
        .collect()
    }

    #[test]
    fn paragraph_forward_skips_blanks() {
        let ls = lines();
        let text = |i: usize| ls[i].clone();
        // From line 0, next paragraph starts at line 4.
        assert_eq!(paragraph_forward(8, 0, text), 4);
    }

    #[test]
    fn paragraph_forward_from_middle() {
        let ls = lines();
        let text = |i: usize| ls[i].clone();
        // From line 1, next paragraph starts at line 4.
        assert_eq!(paragraph_forward(8, 1, text), 4);
    }

    #[test]
    fn paragraph_forward_at_end_returns_last() {
        let ls = lines();
        let text = |i: usize| ls[i].clone();
        // From line 7 (last), no next paragraph — returns last line.
        assert_eq!(paragraph_forward(8, 7, text), 7);
    }

    #[test]
    fn paragraph_backward_skips_blanks() {
        let ls = lines();
        let text = |i: usize| ls[i].clone();
        // From line 4, previous paragraph starts at line 0.
        assert_eq!(paragraph_backward(4, text), 0);
    }

    #[test]
    fn paragraph_backward_from_middle() {
        let ls = lines();
        let text = |i: usize| ls[i].clone();
        // From line 5, previous paragraph starts at line 4.
        assert_eq!(paragraph_backward(5, text), 4);
    }

    #[test]
    fn paragraph_backward_at_start_returns_zero() {
        let ls = lines();
        let text = |i: usize| ls[i].clone();
        assert_eq!(paragraph_backward(0, text), 0);
    }

    #[test]
    fn paragraph_forward_multiple_blanks() {
        let ls = ["a".to_string(), "".into(), "".into(), "".into(), "b".into()];
        let text = |i: usize| ls[i].clone();
        assert_eq!(paragraph_forward(5, 0, text), 4);
    }
}

#[cfg(test)]
mod styled_extraction_tests {
    use super::*;
    use crate::vt::cell::{Cell, Style};

    #[test]
    fn extract_styled_line_basic() {
        let cells = [
            Cell::new('h', 1, Style::default()),
            Cell::new('i', 1, Style::default()),
            Cell::new_empty(1, Style::default()),
            Cell::new('!', 1, Style::default()),
        ];
        let frags = extract_styled_line(&cells);
        assert_eq!(frags.len(), 4);
        assert_eq!(frags[0].text, "h");
        assert_eq!(frags[2].text, " ");
        assert_eq!(frags[3].text, "!");
    }

    #[test]
    fn extract_styled_line_skips_wide_continuation() {
        let mut wide = Cell::new('🎨', 2, Style::default());
        let cont = Cell::new_empty(0, Style::default());
        let _ = &mut wide;
        let cells = [wide, cont, Cell::new('x', 1, Style::default())];
        let frags = extract_styled_line(&cells);
        assert_eq!(frags.len(), 2);
        assert_eq!(frags[0].text, "🎨");
        assert_eq!(frags[1].text, "x");
    }

    #[test]
    fn extract_styled_range_trims_trailing_spaces() {
        let cells = [
            Cell::new('a', 1, Style::default()),
            Cell::new('b', 1, Style::default()),
            Cell::new_empty(1, Style::default()),
            Cell::new_empty(1, Style::default()),
        ];
        let frags = extract_styled_range(&cells, 0, 3);
        assert_eq!(frags.len(), 2);
        assert_eq!(frags[0].text, "a");
        assert_eq!(frags[1].text, "b");
    }

    #[test]
    fn styled_to_plain_concatenates() {
        let frags = [
            StyledFragment {
                text: "hello".into(),
                style: Style::default(),
            },
            StyledFragment {
                text: " ".into(),
                style: Style::default(),
            },
            StyledFragment {
                text: "world".into(),
                style: Style::default(),
            },
        ];
        assert_eq!(styled_to_plain(&frags), "hello world");
    }
}

#[cfg(test)]
mod garbage_detection_tests {
    use super::*;

    #[test]
    fn is_garbage_control_chars() {
        assert!(is_garbage('\0'));
        assert!(is_garbage('\x07'));
        assert!(is_garbage('\x1b'));
        assert!(is_garbage('\u{200b}'));
    }

    #[test]
    fn is_garbage_not_normal_chars() {
        assert!(!is_garbage('a'));
        assert!(!is_garbage(' '));
        assert!(!is_garbage('\n'));
        assert!(!is_garbage('\t'));
    }

    #[test]
    fn clean_selection_text_removes_garbage() {
        let input = "he\x1bllo\x07 wor\u{200b}ld";
        assert_eq!(clean_selection_text(input), "hello world");
    }

    #[test]
    fn clean_selection_text_trims() {
        assert_eq!(clean_selection_text("  hello  "), "hello");
    }

    #[test]
    fn clean_selection_text_preserves_newlines() {
        assert_eq!(clean_selection_text("a\nb"), "a\nb");
    }
}

#[cfg(test)]
mod command_cleaning_tests {
    use super::*;

    #[test]
    fn clean_strips_dollar_prompt() {
        assert_eq!(clean_command_text("$ ls -la"), "ls -la");
    }

    #[test]
    fn clean_strips_hash_prompt() {
        assert_eq!(clean_command_text("# whoami"), "whoami");
    }

    #[test]
    fn clean_strips_angle_prompt() {
        assert_eq!(clean_command_text("> echo hi"), "echo hi");
    }

    #[test]
    fn clean_strips_percent_prompt() {
        assert_eq!(clean_command_text("% ls"), "ls");
    }

    #[test]
    fn clean_removes_control_chars() {
        assert_eq!(clean_command_text("$ \x1b[0mls"), "ls");
    }

    #[test]
    fn clean_plain_command() {
        assert_eq!(clean_command_text("ls -la"), "ls -la");
    }

    #[test]
    fn clean_trims_whitespace() {
        assert_eq!(clean_command_text("  ls  "), "ls");
    }
}
