//! Performance utilities — object pooling, viewport culling, and adaptive
//! refresh rate control.
//!
//! Ported from Go TUIOS `internal/pool/pool.go` and the adaptive refresh
//! logic in `internal/app/os.go`.

use std::sync::Mutex;

/// A simple object pool for reusable buffers, reducing allocation pressure
/// during rendering. Ported from Go's `pool.Pool`.
pub struct BufferPool<T> {
    pool: Mutex<Vec<T>>,
    factory: fn() -> T,
    max_size: usize,
}

impl<T> BufferPool<T> {
    /// Create a new pool with the given factory and max pooled size.
    pub const fn new(factory: fn() -> T, max_size: usize) -> Self {
        Self {
            pool: Mutex::new(Vec::new()),
            factory,
            max_size,
        }
    }

    /// Take an object from the pool, or create a new one if empty.
    pub fn get(&self) -> T {
        if let Ok(mut pool) = self.pool.lock() {
            if let Some(item) = pool.pop() {
                return item;
            }
        }
        (self.factory)()
    }

    /// Return an object to the pool for reuse. If the pool is full, the
    /// object is dropped.
    pub fn put(&self, item: T) {
        if let Ok(mut pool) = self.pool.lock() {
            if pool.len() < self.max_size {
                pool.push(item);
            }
        }
    }

    /// Current number of pooled objects.
    pub fn len(&self) -> usize {
        self.pool.lock().map(|p| p.len()).unwrap_or(0)
    }

    /// Whether the pool is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Clear all pooled objects.
    pub fn clear(&self) {
        if let Ok(mut pool) = self.pool.lock() {
            pool.clear();
        }
    }
}

/// A pool of `Vec<u8>` for PTY output batching.
pub static OUTPUT_POOL: BufferPool<Vec<u8>> = BufferPool::new(Vec::new, 32);

/// A pool of `Vec<String>` for line rendering.
pub static LINE_POOL: BufferPool<Vec<String>> = BufferPool::new(Vec::new, 16);

/// Adaptive refresh rate controller. The focused pane renders at full speed
/// (up to `max_fps`); background panes render at a reduced rate to save CPU.
/// Ported from Go TUIOS's adaptive refresh logic.
pub struct AdaptiveRefresh {
    /// Maximum frames per second for the focused pane.
    max_fps: u32,
    /// Reduced frames per second for background panes.
    bg_fps: u32,
    /// Minimum milliseconds between focused renders.
    focused_interval_ms: u64,
    /// Minimum milliseconds between background renders.
    bg_interval_ms: u64,
}

impl AdaptiveRefresh {
    /// Create a new controller with the given max and background FPS.
    pub fn new(max_fps: u32, bg_fps: u32) -> Self {
        Self {
            max_fps,
            bg_fps,
            focused_interval_ms: 1000 / max_fps.max(1) as u64,
            bg_interval_ms: 1000 / bg_fps.max(1) as u64,
        }
    }

    /// The minimum interval between focused renders, in milliseconds.
    pub fn focused_interval_ms(&self) -> u64 {
        self.focused_interval_ms
    }

    /// The minimum interval between background renders, in milliseconds.
    pub fn bg_interval_ms(&self) -> u64 {
        self.bg_interval_ms
    }

    /// Whether enough time has passed for a focused render.
    pub fn should_render_focused(&self, elapsed_ms: u64) -> bool {
        elapsed_ms >= self.focused_interval_ms
    }

    /// Whether enough time has passed for a background render.
    pub fn should_render_bg(&self, elapsed_ms: u64) -> bool {
        elapsed_ms >= self.bg_interval_ms
    }

    /// The configured max FPS.
    pub fn max_fps(&self) -> u32 {
        self.max_fps
    }

    /// The configured background FPS.
    pub fn bg_fps(&self) -> u32 {
        self.bg_fps
    }
}

impl Default for AdaptiveRefresh {
    fn default() -> Self {
        Self::new(60, 30)
    }
}

/// Check whether a rectangle is at least partially visible within the
/// content area. Used for viewport culling — off-screen panes are skipped
/// during rendering to save CPU.
pub fn is_visible(rect: &crate::layout::Rect, area: &crate::layout::Rect) -> bool {
    // Not visible if entirely to the left, right, above, or below.
    if rect.x + rect.w <= area.x || rect.x >= area.x + area.w {
        return false;
    }
    if rect.y + rect.h <= area.y || rect.y >= area.y + area.h {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Rect;

    #[test]
    fn buffer_pool_get_put() {
        let pool: BufferPool<Vec<u8>> = BufferPool::new(|| Vec::with_capacity(64), 8);
        let mut buf = pool.get();
        buf.extend_from_slice(&[1, 2, 3]);
        assert_eq!(buf.len(), 3);
        buf.clear();
        pool.put(buf);

        let buf2 = pool.get();
        assert!(buf2.is_empty());
        assert!(buf2.capacity() >= 64);
    }

    #[test]
    fn buffer_pool_max_size() {
        let pool: BufferPool<Vec<u8>> = BufferPool::new(Vec::new, 2);
        pool.put(vec![1]);
        pool.put(vec![2]);
        pool.put(vec![3]); // should be dropped, pool is full
        assert_eq!(pool.len(), 2);
    }

    #[test]
    fn adaptive_refresh_intervals() {
        let ar = AdaptiveRefresh::new(60, 30);
        assert_eq!(ar.focused_interval_ms(), 16); // 1000/60 ≈ 16
        assert_eq!(ar.bg_interval_ms(), 33); // 1000/30 ≈ 33
        assert!(ar.should_render_focused(20));
        assert!(!ar.should_render_focused(10));
        assert!(ar.should_render_bg(40));
        assert!(!ar.should_render_bg(20));
    }

    #[test]
    fn viewport_culling_visible() {
        let area = Rect { x: 0, y: 0, w: 80, h: 24 };
        let pane = Rect { x: 10, y: 5, w: 20, h: 10 };
        assert!(is_visible(&pane, &area));
    }

    #[test]
    fn viewport_culling_off_screen_left() {
        let area = Rect { x: 0, y: 0, w: 80, h: 24 };
        let pane = Rect { x: -10, y: 0, w: 5, h: 10 };
        assert!(!is_visible(&pane, &area));
    }

    #[test]
    fn viewport_culling_off_screen_right() {
        let area = Rect { x: 0, y: 0, w: 80, h: 24 };
        let pane = Rect { x: 85, y: 0, w: 10, h: 10 };
        assert!(!is_visible(&pane, &area));
    }

    #[test]
    fn viewport_culling_off_screen_below() {
        let area = Rect { x: 0, y: 0, w: 80, h: 24 };
        let pane = Rect { x: 0, y: 30, w: 10, h: 10 };
        assert!(!is_visible(&pane, &area));
    }

    #[test]
    fn viewport_culling_partially_visible() {
        let area = Rect { x: 0, y: 0, w: 80, h: 24 };
        // Pane extends past the right edge but is partially visible.
        let pane = Rect { x: 70, y: 0, w: 20, h: 10 };
        assert!(is_visible(&pane, &area));
    }
}
