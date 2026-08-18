//! Scrollback browser mode — a structured, vim-navigable browser over parsed
//! command blocks.
//!
//! This module encapsulates the browser state that `Os` currently holds as
//! loose fields (`browser_blocks`, `browser_selected`, `browser_mode`,
//! `browser_scroll`). It adds structured navigation: jump-to-command by index,
//! filter by exit code, and search within block output.
//!
//! The browser is entered from copy mode with `[` and displays parsed
//! [`CommandBlock`]s in one of four [`BrowseMode`]s (Commands, Output, JSON,
//! Paths). Navigation is vim-style: `j`/`k` to move, `m` to cycle modes,
//! `Enter` to jump the copy-mode cursor to the block's start line.

use super::{BrowseMode, CommandBlock};

/// The scrollback browser's state.
#[derive(Debug, Clone)]
pub struct Browser {
    /// Parsed command blocks from scrollback.
    pub blocks: Vec<CommandBlock>,
    /// Currently selected block index.
    pub selected: usize,
    /// Current display mode.
    pub mode: BrowseMode,
    /// Vertical scroll offset within the selected block's output.
    pub scroll: usize,
    /// Optional filter: only show blocks with this exit code.
    filter_exit_code: Option<i32>,
    /// Optional search query for filtering blocks by command text.
    search_query: String,
    /// Indices into `blocks` that pass the current filter.
    filtered: Vec<usize>,
}

impl Default for Browser {
    fn default() -> Self {
        Self::new()
    }
}

impl Browser {
    /// Create an empty browser.
    pub fn new() -> Self {
        Self {
            blocks: Vec::new(),
            selected: 0,
            mode: BrowseMode::Commands,
            scroll: 0,
            filter_exit_code: None,
            search_query: String::new(),
            filtered: Vec::new(),
        }
    }

    /// Load command blocks into the browser, resetting selection and scroll.
    pub fn load(&mut self, blocks: Vec<CommandBlock>) {
        self.blocks = blocks;
        self.selected = 0;
        self.scroll = 0;
        self.filter_exit_code = None;
        self.search_query.clear();
        self.rebuild_filter();
    }

    /// Clear all blocks and reset state.
    pub fn clear(&mut self) {
        self.blocks.clear();
        self.selected = 0;
        self.scroll = 0;
        self.filter_exit_code = None;
        self.search_query.clear();
        self.filtered.clear();
    }

    /// Whether the browser has any blocks.
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// Number of blocks (after filtering).
    pub fn visible_count(&self) -> usize {
        if self.filtered.is_empty() && self.filter_exit_code.is_none() && self.search_query.is_empty()
        {
            self.blocks.len()
        } else {
            self.filtered.len()
        }
    }

    /// The currently selected block, if any.
    pub fn selected_block(&self) -> Option<&CommandBlock> {
        self.visible_index(self.selected)
            .and_then(|i| self.blocks.get(i))
    }

    /// Move selection up by `delta` (wraps around).
    pub fn move_up(&mut self, delta: usize) {
        let count = self.visible_count();
        if count == 0 {
            return;
        }
        self.selected = (self.selected + count - (delta % count)) % count;
        self.scroll = 0;
    }

    /// Move selection down by `delta` (wraps around).
    pub fn move_down(&mut self, delta: usize) {
        let count = self.visible_count();
        if count == 0 {
            return;
        }
        self.selected = (self.selected + delta) % count;
        self.scroll = 0;
    }

    /// Cycle to the next display mode.
    pub fn cycle_mode(&mut self) {
        self.mode = match self.mode {
            BrowseMode::Commands => BrowseMode::Output,
            BrowseMode::Output => BrowseMode::Json,
            BrowseMode::Json => BrowseMode::Paths,
            BrowseMode::Paths => BrowseMode::Commands,
        };
        self.scroll = 0;
    }

    /// Scroll up within the selected block's output.
    pub fn scroll_up(&mut self, delta: usize) {
        self.scroll = self.scroll.saturating_sub(delta);
    }

