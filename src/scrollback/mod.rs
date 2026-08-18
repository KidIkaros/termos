//! Structured scrollback browser — ported from Go TUIOS
//! `internal/scrollback/`.
//!
//! Parses terminal history into command/output blocks using OSC 133 semantic
//! markers (with the prompt line itself as the fallback boundary), then offers
//! a vim-navigable browser over the blocks: Commands, JSON, or file-path
//! modes.

use crate::vt::semantic_markers::{SemanticMarker, SemanticMarkerType};

/// One command and its output extracted from scrollback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandBlock {
    /// The command text (what the user typed).
    pub command: String,
    /// Plain-text output.
    pub output: String,
    /// Exit code, or -1 when unknown.
    pub exit_code: i32,
    /// Absolute content line where the block starts.
    pub start_line: usize,
    /// Absolute content line where the block ends (inclusive).
    pub end_line: usize,
    /// How this block was parsed: "osc133" or "prompt".
    pub method: &'static str,
}

/// The browser's display mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowseMode {
    /// The command text of each block.
    Commands,
    /// The full output of each block.
    Output,
    /// JSON fragments found in each block's output.
    Json,
    /// File paths found in each block's output.
    Paths,
}

/// Extract command blocks from content lines using semantic markers.
///
/// `line_count` is the number of content lines; `line_text` resolves a content
/// line to its text. Markers are optional: when none are present, the parser
/// falls back to "prompt" boundaries — a line that looks like a shell prompt
/// starts a new block.
pub fn parse_blocks(
    markers: &[SemanticMarker],
    line_count: usize,
    line_text: impl Fn(usize) -> String,
) -> Vec<CommandBlock> {
    let blocks = parse_with_markers(markers, line_count, &line_text);
    if !blocks.is_empty() {
        return blocks;
    }
    parse_with_prompts(line_count, &line_text)
}

/// Build blocks from OSC 133 markers: a `CommandExecuted` (C) marker starts a
/// block whose command is the captured text; `CommandFinished` (D) closes it
/// with the exit code. Falls back to prompt-start (A) boundaries when no C
/// markers exist.
fn parse_with_markers(
    markers: &[SemanticMarker],
    line_count: usize,
    line_text: &impl Fn(usize) -> String,
) -> Vec<CommandBlock> {
    let cs: Vec<&SemanticMarker> = markers
        .iter()
        .filter(|m| m.marker_type == SemanticMarkerType::CommandExecuted)
        .collect();
    let ds: Vec<&SemanticMarker> = markers
        .iter()
        .filter(|m| m.marker_type == SemanticMarkerType::CommandFinished)
        .collect();
    if cs.is_empty() {
        return Vec::new();
    }
    let mut blocks = Vec::new();
    for (i, c) in cs.iter().enumerate() {
        let start = c.abs_line.max(0) as usize;
        let end = if let Some(next) = cs.get(i + 1) {
            ((next.abs_line - 1).max(c.abs_line)) as usize
        } else {
            line_count.saturating_sub(1).max(start)
        };
        let exit = ds
            .iter()
            .find(|d| d.abs_line >= c.abs_line && d.abs_line <= end as i32)
            .map(|d| d.exit_code)
            .unwrap_or(-1);
        let output = (start + 1..=end)
            .map(|l| line_text(l))
            .collect::<Vec<_>>()
            .join("\n");
        blocks.push(CommandBlock {
            command: c.captured_text.clone(),
            output,
            exit_code: exit,
            start_line: start,
            end_line: end,
            method: "osc133",
        });
    }
    blocks
}

/// Fallback: a line that looks like a shell prompt starts a new block.
fn parse_with_prompts(
    line_count: usize,
    line_text: &impl Fn(usize) -> String,
) -> Vec<CommandBlock> {
    let mut blocks = Vec::new();
    let mut start: Option<usize> = None;
    for line in 0..line_count {
        let text = line_text(line);
        let is_prompt = looks_like_prompt(&text);
        if is_prompt {
            if let Some(s) = start.take() {
                let end = line.saturating_sub(1).max(s);
                let output = (s + 1..=end)
                    .map(|l| line_text(l))
                    .collect::<Vec<_>>()
                    .join("\n");
                blocks.push(CommandBlock {
                    command: line_text(s).trim().to_string(),
                    output,
                    exit_code: -1,
                    start_line: s,
                    end_line: end,
                    method: "prompt",
                });
            }
            start = Some(line);
        }
    }
    if let Some(s) = start {
        let end = line_count.saturating_sub(1).max(s);
        let output = (s + 1..=end)
            .map(|l| line_text(l))
            .collect::<Vec<_>>()
            .join("\n");
        blocks.push(CommandBlock {
            command: line_text(s).trim().to_string(),
            output,
            exit_code: -1,
            start_line: s,
            end_line: end,
            method: "prompt",
        });
    }
    blocks
}

/// A conservative prompt heuristic: ends with `$`, `#`, `>`, `❯`, or `%`
/// preceded by whitespace, or starts with `$ `.
fn looks_like_prompt(text: &str) -> bool {
    let trimmed = text.trim_end();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.starts_with("$ ") || trimmed.starts_with("> ") {
        return true;
    }
    let last = trimmed.chars().last().unwrap();
    if matches!(last, '$' | '#' | '>' | '%' | '❯') {
        // Require a boundary before the sigil so "echo $PATH" is not a
        // prompt: whitespace, or a path/address sigil (: / ~).
        let before = &trimmed[..trimmed.len() - last.len_utf8()];
        let before = before.trim_end();
        before.is_empty()
            || before.ends_with(' ')
            || before.ends_with(':')
            || before.ends_with('/')
            || before.ends_with('~')
    } else {
        false
    }
}

