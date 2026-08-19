//! Graphics passthrough — Kitty graphics protocol and Sixel forwarding.
//!
//! TermOS is a passthrough-first terminal multiplexer for images: it does not
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
pub mod kitty_parser;
pub mod placement;
pub mod sixel;

pub use capability::{detect_host_terminal, inside_multiplexer, Capabilities, HostTerminal};
pub use kitty::{is_kitty_response, KittyPassthrough};
pub use placement::{
    compute_placement_geometry, hide_all_placements, is_occluded_by_higher_window, position_changed,
    refresh_all_placements, Placement, PlacementGeometry, PlacementStore, RefreshResult,
    WindowPositionInfo,
};
pub use sixel::{SixelPassthrough, SixelPassthroughPlacement};
