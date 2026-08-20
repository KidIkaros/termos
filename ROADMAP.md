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

- ✅ Pane type `command`: a window running `sh -c <command>` via the palette
  command "New command pane…" (text dialog). The border shows the exit
  status once the child is reaped (`[exit N]`, `waitpid(WNOHANG)` polled per
  frame via `Window::poll_exit`), Enter re-runs a finished pane
  (`Window::restart` respawns the same command), and `start_suspended`
  holds the child with a self-`SIGSTOP` before exec — shown as
  `⏸ … [Enter to run]` — until the first Enter (`SIGCONT`). Enter works
  from both WM and terminal mode.
- 🟡 Layout/template support: `LayoutWindow` gained a `suspended` field
  (forward-compatible). Templates already store per-window `command`, but
  applying a template still types commands into shells (tape script);
  applying a template as real command panes is deferred to a later pass.
- ✅ Tests: exit-code capture + re-run and suspended-start (window/pty level),
  plus Os-level spawn/dialog/rerun/resume tests. Also fixed a latent
  hang: `PtyHandle::Drop` blocked forever on a SIGSTOPped child (SIGHUP
  stays pending while stopped); it now sends SIGCONT before reaping.

## Phase 14 — Stacked panes & multi-pane bulk ops (Tier 3)

Zellij layers panes on top of each other (stacked panes, navigate with
arrows, dynamic resize) and supports multi-select bulk operations
(Alt+click drag, bulk close/break/stack).

- ✅ Stacked panes as a layout mode: BSP tree `SplitType::Stacked` with
  `push_to_stack`/`pop_from_stack`/`cycle_stack_focus`/`stack_count`;
  active pane gets full content area, inactive shows as 1-cell tab bar
  with `▶ title` glyph; `StackPane` and `CycleStack` palette commands.
- ✅ Multi-pane select: Alt+click toggles pane selection, `MultiSelect`
  palette command, `select_all_panes`; bulk `BulkClose` (reverse-index
  safe), `BulkStack` (stacks into one group), `BulkBreak` (unstacks);
  checkmark ✓ in title for selected panes; Escape clears selection.
- ✅ Tests: 6 BSP stack tests + 11 Os-level stack/bulk tests; palette
  count assertion relaxed for new commands.

## Phase 15 — Plugin/extension story (Tier 3: long-term)

Zellij's WASM plugin system (status/tab bars, session manager, filepicker as
plugins) is its headline differentiator. A full WASM runtime is a
multi-month project; start with the pragmatic path and document an extension
protocol so a WASM layer can slot in later.

- ✅ Hook-driven external commands for status-line widgets (`status_widgets`
  config: name, command, refresh_ms, alignment; output cached and rendered
  right-aligned in the dock) and custom palette actions (`custom_actions`
  config: name, command, category; dispatched synchronously with TERMOS_*
  env).
- ✅ Document the extension protocol (`docs/EXTENSIONS.md`): hooks, verb
  surface, rendering contract, WASM roadmap.
- ⬜ Re-evaluate WASM plugins only if adoption justifies it.## Phase 16 — Kitty animation protocol (Tier 3: niche)

Explicitly unsupported upstream in TUIOS (`a=f`, `a=a`, `a=c`). Completes
the kitty graphics story (`docs/GRAPHICS.md`).

- ✅ Parse and forward kitty animation frames: `KittyAction::Frame`
  (`a=f`), `::Animate` (`a=A`), `::Compose` (`a=c`), `::Split` (`a=S`);
  `KittyCommand.animation_group` field parsed from `g=` param;
  `AnimationGroup` tracking in `KittyState` with frame list, playing
  state, delay, looping.
- ✅ Interplay with placement tracking: `clear_graphics()` calls
  `clear_groups()` to drop animation frames on pane close / alt-screen.
- ✅ Tests: 4 parser tests + 2 kitty_state animation tests; 18 total
  kitty graphics tests pass.

## Phase 17 — Performance & correctness baselines (Tier 4)

