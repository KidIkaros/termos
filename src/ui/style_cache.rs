//! Style cache — ported from Go TUIOS `internal/app/stylecache.go`.
//!
//! Thread-safe cache of ratatui `Style` objects keyed by (fg, bg, modifier,
//! cursor). Reduces allocation pressure by reusing style objects for
//! identical cell attributes.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use ratatui::style::{Color, Modifier, Style};

/// A cached style entry.
#[derive(Debug, Clone, Copy)]
struct StyleEntry {
    style: Style,
}

/// Thread-safe style cache with LRU eviction and statistics.
pub struct StyleCache {
    cache: Mutex<HashMap<u64, StyleEntry>>,
    max_size: usize,
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
}

impl StyleCache {
    /// Create a new style cache with the given maximum size.
    pub fn new(max_size: usize) -> Self {
        Self {
            cache: Mutex::new(HashMap::with_capacity(max_size.min(1024))),
            max_size: if max_size == 0 { 512 } else { max_size },
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            evictions: AtomicU64::new(0),
        }
    }

    /// Look up or insert a style. Returns the cached or newly created style.
    pub fn get_or_insert(
        &self,
        fg: Option<Color>,
        bg: Option<Color>,
        modifier: Modifier,
        is_cursor: bool,
    ) -> Style {
        let key = hash_style(fg, bg, modifier, is_cursor);
        let mut cache = self.cache.lock().unwrap();
        if let Some(entry) = cache.get(&key) {
            self.hits.fetch_add(1, Ordering::Relaxed);
            return entry.style;
        }
        self.misses.fetch_add(1, Ordering::Relaxed);

        // Evict if at capacity (simple random eviction by draining one entry).
        if cache.len() >= self.max_size {
            if let Some(&k) = cache.keys().next() {
                cache.remove(&k);
                self.evictions.fetch_add(1, Ordering::Relaxed);
            }
        }

        let mut style = Style::default();
        if let Some(fg) = fg {
            style = style.fg(fg);
        }
        if let Some(bg) = bg {
            style = style.bg(bg);
        }
        if !modifier.is_empty() {
            style = style.add_modifier(modifier);
        }
        if is_cursor {
            style = style.add_modifier(Modifier::REVERSED);
        }

        let entry = StyleEntry { style };
        cache.insert(key, entry);
        style
    }

    /// Cache hit count.
    pub fn hits(&self) -> u64 {
        self.hits.load(Ordering::Relaxed)
    }

    /// Cache miss count.
    pub fn misses(&self) -> u64 {
        self.misses.load(Ordering::Relaxed)
    }

    /// Eviction count.
    pub fn evictions(&self) -> u64 {
        self.evictions.load(Ordering::Relaxed)
    }

    /// Hit rate: hits / (hits + misses).
    pub fn hit_rate(&self) -> f64 {
        let h = self.hits() as f64;
        let m = self.misses() as f64;
        if h + m == 0.0 {
            0.0
        } else {
            h / (h + m)
        }
    }

    /// Current entry count.
    pub fn len(&self) -> usize {
        self.cache.lock().unwrap().len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Clear all entries.
    pub fn clear(&self) {
        self.cache.lock().unwrap().clear();
    }
}

impl Default for StyleCache {
    fn default() -> Self {
        Self::new(512)
    }
}

/// Hash a style's components into a u64 key.
fn hash_style(fg: Option<Color>, bg: Option<Color>, modifier: Modifier, is_cursor: bool) -> u64 {
    let mut h: u64 = 0;
    h = h.wrapping_mul(31).wrapping_add(color_hash(fg));
    h = h.wrapping_mul(31).wrapping_add(color_hash(bg));
    h = h.wrapping_mul(31).wrapping_add(modifier.bits() as u64);
    h = h
        .wrapping_mul(31)
        .wrapping_add(if is_cursor { 1 } else { 0 });
    h
}

fn color_hash(c: Option<Color>) -> u64 {
    match c {
        None => 0,
        Some(Color::Reset) => 1,
        Some(Color::Black) => 2,
        Some(Color::Red) => 3,
        Some(Color::Green) => 4,
        Some(Color::Yellow) => 5,
        Some(Color::Blue) => 6,
        Some(Color::Magenta) => 7,
        Some(Color::Cyan) => 8,
        Some(Color::Gray) => 9,
        Some(Color::DarkGray) => 10,
        Some(Color::LightRed) => 11,
        Some(Color::LightGreen) => 12,
        Some(Color::LightYellow) => 13,
        Some(Color::LightBlue) => 14,
        Some(Color::LightMagenta) => 15,
        Some(Color::LightCyan) => 16,
        Some(Color::White) => 17,
        Some(Color::Rgb(r, g, b)) => (r as u64) << 16 | (g as u64) << 8 | (b as u64) | (1u64 << 24),
        Some(Color::Indexed(i)) => (i as u64) | (1u64 << 32),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_get_insert() {
        let cache = StyleCache::new(64);
        let s1 = cache.get_or_insert(Some(Color::Red), Some(Color::Black), Modifier::BOLD, false);
        let s2 = cache.get_or_insert(Some(Color::Red), Some(Color::Black), Modifier::BOLD, false);
        assert_eq!(s1, s2);
        assert_eq!(cache.hits(), 1);
        assert_eq!(cache.misses(), 1);
    }

    #[test]
    fn different_styles_dont_collide() {
        let cache = StyleCache::new(64);
        let _ = cache.get_or_insert(Some(Color::Red), None, Modifier::empty(), false);
        let _ = cache.get_or_insert(Some(Color::Blue), None, Modifier::empty(), false);
        assert_eq!(cache.misses(), 2);
        assert_eq!(cache.hits(), 0);
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn hit_rate() {
        let cache = StyleCache::new(64);
        let _ = cache.get_or_insert(Some(Color::Red), None, Modifier::empty(), false);
        let _ = cache.get_or_insert(Some(Color::Red), None, Modifier::empty(), false);
        let _ = cache.get_or_insert(Some(Color::Red), None, Modifier::empty(), false);
        assert_eq!(cache.hit_rate(), 2.0 / 3.0);
    }

    #[test]
    fn eviction_on_capacity() {
        let cache = StyleCache::new(2);
        let _ = cache.get_or_insert(Some(Color::Red), None, Modifier::empty(), false);
        let _ = cache.get_or_insert(Some(Color::Blue), None, Modifier::empty(), false);
        let _ = cache.get_or_insert(Some(Color::Green), None, Modifier::empty(), false);
        assert!(cache.evictions() >= 1);
        assert!(cache.len() <= 2);
    }

    #[test]
    fn clear() {
        let cache = StyleCache::new(64);
        let _ = cache.get_or_insert(Some(Color::Red), None, Modifier::empty(), false);
        assert!(!cache.is_empty());
        cache.clear();
        assert!(cache.is_empty());
    }

    #[test]
    fn default_size() {
        let cache = StyleCache::default();
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn cursor_flag_changes_hash() {
        let cache = StyleCache::new(64);
        let _ = cache.get_or_insert(Some(Color::Red), None, Modifier::empty(), false);
        let _ = cache.get_or_insert(Some(Color::Red), None, Modifier::empty(), true);
        assert_eq!(cache.misses(), 2);
    }
}
