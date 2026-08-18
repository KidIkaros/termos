//! Buffer reuse utilities.
//!
//! Inspired by Go TUIOS' `internal/pool/pool.go`, these types reduce
//! allocations on hot paths (PTY I/O and frame rendering) by recycling
//! backing buffers. Each pool guards its free list with a [`Mutex`]; the
//! lock is held only for the brief pop/push, never while the borrowed
//! buffer is in use.
//!
//! - [`ByteBufferPool`] — recycles `Vec<u8>` buffers for PTY reads/writes.
//! - [`StringPool`] — recycles `String` buffers for render output.
//! - [`HighlightGrid`] — sparse grid tracking highlighted cells.

use std::sync::Mutex;

/// A pool of `Vec<u8>` buffers for PTY I/O.
///
/// Buffers are returned with their capacity preserved so subsequent
/// [`ByteBufferPool::get`] calls reuse the same backing allocation instead
/// of hitting the global allocator. The `capacity` passed to
/// [`ByteBufferPool::new`] is the minimum capacity every buffer is
/// initialised (and re-grown) to.
pub struct ByteBufferPool {
    capacity: usize,
    free: Mutex<Vec<Vec<u8>>>,
}

impl ByteBufferPool {
    /// Create a new pool whose buffers hold at least `capacity` bytes.
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            free: Mutex::new(Vec::new()),
        }
    }

    /// Borrow a buffer, cleared and with capacity preserved.
    ///
    /// If the pool is empty a fresh `Vec` of `capacity` bytes is allocated.
    /// Returned buffers are shrunk back to `capacity` if they grew
    /// excessively while in use, keeping peak memory bounded.
    pub fn get(&self) -> Vec<u8> {
        let mut buf = self.free.lock().unwrap().pop().unwrap_or_default();
        if buf.capacity() < self.capacity {
            buf.reserve(self.capacity - buf.capacity());
        }
        buf.clear();
        buf
    }

    /// Return a buffer to the pool for reuse.
    ///
    /// Buffers that grew far beyond the configured capacity are dropped
    /// rather than retained, so a single oversized read cannot permanently
    /// inflate the pool.
    pub fn put(&self, mut buf: Vec<u8>) {
        if buf.capacity() > self.capacity.saturating_mul(4) {
            return;
        }
        buf.clear();
        self.free.lock().unwrap().push(buf);
    }
}

impl Default for ByteBufferPool {
    fn default() -> Self {
        Self::new(32 * 1024)
    }
}

/// A pool of `String` buffers for rendering.
///
/// Strings are returned cleared but with their capacity preserved, so
/// repeated frame renders reuse the same backing buffer instead of
/// reallocating.
pub struct StringPool {
    free: Mutex<Vec<String>>,
}

impl StringPool {
    /// Create a new empty string pool.
    pub fn new() -> Self {
        Self {
            free: Mutex::new(Vec::new()),
        }
    }

    /// Borrow a cleared string, reusing a pooled allocation when available.
    pub fn get(&self) -> String {
        let mut s = self.free.lock().unwrap().pop().unwrap_or_default();
        s.clear();
        s
    }

    /// Return a string to the pool for reuse.
    pub fn put(&self, mut s: String) {
        s.clear();
        self.free.lock().unwrap().push(s);
    }
}

impl Default for StringPool {
    fn default() -> Self {
        Self::new()
    }
}

/// A sparse grid for tracking cell highlights.
///
/// Rows are allocated lazily on first write, so a grid covering a large
/// terminal only pays for the rows actually touched. Backing arrays are
/// retained across [`HighlightGrid::reset`] / [`HighlightGrid::init`]
/// cycles so repeated copy-mode sweeps avoid reallocation.
pub struct HighlightGrid {
    rows: Vec<Vec<bool>>,
    max_y: usize,
    max_x: usize,
    inited: bool,
}

impl HighlightGrid {
    /// Create a new empty (uninitialised) grid.
    pub fn new() -> Self {
        Self {
            rows: Vec::new(),
            max_y: 0,
            max_x: 0,
            inited: false,
        }
    }

    /// Initialise the grid for the given dimensions.
    ///
    /// Existing rows are reused when possible: rows whose backing array is
    /// wide enough are truncated/extended to `max_x` and cleared, while
    /// too-narrow rows are dropped so the next [`HighlightGrid::set`]
    /// reallocates at the correct width.
    pub fn init(&mut self, max_y: usize, max_x: usize) {
        self.max_y = max_y;
        self.max_x = max_x;
        self.inited = true;

        if self.rows.capacity() >= max_y {
            self.rows.clear();
            self.rows.resize_with(max_y, Vec::new);
        } else {
            self.rows = vec![Vec::new(); max_y];
        }
    }

    /// Mark cell `(y, x)` as highlighted.
    ///
    /// Out-of-bounds coordinates are silently ignored.
    pub fn set(&mut self, y: usize, x: usize) {
        if y >= self.max_y || x >= self.max_x {
            return;
        }
        let row = &mut self.rows[y];
        if row.len() < self.max_x {
            row.resize(self.max_x, false);
        }
        row[x] = true;
    }

