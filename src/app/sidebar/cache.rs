//! Render caching with FNV-1a signature invalidation.
//!
//! Ported from Go TUIOS `internal/app/sidebar_cache.go`. The cache stores
//! fully rendered lines, mouse hit rectangles, session IDs, nav rows, and
//! section geometry. It is invalidated by a signature that covers geometry,
//! config/theme, focus/cursor, workspace names, session identity/order,
//! accent picker preview, window state, and unread bits.

use super::{NavRow, RowHit};

/// A cached rendered line: the styled spans as a string of cells.
#[derive(Debug, Clone, Default)]
pub struct CachedLine {
    /// The text content of the line.
    pub text: String,
    /// The style per cell (foreground colour index, -1 = default).
    pub fg: Vec<i32>,
    /// Whether each cell is bold.
    pub bold: Vec<bool>,
}

/// The render cache: holds the last frame's output and a signature.
#[derive(Debug, Clone, Default)]
pub struct RenderCache {
    /// The cached lines.
    pub lines: Vec<CachedLine>,
    /// The cached hit rectangles.
    pub hits: Vec<RowHit>,
    /// The cached nav rows.
    pub nav: Vec<NavRow>,
    /// The cached session IDs in order.
    pub session_ids: Vec<String>,
    /// The last signature.
    pub signature: u64,
    /// Whether the cache is valid.
    pub valid: bool,
    /// The width the cache was built at.
    pub width: i32,
    /// The height the cache was built at.
    pub height: i32,
}

impl RenderCache {
    /// Invalidate the cache, forcing a rebuild.
    pub fn invalidate(&mut self) {
        self.valid = false;
    }

    /// Check whether the cache is valid for the given signature and geometry.
    pub fn is_valid(&self, signature: u64, width: i32, height: i32) -> bool {
        self.valid
            && self.signature == signature
            && self.width == width
            && self.height == height
    }

    /// Store a freshly rendered frame.
    #[allow(clippy::too_many_arguments)]
    pub fn store(
        &mut self,
        lines: Vec<CachedLine>,
        hits: Vec<RowHit>,
        nav: Vec<NavRow>,
        session_ids: Vec<String>,
        signature: u64,
        width: i32,
        height: i32,
    ) {
        self.lines = lines;
        self.hits = hits;
        self.nav = nav;
        self.session_ids = session_ids;
        self.signature = signature;
        self.width = width;
        self.height = height;
        self.valid = true;
    }
}

/// FNV-1a 64-bit hash.
pub fn fnv1a_64(data: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in data {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Compute a cache signature from the inputs that affect sidebar rendering.
#[allow(clippy::too_many_arguments)]
pub fn compute_signature(
    width: i32,
    height: i32,
    focused: bool,
    cursor: usize,
    collapsed: bool,
    agents_filter: &str,
    agents_sort: &str,
    workspace_names: &[String],
    session_ids: &[String],
    window_states: &[(String, String, i32, bool, u64)], // (id, title, workspace, done_seen, state_at)
    agent_states: &[(String, String)], // (window_id, state)
    accents: &[(String, u64)], // (window_id, accent_fold)
    peek: &str,
    hover_active: bool,
    hover_x: i32,
    hover_y: i32,
    marquee_key: &str,
) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    let mut feed = |bytes: &[u8]| {
        for &b in bytes {
            hash ^= b as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
    };
    feed(&width.to_le_bytes());
    feed(&height.to_le_bytes());
    feed(&[focused as u8]);
    feed(&cursor.to_le_bytes());
    feed(&[collapsed as u8]);
    feed(agents_filter.as_bytes());
    feed(agents_sort.as_bytes());
    for name in workspace_names {
        feed(name.as_bytes());
        feed(&[0]);
    }
    feed(&[0xff]);
    for id in session_ids {
        feed(id.as_bytes());
        feed(&[0]);
    }
    feed(&[0xff]);
    for (id, title, ws, seen, at) in window_states {
        feed(id.as_bytes());
        feed(&[0]);
        feed(title.as_bytes());
        feed(&[0]);
        feed(&ws.to_le_bytes());
        feed(&[*seen as u8]);
        feed(&at.to_le_bytes());
        feed(&[0]);
    }
    feed(&[0xff]);
    for (wid, state) in agent_states {
        feed(wid.as_bytes());
        feed(&[0]);
        feed(state.as_bytes());
        feed(&[0]);
    }
    feed(&[0xff]);
    for (wid, fold) in accents {
        feed(wid.as_bytes());
        feed(&[0]);
        feed(&fold.to_le_bytes());
        feed(&[0]);
    }
    feed(&[0xff]);
    feed(peek.as_bytes());
    feed(&[hover_active as u8]);
    feed(&hover_x.to_le_bytes());
    feed(&hover_y.to_le_bytes());
    feed(marquee_key.as_bytes());
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a_consistent() {
        let a = fnv1a_64(b"hello");
        let b = fnv1a_64(b"hello");
        assert_eq!(a, b);
    }

    #[test]
    fn fnv1a_different_inputs() {
        let a = fnv1a_64(b"hello");
        let b = fnv1a_64(b"world");
        assert_ne!(a, b);
    }

    #[test]
    fn cache_invalidate_sets_invalid() {
        let mut cache = RenderCache {
            valid: true,
            ..Default::default()
        };
        cache.invalidate();
        assert!(!cache.valid);
    }

    #[test]
    fn cache_is_valid_checks_all() {
        let mut cache = RenderCache::default();
        cache.store(vec![], vec![], vec![], vec![], 42, 30, 10);
        assert!(cache.is_valid(42, 30, 10));
        assert!(!cache.is_valid(43, 30, 10));
        assert!(!cache.is_valid(42, 31, 10));
        assert!(!cache.is_valid(42, 30, 11));
    }

    #[test]
    fn signature_changes_with_cursor() {
        let s1 = compute_signature(
            30, 10, false, 0, false, "all", "priority",
            &[], &[], &[], &[], &[], "", false, 0, 0, "",
        );
        let s2 = compute_signature(
            30, 10, false, 1, false, "all", "priority",
            &[], &[], &[], &[], &[], "", false, 0, 0, "",
        );
        assert_ne!(s1, s2);
    }

    #[test]
    fn signature_changes_with_window_state() {
        let s1 = compute_signature(
            30, 10, false, 0, false, "all", "priority",
            &[], &[], &[("w0".into(), "alpha".into(), 1, false, 0)], &[], &[], "",
            false, 0, 0, "",
        );
        let s2 = compute_signature(
            30, 10, false, 0, false, "all", "priority",
            &[], &[], &[("w0".into(), "beta".into(), 1, false, 0)], &[], &[], "",
            false, 0, 0, "",
        );
        assert_ne!(s1, s2);
    }

    #[test]
    fn signature_stable_for_same_inputs() {
        #![allow(clippy::type_complexity)]
        let args: (&[String], &[(String, String, i32, bool, u64)], &[(String, String)], &[(String, u64)]) = (&[], &[], &[], &[]);
        let s1 = compute_signature(30, 10, true, 2, false, "session", "recent", args.0, &[], args.1, args.2, args.3, "work", true, 5, 3, "w0");
        let s2 = compute_signature(30, 10, true, 2, false, "session", "recent", args.0, &[], args.1, args.2, args.3, "work", true, 5, 3, "w0");
        assert_eq!(s1, s2);
    }
}
