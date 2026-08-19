# TermOS Rust port — roadmap

The Rust port of TUIOS has reached feature-surface parity with the Go
reference. All 14 phases of the parity campaign are complete.

Legend: ✅ done · 🚧 in progress · ⬜ not started

> **All phases 0–14 are complete.** The parity campaign
> (plan-26f58d39a48a4a71.md) delivered: message-pump architecture, app
> overlays, sidebar, session/daemon completion, VT completion (CSI/ESC/OSC/C1/
> DA/DSR/DECRQM/kitty keyboard), web/mobile (auto-TLS, touch, transport
> security), SSH server (caps, session picker, HostCapabilities), CLI parity
> (config edit/reset, resurrect, session/window/layout commands), testing
> (proptests for VT/BSP/config, daemon multi-client, protocol handlers), and
> documentation.
>
> **Test suite**: 1,360+ tests passing, clippy `-D warnings` clean, release
> build OK across default, network, and TLS feature sets.
>
> **Phases 8+** are the post-parity roadmap derived from the competitive
> research in `docs/RESEARCH_TERMINAL_MULTIPLEXER_LANDSCAPE.md`. The gaps are
> interaction models (floating panes), automation surfaces (scriptable CLI),
> and collaboration (web hardening, read-only sharing). Ordering favors
> impact ÷ effort; phases 8–10 are Tier 1 (do next), 11–13 Tier 2, 14–16
> Tier 3, 17 Tier 4 (continuation of Phase 7).

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

## Phase 8 — Floating panes (Tier 1: biggest parity gap)

Zellij ships first-class floating panes; tmux added them in 3.7 and expanded
them through 3.8 (modal panes, mouse move/resize, drag-to-create,
float⇄tile). TermOS's overlay system floats UI panels but not terminals —
this is the most visible missing feature in the space.

Reuse the overlay machinery already in place: geometry recording and
hit-testing (`src/app/overlay_hit.rs`), mouse routing (`src/app/overlay_mouse.rs`),
and the borderless panel renderer (`src/ui/overlay.rs`).

- ✅ Floating terminal panes: toggle on/off, persistent while hidden, live
  process continues (Zellij parity).
- ✅ Move + resize with mouse drag (title row moves, border edges resize)
  and keys (`Ctrl+B F` prefix: `h/j/k/l` move, `H/J/K/L` resize); z-order
  raise on focus and click.
- ✅ Tiled ⇄ floating conversion: `Ctrl+B F f` floats the focused window or
  tiles it back; `t` tiles; `n` spawns a new floating shell (works in
  daemon mode too via `pending_float`).
- ✅ Modal pane variant (tmux 3.8 `-O`): `Ctrl+B F o` toggles modal on the
  focused float; while active it blocks focus cycling and clicks on every
  other pane (⛔ badge in the title) until released or the pane is closed.
- ✅ Pin / always-on-top per float: `Ctrl+B F p` pins the focused float
  (📌 badge); pinned floats stay above unpinned ones through raises and
  win hit-testing (z-order sort key is `(pinned, z)`).
- ✅ Rendering: floating panes composite above the tiled layout and the
  border grid; floats are scoped to their workspace (hidden on other
  workspaces) and included in image-placement geometry.
- ✅ Zoom: floats zoom to the workspace and back; zooming a tiled window now
  hides floats (tmux parity — only the zoomed pane shows) until unzoom,
  and hidden floats are unreachable by mouse/cycle.
- ✅ Tests: geometry, z-order hit-testing (pinned preference), keyboard/
  mouse move+resize, focus cycling through floats+tiles, modal blocking
  (focus + clicks), zoom auto-hide round-trip, window removal index
  shifting, workspace moves (35 tests).

## Phase 9 — Public scriptable control surface (Tier 1: automation axis)

Zellij exposes a full CLI control surface (`zellij action`, JSON state
queries, `zellij subscribe` streaming, `--block-until-exit-success/failure`,
ID capture); tmux has control mode + monitors. TermOS already has an internal
daemon verb protocol (`src/session/verb.rs`, `list_verbs`) and an agent
(`--skill`) mode — this phase makes both first-class and external.

- ✅ Public CLI verbs: `termos action <verb> [key=value ...]` calls any verb
  of the protocol (`new-session`, `new-window`, `close-window`, `send-text`,
  `capture-pane`, `wait-for`, `kill-session`, ...) over the socket; the
  existing named commands (`send-keys`, `capture-pane`, `wait-for`,
  `new-window`, `get-window`, `run-command`) now reach the daemon correctly
  (fixed a pre-existing `args[2..]` off-by-one that panicked every
  zero-argument CLI command and swallowed the first flag).
