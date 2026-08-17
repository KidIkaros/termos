//! Graphics passthrough — Kitty graphics protocol and Sixel forwarding.
//!
//! TUIOS is a passthrough-first terminal multiplexer for images: it does not
//! rasterize images into ratatui cells. Instead it probes the host terminal's
//! capabilities, forwards or rewrites Kitty APC sequences and Sixel streams to
//! the host, and tracks placements so they can be re-placed when panes move,
//! resize, scroll, or change workspace.
//!
//! This mirrors the Go project's `internal/app/kitty_passthrough*.go` and
//! `internal/app/sixel_passthrough.go`, and reuses the pure-std Sixel decoder
//! from the sibling `terminal` crate (`src/sixel.rs`).

pub mod capability;
pub mod kitty;
pub mod placement;
pub mod sixel;

pub use capability::{Capabilities, HostTerminal, detect_host_terminal, inside_multiplexer};
pub use kitty::{KittyPassthrough, is_kitty_response};
pub use placement::{Placement, PlacementStore};
pub use sixel::SixelPassthrough;
