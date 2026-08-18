//! Title change debouncing to prevent flicker.
//!
//! Ported from Go TUIOS `internal/app/sidebar_title_debounce.go`. Terminal
//! titles can flicker rapidly (e.g. during command execution). The debounce
//! holds a new title for a short period before adopting it, so only stable
//! title changes are shown.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// The debounce window: a title must be stable for this long before adoption.
pub const DEBOUNCE: Duration = Duration::from_millis(300);

/// The debounce tracker: maps window IDs to (shown title, when adopted).
#[derive(Debug, Clone, Default)]
pub struct TitleDebounce {
    entries: HashMap<String, (String, Instant)>,
    /// Pending title changes: (window ID, candidate title, first seen).
    pending: HashMap<String, (String, Instant)>,
}

impl TitleDebounce {
    /// Create a new debounce tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Get the currently shown title for a window.
    pub fn shown(&self, window_id: &str) -> Option<&str> {
        self.entries.get(window_id).map(|(t, _)| t.as_str())
    }

    /// Offer a new title for a window. Returns true if the shown title changed.
    pub fn offer(&mut self, window_id: &str, title: &str, now: Instant) -> bool {
        let current = self.entries.get(window_id).map(|(t, _)| t.as_str());
        if current == Some(title) {
            self.pending.remove(window_id);
            return false;
        }
        match self.pending.get(window_id) {
            Some((pending_title, first_seen)) => {
                if pending_title == title && now >= *first_seen + DEBOUNCE {
                    self.entries
                        .insert(window_id.to_string(), (title.to_string(), now));
                    self.pending.remove(window_id);
                    true
                } else if pending_title == title {
                    false
                } else {
                    self.pending
                        .insert(window_id.to_string(), (title.to_string(), now));
                    false
                }
            }
            None => {
                self.pending
                    .insert(window_id.to_string(), (title.to_string(), now));
                false
            }
        }
    }

    /// Remove a window from tracking.
    pub fn remove(&mut self, window_id: &str) {
        self.entries.remove(window_id);
        self.pending.remove(window_id);
    }

    /// Whether any title change is pending.
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// The count of tracked windows.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the tracker is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Clear all tracking.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.pending.clear();
    }

    /// Flush all pending titles that have passed the debounce window.
    pub fn flush(&mut self, now: Instant) -> Vec<(String, String)> {
        let mut adopted = Vec::new();
        let pending: Vec<(String, String, Instant)> = self
            .pending
            .iter()
            .map(|(k, (t, ts))| (k.clone(), t.clone(), *ts))
            .collect();
        for (wid, title, first_seen) in pending {
            if now >= first_seen + DEBOUNCE {
                self.entries
                    .insert(wid.clone(), (title.clone(), now));
                self.pending.remove(&wid);
                adopted.push((wid, title));
            }
        }
        adopted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_tracker_is_empty() {
        let t = TitleDebounce::new();
        assert!(t.is_empty());
        assert!(!t.has_pending());
    }

    #[test]
    fn offer_does_not_adopt_immediately() {
        let mut t = TitleDebounce::new();
        let now = Instant::now();
        assert!(!t.offer("w0", "hello", now));
        assert!(t.has_pending());
        assert_eq!(t.shown("w0"), None);
    }

    #[test]
    fn offer_adopts_after_debounce() {
        let mut t = TitleDebounce::new();
        let now = Instant::now();
        t.offer("w0", "hello", now);
        let later = now + DEBOUNCE;
        assert!(t.offer("w0", "hello", later));
        assert_eq!(t.shown("w0"), Some("hello"));
    }

    #[test]
    fn same_title_does_not_retrigger() {
        let mut t = TitleDebounce::new();
        let now = Instant::now();
        t.offer("w0", "hello", now);
        let later = now + DEBOUNCE;
        t.offer("w0", "hello", later);
        // Offering the same title again should not trigger.
        assert!(!t.offer("w0", "hello", later + Duration::from_secs(1)));
    }

    #[test]
    fn different_title_resets_debounce() {
        let mut t = TitleDebounce::new();
        let now = Instant::now();
        t.offer("w0", "hello", now);
        let later = now + DEBOUNCE / 2;
        t.offer("w0", "world", later);
        // Should not have adopted "hello" yet.
        assert_eq!(t.shown("w0"), None);
        // And "world" should need its own debounce period.
        let even_later = later + DEBOUNCE;
        assert!(t.offer("w0", "world", even_later));
        assert_eq!(t.shown("w0"), Some("world"));
    }

    #[test]
    fn flush_adopts_all_ready() {
        let mut t = TitleDebounce::new();
        let now = Instant::now();
        t.offer("w0", "hello", now);
        t.offer("w1", "world", now);
        let later = now + DEBOUNCE;
        let adopted = t.flush(later);
        assert_eq!(adopted.len(), 2);
        assert!(!t.has_pending());
    }

    #[test]
    fn remove_clears_entry() {
        let mut t = TitleDebounce::new();
        let now = Instant::now();
        t.offer("w0", "hello", now);
        let later = now + DEBOUNCE;
        t.offer("w0", "hello", later);
        t.remove("w0");
        assert_eq!(t.shown("w0"), None);
    }
}