- ✅ Structured state queries: `termos ls --json` (sessions), `termos ls
  --json -W` (sessions + per-window geometry via `list-windows`),
  `session-info`, `get-window`, per-pane exit status (tracked daemon-side).
- ✅ Output streaming: `termos subscribe [-s S] [-w W] [--json]` tails a
  pane's raw output over a long-lived verb connection (`{data}` chunks + a
  final `{closed}`), with daemon-side ring replay (`output_since`).
- ✅ Blocking variants: `termos block-until-exit [-s S] [-w W]
  [--success|--failure] [--timeout ms]` — exit statuses are recorded by the
  PTY pump at EOF (signals as negative codes, close as -1) and surfaced with
  process exit 0/1/2 for scripts.
- ✅ ID capture: `new-window`/`new-session` return the full entity info
  (id, geometry, workspace) — `termos action new-window session=ci` prints
  `{"window":{"id":"w1",...}}` for script targeting.
- ✅ Docs: `docs/CONTROL_SURFACE.md` — socket, verb table with parameters,
  error envelope codes, the scripted workflow, and CI / retry-loop /
  agent-driver examples.
- ✅ Tests: `tests/control_surface.rs` drives a real daemon over a temp
  socket end-to-end (new-session → new-window ID → send/capture/wait →
  subscribe stream → exit code 7 → timeout and not-found error envelopes);
  also fixed `VerbHint` deserialization (`#[serde(default)]`) so client-side
  parsing survives sparse hints.

## Phase 10 — OSC 133 hook events (Tier 1: cheap, high value)

tmux 3.8 fires `pane-command-started`, `pane-command-finished`, and
`pane-shell-prompt` hooks from OSC 133. TermOS already tracks the semantic
markers (`src/vt/semantic_markers.rs`) for its scrollback browser
(`src/scrollback/browser.rs`) — surface them to the hooks system.

- ✅ New hook events: `pane-command-started`, `pane-command-finished`
  (carrying exit status), `pane-shell-prompt` (`src/hooks/mod.rs`).
- ✅ Hook payloads include the window/pane target and exit code: the daemon's
  PTY pump parses OSC 133 from the raw stream
  (`src/session/marker_scan.rs`) and fires with window id/name, workspace and
  `TERMOS_EXIT_CODE` (see `src/hooks/mod.rs` payload conventions).
- ✅ Tests: marker stream → hook firing with correct payloads (scanner unit
  tests + a pump-level hook-firing test).

## Phase 11 — Web client hardening + read-only observers (Tier 2)

Zellij's web client ships auth, persistent/bookmarkable session URLs,
read-only tokens, and HTTPS attach. TermOS's web mode
(`src/network/web.rs`) is an unauthenticated single-owner frontend — this
phase covers the collaboration quadrant.

- ✅ Token auth for the web server (`--network web --token …`); no auth when
  bound to localhost only. The terminal page forwards the token onto the
  WebSocket upgrade URL so the login flow connects end-to-end.
- ✅ Read-only observer mode: viewers see output and scrollback but cannot
  send input (covers SSH too — read-only attach). Input is dropped at the
  per-client `Os` gate.
- ✅ Persistent session URLs (e.g., `http://host:port/<session>`), with a
  session picker page when none is specified. The web server attaches to
  daemon sessions: `/` lists sessions + a new-session form, `/new` creates
  one and redirects, `/ws/<session>` streams it (daemon auto-started by the
  web command).
- ✅ Optional HTTPS via existing auto-TLS machinery from the parity campaign
  (explicit certs, auto-TLS with persisted self-signed cert, plaintext
  refused off localhost).
- ✅ Tests: auth rejection (HTTP + WS), read-only enforcement (input frames
  dropped), picker listing/creation, daemon-attach E2E typing over WS.

## Phase 12 — Light/dark theme detection (Tier 2)

tmux 3.8 added built-in light/dark themes with terminal-theme detection.
TermOS has 21 themes + a swatch picker (`src/config/theme.rs`) but no auto
switching.

- ✅ Query the host terminal's light/dark preference (OSC 11 query with a
  `COLORFGBG` env fallback) and expose it in config as `theme = "auto"`
  (`src/util/theme_detect.rs`). Which themes `auto` picks is configurable via
  `theme_auto_dark` / `theme_auto_light` (defaults: catppuccin-mocha/latte).
