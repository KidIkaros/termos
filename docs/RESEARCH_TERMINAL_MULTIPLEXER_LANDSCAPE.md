# Terminal Multiplexer Landscape — Research Report for TermOS

*Generated: 2026-08-19 | Sources: 9 (primary) | Confidence: High for facts; Medium for community sentiment (see Methodology)*

## Executive Summary

TermOS (the Rust port of TUIOS) has reached feature parity with its Go reference and is
technically the most feature-dense multiplexer in the space: BSP tiling, workspaces, kitty/sixel
graphics passthrough, OSC 133 scrollback browser, tape scripting, daemon sessions with
resurrection, SSH/web modes, and an agent (`--skill`) mode. The competitive gaps are not in
emulation or rendering — they are in **interaction models** (floating panes), **automation
surfaces** (scriptable CLI/state), and **collaboration** (read-only sharing, web client hardening),
all of which Zellij and tmux 3.7+/3.8 have shipped and which the market now treats as table stakes.

## 1. Competitive Landscape

| Player | Language | Positioning | Notable strengths |
|---|---|---|---|
| tmux 3.7/3.8 | C | The incumbent; ubiquitous in servers/SSH workflows | Scriptability (control mode, monitors), themes, floating panes (new), stability, ecosystem (tmuxp/tmuxinator) |
| Zellij | Rust | "Terminal workspace with batteries included" | WASM plugin system, floating + stacked panes, command panes, layouts (KDL), session resurrection, web client with auth/share, advanced CLI scriptability |
| WezTerm | Rust | Terminal emulator with built-in multiplexing domains | Unix/SSH/TLS mux domains, native-GUI integration, local-echo latency hiding |
| Ghostty | Zig | Fast GPU terminal emulator (no mux yet) | Massive adoption; exploring plugin/"+" ecosystem; shows emulators absorbing multiplexer features |
| mtm | C | Minimalist (~1k LOC), "finished" | Simplicity and stability as a deliberate niche |
| TUIOS / TermOS | Go / Rust | Window-manager-for-your-terminal | BSP tiling, workspaces, kitty/sixel graphics, tape scripting, daemon+resurrection, SSH/web, agent mode |

**Positioning insight:** the space has split into *conservative-and-scriptable* (tmux), *modern-and-extensible* (Zellij), *emulator-integrated* (WezTerm, Ghostty), and *minimalist* (mtm). TUIOS/TermOS owns a distinct fifth position — **tiling-window-manager UX inside a multiplexer** — which is genuinely differentiated (nobody else has BSP + workspaces + graphics passthrough + tape automation in one tool).

## 2. Feature Gap Analysis (TermOS vs. the Space)

Legend: ✅ TermOS has it · 🟡 Partial · ❌ Missing

### Interaction models
- ❌ **Floating panes** — Zellij's first-class, persistent, pinnable floating panes; tmux added them in 3.7 and heavily expanded them in 3.8 (modal panes, mouse move/resize, drag-to-create, break/join/swap). TermOS's overlay system only floats *UI panels*, not *terminals*. This is the single biggest missing feature.
- 🟡 **Stacked panes** (Zellij) — layering panes on top of each other like tabs. Not in TermOS.
- ❌ **Multiple-pane bulk operations** (Zellij) — multi-select, bulk close/break/stack.
- ✅ Pane zoom, shared borders, mouse resize, preset selection, multifocus, aggregate view.

### Automation & scriptability
- 🟡 **External scriptability** — Zellij exposes a full CLI control surface (`zellij action`, JSON state queries, `zellij subscribe` streaming, `--block-until-exit-success/failure`, ID capture) so any shell script or CI pipeline can drive a session. TermOS has a solid internal daemon verb protocol (`src/session/verb.rs`) but no comparable *public CLI* surface. Given TermOS already has an agent (`--skill`) mode, a scriptable surface would multiply its value.
- ✅ **Command panes (adjacent)** — Zellij's command panes (exit codes, Enter to re-run, `start_suspended`) are unique; TermOS's tape scripting is the closest analogue and arguably stronger for replay, but tape is *recording*-oriented, not *interactive-run* oriented.
- ✅ Tape scripting/recording, layout templates, hooks (lifecycle events).