/// Extract JSON fragments from a block's output (Go's extractor.go).
pub fn extract_json(output: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (i, c) in output.char_indices() {
        match c {
            '"' if !escaped => in_string = !in_string,
            '\\' if in_string && !escaped => escaped = true,
            '{' if !in_string => {
                if depth == 0 {
                    start = i;
                }
                depth += 1;
                escaped = false;
            }
            '}' if !in_string => {
                depth -= 1;
                if depth == 0 && i > start {
                    let frag = &output[start..=i];
                    if serde_json::from_str::<serde_json::Value>(frag).is_ok() {
                        out.push(frag.to_string());
                    }
                }
                escaped = false;
            }
            _ => escaped = false,
        }
    }
    out
}

/// Extract file paths from a block's output (Go's extractor.go): tokens that
/// start with `/`, `./`, `~/` or `../` and contain no whitespace.
pub fn extract_paths(output: &str) -> Vec<String> {
    let mut out = Vec::new();
    for token in output.split_whitespace() {
        let cleaned = token.trim_matches(|c: char| {
            c.is_ascii_punctuation() && !matches!(c, '/' | '.' | '~' | '-') || c == '\''
        });
        if cleaned.is_empty() {
            continue;
        }
        let is_path = cleaned.starts_with('/')
            || cleaned.starts_with("./")
            || cleaned.starts_with("../")
            || cleaned.starts_with("~/");
        if is_path && cleaned.contains('/') {
            out.push(cleaned.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vt::semantic_markers::SemanticMarker;

    fn cmd_marker(line: i32, text: &str) -> SemanticMarker {
        SemanticMarker::new(SemanticMarkerType::CommandExecuted, line, 0).with_captured_text(text)
    }

    fn done_marker(line: i32, code: i32) -> SemanticMarker {
        SemanticMarker::new(SemanticMarkerType::CommandFinished, line, 0).with_exit_code(code)
    }

    fn lines() -> Vec<String> {
        vec![
            "$ ls".into(),
            "file1".into(),
            "file2".into(),
            "$ echo hi".into(),
            "hi".into(),
            "$ pwd".into(),
            "/tmp".into(),
        ]
    }

    #[test]
    fn blocks_from_markers() {
        let markers = vec![
            cmd_marker(0, "ls"),
            done_marker(2, 0),
            cmd_marker(3, "echo hi"),
            done_marker(4, 0),
            cmd_marker(5, "pwd"),
            done_marker(6, 7),
        ];
        let text = |i: usize| lines()[i].clone();
        let blocks = parse_blocks(&markers, 7, text);
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].command, "ls");
        assert_eq!(blocks[0].output, "file1\nfile2");
        assert_eq!(blocks[0].exit_code, 0);
        assert_eq!(blocks[1].command, "echo hi");
        assert_eq!(blocks[2].command, "pwd");
        assert_eq!(blocks[2].exit_code, 7);
    }

    #[test]
    fn prompt_fallback_without_markers() {
        let text = |i: usize| lines()[i].clone();
        let blocks = parse_blocks(&[], 7, text);
        assert!(!blocks.is_empty());
        assert_eq!(blocks[0].method, "prompt");
        assert!(blocks[0].command.contains("$ ls"));
    }

    #[test]
    fn no_markers_no_prompts_is_empty() {
        let text = |i: usize| format!("plain line {i}");
        let blocks = parse_blocks(&[], 3, text);
        assert!(blocks.is_empty());
    }

    #[test]
    fn empty_marker_list_no_blocks() {
        let text = |i: usize| lines()[i].clone();
        assert!(parse_blocks(&[], 0, text).is_empty());
    }

    #[test]
    fn prompt_heuristic() {
        assert!(looks_like_prompt("user@host:~$"));
        assert!(looks_like_prompt("> "));
        assert!(looks_like_prompt("$ echo"));
        assert!(!looks_like_prompt("echo $PATH"));
        assert!(!looks_like_prompt("plain output"));
    }

    #[test]
    fn extract_json_fragments() {
        let output = "prefix {\"a\": 1} {\"b\": [1, 2]} suffix";
        let frags = extract_json(output);
        assert_eq!(frags.len(), 2);
        assert!(frags[0].contains("\"a\""));
    }

    #[test]
    fn extract_json_ignores_strings() {
        let output = r#"say "not {json}" then {"ok": true}"#;
        let frags = extract_json(output);
        assert_eq!(frags.len(), 1);
        assert!(frags[0].contains("\"ok\""));
    }

    #[test]
    fn extract_paths_from_output() {
        let output = "built ./src/main.rs and /tmp/x.log; see ../docs/readme.md ~/.config";
        let paths = extract_paths(output);
        assert!(paths.iter().any(|p| p.contains("src/main.rs")));
        assert!(paths.iter().any(|p| p == "/tmp/x.log"));
        assert!(paths.iter().any(|p| p.contains("docs/readme.md")));
        assert!(paths.iter().any(|p| p == "~/.config"));
    }
}