Continuation of Phase 7; these are the moat for a port competing on
emulation quality. Ratatui's full-frame redraw is the known weakness of
ratatui-based multiplexers at scale.

- ✅ Benchmark baselines first (`benches/` exists): render, input, PTY
  throughput benchmarks compile and run via criterion.
- ✅ Dirty-region rendering: `AtomicBool` dirty flag per Window set by
  drain thread on PTY output; `paint_pane` skips `paint_emulator` when
  not dirty, relying on ratatui's `Terminal::draw()` buffer diff for
  unchanged cells.
- ✅ Scrollback reflow under resize: 3 tests verifying long-line reflow,
  viewport preservation, and wider-resize content retention.
- ⬜ Structured fuzzing of the VT parser (fuzz/ targets) with the
  VTE/escape-test conformance suites expanded.

## Phase 18 — Pixel canvas: GUI-like visual polish (Tier 2)

The biggest visual gap between a terminal and a modern GUI is depth
(shadows, elevation) and smoothness (gradients, anti-aliased edges). The
`PixelCanvas` layer maps each terminal cell directly to a 24-bit RGB
background, creating a low-resolution framebuffer inside the terminal.
Combined with ratatui for text, this bridges the gap without leaving the
terminal.

The asciline-rust integration was evaluated during this phase. Direct RGB
cell painting was retained instead: it avoids an unnecessary mapper and
RGB/BGR conversion on the hot path while preserving the visual effects.
See `docs/ASCILINE_INTEGRATION.md` for the analysis.

Architecture: dual-layer rendering
- Layer 1 (RGB pixel canvas): gradient backgrounds, shadow effects,
  anti-aliased shape primitives, and sparklines.
- Layer 2 (ratatui text): content, borders, and widgets rendered on top with
  transparent backgrounds showing the canvas through.

- ✅ Evaluate asciline-rust and select the direct-RGB path for ratatui.
- ✅ `src/app/pixel_canvas.rs`: reusable RGB framebuffer (`Vec<u8>`) sized to
  the terminal area and painted as colored cells.
- ✅ Gradient backgrounds: horizontal/vertical/radial gradients for dock
  bar, title bars, and pane backgrounds.
- ✅ Shadow rendering: Gaussian-falloff colored shadows for floating panes,
  giving depth/elevation feel.
- ⬜ SDF rounded-corner integration for overlays (the primitives exist, but
  current overlays still use ratatui cell borders).
- ✅ Gradient sparklines: smooth colored bar graphs for CPU/RAM widgets
  in the dock.
- ✅ The reusable canvas and render invalidation are integrated in Phase 19.
- ✅ Tests cover canvas creation, gradients, shadows, SDF primitives, and
  interpolation edge cases.

## Phase 19 — Interaction reliability & render efficiency (Tier 1: complete)

The post-Phase-18 dogfood and optimization audit found that the remaining
experience gap was less about feature count and more about keeping interaction
responsive under load. This phase closes the reliability-critical portion of
that work without changing the ratatui full-buffer composition model.

- ✅ Status-widget jobs are bounded and non-blocking: completed workers are
  reaped opportunistically, unfinished jobs never block a UI tick, one refresh
  per widget is allowed at a time, the global worker cap is four, and child
  commands have a ten-second timeout.
- ✅ Retained pane render cache: styled VT rows are rebuilt on PTY output,
  resize, or viewport changes, then composited into the current ratatui buffer
  on every required frame.
- ✅ Invalidation-driven rendering: local, remote, SSH, and web loops skip
  idle terminal draws while still rendering input/output/state changes and
  active animations.
- ✅ Removed the pixel-canvas RGB → BGR → RGB round trip; the reusable RGB
  canvas is painted directly into ratatui cells.
- ✅ Cached layout results across `sync_window_sizes`, rendering, and graphics
  placement, keyed by workspace bounds, gap, and serialized BSP tree.
