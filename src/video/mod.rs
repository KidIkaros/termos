//! Phase 36: In-pane Video & Media Player
//!
//! Converts RGB24 frames to half-block (▀/▄) ratatui cell grids for
//! display in terminal panes. Uses ffmpeg for decoding and the half-block
//! technique for 2× vertical resolution.

pub mod halfblock;
pub mod decoder;

pub use halfblock::{HalfBlockMapper, HalfBlockCell};
pub use decoder::FrameDecoder;
