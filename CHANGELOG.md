# Changelog

All notable changes to TermOS are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Packaging and release infrastructure:
  - `Dockerfile` for containerized builds and deployment.
  - `install.sh` curl|bash installer with OS/arch detection.
  - `.github/workflows/release.yml` for cross-compiled release binaries and
    GitHub Releases on tag push.
- `SECURITY.md` with vulnerability reporting policy and response timeline.
- `CONTRIBUTING.md` with development setup, code style, and PR process.
- `CHANGELOG.md` (this file).

### Changed
- **Renamed the crate from `tuios-rs` to `termos`.** The binary, library, and
  all references now use the `termos` name. The project remains a Rust port
  of the upstream Go project [TUIOS](https://github.com/Gaurav-Gosain/tuios).

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
