# Changelog

All notable changes to TermOS are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **Phase 24 — Layout modes: master-stack and scrolling (niri-style).** Three layout modes now available:
  - BSP (default) — binary space partition tiling.
  - Master-Stack — one master pane on the left, rest stacked on the right.
  - Scrolling — niri-style columns on an infinite horizontal strip.
  - Cycle modes with `Prefix+Space` or command palette "Cycle layout mode".
  - Dock shows layout tag (MS/SCR) when not in BSP mode.
  - Scrolling mode: `Alt+Left/Right` shifts focus between columns.
  - Layout cache invalidated on mode switch.
- **Command palette fuzzy search with match highlighting.** Multi-token query support (space-separated terms), fine-grained scoring (prefix > word-boundary > subsequence), bold+yellow highlighted characters, "No matches" empty state, keyboard shortcut hints, recently-used commands sorted first, and mouse click support.
- **Phase 22 — Approachability: welcome screen, mode indicator, key hints bar, mouse-friendly dock.**
  - Welcome overlay on first launch (no config file) with keybinding quick reference.
  - Persistent key-hints bar above dock showing 3-4 contextual keybindings per mode.
  - Dock mode pill now shows prefix type (PREFIX, WS, WIN, MIN, TAPE, DBG, FLOAT).
  - Toggle hints bar with `Ctrl+B H`.
  - Mouse-friendly dock: left-click switches windows, right-click opens context menu, hover tooltips.
  - Config option `[appearance] mouse_friendly = true`.
- **Phase 18 — Pixel canvas: GUI-like visual polish.**
  - Per-cell RGB background framebuffer (`pixel_canvas.rs`) for shadows,
    gradients, and anti-aliased edges.
  - Drop shadows on floating panes.
  - SDF rounded corners on overlay panels.
  - Gradient accent bars below dock.
- **Phase 19 — Interaction reliability & render efficiency.**
  - Continuation-click snapping for wide chars.
  - Selection/scrollback audit.
  - Paint-path wide-char spacing fix.
- **Phase 20 — Render hot path: inline cell content + per-frame palette.**
  - Shadow mask caching.
  - Render coalescer for daemon mode.
  - Dirty-region tracking.
- **Phase 21 — Bug fixes and test hardening.**
  - `select_line_at` inclusive column fix.
  - PTY prompt deadline bump (30s).
  - PTY pool semaphore (capacity 8) for back-pressure.
  - Web/SSH mouse event forwarding (SGR).
  - Daemon Drop closes PTY master fds.
  - Combining marks: adaptive inline+spill (4 inline + heap overflow).
  - Zero-width char fix in VT emulator.
  - Proptest for selection_text phantom spaces.
- **21 built-in themes** with swatch picker.
- **Overlay system** (settings, theme picker, context menus).
- **Floating pane support** (drag, raise, zoom).
- **Dock bar** with workspace pills and session buttons.
- **Agent state notifications** (OSC 9;4 progress, OSC 133 markers).
- **`termos doctor`** health-check subcommand.
- **`termos run`** wrapper for one-shot command execution.
- **VT fuzzing** (cargo-fuzz + structured proptests).
- **Parallel-test stabilization** — PTY pool, SIGSTOP waitpid, deadline-
  polling for daemon I/O tests, ASCII_MODE lock for overlay tests.

### Changed
- **Renamed the crate from `tuios-rs` to `termos`.**
- **Combining marks stored inline** — Cell model extended with base char +
  4 inline marks + optional Arc spill (56→80 bytes). Zero-width chars
  attach to their base instead of being dropped.
- **Wide-char paint spacing** — `paint_emulator` advances buffer column by
  glyph width and marks continuation cells `skip = true`.
- **`selection_text` skips width-0 cells** — no phantom spaces in copied
  text from wide-char content.
- **`select_word_at` column conversion** — word range computed in char
  space, converted to column space for wide chars.
- **Daemon Drop** closes all PTY master fds on shutdown.
- **Config hot-reload** keeps last good config on broken edit.
- **`compute_diff` removed** — dead 660-line wire protocol deleted.

## Release History

The sections below document the development phases of the TermOS Rust port.
Each phase was independently shippable and left the crate building, tested,
and runnable.

### Phase 0 — Project bootstrap

- Set up the Rust crate (`Cargo.toml`, `src/main.rs`, `src/lib.rs`).
- Added core dependencies: `ratatui`, `crossterm`, `nix`, `crossbeam-channel`.
- Established CI workflow (`.github/workflows/ci.yml`) with build, test,
  clippy (`-D warnings`), fmt check, and release builds.

