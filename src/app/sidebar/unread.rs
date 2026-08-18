//! The done/unread bit on finished panes.
//!
//! Ported from Go TUIOS `internal/app/sidebar_unread.go`. When a pane
//! transitions to `done`, it is "unread" until the user looks at it (the
//! cursor lands on it or the pane gains focus). The unread bit drives the
//! badge dot and the priority sort.

use std::collections::HashMap;

/// The unread tracker: maps window IDs to their seen/unseen state.
#[derive(Debug, Clone, Default)]
pub struct UnreadTracker {
    seen: HashMap<String, bool>,
}

impl UnreadTracker {
    /// Create a new tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark a window as seen (the user has looked at its finished state).
    pub fn mark_seen(&mut self, window_id: &str) {
        self.seen.insert(window_id.to_string(), true);
    }

    /// Mark a window as unread (its state transitioned to done).
    pub fn mark_unread(&mut self, window_id: &str) {
        self.seen.insert(window_id.to_string(), false);
    }

    /// Whether a window's done state has been seen.
    pub fn is_seen(&self, window_id: &str) -> bool {
        self.seen.get(window_id).copied().unwrap_or(true)
    }

    /// Whether a window is unread (done but not yet seen).
    pub fn is_unread(&self, window_id: &str) -> bool {
        !self.is_seen(window_id)
    }

    /// Remove a window from tracking (e.g. when it's closed).
    pub fn remove(&mut self, window_id: &str) {
        self.seen.remove(window_id);
    }

    /// The count of unread windows.
    pub fn unread_count(&self) -> usize {
        self.seen.values().filter(|&&v| !v).count()
    }

    /// Clear all tracking.
    pub fn clear(&mut self) {
        self.seen.clear();
    }

    /// Transition a window's unread state based on a state change.
    /// If the new state is "done", mark unread. If it's anything else, mark
    /// seen (the done state is over).
    pub fn on_state_change(&mut self, window_id: &str, new_state: &str) {
        if new_state == "done" {
            self.mark_unread(window_id);
        } else if !new_state.is_empty() && new_state != "none" {
            // Any active state clears the unread bit.
            self.mark_seen(window_id);
        }
    }
}

/// The unread badge glyph.
pub const UNREAD_GLYPH: &str = "\u{25cf}"; // ●

/// The seen badge glyph (dimmer).
pub const SEEN_GLYPH: &str = "\u{25cb}"; // ○

/// The badge glyph for a window, given its state and seen flag.
pub fn badge_glyph(state: &str, done_seen: bool) -> &'static str {
    if state == "done" {
        if done_seen {
            SEEN_GLYPH
        } else {
            UNREAD_GLYPH
        }
    } else {
        ""
    }
}

/// Whether a window should show an unread badge.
pub fn has_unread_badge(state: &str, done_seen: bool) -> bool {
    state == "done" && !done_seen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_tracker_all_seen() {
        let t = UnreadTracker::new();
        assert!(t.is_seen("w0"));
        assert!(!t.is_unread("w0"));
    }

    #[test]
    fn mark_unread_then_seen() {
        let mut t = UnreadTracker::new();
        t.mark_unread("w0");
        assert!(t.is_unread("w0"));
        assert!(!t.is_seen("w0"));
        t.mark_seen("w0");
        assert!(!t.is_unread("w0"));
        assert!(t.is_seen("w0"));
    }

    #[test]
    fn on_state_change_done_marks_unread() {
        let mut t = UnreadTracker::new();
        t.on_state_change("w0", "done");
        assert!(t.is_unread("w0"));
    }

    #[test]
    fn on_state_change_working_marks_seen() {
        let mut t = UnreadTracker::new();
        t.mark_unread("w0");
        t.on_state_change("w0", "working");
        assert!(t.is_seen("w0"));
    }

    #[test]
    fn on_state_change_none_does_nothing() {
        let mut t = UnreadTracker::new();
        t.mark_unread("w0");
        t.on_state_change("w0", "none");
        assert!(t.is_unread("w0"));
    }

    #[test]
    fn unread_count() {
        let mut t = UnreadTracker::new();
        t.mark_unread("w0");
        t.mark_unread("w1");
        t.mark_seen("w2");
        assert_eq!(t.unread_count(), 2);
    }

    #[test]
    fn remove_clears_entry() {
        let mut t = UnreadTracker::new();
        t.mark_unread("w0");
        t.remove("w0");
        assert!(t.is_seen("w0"));
    }

    #[test]
    fn badge_glyph_done_unread() {
        assert_eq!(badge_glyph("done", false), UNREAD_GLYPH);
    }

    #[test]
    fn badge_glyph_done_seen() {
        assert_eq!(badge_glyph("done", true), SEEN_GLYPH);
    }

    #[test]
    fn badge_glyph_other_state() {
        assert_eq!(badge_glyph("working", false), "");
    }

    #[test]
    fn has_unread_badge_only_for_unseen_done() {
        assert!(has_unread_badge("done", false));
        assert!(!has_unread_badge("done", true));
        assert!(!has_unread_badge("working", false));
    }
}
