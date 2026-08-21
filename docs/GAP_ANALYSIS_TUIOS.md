# Gap Analysis: TermOS vs TUIOS

*Generated: 2026-08-20 | Source: TUIOS v0.7.0 | Confidence: High*

## Executive Summary

TermOS has the core architecture — BSP tiling, workspaces, daemon mode, command palette, dock, sidebar, context menus, zoom, floating panes, and event-driven rendering. The **foundations match**. The gaps are in **UX polish, visual sophistication, and advanced navigation modes** — the things that make TUIOS feel like a desktop window manager rather than a terminal multiplexer.

## The TUIOS "Feel" — What Makes It Special

TUIOS feels modern because of three things:

1. **Every interaction has visual feedback** — animations, smooth transitions, hover states, mode indicators
2. **Multiple navigation paradigms coexist** — keyboard-first but mouse-friendly, with discovery aids
3. **The chrome is configurable and hides itself** — zen mode, borderless, minimized entries, workspace pills

## Gap Analysis by Feature Area

### 1. 🔴 Dock & Status Bar — Visual Polish

| Feature | TUIOS | TermOS | Gap |
|---------|-------|--------|-----|
| Dock position | Configurable (top/bottom/hidden) | Bottom only | Missing `dockbar_position` config |
| Workspace pills | Draggable reordering, `+` tab, overflow arrows, scroll strip | Static numbered tabs | No drag reorder, no scroll strip |
| Session buttons | Detach, kill, attach from dock | Not in dock | Missing dock session controls |
| Dock overflow | Truncated count with aggregate-view popup | Truncated with "…" | No overflow click handler |
| Minimized window entries | Clickable minimized icons in dock | Not visible | Missing minimized entry display |
| Dock stats | Optional clock, CPU, RAM | Clock only | Missing CPU/RAM widgets |
| Mode pill | Color-coded chip with type label (Terminal/Window/Prefix/Copy) | Basic mode label | Less visual distinction |
| Z indicator | Shows "Z" when a pane is zoomed | Not shown | Missing zoom indicator |

**Impact:** High. The dock is what users see 100% of the time.

### 2. 🔴 Layout Modes — Tiling Variety

| Feature | TUIOS | TermOS | Gap |
|---------|-------|--------|-----|
| BSP tiling | ✅ Spiral layout | ✅ BSP tree | Match |
| Master-stack | ✅ Dedicated mode | ❌ Not available | Missing layout mode |
| Scrolling (niri-style) | ✅ Infinite horizontal strip | ❌ Not available | Missing layout mode |
| Layout switcher | `Prefix+Space` cycles modes | Only BSP toggle | Missing mode cycling |
| Shared borders | `--shared-borders` thin separators | Full box-drawing borders | Different visual style |
| Preselection | Alt+h/j/k/l to control next spawn position | BSP preselect exists | Mostly matched |
| Equalize splits | `Prefix+=` resets to 50/50 | Not available | Missing |
| Rotate split | `Prefix+R` rotates split direction | Not available | Missing |
| Smart auto-split | Aspect-ratio-aware splitting | Basic BSP split | Less intelligent |
| Layout templates | Save/load/export with CLI | Save/load only | Missing export |

**Impact:** High. Power users expect layout variety.

### 3. 🔴 Scrollback & Copy Mode — Interaction Depth

| Feature | TUIOS | TermOS | Gap |
|---------|-------|--------|-----|
| Vim-style copy mode | `Prefix+[`, hjkl, w/b/e, 0/^/$, gg/G | Basic scrollback | Missing vim motions |
| Count prefix | `10j` moves 10 lines | Not available | Missing |
| Character search | `f{char}`, `F{char}`, `t{char}`, `T{char}` | Not available | Missing |
| Visual line mode | `Shift+V` highlights line | Not available | Missing |
| Search in scrollback | `/` searches, `n`/`N` for next/prev | Basic search | Less feature-complete |
| Mouse wheel scrollback | Wheel enters copy mode directly | Basic wheel scroll | Different behavior |
| Interactive scrollbar | Click/drag right border thumb | Not available | Missing |
| Selection auto-scroll | Drag above/below pane scrolls | Not available | Missing |
| Scroll position indicator | `offset/total` on bottom border | Not available | Missing |
| Scrollback browser | OSC 133-aware block navigation | Basic block nav | Less sophisticated |
| Edit in $EDITOR | Open scrollback in external editor | Not available | Missing |

**Impact:** High. Copy mode is a primary workflow for power users.

### 4. 🟠 Visual Effects & Animations

| Feature | TUIOS | TermOS | Gap |
|---------|-------|--------|-----|
| Snap animations | Smooth window resize/move transitions | Snap animations exist | Match |
| Zen mode | Borders hide on idle, reveal on mouse move | Not available | Missing |
| Border styles | 9 styles (rounded, thick, double, hidden, block, ascii, etc.) | Limited styles | Fewer options |
| Border colors | Focused/unfocused configurable via theme + override | Theme-based | Less configurable |
| Window buttons | Hideable window control buttons on borders | Not available | Missing |
| Synchronized output | Mode 2026 prevents tearing | Not implemented | Missing |
| Style caching | LRU cache with sequence-based change detection | Basic caching | Less optimized |

**Impact:** Medium-High. Visual polish creates the "modern" feel.

### 5. 🟠 Navigation & Discovery