    /// Scroll down within the selected block's output.
    pub fn scroll_down(&mut self, delta: usize) {
        self.scroll = self.scroll.saturating_add(delta);
    }

    /// Jump directly to block index `idx` (0-indexed among visible blocks).
    pub fn jump_to(&mut self, idx: usize) {
        let count = self.visible_count();
        if count == 0 {
            return;
        }
        self.selected = idx.min(count - 1);
        self.scroll = 0;
    }

    /// Jump to the next block with a non-zero exit code (error).
    /// Returns true if an error block was found.
    pub fn next_error(&mut self) -> bool {
        let count = self.visible_count();
        if count == 0 {
            return false;
        }
        for offset in 1..=count {
            let idx = (self.selected + offset) % count;
            if let Some(block) = self.visible_index(idx).and_then(|i| self.blocks.get(i)) {
                if block.exit_code != 0 && block.exit_code != -1 {
                    self.selected = idx;
                    self.scroll = 0;
                    return true;
                }
            }
        }
        false
    }

    /// Jump to the previous block with a non-zero exit code (error).
    /// Returns true if an error block was found.
    pub fn prev_error(&mut self) -> bool {
        let count = self.visible_count();
        if count == 0 {
            return false;
        }
        for offset in 1..=count {
            let idx = (self.selected + count - offset) % count;
            if let Some(block) = self.visible_index(idx).and_then(|i| self.blocks.get(i)) {
                if block.exit_code != 0 && block.exit_code != -1 {
                    self.selected = idx;
                    self.scroll = 0;
                    return true;
                }
            }
        }
        false
    }

    /// Filter blocks by exit code. `None` clears the filter.
    pub fn filter_by_exit_code(&mut self, code: Option<i32>) {
        self.filter_exit_code = code;
        self.selected = 0;
        self.scroll = 0;
        self.rebuild_filter();
    }

    /// Set a search query to filter blocks by command text. Empty clears it.
    pub fn search(&mut self, query: &str) {
        self.search_query = query.to_string();
        self.selected = 0;
        self.scroll = 0;
        self.rebuild_filter();
    }

    /// The start content line of the selected block, for cursor jumping.
    pub fn selected_start_line(&self) -> Option<usize> {
        self.selected_block().map(|b| b.start_line)
    }

    /// The text rows for the selected block in the current mode (for rendering).
    pub fn rows(&self) -> Vec<String> {
        let Some(block) = self.selected_block() else {
            return Vec::new();
        };
        match self.mode {
            BrowseMode::Commands => {
                let marker = "› ";
                vec![format!("{marker}{}", block.command)]
            }
            BrowseMode::Output => {
                let mut rows: Vec<String> = Vec::new();
                rows.push(format!("── {} ──", block.command));
                let lines: Vec<&str> = block.output.lines().collect();
                let start = self.scroll.min(lines.len().saturating_sub(1));
                for line in lines.iter().skip(start) {
                    rows.push((*line).to_string());
                }
                if rows.len() <= 1 {
                    rows.push("(no output)".into());
                }
                rows
            }
            BrowseMode::Json => {
                let mut rows: Vec<String> = Vec::new();
                rows.push(format!("── {} ──", block.command));
                for frag in super::extract_json(&block.output) {
                    rows.push(frag);
                }
                if rows.len() <= 1 {
                    rows.push("(no JSON found)".into());
                }
                rows
            }
            BrowseMode::Paths => {
                let mut rows: Vec<String> = Vec::new();
                rows.push(format!("── {} ──", block.command));
                for p in super::extract_paths(&block.output) {
                    rows.push(p);
                }
                if rows.len() <= 1 {
                    rows.push("(no paths found)".into());
                }
                rows
            }
        }
    }

    /// All visible block rows (for the Commands mode list view).
    pub fn list_rows(&self) -> Vec<String> {
        (0..self.visible_count())
            .map(|i| {
                let marker = if i == self.selected { "› " } else { "  " };
                let block = self
                    .visible_index(i)
                    .and_then(|idx| self.blocks.get(idx));
                let cmd = block.map(|b| b.command.as_str()).unwrap_or("");
                format!("{marker}{cmd}")
            })
            .collect()
    }