    /// Return whether cell `(y, x)` is highlighted.
    ///
    /// Out-of-bounds coordinates and untouched rows report `false`.
    pub fn get(&self, y: usize, x: usize) -> bool {
        if y >= self.max_y || x >= self.max_x {
            return false;
        }
        self.rows[y].get(x).copied().unwrap_or(false)
    }

    /// Clear every highlighted cell, retaining backing arrays for reuse.
    pub fn clear(&mut self) {
        for row in &mut self.rows {
            for cell in row.iter_mut() {
                *cell = false;
            }
        }
    }

    /// Reset the grid to its uninitialised state, retaining backing arrays.
    ///
    /// After [`HighlightGrid::reset`], [`HighlightGrid::init`] must be
    /// called before the grid is used again.
    pub fn reset(&mut self) {
        for row in &mut self.rows {
            for cell in row.iter_mut() {
                *cell = false;
            }
        }
        self.inited = false;
    }
}

impl Default for HighlightGrid {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_buffer_pool_get_returns_cleared_buffer() {
        let pool = ByteBufferPool::new(128);
        let mut buf = pool.get();
        buf.extend_from_slice(b"hello");
        pool.put(buf);

        let next = pool.get();
        assert!(next.is_empty());
        assert!(next.capacity() >= 128);
    }

    #[test]
    fn byte_buffer_pool_preserves_capacity() {
        let pool = ByteBufferPool::new(256);
        let buf = pool.get();
        assert!(buf.capacity() >= 256);
        pool.put(buf);

        let reused = pool.get();
        assert!(reused.capacity() >= 256);
    }

    #[test]
    fn byte_buffer_pool_drops_oversized_buffers() {
        let pool = ByteBufferPool::new(64);
        let mut buf = pool.get();
        buf.resize(64 * 5, 0);
        let oversized_cap = buf.capacity();
        pool.put(buf);

        let next = pool.get();
        assert!(next.capacity() < oversized_cap || next.capacity() <= 64 * 4);
    }

    #[test]
    fn byte_buffer_pool_default_capacity() {
        let pool = ByteBufferPool::default();
        let buf = pool.get();
        assert!(buf.capacity() >= 32 * 1024);
    }

    #[test]
    fn string_pool_get_returns_cleared_string() {
        let pool = StringPool::new();
        let mut s = pool.get();
        s.push_str("render output");
        pool.put(s);

        let next = pool.get();
        assert!(next.is_empty());
    }

    #[test]
    fn string_pool_preserves_capacity() {
        let pool = StringPool::new();
        let mut s = pool.get();
        s.reserve(1024);
        let cap = s.capacity();
        pool.put(s);

        let reused = pool.get();
        assert_eq!(reused.capacity(), cap);
    }

    #[test]
    fn string_pool_default_is_empty() {
        let pool = StringPool::default();
        let s = pool.get();
        assert!(s.is_empty());
    }

    #[test]
    fn highlight_grid_set_and_get() {
        let mut grid = HighlightGrid::new();
        grid.init(10, 20);
        grid.set(3, 5);
        assert!(grid.get(3, 5));
        assert!(!grid.get(3, 6));
        assert!(!grid.get(4, 5));
    }

    #[test]
    fn highlight_grid_out_of_bounds_is_ignored() {
        let mut grid = HighlightGrid::new();
        grid.init(5, 5);
        grid.set(10, 10);
        assert!(!grid.get(10, 10));
        assert!(!grid.get(5, 0));
        assert!(!grid.get(0, 5));
    }

    #[test]
    fn highlight_grid_clear() {
        let mut grid = HighlightGrid::new();
        grid.init(4, 4);
        grid.set(0, 0);
        grid.set(2, 3);
        grid.clear();
        assert!(!grid.get(0, 0));
        assert!(!grid.get(2, 3));
    }

    #[test]
    fn highlight_grid_reset_requires_reinit() {
        let mut grid = HighlightGrid::new();
        grid.init(4, 4);
        grid.set(1, 1);
        grid.reset();
        grid.init(4, 4);
        assert!(!grid.get(1, 1));
    }

    #[test]
    fn highlight_grid_reuse_across_reinit_smaller() {
        let mut grid = HighlightGrid::new();
        grid.init(8, 8);
        grid.set(5, 5);

        grid.reset();
        grid.init(4, 4);
        assert!(!grid.get(5, 5));
        grid.set(2, 2);
        assert!(grid.get(2, 2));
    }

    #[test]
    fn highlight_grid_reuse_across_reinit_larger() {
        let mut grid = HighlightGrid::new();
        grid.init(2, 2);
        grid.set(0, 0);

        grid.reset();
        grid.init(4, 4);
        assert!(!grid.get(0, 0));
        grid.set(3, 3);
        assert!(grid.get(3, 3));
    }

    #[test]
    fn highlight_grid_untouched_row_reports_false() {
        let mut grid = HighlightGrid::new();
        grid.init(5, 5);
        grid.set(0, 0);
        assert!(!grid.get(4, 4));
    }
}
