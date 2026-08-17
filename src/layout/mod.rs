//! Window tiling and layout management — ported from TUIOS `internal/layout`.
//!
//! Three layout families live here:
//! - [`bsp`] — a binary space partition tree (the default, bspwm-style)
//! - [`tiling`] — master-stack tiling with a master pane and stacked rest
//! - [`scrolling`] — niri-style columns on an infinite horizontal strip

pub mod bsp;
pub mod scrolling;
pub mod tiling;

pub use bsp::{
    AutoScheme, BSPTree, PreselectionDir, Rect, ResizeEdge, SplitLine, SplitType, TileNode,
};
pub use scrolling::ScrollingLayout;
pub use tiling::{calculate_tiling_layout, TileLayout};

/// Minimum window width in cells (mirrors `config.DefaultWindowWidth`).
pub const DEFAULT_WINDOW_WIDTH: i32 = 20;
/// Minimum window height in cells (mirrors `config.DefaultWindowHeight`).
pub const DEFAULT_WINDOW_HEIGHT: i32 = 5;
