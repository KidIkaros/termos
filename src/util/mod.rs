//! Shared utilities — buffer pools and other reuse helpers.

pub mod buffer;
pub mod guestenv;
pub mod linewidth;
pub mod ratelimit;
pub mod snowflake;
pub mod theme_detect;

pub use buffer::{ByteBufferPool, HighlightGrid, StringPool};
pub use ratelimit::{TieredRateLimiter, TokenBucket};
pub use snowflake::Snowflake;
