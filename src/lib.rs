//! TUIOS — a terminal multiplexer and window manager, ported to Rust.
//!
//! This crate mirrors the architecture of the Go TUIOS project:
//! - [`vt`] — the VT emulator (parser + screen + scrollback)
//! - [`layout`] — BSP tiling, master-stack, and scrolling layouts
//! - [`terminal`] — PTY management and the terminal window
//! - [`config`] — user configuration, keybindings, themes
//! - [`app`] — the window manager (workspaces, modes, input)
//! - [`ui`] — shared rendering helpers
//! - [`hooks`] — shell-command lifecycle hooks

pub mod app;
pub mod config;
pub mod hooks;
pub mod layout;
pub mod session;
pub mod terminal;
pub mod ui;
pub mod vt;

pub use layout::{BSPTree, Rect};
pub use vt::Emulator;
