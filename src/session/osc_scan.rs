//! Incremental OSC 9;4 progress-sequence scanner.
//!
//! The daemon never emulates a VT stream (clients run their own emulator), but
//! it still needs to hear a pane's `OSC 9 ; 4 ; <state> ; <percent> ST`/BEL
//! progress reports to drive agent state. This scanner is a minimal, streaming
//! parser over the raw PTY byte stream: it carries bytes across chunk
//! boundaries, finds complete OSC sequences, and yields the progress state
//! each one reports. State decoding reuses `vt::progress`, the port of Go's
//! `internal/vt/progress.go`.

use crate::vt::progress::{parse_progress, ProgressState};

use super::agent_state::AgentState;

/// The maximum length of an OSC payload we will scan; longer runs are dropped
/// so a runaway stream cannot grow the carry buffer without bound.
const MAX_OSC_LEN: usize = 1024;

/// One completed `OSC 9;4` sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OscProgress {
    /// The progress state the sequence reported.
    pub state: ProgressState,
    /// The percent, when the state carries one (0-100).
    pub percent: Option<u8>,
}

/// Map a progress state onto an agent state, mirroring Go's
/// `agentStateForProgress` in `internal/session/agent_osc.go`.
pub fn agent_state_for_progress(state: ProgressState) -> Option<AgentState> {
    match state {
        ProgressState::Clear => Some(AgentState::Idle),
        ProgressState::Normal | ProgressState::Indeterminate => Some(AgentState::Working),
        ProgressState::Error => Some(AgentState::Errored),
        ProgressState::Warning => Some(AgentState::NeedsInput),
    }
}

/// A streaming OSC 9;4 scanner. Feed PTY chunks; collect completed reports.
#[derive(Debug, Default)]
pub struct OscProgressScanner {
    /// Bytes not yet consumed (partial sequences across chunk boundaries).
    buf: Vec<u8>,
}

impl OscProgressScanner {
    /// Create a new scanner.
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// Feed one chunk of PTY output and return the completed OSC 9;4
    /// progress reports it contains (in order).
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<OscProgress> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        loop {
            // Find the next ESC ] (0x1b 0x5d).
            let start = match find_esc_bracket(&self.buf) {
                Some(i) => i,
                None => {
                    // No ESC in sight: keep only a possible trailing ESC byte.
                    if let Some(last) = self.buf.last() {
                        if *last == 0x1b {
                            self.buf.drain(..self.buf.len() - 1);
                        } else {
                            self.buf.clear();
                        }
                    } else {
                        self.buf.clear();
                    }
                    break;
                }
            };
            // The sequence must start with ESC ] — drop anything before it.
            if start > 0 {
                self.buf.drain(..start);
            }
            // Find the terminator: BEL (0x07) or ST (0x1b 0x5c).
            let (end, term_len) = match find_terminator(&self.buf) {
                Some(t) => t,
                None => {
                    // Incomplete. If it has grown past the cap, drop it.
                    if self.buf.len() > MAX_OSC_LEN {
                        self.buf.clear();
                    }
                    break;
                }
            };
            // The payload is between ESC ] (2 bytes) and the terminator.
            if end >= 2 {
                let payload = &self.buf[2..end];
                let text = String::from_utf8_lossy(payload);
                if let Some(progress) = parse_osc_94(&text) {
                    out.push(progress);
                }
            }
            self.buf.drain(..end + term_len);
        }
        out
    }
}

/// Find `ESC ]` (0x1b 0x5d).
fn find_esc_bracket(buf: &[u8]) -> Option<usize> {
    buf.windows(2).position(|w| w == [0x1b, 0x5d])
}

