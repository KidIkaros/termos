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
│   ├── actions.rs       Custom action dispatch
│   ├── agent_alert.rs   Agent state notification policy
│   ├── border_grid.rs   Dock border-cell grid rendering
│   ├── clipboard.rs     OSC 52 clipboard integration
│   ├── copymode_ext.rs  Selection text, styled fragments, clean text
│   ├── dock.rs          Bottom dock bar (workspace pills, indicators)
│   ├── dock_session_buttons.rs  Session picker in dock
│   ├── effect.rs        Effect channel (Quit, Clipboard, etc.)
│   ├── float.rs         Floating pane management
│   ├── interaction.rs   Mouse/keyboard interaction helpers
│   ├── layout_templates.rs  Named layout presets
│   ├── msg.rs           Message enum (Tick, Key, Mouse, Resize…)
│   ├── overlay_hit.rs   Hit-testing for overlays
│   ├── overlay_mouse.rs Overlay drag/mouse routing
│   ├── pixel_canvas.rs  Per-cell RGB background (shadows, gradients, SDF corners)
│   └── sidebar/         Multi-page sidebar (accent, agents, cache, marquee)
├── config/
│   ├── userconfig.rs    TOML configuration, keybindings, themes
│   ├── watcher.rs       Config file polling watcher
│   └── theme.rs         Theme system (21 built-in, custom, swatch picker)
├── graphics/
│   ├── capability.rs    Host terminal detection (kitty, ghostty, wezterm...)
│   ├── kitty.rs         Kitty APC graphics passthrough + id remapping
│   ├── sixel.rs         Sixel DCS passthrough
│   └── placement.rs     Per-window image placement tracking
├── hooks/mod.rs         Shell-command lifecycle hooks (TOML config)
├── keys.rs              Key-name encoding for verbs and tape scripting
├── layout/
│   └── bsp.rs           BSP tiling, master-stack, scrolling layouts
├── network/             SSH/web/TLS (gated behind `network` feature)
│   ├── ssh.rs           russh SSH server + SGR mouse forwarding
│   ├── web.rs           axum + WebSocket web terminal + mouse forwarding
│   ├── web/index.html   xterm.js frontend (mouse, keyboard)
│   └── tls.rs           rustls TLS wrapper
├── session/             Daemon, persistence, agent state
│   ├── daemon.rs        Unix-socket daemon, session management, PTY pool
│   ├── protocol.rs      Wire protocol messages
│   ├── model.rs         Session metadata
│   ├── remote.rs        Remote window (daemon-backed)
│   ├── persistence.rs   Session save/restore (JSON)
│   ├── osc_scan.rs      OSC 9;4 progress scanning
│   ├── marker_scan.rs   OSC 133 semantic marker scanning
│   ├── agent_*.rs       Agent state detection (detect, hold, osc, screen)
│   ├── tree.rs          Session tree structure
│   └── verb*.rs         Verb protocol (commands + responses)
├── tape/
│   ├── parser.rs        AST generation (replaced separate lexer)
│   ├── command.rs       Command model + serialization
│   ├── executor.rs      Command execution against Os
│   ├── player.rs        Playback engine with progress
│   └── trust.rs         Per-machine trust store (SHA-256 + canonical path)
├── terminal/
│   ├── pty.rs           Unix PTY creation, PTY pool semaphore, reader/writer
│   ├── window.rs        PTY + emulator pairing, geometry
│   └── window_io.rs     DaemonOutputWriter, render coalescer, response reader
├── ui/
│   ├── animation.rs     Cubic easing, transition states
│   ├── overlay.rs       Hint overlay, settings, theme picker
│   └── perf.rs          Performance counters
├── util/
│   ├── guestenv.rs      Guest environment variables
│   ├── linewidth.rs     Unicode line-width calculation
│   └── theme_detect.rs  OSC 11 light/dark theme detection
└── vt/                  VT100/xterm emulator (parser, screen, cells)
    ├── cell.rs          Cell: base char + adaptive combining run (inline+spill)
    ├── emulator.rs      Main/alt screens, scrollback, OSC 52, graphics APC
    ├── parser.rs        VT parser state machine (CSI, OSC, DCS, APC, PM, SOS)
    ├── screen.rs        Screen buffer, cursor, scroll region
    └── scrollback.rs    Scrollback ring buffer
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

## Pixel Canvas

The pixel canvas (`app/pixel_canvas.rs`) maps each terminal cell to a 24-bit
RGB background, creating a low-resolution framebuffer inside the terminal.
Used for:
- Drop shadows on floating panes
- Gradient backgrounds and accent bars
- SDF rounded corners on overlay panels
- Dock sparklines and status visualizations

## Combining Marks

The VT cell model (`vt/cell.rs`) stores an inline combining run (base char +
up to 4 zero-width marks) with an optional heap spill for longer clusters.
This handles Devanagari (virama + matra stack), Hangul jamo, and
decomposed Latin (e + U+0301 → é) without losing data.

## Network Modes

The `network` cargo feature enables SSH and web server modes:

- `--network ssh`: russh SSH server, each connection gets a fresh Os.
  Supports SGR mouse forwarding for selection/copy.
- `--network web`: axum HTTP + WebSocket, serves xterm.js frontend.
  Forwards mouse events (SGR) for in-app selection.
- The `tls` feature adds rustls encryption for both.

## PTY Pool

`Window::spawn` acquires a slot from a global PTY pool semaphore
(capacity 8, timeout 120s) before forking. This provides back-pressure
under load instead of failing with ENXIO/EMFILE. The slot is released
immediately after PTY setup, so daemon pump threads (which hold Arc clones
of the windows map) don't deadlock.

## Tape Scripting

Tapes are reproducible terminal automation scripts. See
[TAPE_SCRIPTING.md](TAPE_SCRIPTING.md) for the language reference.

## Sessions and Daemon

TermOS runs a Unix-socket daemon for session persistence. See
[DAEMON.md](DAEMON.md) for details.

## Testing

- **Unit tests**: `#[cfg(test)]` modules in each source file (2100+ tests).
- **Integration tests**: `tests/` directory — VT conformance, BSP tiling,
  tape parsing, daemon protocol, control surface, proptests.
- **Property tests**: `proptest_vt.rs` — structured token-pool fuzzing,
  grid invariants, selection-text correctness.
- **Fuzz targets**: `fuzz/` — cargo-fuzz targets for the VT emulator.
- **PTY pool guard**: `skip_if_pty_exhausted!` macro checks live PTY
  availability; the pool semaphore serializes spawns under load.
