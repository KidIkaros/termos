//! User configuration, keybindings, and themes — ported from TUIOS
//! `internal/config` and `internal/theme`.

pub mod keybindings;
pub mod registry;
pub mod theme;
pub mod userconfig;
pub mod validation;

pub use keybindings::Keybinding;
pub use theme::Theme;
pub use userconfig::{AppearanceConfig, KeybindingsConfig, StartupConfig, UserConfig};
