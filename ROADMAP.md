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
- ✅ Structured fuzzing of the VT parser: token-pool proptests
  (`structured_token_stream_keeps_grid_invariant`, CJK-wide streams),
  6 new corpus seeds (wide CJK, alt-screen, OSC hyperlink, edit ops,
  zero-width regression), plus the VTE conformance suites.

  **Finding:** the fuzz surfaced a real bug — zero-width characters
  (combining marks like U+0301) were written as occupied width-0 cells,
  violating the grid invariant and being overwritten by the next char.
  The fix went further than dropping: `Cell` now stores an inline
  `combining: [char; 4]` run on the base cell, `write_cell` attaches
  zero-width marks to it (walking past wide continuations to the lead), and
  every consumer (render, copy, selection, scrollback) emits the full
  grapheme — `e` + U+0301 renders as `é` in one terminal cell. Capacity is
  adaptive: 4 marks inline (covers every real script — Devanagari
  virama+matra stacks, Hangul jamo, polytonic Greek, Vietnamese — per
  measured `unicode-width` values), then a shared `Arc<Vec<char>>` spill with
  no ceiling for pathological linguistic stacks. The inline budget keeps the
  render hot path allocation-free; the spill is shared (not copied) across
  scrollback line clones. The grid invariant asserts combining runs only ride
  on occupied bases and that spills only exist past the inline budget.
  Locked in by 7 unit tests, two `paint_emulator` symbol tests, the
  strengthened proptest, and the fuzz seeds.

  **Live dogfood follow-up (wide-char paint bug):** rendering decomposed
  Devanagari/Hangul in tmux exposed a pre-existing alignment bug —
  `paint_emulator` laid out `StyledChar` columns consecutively, but ratatui's
  buffer diff treats a wide symbol as occupying two terminal columns and
  skips the next buffer cell. Every wide glyph shifted subsequent content
  left by one column and ate the following character (`你你XX` rendered as
  `你 X` with the pane border drifting). Fixed by carrying the glyph width on
  `StyledChar`, advancing the buffer column by it, and marking continuation
  cells `skip = true` (ratatui's sanctioned mechanism, cleared every frame by
  `Cell::reset`). Verified live: `你你XX` = 6 columns, `|한|` = 4, the
  Devanagari 3-mark stack and 5-mark spill each occupy one column with their
  trailing markers intact.  Locked in by
  `paint_emulator_spaces_wide_glyphs_and_trailing_text` and
  `paint_emulator_spaces_hangul_jamo_run`.

  **Wide-char audit (selection + scrollback):** `paint_selection` maps
  selection columns (emulator/content space) 1:1 to buffer positions, which
  is correct because wide leads sit at their emulator column; continuation
  cells are `skip`, so the highlight lands on the glyph leads and trailing
  text without drift. Scrollback view rows flow through the same
  width-carrying `row_to_styled` → `paint_emulator` chain as live rows, and
  the cursor lands one column past the last char after a wide run. Copy text
  extraction (`selection_text`) was already width- and grapheme-aware.
  Locked in by `paint_selection_reverses_wide_lead_cells_at_correct_columns`
  and `paint_scrollback_rows_keep_wide_spacing`.

  **Continuation-click snapping:** mouse clicks on a wide glyph's second
  column previously anchored the selection at the *next* column, so a click
  on the right half of `你` selected the char after it. `content_position_at`
  now walks the content line's cell widths and snaps any click inside a
  wide cell's span to its lead column (clicks beyond the content keep their
  raw column). This fixes begin/extend mouse selection, word select, and
  line select uniformly; copy mode navigates by emulator columns and was
  already safe. Locked in by
  `mouse_click_on_wide_continuation_snaps_to_lead`.

  **Word-select column conversion:** `select_word_at` computed the word
  range in text-char space but stored it as columns, so double-clicking a
  CJK run truncated the selection (`你你XX` → `你你X`). It now converts
  the char range to column space (wide runes count 2) and stores the end
  column inclusively, matching `selection_text`'s semantics. Locked in by
  `word_select_on_wide_run_selects_full_word`.

  **Phantom-space copy fix:** `selection_text` visited every cell and
  pushed `' '` for width-0 wide continuations, so copying `你你XX` yielded
  `你 你 XX` (phantom spaces) — the other extractors (`line_text`, screen
  rows) index by column and skip continuations correctly. It now skips
  width-0 cells the same way. Locked in by
  `mouse_drag_yanks_clean_wide_text` and
  `mouse_drag_starting_on_wide_continuation_yanks_full_word` (the latter
  also proves a drag starting on a continuation snaps to the lead and
  copies cleanly).

  **Network-path verification:** the web/SSH clients render through the
  shared `render()` → `paint_emulator` path and forward key events only
  (no mouse), and daemon-attached windows are `Window::remote` emulators
  fed raw PTY bytes through a channel + `drain_thread` — the exact same
  `Emulator` type, so every wide/combining/selection fix applies
  identically. Locked in by
  `remote_window_emulator_path_handles_wide_and_combining`, which drives
  the channel-fed remote path end to end (clean selection text, combining
  mark riding its base, no width-0 occupied cells). Also fixed a pre-
  existing parallel-test race on the global `SHADOW_MASK_CACHE` (two tests
  asserted its length while the other inserted 68 entries) surfaced under
  `--features network`; the cache-asserting tests are now serialized.

  **Parallel-test stabilization:** eliminated four classes of flaky tests
  under `--test-threads=12`:
  1. PTY-spawning tests (`skip_if_pty_exhausted!`): all 16 tests serialized
     via a global `PTY_POOL` semaphore (capacity 8) — the machine runs near
     its PTY ceiling and concurrent shell spawns blow fixed deadlines.
     `Window::spawn` now acquires a pool slot before fork, blocking instead
     of failing; the slot is released after the PTY is fully set up, so
     daemon pump threads (which hold Arc clones of windows) don't deadlock.
     The old serialized test lock was removed; the pool provides graduated
     back-pressure instead of binary serialization.
  2. Suspended-child race (`suspended_command_pane_waits_for_trigger`): the
     parent now calls `waitpid(WUNTRACED)` so the child's `SIGSTOP` is
     guaranteed observable before the hint text is written.
  3. Daemon I/O timing (`render_coalescer_fires_signal`,
     `daemon_response_reader_drains_responses`, `writer_writes_output_...`):
     replaced fixed-sleep-then-assert with deadline-polling via a
     `wait_emulator` helper that tolerates thread starvation.
  4. Global-state races (`truncate_long_string`, `ellipsis_ascii`,
     `sigil_mark_ascii`, `dash_rule_basic`, `rule_basic`): serialized the
     `ASCII_MODE` toggle tests with a static mutex.
  Result: 8/8 clean runs at 12-thread parallelism, all integration suites
  green, network feature stable x3.

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
- ✅ SDF rounded-corner integration for overlays: `render_overlay` blends
  each corner cell toward the content behind it via
  `pixel_canvas::rounded_corner_alpha` and drops the square corner glyph,
  so overlays read as rounded panels. Regression test
  `rounded_corner_alpha_edges` documents the corner shape.
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

