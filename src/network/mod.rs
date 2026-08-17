//! Network modes — SSH server, web server, and TLS support.
//!
//! These are gated behind the `network` and `tls` cargo features so the
//! local TUI binary doesn't pull in tokio/russh/axum when network access
//! isn't needed. The Go reference uses Wish (SSH) and a separate
//! `tuios-web` binary; the Rust port uses `russh` (pure-Rust SSH) and
//! `axum` + `tokio-tungstenite` (web), all in the same binary with
//! `--network ssh` / `--network web` flags.
//!
//! Architecture:
//! - `ssh`: russh server that spawns a TUIOS session per connection,
//!   forwarding PTY I/O and resize events. Graphics passthrough works
//!   over SSH because the APC/DCS sequences are forwarded as-is.
//! - `web`: axum HTTP server serves a static xterm.js frontend; a
//!   WebSocket upgrade carries terminal I/O as JSON frames.
//! - `tls`: rustls wraps the SSH/web listeners for encrypted transport.

#[cfg(feature = "network")]
pub mod ssh;
#[cfg(feature = "tls")]
pub mod tls;
#[cfg(feature = "network")]
pub mod web;
