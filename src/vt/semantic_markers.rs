//! OSC 133 semantic zone markers — ported from Go TUIOS `internal/vt/semantic_markers.go`.
//!
//! Tracks prompt/command boundary markers emitted by shells via OSC 133.

use std::sync::Mutex;

/// The type of a semantic marker, corresponding to the OSC 133 letter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticMarkerType {
    /// `OSC 133 ; A ST` — prompt start.
    PromptStart,
    /// `OSC 133 ; B ST` — command start (user pressed enter).
    CommandStart,
    /// `OSC 133 ; C ST` — command executed (output begins).
    CommandExecuted,
    /// `OSC 133 ; D ; <exit> ST` — command finished.
    CommandFinished,
}

impl SemanticMarkerType {
    /// The letter used in the OSC 133 sequence.
    pub fn letter(&self) -> char {
        match self {
            Self::PromptStart => 'A',
            Self::CommandStart => 'B',
            Self::CommandExecuted => 'C',
            Self::CommandFinished => 'D',
        }
    }
}

/// Parse a marker type from a character.
pub fn parse_marker_type(ch: char) -> Option<SemanticMarkerType> {
    match ch {
        'A' => Some(SemanticMarkerType::PromptStart),
        'B' => Some(SemanticMarkerType::CommandStart),
        'C' => Some(SemanticMarkerType::CommandExecuted),
        'D' => Some(SemanticMarkerType::CommandFinished),
        _ => None,
    }
}

/// A single semantic marker.
#[derive(Debug, Clone)]
pub struct SemanticMarker {
    pub marker_type: SemanticMarkerType,
    /// `scrollback_len + cursor_y` at time of emission.
    pub abs_line: i32,
    /// Cursor x at time of emission.
    pub col: i32,
    /// Exit code (only meaningful for `CommandFinished`; -1 = unknown).
    pub exit_code: i32,
    /// Command text captured at C-marker time.
    pub captured_text: String,
}

impl SemanticMarker {
    /// Create a new marker.
    pub fn new(marker_type: SemanticMarkerType, abs_line: i32, col: i32) -> Self {
        Self {
            marker_type,
            abs_line,
            col,
            exit_code: -1,
            captured_text: String::new(),
        }
    }

    /// Set the exit code (for CommandFinished markers).
    pub fn with_exit_code(mut self, code: i32) -> Self {
        self.exit_code = code;
        self
    }

    /// Set captured text.
    pub fn with_captured_text(mut self, text: &str) -> Self {
        self.captured_text = text.to_string();
        self
    }
}

/// A thread-safe bounded list of semantic markers.
#[derive(Debug)]
pub struct SemanticMarkerList {
    markers: Mutex<Vec<SemanticMarker>>,
    max_items: usize,
}

impl SemanticMarkerList {
    /// Create a new list with the given max size.
    pub fn new(max_items: usize) -> Self {
        Self {
            markers: Mutex::new(Vec::new()),
            max_items,
        }
    }

    /// Push a marker, evicting the oldest if at capacity.
    pub fn push(&self, marker: SemanticMarker) {
        let mut markers = self.markers.lock().unwrap();
        if markers.len() >= self.max_items {
            markers.remove(0);
        }
        markers.push(marker);
    }

    /// Get a snapshot of all markers.
    pub fn markers(&self) -> Vec<SemanticMarker> {
        self.markers.lock().unwrap().clone()
    }

    /// Current marker count.
    pub fn len(&self) -> usize {
        self.markers.lock().unwrap().len()
    }

    /// Whether the list is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Clear all markers.
    pub fn clear(&self) {
        self.markers.lock().unwrap().clear();
    }

    /// Get the last marker.
    pub fn last(&self) -> Option<SemanticMarker> {
        self.markers.lock().unwrap().last().cloned()
    }

    /// Get the last `CommandFinished` marker.
    pub fn last_finished(&self) -> Option<SemanticMarker> {
        let markers = self.markers.lock().unwrap();
        markers
            .iter()
            .rev()
            .find(|m| m.marker_type == SemanticMarkerType::CommandFinished)
            .cloned()
    }
}

impl Default for SemanticMarkerList {
    fn default() -> Self {
        Self::new(10_000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_marker_types() {
        assert_eq!(
            parse_marker_type('A'),
            Some(SemanticMarkerType::PromptStart)
        );
        assert_eq!(
            parse_marker_type('B'),
            Some(SemanticMarkerType::CommandStart)
        );
        assert_eq!(
            parse_marker_type('C'),
            Some(SemanticMarkerType::CommandExecuted)
        );
        assert_eq!(
            parse_marker_type('D'),
            Some(SemanticMarkerType::CommandFinished)
        );
        assert_eq!(parse_marker_type('X'), None);
    }

    #[test]
    fn marker_letters() {
        assert_eq!(SemanticMarkerType::PromptStart.letter(), 'A');
        assert_eq!(SemanticMarkerType::CommandFinished.letter(), 'D');
    }

    #[test]
    fn list_push_and_len() {
        let list = SemanticMarkerList::new(100);
        assert!(list.is_empty());
        list.push(SemanticMarker::new(SemanticMarkerType::PromptStart, 0, 0));
        list.push(SemanticMarker::new(SemanticMarkerType::CommandStart, 1, 0));
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn list_last() {
        let list = SemanticMarkerList::new(100);
        list.push(SemanticMarker::new(SemanticMarkerType::PromptStart, 0, 0));
        list.push(SemanticMarker::new(SemanticMarkerType::CommandStart, 1, 0));
        let last = list.last().unwrap();
        assert_eq!(last.marker_type, SemanticMarkerType::CommandStart);
    }

    #[test]
    fn list_last_finished() {
        let list = SemanticMarkerList::new(100);
        list.push(SemanticMarker::new(SemanticMarkerType::PromptStart, 0, 0));
        list.push(SemanticMarker::new(SemanticMarkerType::CommandFinished, 5, 0).with_exit_code(0));
        list.push(SemanticMarker::new(SemanticMarkerType::PromptStart, 6, 0));
        let finished = list.last_finished().unwrap();
        assert_eq!(finished.marker_type, SemanticMarkerType::CommandFinished);
        assert_eq!(finished.exit_code, 0);
    }

    #[test]
    fn list_bounded_eviction() {
        let list = SemanticMarkerList::new(3);
        for i in 0..5 {
            list.push(SemanticMarker::new(SemanticMarkerType::PromptStart, i, 0));
        }
        assert_eq!(list.len(), 3);
        let markers = list.markers();
        assert_eq!(markers[0].abs_line, 2);
        assert_eq!(markers[2].abs_line, 4);
    }

    #[test]
    fn list_clear() {
        let list = SemanticMarkerList::new(100);
        list.push(SemanticMarker::new(SemanticMarkerType::PromptStart, 0, 0));
        assert!(!list.is_empty());
        list.clear();
        assert!(list.is_empty());
    }

    #[test]
    fn marker_with_exit_code_and_text() {
        let m = SemanticMarker::new(SemanticMarkerType::CommandFinished, 10, 5)
            .with_exit_code(127)
            .with_captured_text("ls");
        assert_eq!(m.exit_code, 127);
        assert_eq!(m.captured_text, "ls");
    }

    #[test]
    fn last_finished_none_when_empty() {
        let list = SemanticMarkerList::new(100);
        assert!(list.last_finished().is_none());
    }
}