Deferred follow-ups from this phase: shadow-mask caching, a printable-run
write fast path, and a dedicated daemon response-reader stop signal are now
done (perf pass + the follow-up sweep). The forwarder select loop uses a
crossbeam `select!` stop channel instead of polling, and shadow masks are
cached per `(w, h, radius)` (bounded at 64 entries).

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
- ✅ **Removed** the screen-diff wire protocol (`src/terminal/diff.rs`,
  ~660 lines including tests): `compute_diff`/`apply_screen_diff`/
  `serialize_diff`/`DiffCell` had **no production callers**.  The daemon
  streams raw `PtyOutput` and every client (TUI, web, SSH) runs its own
  emulator, so the protocol was a Go-era relic.  Recoverable from git if a
  future server-side-emulation path ever needs it.

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
  that should be fully specified).  Custom theme files (`ThemeJson`) and tape types already report errors properly.

## Phase 21 — Bug fixes and test hardening (Tier 1)

### select_line_at column-space hazard

- ✅ **`cursor_col` stores inclusive end column.** `select_line_at` stored
  `cursor_col: width` (exclusive), while `selection_text` treats the end
  column as inclusive. For lines ending with a wide char this overflowed
  by one column. Now `cursor_col: (width - 1).max(0)` — inclusive,
  consistent with every other Selection producer.
