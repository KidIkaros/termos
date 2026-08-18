//! User configuration, keybindings, and themes — ported from TUIOS
//! `internal/config` and `internal/theme`.

pub mod keybindings;
pub mod keynormalizer;
pub mod overrides;
pub mod registry;
pub mod save;
pub mod theme;
pub mod userconfig;
pub mod validation;
pub mod watcher;

pub use keybindings::Keybinding;
pub use overrides::Overrides;
pub use theme::Theme;
pub use userconfig::{
    AppearanceConfig, DebugConfig, KeybindingsConfig, ScrollbarConfig, SidebarConfig, StartupConfig,
    TapeConfig, UserConfig,
};