/// Find the terminator of an OSC started at byte 0: BEL or ST.
/// Returns (index of the terminator start, bytes consumed by it).
fn find_terminator(buf: &[u8]) -> Option<(usize, usize)> {
    for (i, b) in buf.iter().enumerate() {
        if *b == 0x07 {
            return Some((i, 1));
        }
        if *b == 0x1b && buf.get(i + 1) == Some(&0x5c) {
            return Some((i, 2));
        }
        // A second ESC that is not a terminator means the first OSC was
        // malformed; treat this ESC as the end so the outer loop rescans.
        if *b == 0x1b && buf.get(i + 1) != Some(&0x5c) && i > 0 {
            return Some((i, 0));
        }
    }
    None
}

/// Parse an OSC `9;4;...` payload into a progress report.
fn parse_osc_94(text: &str) -> Option<OscProgress> {
    let rest = text.strip_prefix("9;")?;
    if !crate::vt::progress::is_progress_payload(rest) {
        return None;
    }
    let (state, percent) = parse_progress(rest);
    Some(OscProgress { state, percent })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_bel_terminated() {
        let mut s = OscProgressScanner::new();
        let out = s.feed(b"\x1b]9;4;1;42\x07");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].state, ProgressState::Normal);
        assert_eq!(out[0].percent, Some(42));
        assert_eq!(
            agent_state_for_progress(out[0].state),
            Some(AgentState::Working)
        );
    }

    #[test]
    fn single_st_terminated() {
        let mut s = OscProgressScanner::new();
        let out = s.feed(b"\x1b]9;4;3\x1b\\");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].state, ProgressState::Indeterminate);
        assert_eq!(
            agent_state_for_progress(out[0].state),
            Some(AgentState::Working)
        );
    }

    #[test]
    fn clear_state_maps_to_idle() {
        let mut s = OscProgressScanner::new();
        let out = s.feed(b"\x1b]9;4\x07");
        assert_eq!(out[0].state, ProgressState::Clear);
        assert_eq!(
            agent_state_for_progress(out[0].state),
            Some(AgentState::Idle)
        );
    }

    #[test]
    fn warning_state_maps_to_needs_input() {
        let mut s = OscProgressScanner::new();
        let out = s.feed(b"\x1b]9;4;4;75\x07");
        assert_eq!(out[0].state, ProgressState::Warning);
        assert_eq!(
            agent_state_for_progress(out[0].state),
            Some(AgentState::NeedsInput)
        );
    }

    #[test]
    fn error_state_maps_to_errored() {
        let mut s = OscProgressScanner::new();
        let out = s.feed(b"\x1b]9;4;2\x07");
        assert_eq!(
            agent_state_for_progress(out[0].state),
            Some(AgentState::Errored)
        );
    }

    #[test]
    fn split_across_chunks() {
        let mut s = OscProgressScanner::new();
        assert!(s.feed(b"\x1b]9;4;1;").is_empty());
        let out = s.feed(b"99\x07tail");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].percent, Some(99));
        // Trailing "tail" stays buffered for the next feed.
        assert_eq!(s.feed(b"x").len(), 0);
    }

    #[test]
    fn multiple_sequences_one_chunk() {
        let mut s = OscProgressScanner::new();
        let out = s.feed(b"\x1b]9;4;1;10\x07\x1b]9;4\x07");
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].state, ProgressState::Normal);
        assert_eq!(out[1].state, ProgressState::Clear);
    }

    #[test]
    fn interleaved_text_is_ignored() {
        let mut s = OscProgressScanner::new();
        let out = s.feed(b"hello\x1b]9;4;1;50\x07world");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].percent, Some(50));
    }

    #[test]
    fn non_94_osc_is_ignored() {
        let mut s = OscProgressScanner::new();
        // OSC 0 title sequence must not be misread as progress.
        let out = s.feed(b"\x1b]0;my title\x07\x1b]9;4\x07");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].state, ProgressState::Clear);
    }

    #[test]
    fn incomplete_then_abandoned() {
        let mut s = OscProgressScanner::new();
        assert!(s.feed(b"\x1b]9;4;1;").is_empty());
        // A new chunk that never terminates, longer than the cap, is dropped.
        let big = vec![b'a'; 2048];
        assert!(s.feed(&big).is_empty());
        assert!(s.buf.is_empty());
    }
}