### Sessions & collaboration
- ✅ Daemon sessions, multi-client attach, session switcher, resurrection/persistence, layout templates, SSH server mode, web mode.
- ❌ **Read-only / shareable access** — Zellij: read-only tokens, HTTPS attach from another terminal, bookmarked URLs, pair-programming. TermOS web mode is unauthenticated single-owner; no read-only observer support.
- ❌ **Web client hardening** — Zellij's web client ships auth + persistent URLs; TermOS's is a bare xterm.js frontend.
- ✅ Multi-client broadcast (though Zellij/tmux also allow *simultaneous independent* clients; TermOS broadcast model is a reasonable starting point).

### Terminal protocol & UX depth
- ✅ OSC 8 hyperlinks (per-cell), OSC 133 semantic markers + command/output scrollback browser, kitty keyboard protocol, synchronized output, sixel + kitty graphics.
- 🟡 **OSC 133 as hook events** — tmux 3.8 fires `pane-command-started/finished` and `pane-shell-prompt` hooks from OSC 133. TermOS tracks markers for its scrollback browser but doesn't expose them to its hooks system.
- ✅ Vim copy mode, scrollback scrollbar, mouse selection, click-to-open paths.
- 🟡 **Light/dark theme detection** — tmux 3.8 added built-in light/dark themes with terminal-theme detection. TermOS has 21 themes + swatch picker but no auto light/dark switching.

### Extensibility
- ❌ **Plugin system** — Zellij's WASM plugin system (custom status/tab bars, filepicker, session manager as plugins; single-.wasm distribution) is its headline differentiator. TermOS has no extension story beyond config. A full WASM plugin system is a large lift; a lighter scripted-extension path (e.g., external status-line commands, hook-driven plugins) is more realistic.
- ✅ Config hot-reload, custom themes, keybinding customization, hooks.

### Performance & correctness (from TermOS ROADMAP)
- 🚧 Dirty-region rendering, scrollback reflow under resize, benchmark baselines, structured fuzzing, interactive QA — all open roadmap items. Ratatui (immediate-mode, full-frame redraw) is the known weak spot for multiplexer workloads; incremental rendering is the highest-leverage perf work.
- ✅ Event-driven rendering, viewport culling, style caching, kitty ID reuse (zero-idle-CPU design ported from TUIOS).

## 3. Recommendations (ranked by impact ÷ effort)

**Tier 1 — do next (high impact, bounded effort):**

1. **Floating panes.** Zellij and tmux both treat this as core; its absence is the most visible parity gap. TermOS's existing overlay geometry/hit-testing machinery (`src/app/overlay_hit.rs`, `src/app/overlay_mouse.rs`) is reusable infrastructure — floating *terminal* panes can build on it. Ship: toggle, move/resize (mouse + keys), pin/z-order, float⇄tile conversion.
2. **Public scriptable CLI over the daemon verb protocol.** Add `termos action …`-style commands: spawn/close/split panes, send input, list sessions/panes/windows as JSON, stream output (`termos subscribe`), and block-until-exit variants. This unlocks CI pipelines, external tooling, and — directly aligned with the existing `--skill` agent mode — gives AI agents a first-class control channel instead of the current TUI-only interaction.
3. **OSC 133 → hooks.** Wire the existing semantic markers into the hooks system (pane-command-started/finished, pane-shell-prompt), matching tmux 3.8. Cheap, and it makes scrollback-block UX scriptable.

**Tier 2 — competitive parity (moderate effort):**

4. **Web client hardening:** auth (token), read-only observer mode, persistent/bookmarkable session URLs, optional HTTPS. Enables the pair-programming/demo use cases Zellij sells.
5. **Read-only attach over SSH/web** for observers.
6. **Light/dark theme detection** (report terminal theme via CSI queries and switch theme sets).
7. **Command panes** (run, show exit code, Enter to re-run, `start_suspended`) — a lightweight automation win distinct from tape recording.

**Tier 3 — differentiators & long-term:**

8. **Stacked panes** (Zellij) as a layout mode for task-focused workspace organization.
9. **Multiple-pane bulk select** (Alt+click drag, bulk close/break/stack).
10. **Plugin story.** Full WASM plugins (Zellij) is a multi-month project. Pragmatic path: hook-driven external commands for status-line/widgets first; document an extension protocol; revisit WASM later if adoption warrants.
11. **Kitty animation protocol** (a=f/a=a/a=c) — explicitly unsupported upstream; niche but completes the kitty graphics story.