- ✅ Daemon output coalescing is signal-driven and bounded rather than waking
  periodically while idle; graphics scans are skipped when passthrough is
  inactive.
- ✅ Closed the audited interaction gaps: settings tab stepping, coordinate-
  based accent selection, asynchronous custom actions, and the
  `signal_new_output` dirty contract.
- ✅ Added regression coverage for PTY dirty propagation, persistent pane
  rendering, widget worker bounds, direct-RGB edge cases, idle invalidation,
  and overlay clicks. Existing VT render benchmarks remain the performance
  baseline.

Deferred follow-ups are deliberately narrow: cache stable gradient/shadow
masks and wake every daemon response reader through a dedicated signal. Those
are optimization opportunities for the next performance phase, not blockers
for the interaction-reliability work completed here.

## Phase 20 — Render hot path: inline cell content + per-frame palette

The post-Phase-19 performance audit identified the two per-cell costs that
dominate rendering. Tier 1 removed the structural one (a heap-allocated
`String` per cell); Tier 2 removed the per-cell color-resolution branches.

### Tier 1 — inline cell content

`Cell.content` was a heap-allocated `String` per cell, even though the VT
parser emits exactly one `char` per `print` call. A 207×55 pane snapshot
cloned ~11k `String`s per dirty frame (~355 µs measured).

- ✅ Store cell content inline as `Option<char>` (4 bytes, `Copy`, no
  allocation) instead of `String` (24 bytes + heap per occupied cell).
- ✅ Render snapshots return `(char, Style)` instead of `(String, Style)`,
  removing per-cell string cloning from `render_view_lines`
  (355 µs → 69 µs at 207×55; 118 µs → 13 µs at 80×24).
- ✅ Updated screen/scrollback reflow, daemon diff conversion, copy-mode
  extraction, and paint paths to the inline representation (multi-code-point
  wire content is truncated to its first `char`, matching the existing
  `chars().next()` render behaviour).

### Tier 2 — per-frame style palette

`paint_emulator` converted every cell's style through `to_tui_style`, which
re-resolved `Color::Default`/`Color::Indexed` through an `Option<&Theme>`
and a `match` for each cell. A `StyleCache` built for this purpose was dead
code (and took a `Mutex` lock per lookup).

- ✅ `StylePalette` (`src/ui/mod.rs`) precomputes the 256 indexed-color slots
  and default foreground once per frame; `resolve`/`style` are O(1) lookups
  (`vt_style` benchmark: 24 µs at 207×55, 4 µs at 80×24).
- ✅ Fixed a latent panic: `Color::Indexed(16..=255)` previously indexed the
  16-entry theme ANSI array and would panic; indices now resolve through the
  standard xterm 256-color cube/grayscale.
- ✅ Removed the unused `StyleCache` module (`src/ui/style_cache.rs`) and its
  stale doc; `docs/THEMES.md` now describes the palette resolution.

### Tier 3 — algorithmic

- ✅ `ScreenBuffer::insert_cell`/`delete_cell` now shift cells in place with
  `Vec::drain` + `Vec::splice` instead of cloning the entire row twice.
- ✅ `scroll_up` recycles evicted scrollback buffers as fresh blank rows:
  `push_line_recycle` now reclaims the evicted line's backing `Vec` (cleared)
  rather than allocating a new one, removing a per-line allocation during
  steady-state scrolling.
- ⬜ `compute_diff` in `src/terminal/diff.rs` (and the whole screen-diff wire
  protocol) turned out to have **no production callers** — only its own tests.
  Optimizing it would be wasted effort; it is deferred for either wiring into
  the daemon protocol or removal.

### Tier 4 — pixel canvas caching

- ✅ `PixelCanvas::fill_background` caches the solid fill + accent gradient +
  dock row keyed by the three colors; an unchanged theme memcpys the cached
  RGB buffer instead of recomputing gradients/lerps every frame. Also fixed a
  latent underflow when rendering a 1-row terminal.