| Feature | TUIOS | TermOS | Gap |
|---------|-------|--------|-----|
| Aggregate view | Searchable list of ALL windows across ALL workspaces with previews | Basic aggregate view | Missing previews |
| Multifocus | Broadcast typing to multiple panes, Ctrl+click to select | Not available | Missing |
| Which-key popup | Hold prefix to see available chords | Which-key exists | Match |
| Command palette | `Ctrl+P` with fuzzy search | `Ctrl+B P` with fuzzy search | Match (after recent work) |
| Session switcher | In-app session list `Prefix+S` | Session overlay exists | Match |
| Theme picker | Searchable with 8-color swatch + live preview | Theme picker exists | Match |
| Showkeys overlay | Display pressed keys for presentations | Showkeys exists | Match |
| Config hot-reload | File watcher with 200ms debounce | Config reload exists | Match |
| Click-to-open | Click URLs/paths to open in browser/editor | Click-to-open exists | Match |

**Impact:** Medium. Discovery is good but the fundamentals are there.

### 6. 🟡 Session Management

| Feature | TUIOS | TermOS | Gap |
|---------|-------|--------|-----|
| Daemon mode | Persistent sessions with detach/reattach | Daemon mode exists | Match |
| Session resurrection | Sessions survive daemon restart | Session persistence exists | Match |
| Layout templates | Save/load/export with CWD + startup commands | Save/load only | Missing export + startup cmds |
| Multi-client | Several clients on one session | Web/SSH clients exist | Partial |
| Control protocol | JSON verb protocol for external control | Not available | Missing |
| Hooks | Shell commands on window create/close/focus | Hooks exist | Match |

**Impact:** Medium. Core session management is solid.

### 7. 🟡 Graphics & Protocols

| Feature | TUIOS | TermOS | Gap |
|---------|-------|--------|-----|
| Kitty graphics | Full image rendering, flicker-free video, SHM | Not implemented | Missing |
| Sixel graphics | Sixel passthrough | Not implemented | Missing |
| Kitty keyboard | CSI u with push/pop/query | Basic CSI u | Less complete |
| OSC 52 clipboard | Native clipboard via terminal | Clipboard module exists | Partial |
| Terminal queries | OSC 4, OSC 10-12, CSI 14/16/18t, DA1/DA2 | OSC scanning exists | Partial |

**Impact:** Low-Medium. Nice-to-have for advanced users.

### 8. 🟡 Mouse Interaction

| Feature | TUIOS | TermOS | Gap |
|---------|-------|--------|-----|
| Drag to select | Drag selection with copy on release | Basic selection | Less polished |
| Double-click word | Double-click selects word | Not available | Missing |
| Triple-click line | Triple-click selects line | Not available | Missing |
| Window drag | Drag title bar to move window | Not available (floating panes only) | Missing for tiled |
| Window resize | Drag border to resize | Not available | Missing |
| Scrollbar interaction | Click/drag scrollbar thumb | Not available | Missing |
| Edge snapping | Drag to screen edges for snap positions | Not available | Missing |

**Impact:** Medium. Mouse users expect these interactions.

## Priority Ranking

### Tier 1 — Closest to the "TUIOS feel" (do first)

1. **Layout modes** — Add master-stack and scrolling layout. This is the biggest functional gap.
2. **Copy mode** — Vim-style scrollback with full motions. Power users expect this.
3. **Dock polish** — Configurable position, session controls, zoom indicator, minimized entries.
4. **Border styles** — Multiple border styles (rounded, thick, double, hidden, block, ascii).

### Tier 2 — Visual modernization

5. **Zen mode** — Hide borders on idle, reveal on mouse. The "clean desktop" feel.
6. **Animations** — Smooth transitions for all layout changes.
7. **Synchronized output** — Mode 2026 for tear-free rendering.
8. **Interactive scrollbar** — Click/drag to navigate scrollback.

### Tier 3 — Advanced features

9. **Aggregate view with previews** — Show content previews in the window list.
10. **Multifocus** — Broadcast typing to multiple panes.
11. **Kitty graphics** — Image rendering and video passthrough.
12. **Mouse enhancements** — Double-click word, triple-click line, edge snapping.

### Tier 4 — Power user extras

13. **Control protocol** — JSON API for external automation.
14. **Layout export** — Convert layouts to tape scripts.
15. **Sixel graphics** — Experimental image passthrough.
16. **Selection auto-scroll** — Drag above/below pane to scroll during selection.

## What TermOS Already Does Well (No Gap)

- ✅ BSP tiling with preselection
- ✅ 9 workspaces with instant switching
- ✅ Modal interface (WM + Terminal modes)
- ✅ Command palette with fuzzy search
- ✅ Which-key popup
- ✅ Daemon mode with session persistence
- ✅ Context menus (right-click)
- ✅ Pane zoom
- ✅ Floating panes
- ✅ Config hot-reload
- ✅ Click-to-open URLs
- ✅ Showkeys overlay
- ✅ Theme system (21 built-in)
- ✅ Hooks on window events
- ✅ Welcome screen (first launch)
- ✅ Key hints bar
- ✅ Mode indicator
- ✅ Mouse-friendly dock (click/right-click/hover)
- ✅ Event-driven rendering (render_requested flag)
- ✅ PTY pool semaphore

## Estimated Effort

| Tier | Features | Estimated Phases |
|------|----------|-----------------|
| Tier 1 | Layout modes, copy mode, dock polish, borders | 4-6 phases |
| Tier 2 | Zen mode, animations, sync output, scrollbar | 3-4 phases |
| Tier 3 | Aggregate previews, multifocus, kitty graphics | 3-4 phases |
| Tier 4 | Control protocol, layout export, sixel, auto-scroll | 2-3 phases |

**Total: 12-17 phases to reach TUIOS parity**

## Recommendation

Focus on **Tier 1** first — these are the features that make TUIOS feel like a window manager rather than a multiplexer. The layout modes (master-stack + scrolling) and full copy mode are the two biggest gaps that would transform the user experience.

Start with **layout modes** since they're the most visible and impactful change — a user opening TermOS for the first time would immediately see the difference.