- ✅ `select_line_at_wide_chars_yanks_full_line` regression test.

### PTY prompt timing flakes

- ✅ **Deadlines bumped to30s.** The three remaining flaky tests that
  spawn `/bin/bash` and wait for a prompt (`pty_writes_spaces_and_enter_
  execute_command`, `terminal_mode_forwards_key_input_to_focused_pty`,
  `terminal_output_is_visible_after_an_initial_render`) now use30s
  deadlines. On a box near its PTY ceiling with12 parallel test threads,
  bash startup can exceed15s.

### selection_text coverage

- ✅ **Proptest: `selection_text_never_has_phantom_wide_spaces`.** Writes
  wide CJK + combining marks + ASCII, selects across both lines, and
  asserts no phantom spaces from continuation cells, no lost combining
  marks, and no space before combining marks.
- ✅ **`select_line_at_wide_chars_yanks_full_line`** — triple-click on a
  line containing wide chars verifies the inclusive cursor_col covers the
  full line.

### Daemon pump thread leak

- ✅ **`Daemon::drop` closes all PTY master fds.** Without this, pump threads
  hold `Arc::clone(&self.windows)` and keep `LiveWindow`s (and their
  `PtyHandle`s) alive indefinitely after the Daemon is dropped — orphaning
  PTYs until the shell exits.  `Drop` now locks the windows map and calls
  `handle.close()` on each, causing reader threads to exit (EOF), channels
  to close, and pump threads to drain.
- ✅ `LiveWindow._handle` → `LiveWindow.handle` (`pub(crate)`) so Drop can
  reach it.

### Web/SSH mouse support

- ✅ **Web client forwards mouse events.** xterm.js `onBinary` sends SGR
  mouse sequences as base64-encoded JSON frames; the server parses them
  via `parse_sgr_mouse` and feeds `Msg::Mouse` to the Os.
- ✅ **SSH client forwards mouse events.** The `data()` handler tries SGR
  mouse parsing first; falls back to key-event parsing if it doesn't match.
- ✅ `parse_sgr_mouse` handles SGR button codes (left/middle/right,
  scroll, drag, motion) and converts to crossterm `MouseEvent`.

### Config watcher (dead code removed)

- ✅ The `ConfigWatcher` polling module was dead code — the actual hot-reload
  uses `UserConfig::watch` which already uses the `notify` crate (event-
  driven, not polling).  No changes needed.

### cert/key unused variable warnings

- ✅ `#[allow(unused_variables)]` on `run_web_server` suppresses the
  `cert`/`key` warnings when the `tls` feature is disabled.

## Phase 22 — Approachability (Tier 1)

Research-driven UX improvements to make TermOS less intimidating for casual
terminal users and GUI power users.  Based on `docs/RESEARCH_TUI_APPROACHABILITY.md`
(compare Zellij, Bubbletea, Spotify Player, lazygit).

### Welcome overlay on first launch

- ✅ **`show_welcome` flag on `Os`.** Set to `true` only when no config file
  exists (first launch).  Dismissed by any keypress.
- ✅ **`render_welcome_overlay`** shows a centered overlay with:
  - Greeting + mode explanation
  - 5 keybindings (new window, jump, settings, help, WM mode)
  - "Press any key to dismiss" + wizard hint
- ✅ Any keypress in `handle_key` dismisses the overlay before other
  routing.

### Mode indicator in dock

- ✅ **Dock mode pill now shows prefix type** when a prefix is active
  (PREFIX, WS, WIN, MIN, TAPE, DBG, FLOAT) instead of the generic
  keyboard icon.  Users always see what mode they're in.

### Persistent key-hints bar