**Tier 4 — polish (already on roadmap):**

12. Dirty-region rendering and scrollback reflow under resize are the right perf targets; benchmark baselines first so the incremental renderer has something to prove against.
13. Structured fuzzing of the VT parser and expanded VTE/escape-test conformance (roadmap) — this is the correctness moat that a port competing on emulation quality needs.

## 4. Key Takeaways

- **TermOS's moat is its combination, not any single feature**: tiling-WM UX + graphics passthrough + tape automation + daemon/agent support. No competitor overlaps all four. Lean into that identity rather than chasing tmux's breadth.
- **The two features the market now expects that TermOS lacks are floating panes and external scriptability.** Both are achievable with existing infrastructure (overlay system; daemon verb protocol).
- **Automation is the fastest-growing axis** (Zellij's CLI surface, tmux control mode, AI agents in terminals). TermOS's `--skill` mode is ahead of the curve — a public scriptable/agent API turns a marketing bullet into a platform capability.
- **Collaboration (read-only sharing, web auth) is an underserved quadrant**; Zellij is the only incumbent selling it, and TermOS's web/SSH modes are a head start.
- **Performance work on the roadmap (dirty regions, reflow) is the right call** — ratatui's full-frame redraw is the known weakness of ratatui-based multiplexers at scale.

## 5. Sources

1. [TUIOS GitHub](https://github.com/Gaurav-Gosain/tuios) — feature list, v0.7.0 changelog, architecture (the Go reference TermOS ports).
2. [Zellij Features](https://zellij.dev/features/) — floating/stacked panes, command panes, layouts, web client, plugin system, CLI scriptability.
3. [Zellij Roadmap](https://zellij.dev/roadmap/) — direction of the most active modern competitor.
4. [Ratatui](https://ratatui.rs/) — 5,300+ crates, 22.3k stars; version 0.30.2 current (TermOS pins 0.29 per AGENTS.md).
5. [tmux Getting Started wiki](https://github.com/tmux/tmux/wiki/Getting-Started) — session/window/pane model, prefix keys, status line.
6. [tmux CHANGES (3.6→3.8)](https://raw.githubusercontent.com/tmux/tmux/master/CHANGES) — floating panes (3.7, expanded 3.8), themes/light-dark detection, control-mode monitors, OSC 133 hook events, scrollbar auto-hide, multi-format modifiers.
7. [WezTerm Multiplexing docs](https://wezterm.org/multiplexing.html) — Unix/SSH/TLS mux domains, local-echo latency hiding.
8. [Ghostty Docs](https://ghostty.org/docs) — fast GPU emulator; emulator-side feature absorption.
9. [mtm GitHub](https://github.com/deadpixi/mtm) — minimalist counterpoint (~1k LOC, deliberately "finished").

## 6. Methodology

- **Sub-questions investigated:** (1) What is TUIOS and what does its Rust port (TermOS) actually ship today? (2) What do the leading multiplexers (tmux, Zellij) offer that TermOS does not? (3) What is the state of the Rust TUI ecosystem TermOS is built on? (4) What features are trending/expected in the space (plugins, scripting, web/collab, AI)? (5) What performance/correctness work matters for a multiplexer?
- **Approach:** primary-source fetches of official docs, READMEs, and changelogs for TUIOS, Zellij, tmux, WezTerm, Ghostty, mtm, and Ratatui; code inspection of TermOS (`src/app/overlay_*`, `src/vt/semantic_markers.rs`, `src/session/verb.rs`, ROADMAP.md) to verify parity claims.
- **Limitation:** the search-engine backend was unavailable during this session, so no third-party comparisons or community sentiment (HN/Reddit threads, blog posts) could be gathered. All claims rest on first-party documentation, which raises confidence on *feature existence* and lowers confidence on *how much users value* each feature. Sentiment-gathering via search should be re-run to validate the priority ordering. Secondary gap: Zellij's roadmap page returned only a shell (linked issues not extractable), and tmux 3.8 changelog details were read from the master CHANGES file (pre-release section).