- ✅ `drop_shadow` now compares squared distance and computes the Gaussian
  falloff from `dist²`, eliminating a per-cell `sqrt`.
- ✅ `drop_shadow` shadow masks are now cached per `(rect_w, rect_h, radius)`
  (bounded at 64 entries), so the per-cell `exp` runs once per rect size
  instead of every frame; the mask is position-independent and shared across
  frames and float positions.
- ✅ `Emulator::write` now batches runs of printable ASCII straight into the
  screen (`print_ascii_run`), skipping the per-byte parser dispatch.  Both
  paths share the `write_cell` core so they cannot drift; equivalence is
  locked in by fast-vs-slow tests and the VT conformance/proptest suites.
  Measured: plain 1 MiB ingest ~89→94 ms vs ~123 ms for escape-heavy input
  (~25% faster on printable streams).
- ⬜ Assessed & rejected: skipping the background fill under "fully opaque"
  panes.  `StylePalette::style` deliberately omits the bg for `Color::Default`
  cells (`src/ui/mod.rs`), so most pane cells are transparent and the canvas
  shows through — the dual-layer design is load-bearing.  Making those cells
  opaque would break the transparency contract, and the fill is already a
  cached memcpy, so the saving is negligible.

### Follow-up fixes from the daemon-mode dogfood

- ✅ **Config silent-discard bug (major):** any partial `[keybindings]`,
  `[startup]`, `[debug]`, `[tape]`, `[notifications]`, `[appearance.scrollbar]`,
  `[appearance.sidebar]`, or `[daemon]` section made the **entire config**
  silently fall back to defaults (`toml::from_str` failed on missing inner
  fields and `parse_str` swallowed the error).  So a user who set
  `leader = "ctrl-b"` in `[keybindings]` lost their theme, widgets, and every
  other setting.  Fixed: added the `#[serde(default)]` container attribute to
  all of those structs (matching `AppearanceConfig`, which already had it);
  partial sections now merge with defaults.  Regression test:
  `parses_partial_sections_without_discarding`.
- ✅ **`[startup] start_in_terminal_mode` was dead config** — `Os::new`
  hardcoded `Mode::WindowManagement` and the flag was never read, so the TUI
  always launched in window-management mode and the first keystrokes a user
  typed were swallowed as window commands (the "first input drop" seen in
  dogfood).  The mode now honors the flag; regression test
  `start_mode_follows_startup_config`.
- ✅ **Showkeys overlay rendered unconditionally** — `render_showkeys` drew the
  last key chord at the pane's bottom-right on every keypress, ignoring the
  `[debug] show_key_events` opt-in (default off).  Now gated on the flag, the
  debug-prefix `k` toggles it, and `--show-keys` actually enables it.

### Config reload-path audit

- ✅ **Hot-reload keeps the last good config on a broken edit.**  The watcher
  (`UserConfig::watch` → `load_from`) previously fell back to **defaults** on
  any read/parse failure, so a typo while editing the config reset the running
  session's theme and keybindings.  New `try_load_from`/`parse_str_checked`
  report errors; the watcher skips the update (keeps the old config) on
  failure.  Startup `load()` still falls back to defaults for a missing file.
- ✅ **Hot-reload now applies theme changes.**  `Msg::ConfigReloaded` only
  swapped `os.config`; the resolved `os.theme`/`auto_theme` stayed stale until
  restart.  The handler now re-resolves the theme exactly as `Os::new` does.
- ✅ Audited every `Deserialize` struct: all config sections now carry the
  `#[serde(default)]` container attribute (`StatusWidgetConfig`/
  `CustomActionConfig` intentionally remain strict — they are `Vec` elements
  that should be fully specified).  Custom theme files (`ThemeJson`) and tape
  types already report errors properly.

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
| 18 | 2 | Pixel canvas: GUI-like visual polish | Medium (asciline-rust) |
| 19 | 1 | Interaction reliability & render efficiency | Large (complete) |
| 20 | 1 | Render hot path: content, palette, screen ops, canvas cache | Large |