- ✅ **1-row hints bar** above the dock, showing 3-4 contextual keybindings:
  - Terminal mode: `Ctrl+B:prefix  ?:help  Esc:WM mode`
  - WM mode: `i:terminal  q:quit  H/J/K/L:swap  ?:help`
  - Prefix mode: `c:new  x:close  ,:settings  ?:all cmds`
  - Scrollback mode: `v:select  y:yank  /:search  q:leave`
- ✅ **Toggle with `Ctrl+B H`.** `hints_visible` field on `Os`.
- ✅ Hints bar hidden when welcome overlay is active (no overlap).

### Mouse-friendly dock

- ✅ **Left-click on dock pill switches window.** `dock_item_at` hit-tests
  visible dock pills by computing layout on demand.
- ✅ **Right-click on dock pill opens context menu** for that window.
- ✅ **Hover tooltips on dock pills** show window title + agent state.
- ✅ **Config option** `[appearance] mouse_friendly = true` (default on).
  Set to `false` to disable dock mouse interaction.
- ✅ **Unit tests** for `dock_item_at` edge cases.

## Phase 28 — CPU compositor architecture and Liquid Glass-ready rendering 🚧

### Destination

Deliver a portable, CPU-only TermOS cell compositor based on the techniques in
`asciline-rust`, while preserving Ratatui text/widgets and all terminal,
SSH, and web clients. The result should make the terminal frontend feel more
like a modern layered UI without pretending that terminal cells can provide
true desktop backdrop blur.

### Scope and decisions

- The compositor targets terminal-cell output first: RGB cell surfaces,
  gradients, shadows, rounded surfaces, sparklines, frame caching, and damage
  tracking.
- `asciline-rust` is the reference and possible focused dependency for
  row-parallel mapping and encoding. We will not import its ffmpeg, video
  player, WebSocket server, or ASC2 container unless a separate product need
  justifies them.
- No GPU is required. CPU/Rayon performance is the baseline and must be
  measured on machines with and without many cores.
- Ratatui remains responsible for terminal text, widgets, and compatibility.
  A native GPU GUI is explicitly out of scope for this phase and can consume
  the later renderer-neutral scene boundary.

### Milestones

1. ✅ **Baseline and API audit** — `benches/compositor.rs` covers PixelCanvas,
   damage flush, cache hit/miss, and asciline head-to-head at 80×24 through
   480×135.
2. ✅ **Compositor seam** — `CellCompositor` trait in `pixel_canvas.rs` with
   RGB, surface, shadow, and `DamageRect` types. `PixelCanvasCompositor`
   adapter drops in with no visual change.
3. ✅ **Scene/effect ownership** — background fills, gradients, shadows,
   rounded-corner blending, and sparklines all flow through the trait.
   Ratatui paints text on top.
4. ✅ **Asciline prototype backend** — `AscilineCompositor` behind
   `asciline-compositor` feature flag; uses `map_ascii` for palette-character
   rendering. Config option `[appearance] renderer = "asciline"`.
5. ✅ **Benchmark decision** — direct RGB wins at all measured sizes
   (9.1 µs vs 72.2 µs at 80×24); asciline only competitive at 480×135+
   with Rayon. Direct RGB retained as default.
6. 🚧 **Dogfood and hardening** — test top/bottom/hidden docks, floats,
   overlays, wide and combining glyphs, SSH/web output, small terminals,
   resize churn, and reduced-core environments. Asciline off by default.

### Acceptance criteria

- The existing terminal UI remains pixel-equivalent or intentionally
  documented where the new compositor differs.
- No GPU, ffmpeg, or desktop runtime is required for the default build.
- The compositor adds no unbounded per-frame allocations and does not regress
  the current render benchmark by more than 5% on the baseline workload.
- CPU-only output is deterministic across Rayon thread counts.
- ANSI output remains compatible with local terminals, SSH, and web/remote
  clients.
- A benchmark report records when the asciline backend wins, ties, or loses;
  retaining the direct implementation is an acceptable result.

### Initial benchmark finding

