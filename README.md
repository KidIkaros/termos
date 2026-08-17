# TUIOS — Rust port

A Rust port of [TUIOS](https://github.com/Gaurav-Gosain/tuios) — the terminal
multiplexer and window manager with a vim-like modal interface, BSP tiling,
workspaces, and scrollback.

This crate mirrors the architecture of the Go project module-for-module:

| Go package              | Rust module            | Contents |
|-------------------------|------------------------|----------|
| `internal/vt`           | `tuios::vt`            | VT emulator (ANSI parser, screen, scrollback, SGR/CSI/OSC/ESC) |
| `internal/layout`       | `tuios::layout`        | BSP tree, master-stack tiling, scrolling columns |
| `internal/terminal`     | `tuios::terminal`      | PTY management (nix-based) and the terminal window |
| `internal/config`       | `tuios::config`        | TOML user config, keybindings, themes |
| `internal/theme`        | `tuios::config::theme` | Built-in themes + ANSI color conversion |
| `internal/app`          | `tuios::app`           | Window manager (OS state, workspaces, modes, input, render) |

## Status

This is a **working core** of the port — a standalone binary that runs inside
your existing terminal, spawns shell sessions in panes, and manages them with
vim-like controls. The port is built on `ratatui` + `crossterm` (the Rust
counterparts of the Charm stack the Go project uses) and `nix` for PTY
management.

### Implemented

- **VT emulator** — a from-scratch ANSI state-machine parser (Ground/Escape/
  CSI/OSC/DCS/UTF-8) with a main + alternate screen, scrollback, SGR (truecolor,
  256-color, bold/dim/italic/underline/reverse/blink/strike), cursor movement,
  insert/delete line/cell, scrolling regions (DECSTBM), alt-screen switching,
  OSC 0/2 title, OSC 7 cwd, OSC 52 clipboard, and DA/DSR responses.
- **Layout engine** — the BSP tree ported faithfully from `bsp.go`: spiral/
  alternate/longest-side/smart-split auto schemes, preselection, resize with
  min-extent clamping, ratio sync from geometry, rotate/swap/equalize,
  separator collection, and serialization. Plus master-stack tiling and
  niri-style scrolling columns.
- **PTY layer** — the same kernel path the workspace's GPU terminal uses
  (`posix_openpt` → `grantpt`/`unlockpt` → fork → `execvp`), with a poll-based
  reader thread, backpressure via the `reading` flag, and resize.
- **Window manager** — workspaces 1-9 with per-workspace BSP trees, window
  focus, next/prev, workspace switching (Alt+1..9), move-and-follow
  (Alt+Shift+1..9), and window create/close/split.
- **Modal input** — window-management mode vs terminal mode, the Ctrl+B leader
  key with sub-prefixes (workspace/window/minimize), and key encoding for PTY
  passthrough.
- **Config** — TOML user config with the default keybinding tables, built-in
  themes (Catppuccin, Dracula), and XDG config loading.
- **Rendering** — a ratatui compositor that paints each pane's emulator screen,
  draws focused/unfocused borders, and renders a dock bar.
- **Command palette** — `Ctrl+B` then `P` opens a fuzzy-filtered command list
  (new/close window, split, next/prev, switch workspace, scrollback, quit);
  type to filter, arrows to move, Enter to run.
- **Switcher** — `Ctrl+B` then `W` lists workspaces; `Ctrl+B` then `S` lists
  every window across workspaces (local mode) or lists daemon sessions (remote
  mode, with `Ctrl+D` to kill); type to filter and Enter to jump/switch.
- **Which-key popup** — after the leader or a sub-prefix, the available
  keybindings are shown in a centered overlay (configurable via
  `appearance.which_key_enabled`).
- **Scrollback + copy mode** — mouse-wheel up/down scrolls any pane back into
  its scrollback (implicit copy mode); `Ctrl+B` then `[` enters vim-like
  scrollback navigation with a content-anchored cursor (h/j/k/l, PgUp/PgDn,
  g/G, `v` visual select, `y` yank, q/Esc).
- **Scrollbar** — a proportional 1-column thumb on the right edge of any
  scrolled-back pane (respects `appearance.hide_scrollbar`).
- **Selection & copy** — rectangular text selection via vim visual mode or
  mouse drag; yank writes the text to the host terminal clipboard via OSC 52
  and to an internal clipboard slot.
- **Mouse** — left-click focuses the pane under the cursor; drag selects text;
  wheel scrolls scrollback, or is forwarded to mouse-tracking apps (vim, htop,
  less) in terminal mode via SGR-1006 encoding.

### Not yet ported (next sessions)

Kitty graphics passthrough, sixel, tape scripting, hooks, and SSH/web server
modes. The module structure and the core primitives they depend on are already
in place.

## Build & test

```bash
cargo build --release
cargo test          # VT conformance, BSP layout, parser, PTY, palette, switcher, selection
```

## Run

```bash
cargo run --release
```

The binary takes over the terminal (alternate screen). Keybindings:

| Key | Action |
|-----|--------|
| `i` / `Enter` | Enter terminal mode (type into the focused shell) |
| `Esc` (window mode) | Back to window management |
| `Ctrl+B` then `c` | New window |
| `Ctrl+B` then `-` / `\|` | Split horizontal / vertical |
| `Ctrl+B` then `n` / `p` | Next / previous window |
| `Ctrl+B` then `w` then `1-9` | Switch workspace |
| `Ctrl+B` then `W` | Workspace switcher |
| `Ctrl+B` then `S` | Window switcher |
| `Ctrl+B` then `P` | Command palette |
| `Ctrl+B` then `[` | Scrollback mode |
| `Ctrl+B` then `q` | Quit (with confirmation) |
| `Alt+1..9` | Switch workspace directly |
| `Alt+Shift+1..9` | Move focused window to workspace |
| `Alt+Esc` | Leave terminal mode |
| Mouse wheel | Scroll pane scrollback (or forward to app) |
| Mouse left-click | Focus pane under cursor |
| Mouse drag | Select text (yanks on release) |

## Daemon & sessions

A persistent-session daemon owns each session's shells; clients attach and
run their own emulator/renderer (see `docs/DAEMON.md`).

```bash
cargo run --release -- daemon        # run the daemon in the foreground
cargo run --release -- list          # list sessions
cargo run --release -- run myname    # start daemon, create/attach a session
cargo run --release -- attach myname # attach to an existing session
cargo run --release -- kill myname   # kill a session
```

Sessions survive detach and are restored (respawned) when the daemon restarts.
`run` and `attach` open the full multiplexer TUI: daemon windows become real
panes, input is forwarded to the daemon, and `Ctrl+B` then `S` switches
sessions. Multiple clients may attach to the same session and all receive its
PTY output (multi-client broadcast).

## Architecture notes

The Go project's pointer-graph BSP tree is reimplemented as an arena (`Vec` of
nodes) so Rust's borrow checker can verify the tree invariants. Node indices
replace raw pointers, and `window_to_node: HashMap<i32, usize>` keeps the O(1)
window→node lookup.

The emulator is single-threaded per window: the PTY reader thread drains output
into a `Mutex<Emulator>`, and the renderer locks it per frame to paint. This
mirrors the Go code's window `ioMu` discipline without the cross-goroutine
atomic-mode caches (the lock already serializes access).

## License

MIT — ported from the MIT-licensed TUIOS project.