    /// Convert a visible index to a raw block index.
    fn visible_index(&self, visible_idx: usize) -> Option<usize> {
        if self.filtered.is_empty()
            && self.filter_exit_code.is_none()
            && self.search_query.is_empty()
        {
            // No filter: visible index is the raw index.
            (visible_idx < self.blocks.len()).then_some(visible_idx)
        } else {
            self.filtered.get(visible_idx).copied()
        }
    }

    /// Rebuild the filtered index list.
    fn rebuild_filter(&mut self) {
        self.filtered.clear();
        if self.filter_exit_code.is_none() && self.search_query.is_empty() {
            return;
        }
        let query = self.search_query.to_lowercase();
        for (i, block) in self.blocks.iter().enumerate() {
            let exit_ok = self
                .filter_exit_code
                .is_none_or(|code| block.exit_code == code);
            let search_ok = query.is_empty()
                || block.command.to_lowercase().contains(&query)
                || block.output.to_lowercase().contains(&query);
            if exit_ok && search_ok {
                self.filtered.push(i);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(cmd: &str, output: &str, exit: i32, start: usize, end: usize) -> CommandBlock {
        CommandBlock {
            command: cmd.into(),
            output: output.into(),
            exit_code: exit,
            start_line: start,
            end_line: end,
            method: "osc133",
        }
    }

    fn sample_blocks() -> Vec<CommandBlock> {
        vec![
            block("ls", "file1\nfile2", 0, 0, 2),
            block("make", "error: failed", 1, 3, 4),
            block("echo hi", "hi", 0, 5, 6),
            block("false", "", 1, 7, 7),
        ]
    }

    #[test]
    fn load_and_select() {
        let mut b = Browser::new();
        b.load(sample_blocks());
        assert_eq!(b.visible_count(), 4);
        assert_eq!(b.selected_block().unwrap().command, "ls");
    }

    #[test]
    fn move_down_wraps() {
        let mut b = Browser::new();
        b.load(sample_blocks());
        b.move_down(1);
        assert_eq!(b.selected_block().unwrap().command, "make");
        b.move_down(3); // wraps from index 1 → 0
        assert_eq!(b.selected_block().unwrap().command, "ls");
    }

    #[test]
    fn move_up_wraps() {
        let mut b = Browser::new();
        b.load(sample_blocks());
        b.move_up(1); // wraps from 0 → 3
        assert_eq!(b.selected_block().unwrap().command, "false");
    }

    #[test]
    fn cycle_mode() {
        let mut b = Browser::new();
        b.load(sample_blocks());
        assert_eq!(b.mode, BrowseMode::Commands);
        b.cycle_mode();
        assert_eq!(b.mode, BrowseMode::Output);
        b.cycle_mode();
        assert_eq!(b.mode, BrowseMode::Json);
        b.cycle_mode();
        assert_eq!(b.mode, BrowseMode::Paths);
        b.cycle_mode();
        assert_eq!(b.mode, BrowseMode::Commands);
    }

    #[test]
    fn next_error_finds_errors() {
        let mut b = Browser::new();
        b.load(sample_blocks());
        assert!(b.next_error());
        assert_eq!(b.selected_block().unwrap().command, "make");
        assert!(b.next_error());
        assert_eq!(b.selected_block().unwrap().command, "false");
        // Wraps back to "make".
        assert!(b.next_error());
        assert_eq!(b.selected_block().unwrap().command, "make");
    }

    #[test]
    fn prev_error_finds_errors() {
        let mut b = Browser::new();
        b.load(sample_blocks());
        b.selected = 2; // "echo hi"
        assert!(b.prev_error());
        assert_eq!(b.selected_block().unwrap().command, "make");
    }

    #[test]
    fn next_error_no_errors() {
        let mut b = Browser::new();
        b.load(vec![block("ls", "out", 0, 0, 1)]);
        assert!(!b.next_error());
    }

    #[test]
    fn filter_by_exit_code() {
        let mut b = Browser::new();
        b.load(sample_blocks());
        b.filter_by_exit_code(Some(1));
        assert_eq!(b.visible_count(), 2);
        assert_eq!(b.selected_block().unwrap().command, "make");
        b.move_down(1);
        assert_eq!(b.selected_block().unwrap().command, "false");
    }

    #[test]
    fn filter_clears() {
        let mut b = Browser::new();
        b.load(sample_blocks());
        b.filter_by_exit_code(Some(1));
        assert_eq!(b.visible_count(), 2);
        b.filter_by_exit_code(None);
        assert_eq!(b.visible_count(), 4);
    }

    #[test]
    fn search_filters_by_command() {
        let mut b = Browser::new();
        b.load(sample_blocks());
        b.search("echo");
        assert_eq!(b.visible_count(), 1);
        assert_eq!(b.selected_block().unwrap().command, "echo hi");
    }

    #[test]
    fn search_filters_by_output() {
        let mut b = Browser::new();
        b.load(sample_blocks());
        b.search("file1");
        assert_eq!(b.visible_count(), 1);
        assert_eq!(b.selected_block().unwrap().command, "ls");
    }

    #[test]
    fn search_case_insensitive() {
        let mut b = Browser::new();
        b.load(sample_blocks());
        b.search("MAKE");
        assert_eq!(b.visible_count(), 1);
    }

    #[test]
    fn combined_filter_and_search() {
        let mut b = Browser::new();
        b.load(sample_blocks());
        b.filter_by_exit_code(Some(1));
        b.search("false");
        assert_eq!(b.visible_count(), 1);
        assert_eq!(b.selected_block().unwrap().command, "false");
    }

    #[test]
    fn jump_to_block() {
        let mut b = Browser::new();
        b.load(sample_blocks());
        b.jump_to(2);
        assert_eq!(b.selected_block().unwrap().command, "echo hi");
        // Out of bounds clamps to last.
        b.jump_to(100);
        assert_eq!(b.selected, 3);
    }

    #[test]
    fn selected_start_line() {
        let mut b = Browser::new();
        b.load(sample_blocks());
        b.jump_to(2);
        assert_eq!(b.selected_start_line(), Some(5));
    }

    #[test]
    fn scroll_within_block() {
        let mut b = Browser::new();
        b.load(sample_blocks());
        b.mode = BrowseMode::Output;
        b.scroll_down(5);
        assert_eq!(b.scroll, 5);
        b.scroll_up(3);
        assert_eq!(b.scroll, 2);
        b.scroll_up(100);
        assert_eq!(b.scroll, 0);
    }

    #[test]
    fn rows_commands_mode() {
        let mut b = Browser::new();
        b.load(sample_blocks());
        let rows = b.rows();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].contains("ls"));
    }

    #[test]
    fn rows_output_mode() {
        let mut b = Browser::new();
        b.load(sample_blocks());
        b.mode = BrowseMode::Output;
        let rows = b.rows();
        assert!(rows[0].contains("ls"));
        assert!(rows.iter().any(|r| r.contains("file1")));
    }

    #[test]
    fn list_rows_all_blocks() {
        let mut b = Browser::new();
        b.load(sample_blocks());
        let rows = b.list_rows();
        assert_eq!(rows.len(), 4);
        assert!(rows[0].starts_with("› "));
        assert!(rows[1].starts_with("  "));
    }

    #[test]
    fn clear_resets_state() {
        let mut b = Browser::new();
        b.load(sample_blocks());
        b.move_down(2);
        b.scroll_down(5);
        b.clear();
        assert!(b.is_empty());
        assert_eq!(b.selected, 0);
        assert_eq!(b.scroll, 0);
    }

    #[test]
    fn empty_browser() {
        let b = Browser::new();
        assert!(b.is_empty());
        assert_eq!(b.visible_count(), 0);
        assert!(b.selected_block().is_none());
        assert!(b.selected_start_line().is_none());
    }
}
