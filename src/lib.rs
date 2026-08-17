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
//! - [`keys`] — key-name encoding for verbs and tape scripting
//! - [`tape`] — the `.tape` scripting language (lexer, parser, executor)
//! - [`graphics`] — Kitty/Sixel graphics passthrough and placement tracking

#![deny(clippy::all)]

pub mod app;
pub mod config;
pub mod graphics;
pub mod hooks;
pub mod keys;
pub mod layout;
#[cfg(feature = "network")]
pub mod network;
pub mod session;
pub mod tape;
pub mod terminal;
pub mod ui;
pub mod vt;

pub use layout::{BSPTree, Rect};
pub use vt::Emulator;