The first mapper-only run was informative but not yet a backend decision. The
initial direct-RGB benchmark measured a no-op/read loop rather than a real
frame copy, so it understated the direct path. After correcting it to use a
real `copy_from_slice`, the asciline mapper remained a separate BGR transform
rather than an obvious win: any advantage at larger frames must be weighed
against the required channel-layout conversion and final Ratatui flush. The
benchmark therefore remains an experiment, not a reason to make asciline the
default.

### Refined implementation plan

The target is **GPU-like visual quality at CPU cost**, not literal GPU
throughput. A terminal frame contains tens of thousands of cells rather than
millions of desktop pixels, so the winning strategy is to avoid recomputing
unchanged cells and reserve parallel CPU work for large dirty regions.

- ✅ Add `DamageSet`: merge dirty rectangles and track reasons such as PTY
  output, float movement, overlay changes, dock/theme changes, and resize.
  Moving a float damages both its old and new bounds plus its shadow margin.
  Wired into Os, main loop, web loop, SSH loop. Bounds seeded on init.
- ✅ Add retained compositor surfaces keyed by `CanvasCacheKey` (bg, accent,
  dock, position, dims, floats hash). Background + shadow effects skipped
  when key matches; restore is a memcpy.
- ✅ Keep Ratatui's existing pane render cache and window dirty flags as the
  text-layer baseline; compositor damage must not replace semantic VT dirty
  tracking.
- ✅ Change the compositor contract toward `compose_into(scene, damage)`:
  `CellCompositor` trait gains `compose_into(&mut self, scene, damage)` which
  handles cache key computation, background/shadow effects, and cache
  restore. `Scene` struct captures bg, accent, dock, position, floats, and
  color capability. `render()` builds a `Scene` and calls `compose_into` +
  `flush_compositor_to_buffer` instead of manually calling
  `fill_background`/`drop_shadow`/`commit_cache`.
- ⬜ Add an asciline RGB-output mapper or channel-order parameter upstream/local
  adapter. Do not pay a BGR→RGB pass when the final target is RGB.
- ⬜ Use scalar loops below a measured cell threshold and Rayon above it. The
  threshold must be configurable for benchmarks and deterministic output must
  hold across worker counts.
- ⬜ Replace floating-point hot-path blending with cached fixed-point blend,
  gradient, SDF, and shadow tables where measurements justify it.
- ✅ Benchmark damage ratios of 1%, 10%, 50%, and 100%, including effects,
  mapping, Ratatui flush, and ANSI serialization. Full pipeline benchmarks
  in `benches/compositor.rs` measure cache hit/miss × damage level.
- ✅ Add capability tiers: `ColorCapability` enum (TrueColor/Indexed256/Ansi)
  detected from `COLORTERM`/`TERM` env vars. `quantize_256()` degrades
  RGB values to the xterm-256 cube or nearest ANSI16 basic color. Stored
  on `Os`, passed through `Scene`, applied in `compose_into`.

### Backend decision gate

Adopt an asciline-backed path only if it improves the full pipeline at the
workloads TermOS actually sees, does not regress small terminals, preserves
visual equivalence, and remains deterministic. Otherwise retain direct RGB as
the default and keep the asciline adapter as an optional large-frame or
high-rate path.

### Risks and explicit non-goals

- Terminal-cell RGB is not true alpha compositing: “glass” is simulated with
  gradients, contrast, shadows, and controlled translucency.
- CPU parallelism can cost more than it saves for small terminal sizes; the
  backend must avoid Rayon overhead on tiny frames.
- ANSI transport and terminal repaint may dominate mapper time, so mapper-only
  wins are insufficient.
- True desktop blur, subpixel window geometry, native accessibility, and
  high-DPI GUI surfaces belong to a future graphical frontend.


TermOS currently renders through ratatui/crossterm. That remains the correct
terminal-native backend: it preserves SSH, web, tmux, Kitty/Sixel passthrough,
and the existing low-dependency distribution. However, a terminal cannot
provide true iOS-style Liquid Glass because the terminal owns the final
compositor; blur of arbitrary desktop content, translucent window materials,
subpixel positioning, native titlebars, and GPU composition are outside the
cell protocol.

