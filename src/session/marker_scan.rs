//! Incremental OSC 133 semantic-marker scanner.
//!
//! Shells emit `OSC 133 ; A/B/C/D [; <exit>] ST`/BEL sequences around prompt
//! and command boundaries (the semantic markers the TUI's emulator also
//! tracks for its scrollback browser). The daemon never emulates a VT stream
//! (clients run their own emulator), so this is a minimal streaming parser
//! over the raw PTY byte stream — the same shape as [`super::osc_scan`]'s
//! OSC 9;4 scanner: it carries bytes across chunk boundaries, finds complete
//! sequences, and yields the marker each one reports. The pump maps those
//! onto the `pane-shell-prompt` / `pane-command-started` /
//! `pane-command-finished` hook events.

/// The maximum length of an OSC payload we will scan; longer runs are dropped
/// so a runaway stream cannot grow the carry buffer without bound.
const MAX_OSC_LEN: usize = 1024;

/// One completed OSC 133 sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Osc133Marker {
    /// The marker letter (A = prompt, B = command started, C = command
    /// executed, D = command finished).
    pub kind: MarkerKind,
    /// The exit code carried by a `D` marker (`-1` when none was reported).
    pub exit_code: i32,
}

/// The four OSC 133 marker letters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerKind {
    PromptStart,
    CommandStart,
    CommandExecuted,
    CommandFinished,
}

/// Parse a marker kind from its letter.
pub fn parse_marker_kind(ch: char) -> Option<MarkerKind> {
    match ch {
        'A' => Some(MarkerKind::PromptStart),
        'B' => Some(MarkerKind::CommandStart),
        'C' => Some(MarkerKind::CommandExecuted),
        'D' => Some(MarkerKind::CommandFinished),
        _ => None,
    }
}

/// A streaming OSC 133 scanner. Feed PTY chunks; collect completed markers.
#[derive(Debug, Default)]
pub struct Osc133Scanner {
    /// Bytes not yet consumed (partial sequences across chunk boundaries).
    buf: Vec<u8>,
}

impl Osc133Scanner {
    /// Create a new scanner.
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// Feed one chunk of PTY output and return the completed OSC 133 markers
    /// it contains (in order).
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<Osc133Marker> {
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
                if let Some(marker) = parse_osc_133(&text) {
                    out.push(marker);
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

/// Parse an OSC `133;...` payload into a marker. `text` is everything
/// between `ESC ]` and the terminator, e.g. `133;D;7`.
fn parse_osc_133(text: &str) -> Option<Osc133Marker> {
    let rest = text.strip_prefix("133;")?;
    let mut parts = rest.split(';');
    let code = parts.next()?.trim();
    let kind = parse_marker_kind(code.chars().next()?)?;
    let exit_code = parts
        .next()
        .and_then(|p| p.trim().parse::<i32>().ok())
        .unwrap_or(-1);
    Some(Osc133Marker { kind, exit_code })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bel_terminated_markers() {
        let mut s = Osc133Scanner::new();
        let out = s.feed(b"\x1b]133;A\x07");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, MarkerKind::PromptStart);
        assert_eq!(out[0].exit_code, -1);
    }

    #[test]
    fn st_terminated_with_exit_code() {
        let mut s = Osc133Scanner::new();
        let out = s.feed(b"\x1b]133;D;7\x1b\\");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, MarkerKind::CommandFinished);
        assert_eq!(out[0].exit_code, 7);
    }

    #[test]
    fn multiple_markers_in_one_chunk() {
        let mut s = Osc133Scanner::new();
        let out = s.feed(b"\x1b]133;B\x07echo hi\r\n\x1b]133;D;0\x07");
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].kind, MarkerKind::CommandStart);
        assert_eq!(out[1].kind, MarkerKind::CommandFinished);
        assert_eq!(out[1].exit_code, 0);
    }

    #[test]
    fn split_across_chunks() {
        let mut s = Osc133Scanner::new();
        assert!(s.feed(b"\x1b]133;D;4").is_empty());
        let out = s.feed(b"2\x07tail");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, MarkerKind::CommandFinished);
        assert_eq!(out[0].exit_code, 42);
        // Trailing "tail" stays buffered for the next feed.
        assert_eq!(s.feed(b"x").len(), 0);
    }

    #[test]
    fn command_executed_marker_is_yielded() {
        let mut s = Osc133Scanner::new();
        let out = s.feed(b"\x1b]133;C\x07");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, MarkerKind::CommandExecuted);
    }

    #[test]
    fn garbage_and_other_osc_codes_ignored() {
        let mut s = Osc133Scanner::new();
        assert!(s.feed(b"plain text").is_empty());
        assert!(s.feed(b"\x1b]9;4;1\x07").is_empty());
        assert!(s.feed(b"\x1b]133\x07").is_empty());
        assert!(s.feed(b"\x1b]133;X\x07").is_empty());
    }

    #[test]
    fn malformed_payload_without_letter_ignored() {
        let mut s = Osc133Scanner::new();
        assert!(s.feed(b"\x1b]133;\x07").is_empty());
    }
}
