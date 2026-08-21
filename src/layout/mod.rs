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
    AutoScheme, BSPTree, PreselectionDir, Rect, ResizeEdge, SerializedBSPTree, SplitLine,
    SplitType, TileNode,
};
pub use scrolling::ScrollingLayout;
pub use tiling::{calculate_tiling_layout, TileLayout};

/// The active layout mode for tiled windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LayoutMode {
    /// Binary space partition (the default).
    #[default]
    BSP,
    /// One master pane on the left, rest stacked on the right.
    MasterStack,
    /// niri-style columns on an infinite horizontal strip.
    Scrolling,
}

impl LayoutMode {
    /// Short label shown in the dock mode pill.
    pub fn label(self) -> &'static str {
        match self {
            LayoutMode::BSP => "BSP",
            LayoutMode::MasterStack => "MS",
            LayoutMode::Scrolling => "SCR",
        }
    }

    /// Cycle to the next layout mode.
    pub fn next(self) -> Self {
        match self {
            LayoutMode::BSP => LayoutMode::MasterStack,
            LayoutMode::MasterStack => LayoutMode::Scrolling,
            LayoutMode::Scrolling => LayoutMode::BSP,
        }
    }
}

/// Minimum window width in cells (mirrors `config.DefaultWindowWidth`).
pub const DEFAULT_WINDOW_WIDTH: i32 = 20;
/// Minimum window height in cells (mirrors `config.DefaultWindowHeight`).
pub const DEFAULT_WINDOW_HEIGHT: i32 = 5;
