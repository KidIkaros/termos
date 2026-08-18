//! TermOS — a terminal multiplexer and window manager, ported to Rust.
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
//! - [`harness`] — agent harness detection and manifests
//! - [`util`] — buffer pools and shared utilities

#![deny(clippy::all)]
#![allow(clippy::result_large_err)]

pub mod app;
pub mod cli;
pub mod config;
pub mod graphics;
pub mod harness;
pub mod hooks;
pub mod keys;
pub mod layout;
#[cfg(feature = "network")]
pub mod network;
pub mod server;
pub mod session;
pub mod tape;
pub mod terminal;
pub mod testutil;
pub mod ui;
pub mod util;
pub mod vt;
pub mod web;

pub use layout::{BSPTree, Rect};
pub use vt::Emulator;