The goal of this phase is **not** an immediate ratatui rewrite. It is to make
TermOS renderer-neutral enough to add a graphical client later while keeping
the terminal client stable.

### Target architecture

```text
                         ┌── Ratatui + crossterm ── terminal / SSH
Os + VT + sessions ─────┤
   shared UI snapshot    ├── xterm.js / WebSocket ── web client
                         └── GPU GUI frontend ───── desktop Liquid Glass
```

- ✅ Research decision: retain ratatui for terminal text/widgets, but treat
  the existing CPU-only asciline-inspired `PixelCanvas` as the seed of a
  TermOS cell compositor. The repository's asciline-rust renderer uses Rust +
  Rayon CPU parallelism; it does not require a GPU. Electron is viable but
  unnecessarily heavy for a Rust-first app. GPUI/Slint/Tauri/wgpu remain
  options only for a later native desktop client. See the renderer comparison
  below.
- ⬜ Define a renderer-neutral `UiSnapshot`/`RenderState` containing pane
  geometry, modes, dock/session state, overlays, selections, theme tokens,
  graphics placements, and input hit regions. It must contain no ratatui,
  crossterm, or terminal buffer types.
- ⬜ Extract input intent from terminal events: map keyboard/mouse events into
  shared commands while retaining terminal-specific escape encoding at the
  Ratatui boundary.
- ⬜ Add a snapshot adapter from `Os` without changing the current renderer.
- ⬜ Build a headless snapshot test fixture covering panes, floats, overlays,
  dock positions, zoom, selections, and theme changes.
- ⬜ Extract a `CellCompositor` seam around the existing `PixelCanvas`; use
  asciline's CPU mapper/encoding techniques where they improve throughput,
  without importing its video server, ffmpeg pipeline, WebSocket server, or
  container format.
- ⬜ Prototype one graphical client against the snapshot boundary. Compare
  GPUI, Slint, and Tauri on startup cost, binary size, accessibility, IME/
  clipboard support, GPU effects, packaging, and maintenance burden before
  committing to a production frontend.
- ⬜ Implement Liquid Glass materials in the selected GUI backend: blur,
  translucent surfaces, elevation/shadows, rounded clipping, animated
  transitions, high-DPI scaling, keyboard/mouse/touch input, and native
  window integration.
- ⬜ Keep terminal/web/SSH clients as supported first-class frontends; the GUI
  client is additive, not a replacement for remote attach.

### Renderer comparison

| Backend | True blur/materials | Rust fit | Weight | Decision |
|---|---:|---:|---:|---|
| Ratatui + custom RGB canvas | No; terminal-cell limited | Excellent | Very low | Keep for terminal mode |
| Crossterm directly | No; more cell control | Excellent | Very low | Not a Liquid Glass solution |
| Electron + web UI | Yes | Low/medium | High | Use only if web ecosystem wins |
| Tauri + web UI | Yes | Good | Low/medium | Practical fallback |
| GPUI | Yes, native GPU | Excellent | Medium | Primary prototype candidate |
| Slint | Yes, declarative GPU | Excellent | Low/medium | Lightweight alternative |
| Iced | Partial/custom work | Excellent | Medium | Secondary Rust option |
| `wgpu` custom renderer | Yes, maximum control | Excellent | Variable/high | Long-term escape hatch |

### Explicit non-goals

- Replacing ratatui before a measurable limitation exists.
- Making terminal mode depend on a desktop compositor or GPU.
- Promising true blur or anti-aliased text inside SSH/tmux terminals.
- Forking the VT/session/input model for each frontend.

### Acceptance criteria

- Terminal mode remains behaviorally compatible and passes the existing test
  and network suites.
- A renderer-neutral snapshot can represent every current visible UI state.
- A prototype GUI can render panes, dock, overlays, and theme changes without
  importing ratatui.
- The selected GUI backend demonstrates real translucent surfaces and blur on
  at least one supported desktop platform.
- The decision is documented with measured startup, memory, binary-size, and
  interaction/accessibility results.

## Renderer decision notes

