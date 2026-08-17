# TermOS Architecture

A Rust port of [TUIOS](https://github.com/Gaurav-Gosain/tuios), the terminal
multiplexer and window manager. This document describes the Rust crate's
module layout and how the pieces fit together.

## Module Map

```
src/
├── main.rs              CLI entry point (cobra-style subcommands)
├── lib.rs               Crate root
├── app/
│   ├── mod.rs           Os: central state, windows, workspaces, modes
│   ├── input.rs         Modal key routing, leader prefixes, tape controls
│   ├── render.rs        Ratatui compositor, panes, overlays, tape manager
│   └── agent_alert.rs   Agent state notification policy
├── config/userconfig.rs TOML configuration, keybindings, themes
├── graphics/
│   ├── capability.rs    Host terminal detection (kitty, ghostty, wezterm...)
│   ├── kitty.rs         Kitty APC graphics passthrough + id remapping
│   ├── sixel.rs         Sixel DCS passthrough
│   └── placement.rs     Per-window image placement tracking
├── hooks/mod.rs         Shell-command lifecycle hooks (TOML config)
├── keys.rs              Key-name encoding for verbs and tape scripting
├── layout/bsp.rs        BSP tiling, master-stack, scrolling layouts
├── network/             SSH/web/TLS (gated behind `network` feature)
│   ├── ssh.rs           russh SSH server
│   ├── web.rs           axum + WebSocket web terminal
│   └── tls.rs           rustls TLS wrapper
├── session/
│   ├── daemon.rs        Unix-socket daemon, session management
│   ├── protocol.rs      Wire protocol messages
│   ├── model.rs         Session metadata
│   └── remote.rs        Remote window (daemon-backed)
├── tape/
│   ├── lexer.rs         Tokenizer
│   ├── parser.rs        AST generation
│   ├── command.rs       Command model + serialization
│   ├── executor.rs      Command execution against Os
│   ├── player.rs        Playback engine with progress
│   ├── recorder.rs      Recording from live input
│   ├── header.rs        Tape file header (version, metadata)
│   ├── tapes.rs         Tape storage (XDG data dir)
│   └── trust.rs         Per-machine trust store (SHA-256 + canonical path)
├── terminal/
│   ├── pty.rs           Unix PTY creation, reader thread, writer, resizing
│   └── window.rs        PTY + emulator pairing
├── ui/                  Shared rendering helpers
└── vt/
    ├── parser.rs        VT parser state machine (CSI, OSC, DCS, APC, PM, SOS)
    └── emulator.rs      Main/alt screens, scrollback, OSC 52, graphics APC
```

## Architecture

TermOS follows a Model-View-Update loop:

- **Model**: `Os` struct in `app/mod.rs` — windows, workspaces, modes,
  hooks, agent state, recording, graphics passthrough.
- **View**: `render()` in `app/render.rs` — draws panes, borders,
  overlays, tape manager, trust review, recording indicator.
- **Update**: `handle_key()` in `app/input.rs` — modal key routing,
  leader-prefix handling, tape controls, terminal passthrough.

The main event loop (`main.rs`) polls crossterm events and PTY output,
feeds them to the Os, and renders at ~60 FPS.

## Graphics Passthrough

TermOS is a passthrough-first multiplexer for images: it does not rasterize
images into ratatui cells. Instead it:

1. Probes the host terminal's capabilities (`graphics/capability.rs`).
2. Collects Kitty APC and Sixel DCS sequences from each window's VT
   emulator (`vt/emulator.rs`).
3. Forwards them to the host terminal with id remapping and coordinate
   offsetting (`graphics/kitty.rs`, `graphics/sixel.rs`).
4. Tracks placements so images can be re-placed when panes move, resize,
   or switch workspace (`graphics/placement.rs`).

## Network Modes

The `network` cargo feature enables SSH and web server modes:

- `--network ssh`: russh SSH server, each connection gets a fresh Os.
- `--network web`: axum HTTP + WebSocket, serves xterm.js frontend.
- The `tls` feature adds rustls encryption for both.

## Tape Scripting

Tapes are reproducible terminal automation scripts. See
[TAPE_SCRIPTING.md](TAPE_SCRIPTING.md) for the language reference.

## Sessions and Daemon

TermOS runs a Unix-socket daemon for session persistence. See
[DAEMON.md](DAEMON.md) for details.
