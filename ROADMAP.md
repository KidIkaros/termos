# TermOS Rust port — roadmap

Everything not yet shipped, organized into phases by dependency, value, and
risk. Each phase is independently shippable and leaves the crate building,
tested, and runnable. Phases are ordered so later work builds on primitives
from earlier phases.

Legend: ✅ done · 🚧 in progress · ⬜ not started

> **Phases 1–6 are complete.** Phase 7 (hardening & interactive QA) is in
> progress; the parity campaign (see plan-26f58d39a48a4a71.md) continues
> with the message-pump architecture, app overlays, sidebar, session/daemon
> completion, and the remaining subsystems.

## Phase 1 — Scrollback & copy UX (self-contained, high value)

No new architecture; builds on the emulator's existing scrollback + viewport.

- ✅ Scrollbar indicator: a 1-column thumb on the right edge of a scrolled-back
  pane, proportional to viewport/total, respecting `appearance.hide_scrollbar`.
- ✅ Copy-mode cursor + vim visual selection: a content-anchored cursor in
  scrollback mode (`h`/`j`/`k`/`l`, `v` to select, `y` to yank).
- ✅ Yank → OSC 52 to the host terminal (system clipboard), plus an internal
  clipboard slot.
- ✅ Mouse drag selection: left-down anchors, drag extends, up finalizes + yanks.
- ✅ Tests for selection extraction, yank, and scrollbar geometry.

## Phase 2 — Daemon/attach + session model (architectural foundation)

Introduces the session abstraction that the remaining network/graphics phases
build on. The daemon owns PTYs; clients run their own emulator/renderer and
exchange raw bytes (see `docs/DAEMON.md`).

- ✅ Session model: named, persistent sessions with attach/detach lifecycle
  (`session::model`, `session::manager`, name validation).
- ✅ Daemon process + client protocol: Unix socket, length-prefixed JSON
  frames, `tuios` run/attach/list/kill subcommands (`session::daemon`,
  `session::client`, `session::protocol`).
- ✅ Session persistence/restore: sessions saved as JSON and respawned at
  daemon start (`session::persistence`).
- ✅ Full TUI attach: `attach`/`run` open the multiplexer UI in daemon mode.
  Every daemon window becomes a `Window::remote` pane (input/resize forwarded,
  output routed back per-window), with window create/close/split over the
  control protocol.
- ✅ Session switcher: `Ctrl+B` then `S` lists sessions in daemon mode, with
  filter + switch (`pending_switch`) and `Ctrl+D` kill (`pending_kill`).
- ✅ Multi-client broadcast: a session's PTY output fans out to every attached
  client via a per-session broadcast hub (see `docs/DAEMON.md`).
- ✅ Session tree data model (`session::tree`, ported from `internal/sessiontree`).

## Phase 3 — Hooks

- ✅ Lifecycle hooks (session start, window spawn/close, mode change) running
  user-configured commands; port `internal/hooks`.

## Phase 4 — Tape scripting

- ✅ Record a window's output to a tape; replay/seek; port `internal/tape`
  (parser + command format).

## Phase 5 — Graphics passthrough

- ✅ Kitty graphics protocol (APC passthrough, placement/queries, z-index).
- ✅ Sixel decoding to cell grids.
- ✅ Image protocol config + interplay with scrollback and selection.

## Phase 6 — Network modes

- ✅ SSH server mode (attach a session over SSH).
  - `src/network/ssh.rs`: russh server with terminal handle, render loop,
    SSH input parsing (CSI sequences, modifier keys, function keys),
    kitty/sixel detection, session picker.
  - Client sessions get fresh `Os` with independent windows/workspaces.
  - Render loop: Os state → `CrosstermBackend<Vec<u8>>` → `TerminalHandle` → SSH channel.
  - Input loop: SSH channel bytes → `parse_ssh_input()` → `handle_key()`.
- ✅ Web server mode (render + forward I/O over WebSocket).
  - `src/network/web.rs`: axum HTTP server, tokio-tungstenite WebSocket.
  - Static HTML page with xterm.js frontend (`src/network/web/index.html`).
  - Render loop: Os state → `CrosstermBackend<Vec<u8>>` → ANSI → JSON frames → WebSocket.
  - Input loop: WebSocket JSON frames → `parse_web_input()` → `handle_key()`.
  - Supports resize events from the frontend.
  - 7 unit tests for web input parsing (ctrl+letter, arrows, function keys, etc.).

## Phase 7 — Hardening & interactive QA

- 🚧 Interactive QA in a real terminal across the palette, switcher, scrollback,
  selection, and mouse flows. (Palette/theme picker exercised; the structured
  scrollback browser and switcher overlays are part of the parity campaign.)
- 🚧 VT conformance expansion (VTE/escape-test suites), fuzzing the parser.
  (Conformance tests exist; structured fuzzing targets are part of the parity
  campaign.)
- 🚧 Performance: frame-budget tuning, dirty-region rendering, scrollback
  reflow under resize. (Frame-budget loop in place; dirty-region rendering and
  benchmark baselines are part of the parity campaign.)
