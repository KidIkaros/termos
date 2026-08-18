//! Scrolling long titles that don't fit (marquee).
//!
//! Ported from Go TUIOS `internal/app/sidebar_marquee.go`. Only hovered
//! overflowing rows scroll. The initial pause is 900 ms, the cell interval
//! is 220 ms, and the gap is 4 cells.

use std::time::{Duration, Instant};

/// The initial pause before scrolling starts.
pub const INITIAL_PAUSE: Duration = Duration::from_millis(900);

/// The interval between scroll steps.
pub const CELL_INTERVAL: Duration = Duration::from_millis(220);

/// The gap between the end of the text and the start of the repeat.
pub const GAP: usize = 4;

/// The marquee state: which row is scrolling and when it started.
#[derive(Debug, Clone, Default)]
pub struct Marquee {
    /// The key identifying the scrolling row (window ID or session ID).
    pub key: String,
    /// When the marquee started.
    pub start: Option<Instant>,
}

impl Marquee {
    /// Create an empty (inactive) marquee.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the marquee is active.
    pub fn active(&self) -> bool {
        !self.key.is_empty()
    }

    /// Start scrolling a row.
    pub fn start(&mut self, key: impl Into<String>, now: Instant) {
        let key = key.into();
        if self.key == key {
            return;
        }
        self.key = key;
        self.start = Some(now);
    }

    /// Stop scrolling.
    pub fn stop(&mut self) {
        self.key.clear();
        self.start = None;
    }

    /// The current scroll offset for the given text and width.
    pub fn offset(&self, text_len: usize, width: usize, now: Instant) -> usize {
        if !self.active() {
            return 0;
        }
        if text_len <= width {
            return 0;
        }
        let Some(start) = self.start else {
            return 0;
        };
        if now < start {
            return 0;
        }
        let elapsed = now - start;
        if elapsed < INITIAL_PAUSE {
            return 0;
        }
        let scroll_elapsed = elapsed - INITIAL_PAUSE;
        let steps = (scroll_elapsed.as_millis() / CELL_INTERVAL.as_millis()) as usize;
        let period = text_len + GAP;
        if period == 0 {
            return 0;
        }
        steps % period
    }

    /// Render the marquee text for the given width.
    pub fn render(&self, text: &str, width: usize, now: Instant) -> String {
        let text_len = text.chars().count();
        if text_len <= width {
            return text.to_string();
        }
        let offset = self.offset(text_len, width, now);
        let chars: Vec<char> = text.chars().collect();
        let mut result = String::with_capacity(width);
        for i in 0..width {
            let idx = (offset + i) % (text_len + GAP);
            if idx < text_len {
                result.push(chars[idx]);
            } else {
                result.push(' ');
            }
        }
        result
    }
}

/// Whether a row should marquee: it must be hovered and its text must overflow.
pub fn should_marquee(text_len: usize, width: usize, hovered: bool) -> bool {
    hovered && text_len > width
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inactive_marquee_returns_zero() {
        let m = Marquee::new();
        assert_eq!(m.offset(50, 10, Instant::now()), 0);
    }

    #[test]
    fn short_text_no_scroll() {
        let mut m = Marquee::new();
        let now = Instant::now();
        m.start("w1", now);
        assert_eq!(m.offset(5, 10, now), 0);
    }

    #[test]
    fn initial_pause_prevents_scroll() {
        let mut m = Marquee::new();
        let now = Instant::now();
        m.start("w1", now);
        // During initial pause, offset is 0.
        assert_eq!(m.offset(50, 10, now), 0);
    }

    #[test]
    fn offset_advances_after_pause() {
        let mut m = Marquee::new();
        let now = Instant::now();
        m.start("w1", now);
        // After pause + one interval, offset should be 1.
        let later = now + INITIAL_PAUSE + CELL_INTERVAL;
        assert_eq!(m.offset(50, 10, later), 1);
    }

    #[test]
    fn offset_wraps_around() {
        let mut m = Marquee::new();
        let now = Instant::now();
        m.start("w1", now);
        let text_len = 20;
        let period = text_len + GAP;
        let later = now + INITIAL_PAUSE + CELL_INTERVAL * (period as u32);
        assert_eq!(m.offset(text_len, 10, later), 0);
    }

    #[test]
    fn render_truncates_short_text() {
        let m = Marquee::new();
        assert_eq!(m.render("hello", 10, Instant::now()), "hello");
    }

    #[test]
    fn render_scrolls_long_text() {
        let mut m = Marquee::new();
        let now = Instant::now();
        m.start("w1", now);
        let text = "abcdefghijklmnopqrstuvwxyz";
        let later = now + INITIAL_PAUSE + CELL_INTERVAL * 2;
        let rendered = m.render(text, 10, later);
        assert_eq!(rendered.chars().count(), 10);
        // Offset 2 means it starts at 'c'.
        assert!(rendered.starts_with('c'));
    }

    #[test]
    fn start_same_key_does_not_reset() {
        let mut m = Marquee::new();
        let now = Instant::now();
        m.start("w1", now);
        let start1 = m.start;
        m.start("w1", now + Duration::from_secs(1));
        assert_eq!(m.start, start1);
    }

    #[test]
    fn stop_clears_state() {
        let mut m = Marquee::new();
        m.start("w1", Instant::now());
        assert!(m.active());
        m.stop();
        assert!(!m.active());
    }

    #[test]
    fn should_marquee_requires_hover_and_overflow() {
        assert!(!should_marquee(5, 10, true));
        assert!(!should_marquee(15, 10, false));
        assert!(should_marquee(15, 10, true));
    }
}
