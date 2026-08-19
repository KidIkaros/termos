//! Shared utilities — buffer pools and other reuse helpers.

pub mod buffer;
pub mod guestenv;

pub use buffer::{ByteBufferPool, HighlightGrid, StringPool};