- ✅ Respect the setting dynamically: the TUI re-queries the live terminal
  after entering raw mode at startup, and the palette command
  "Re-detect light/dark theme" re-runs detection on demand.
- ✅ Tests: OSC 11 parsing (xterm/hex/ST/BEL/fragmented), luminance
  classification, COLORFGBG parsing, auto resolution fallbacks, config
  backward-compat, and Os-level auto resolution + redetect behavior.
- ✅ End-to-end: `tests/theme_detect_osc.rs` runs the real binary in a PTY,
  answers the startup query light and the re-detect query dark (palette
  driven via real keystrokes), and asserts the dockbar swaps
  latte → mocha with the "dark detected" notification.

## Phase 13 — Command panes (Tier 2: lightweight automation)

Zellij's command panes treat commands as first-class pane citizens: show the
exit code, re-run with Enter, `start_suspended` for on-demand execution.
Tape scripting covers *recording/replay*; command panes cover *interactive
run* — complementary.

- ⬜ Pane type `command`: shows exit status after completion; Enter re-runs;
  `start_suspended` waits for manual trigger.
- ⬜ Layout/template support: command panes in saved layouts with
  `start_suspended` semantics.
- ⬜ Tests: exit-code capture, re-run, suspended start.

## Phase 14 — Stacked panes & multi-pane bulk ops (Tier 3)

Zellij layers panes on top of each other (stacked panes, navigate with
arrows, dynamic resize) and supports multi-select bulk operations
(Alt+click drag, bulk close/break/stack).

- ⬜ Stacked panes as a layout mode: layer multiple panes per stack cell,
  arrow navigation, per-stack resize.
- ⬜ Multi-pane select: Alt+click toggle / drag rectangle; bulk close, break
  to new window, stack selected, move focus through selection.
- ⬜ Tests: stacking state transitions, bulk op effects.

## Phase 15 — Plugin/extension story (Tier 3: long-term)

Zellij's WASM plugin system (status/tab bars, session manager, filepicker as
plugins) is its headline differentiator. A full WASM runtime is a
multi-month project; start with the pragmatic path and document an extension
protocol so a WASM layer can slot in later.

- ⬜ Hook-driven external commands for status-line widgets and custom
  actions (run a binary, feed its output into the UI).
- ⬜ Document the extension protocol (`docs/EXTENSIONS.md`): hooks, verb
  surface, rendering contract.
- ⬜ Re-evaluate WASM plugins only if adoption justifies it.

## Phase 16 — Kitty animation protocol (Tier 3: niche)

Explicitly unsupported upstream in TUIOS (`a=f`, `a=a`, `a=c`). Completes the
kitty graphics story (`docs/GRAPHICS.md`).

- ⬜ Parse and forward kitty animation frames (APC `a=` transmissions).
- ⬜ Interplay with placement tracking, scrollback, and zoom.
- ⬜ Tests: frame sequences, id reuse, cleanup on pane close.

## Phase 17 — Performance & correctness baselines (Tier 4)

Continuation of Phase 7; these are the moat for a port competing on
emulation quality. Ratatui's full-frame redraw is the known weakness of
ratatui-based multiplexers at scale.

- ⬜ Benchmark baselines first (`benches/` exists): render, input, PTY
  throughput, attach latency — so the incremental renderer has targets.
- ⬜ Dirty-region rendering: emit only changed cells/lines per frame
  (respecting ratatui's backend diff where possible).
- ⬜ Scrollback reflow under resize: reflow tests across width changes.
- ⬜ Structured fuzzing of the VT parser (fuzz/ targets) with the
  VTE/escape-test conformance suites expanded.

## Priorities at a glance

| Phase | Tier | Theme | Effort |
|---|---|---|---|
| 8 | 1 | Floating panes | Medium (reuse overlay machinery) |
| 9 | 1 | Scriptable CLI / control surface | Medium (wrap existing verb protocol) |
| 10 | 1 | OSC 133 hook events | Small |
| 11 | 2 | Web hardening + read-only observers | Medium |
| 12 | 2 | Light/dark theme detection | Small |
| 13 | 2 | Command panes | Small–Medium |
| 14 | 3 | Stacked panes + bulk ops | Large |
| 15 | 3 | Plugin/extension story | Large (WASM: very large) |
| 16 | 3 | Kitty animation protocol | Small |
| 17 | 4 | Perf baselines, dirty regions, reflow, fuzzing | Large (ongoing) |