The primary product constraint is the compositor boundary. Ratatui is a
cell-oriented compositor and is not the reason terminal text looks old;
terminals receive cells and ANSI attributes, then perform final glyph
rasterization. Asciline-rust is a valuable CPU-only renderer foundation for
that cell-level layer: it can map and encode frames quickly without a GPU.
A desktop GUI backend is still additive architecture, not an in-place
renderer swap. The existing `PixelCanvas` can evolve into a TermOS
`CellCompositor`, but neither it nor asciline can provide true desktop
backdrop blur inside a terminal.

Recommended sequence: **snapshot boundary → headless tests → GPUI/Slint
prototype → measured decision → production GUI backend**, while retaining
Ratatui for terminal/SSH and xterm.js for web.

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
| 21 | 1 | Bug fixes: select_line_at column hazard, daemon test flakes, web mouse support, config docs | Medium |
| 22 | 1 | Approachability: welcome screen, mode indicator, key hints bar | Small |
| 23 | 1 | Command palette: fuzzy search, match highlighting, empty state, keybinding hints, recency, mouse clicks | Small |
| 24 | 1 | Layout modes: master-stack + scrolling (niri-style) | Large |
| 25 | 1 | Vim copy mode: hjkl, word motions, character search, visual selection | Large |
| 26 | 1 | Dock polish: configurable position, session controls, zoom indicator, minimized entries | Medium |
| 27 | 1 | Border styles: 9 styles (rounded, thick, double, hidden, block, ascii, etc.) | Medium |
| 28 | 2 | Zen mode: hide borders on idle, reveal on mouse | Small |
| 29 | 2 | Interactive scrollbar: click/drag right border thumb | Medium |
| 30 | 2 | Aggregate view with content previews | Medium |
| 31 | 2 | Multifocus: broadcast typing to multiple panes | Large |
| 32 | 3 | Kitty graphics: image rendering, flicker-free video, SHM | Large |
| 33 | 3 | Mouse enhancements: double-click word, triple-click line, edge snapping | Medium |
| 34 | 4 | Control protocol: JSON API for external automation | Medium |
| 35 | 4 | Layout export: convert layouts to tape scripts | Small |
| 36 | 2 | In-pane Video & Media Player (asciline-rust) | Medium–Large |

## Tier 1 — TUIOS Parity (Phases 24–27)

The goal: close the biggest functional gaps between TermOS and TUIOS.

### Phase 24 — Layout modes: master-stack + scrolling ✅

TermOS currently only supports BSP tiling. TUIOS offers three layout modes:
BSP, master-stack, and scrolling (niri-style columns). This phase adds the
missing two.

#### Master-stack layout

- **`LayoutMode::MasterStack`** enum variant alongside `BSP`.
- One master pane on the left (configurable width ratio), rest stacked on the
  right. Master pane gets focus by default.
- `Prefix+M` to toggle master-stack mode. `Prefix+T` cycles BSP → MasterStack
  → Scrolling → BSP.
- Resize master width with `Prefix+,` / `Prefix+.` (like TUIOS).
- Reuses existing BSP tree for window tracking; master-stack is a layout
  *renderer*, not a separate data structure.
- Config: `[appearance] layout_mode = "master-stack"` (default: `"bsp"`).

#### Scrolling layout (niri-style)

- **`LayoutMode::Scrolling`** enum variant.
- Columns on an infinite horizontal strip. One window per column. Focused
  column centered. Adjacent columns peek from edges.
- `Alt+Left` / `Alt+Right` to shift focus between columns.
- Column width cycles: `Prefix+]` toggles narrow/medium/wide/full.
- New window opens to the right of focused column.
- Close a column → neighbors shift to fill the gap.
- Horizontal scroll with mouse wheel (reversible via config).

#### Shared changes

- `Os::layout_mode()` returns current mode.
- `Os::set_layout_mode()` switches + retiles.
- `Os::tile_windows()` dispatches to the active layout's tiler.
- `render_dock` shows layout mode indicator (BSP/MS/SCR).
- `Command::LayoutSwitcher` cycles modes.
- Hints bar updates per layout mode.

#### Testing

