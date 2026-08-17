//! User configuration, keybindings, and themes — ported from TUIOS
//! `internal/config` and `internal/theme`.

pub mod keybindings;
pub mod theme;
pub mod userconfig;

pub use keybindings::Keybinding;
pub use theme::Theme;
pub use userconfig::{AppearanceConfig, KeybindingsConfig, StartupConfig, UserConfig};
