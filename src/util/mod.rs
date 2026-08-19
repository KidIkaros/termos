//! Shared utilities — buffer pools and other reuse helpers.

pub mod buffer;
pub mod guestenv;

pub use buffer::{ByteBufferPool, HighlightGrid, StringPool};

/// Acquire a mutex guard, recovering from poison instead of panicking.
///
/// A poisoned mutex means another thread panicked while holding the lock.
/// The data may be in an inconsistent state, but for a TUI app it's better
/// to continue with potentially-stale state than to crash the whole session.
#[inline]
pub fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