### Phase 1 — VT emulation

- Full ANSI/VT100 parser state machine (`src/vt/`).
- Screen buffer with alternate screen, scrollback, and tab stops.
- CSI sequence handlers for cursor movement, erase, scroll, and SGR.
- OSC handlers including OSC 52 (clipboard) and OSC 8 (hyperlinks).
- Mouse tracking modes (SGR and legacy).
- VT conformance tests (`tests/vt.rs`).
- Benchmarks for write, scroll, and render paths (`benches/`).

### Phase 2 — BSP layout

- Binary space partition tiling algorithm (`src/layout/`).
- Master-stack and scrolling layout modes.
- Window split (horizontal/vertical), close, and focus navigation.
- Workspace support with up to 9 workspaces and `Alt+1-9` switching.
- BSP tiling tests (`tests/bsp.rs`).

### Phase 3 — PTY integration

- PTY management via `nix` (`src/pty/`).
- Shell process spawning with environment and window-size propagation.
- Cross-thread I/O via `crossbeam-channel` reader threads.
- Resize forwarding (`ioctl` `TIOCSWINSZ`).
- Clean PTY teardown on window close.

### Phase 4 — TUI rendering

- Ratatui-based rendering loop (`src/app/`).
- Modal input: window-management mode and terminal mode.
- tmux-style leader key (`Ctrl+B`) with prefix sub-menus.
- Status bar with workspace indicators and mode display.
- Frame-budget-aware refresh (60Hz focused, 30Hz background).

### Phase 5 — Session daemon

- Session model with named, persistent sessions (`src/session/`).
- Unix-socket daemon with length-prefixed JSON protocol.
- Client subcommands: `run`, `attach`, `list`, `kill`.
- Session persistence/restore (JSON snapshots respawned at daemon start).
- Full TUI attach: daemon windows rendered as remote panes.
- Session switcher (`Ctrl+B`, then `S`) with filter and switch.
- Multi-client broadcast via per-session broadcast hub.
- Session tree data model.
- Daemon tests (`tests/daemon.rs`).

### Phase 6 — Scrollback & copy UX

- Scrollbar indicator (1-column thumb, proportional to viewport/total).
- Copy-mode cursor with vim visual selection (`h`/`j`/`k`/`l`, `v`, `y`).
- Yank to OSC 52 (system clipboard) plus internal clipboard slot.
- Mouse drag selection: left-down anchors, drag extends, up finalizes + yanks.
- Tests for selection extraction, yank, and scrollbar geometry.

### Phase 7 — Mouse support

- SGR and legacy mouse tracking modes in the VT emulator.
- Mouse event routing to focused panes and the TUI layer.
- Drag selection and scrollback interaction.
- Configurable mouse enable/disable via `appearance` settings.

### Phase 8 — Hooks

- Lifecycle hooks framework (session start, window spawn/close, mode change).
- TOML-configured hook commands.
- Hook execution with environment context.

### Phase 9 — Agent state

- `--skill` mode for AI agents driving TermOS panes.
- Pane addressing, reading, and writing primitives.
- Agent state reporting and condition waiting.
- Skill documentation embedded in the binary.

### Phase 10 — Tape scripting

- Tape lexer and parser (`src/tape/`).
- Command format ported from the Go `internal/tape` package.
- Record a window's output to a tape; replay and seek.
- Secure per-machine trust store with SHA-256 hashing.
- Tape parsing tests against all Go example tapes
  (`tests/tape_parse_examples.rs`).

### Phase 11 — Graphics passthrough

- Kitty graphics protocol (APC passthrough, placement, queries, z-index).
- Sixel forwarding to cell grids (no rasterization).
- Image protocol configuration and interplay with scrollback and selection.

### Phase 12 — Network modes

- SSH server mode via `russh` (optional, behind `network` feature).
- Web terminal mode via `axum` + WebSocket (optional, `network` feature).
- TLS support via `rustls` (optional, `tls` feature).
- Host key management and authentication for SSH.
- WebSocket I/O forwarding for web terminal clients.

### Phase 13 — Testing & debugging

- 240+ tests across unit, integration, and conformance suites.
- Fuzzing harnesses for the VT parser (`fuzz/`).
- Benchmarks for VT write, scroll, and render.
- `--debug` logging via `env_logger`.
- CI matrix covering default and `network` feature builds.

### Phase 14 — Rename to TermOS

- Renamed crate, binary, and library from `tuios-rs` to `termos`.
- Updated all documentation, CI, and packaging references.
- Retained attribution to the upstream Go project
  [TUIOS](https://github.com/Gaurav-Gosain/tuios).