- Unit tests for master-stack ratio calculation.
- Unit tests for scrolling column positioning.
- Integration test: switch modes mid-session, verify windows retile.

### Phase 25 — Vim copy mode ✅

Full vim-style scrollback navigation matching TUIOS's copy mode.

- ✅ **Enter**: `Prefix+[` (or `Ctrl+B [`).
- ✅ **Navigation**: `h/j/k/l`, `w/b/e`, `0/^/$`, `gg/G`, `{/}`.
- ✅ **Count prefix**: `10j` moves 10 lines and repeated word motions honor
  counts; standalone `0` remains the line-start command.
- ✅ **Character search**: `f{char}`, `F{char}`, `t{char}`, `T{char}`, with
  `;`/`,` repeat.
- ✅ **Visual line mode**: `V` highlights entire lines; `v` is char-wise.
- ✅ **Search**: `/` and `?` enter search, `n/N` move between matches.
- ✅ **Yank**: `y` copies selection to the internal and host clipboard.
- ✅ **Exit**: `q` or `Esc` returns to window management mode.
- ✅ **Scroll indicator**: viewport state is shown in the copy-mode dock and
  pane scrollbar.
- ✅ **Regression tests**: count dispatch, `gg`, standalone `0`, visual
  selection, wide-character selection, word/character motions, search, and
  yank behavior.

### Phase 26 — Dock polish 🚧

- ✅ **Configurable position**: `[appearance] dockbar_position = "bottom"`
  (top/bottom/hidden), including matching workspace bounds and mouse hit rows.
- ✅ **Session controls**: detach, close/kill, and rename buttons are rendered
  when the dock has sufficient width.
- ✅ **Zoom indicator**: focused zoomed panes show a `Z` badge in the dock mode
  pill.- ✅ **Minimized entries**: `Window.minimized` field, `minimize_focused()`/`restore_window()`/`restore_last_minimized()` methods, `Prefix+M m` minimizes focused window, `Prefix+M r` restores last minimized, number keys restore by index. `get_dock_items()` returns minimized windows as clickable dock icons; dock click restores them. Minimized windows excluded from tiling layout. 5 unit tests.
- ✅ **Overflow**: the `+N` truncation indicator has shared hit geometry and
  opens the existing aggregate view on click; top/bottom dock placement is
  respected.

### Phase 27 — Border styles 🚧

- ✅ **9 styles/aliases**: rounded (default), normal/single, thick, double,
  plain, hidden/none, block, ascii, outer-half-block, and inner-half-block.
- ✅ **Config**: `[appearance] border_style = "rounded"`, generated config
  comments, wizard choices, CLI completion, and validation all share the
  supported names.
- ✅ **Hidden mode**: suppresses border glyphs, titles, and scrollbars.
- ✅ **Configurable colors**: `border_focused_color` and
  `border_unfocused_color` hex overrides.- ✅ **Dogfood remaining visual differences**: all 9 border styles rendered and verified via `border_type` tests. `Plain`/quadrant types are terminal-cell approximations — this is inherent to the cell protocol and documented.

### Phase 36 — In-pane Video & Media Player (asciline-rust) 🚧

Leverage the `asciline-rust` engine to enable native half-block video and media playback inside TermOS terminal panes without GPU dependencies.

- ⬜ **`PaneKind::Video` enum variant**: add dedicated video pane support alongside standard PTY terminal panes.
- ⬜ **Frame decoder pipeline**: integrate `asciline-rust` frame reader (ffmpeg pipe -> RGB24 raw frame buffer stream).
- ⬜ **Asciline Mapper integration**: convert RGB24 frames to half-block (`▀` / `▄`) 2× vertical resolution ratatui cell grids.
- ⬜ **Playback controls**: pause/play (`Space`), seek backward/forward (`Left`/`Right`, `h`/`l`), volume adjust (`Up`/`Down`, `k`/`j`), and loop toggle (`r`).
- ⬜ **Audio sync & framerate cap**: sync frame rendering to audio clock / target 30–60 FPS with frame dropping on slow terminals.
- ⬜ **Command / CLI launch**: `termos play <video_file>` or `Prefix+V` to open a video pane.
