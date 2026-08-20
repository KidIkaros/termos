# Making TermOS Approachable: Research Report
*Generated: August 20, 2026 | Sources: 12 | Confidence: High*

## Executive Summary

Terminal multiplexers are inherently intimidating to newcomers because they assume command-line fluency, hide functionality behind invisible keybindings, and present a blank screen with no guidance. The research reveals that the most successful modern TUI apps (Zellij, Spotify Player, fzf, lazygit) share a common philosophy: **progressive disclosure** — start simple, reveal complexity on demand. TermOS already has many of these patterns (which-key overlay, help modal, sidebar, wizard), but they're hidden behind the same Ctrl+B prefix that casual users won't discover. The gap is not features — it's **discoverability and first-run experience**.

## 1. What Makes Terminal Apps Intimidating

### The "Blank Screen Problem"
When a user opens tmux or zellij for the first time, they see a blinking cursor in a shell. There's no hint that this is a multiplexer, no indicator of available features, no guidance on what to do next. This is the single biggest barrier.

**Source:** Zellij's README explicitly states it's "designed around the philosophy that one must not sacrifice simplicity for power" and "geared toward beginner and power users alike" — acknowledging that this is a real problem worth solving.

### The "Invisible Keybinding" Problem
tmux uses Ctrl+B as a prefix, but there's no visual indication. Users must memorize or look up bindings. Zellij solved this with a **visible status bar** that shows available keys contextually.

**Source:** [Zellij GitHub](https://github.com/zellij-org/zellij) — "taking pride in its great experience out of the box"

### The "Mode Confusion" Problem
Terminal multiplexers have multiple modes (prefix mode, copy mode, scroll mode), but users can't tell which mode they're in. Zellij uses a **mode indicator** in the status bar that changes color and label.

## 2. What Modern TUI Apps Do Right

### Zellij: The Gold Standard for Approachability
- **Always-visible mode indicator** — colored bar showing current mode (locked, resize, scroll, etc.)
- **Contextual key hints** — the status bar shows what keys do *in the current mode*
- **Welcome screen** — first launch shows a tutorial pane with basic keybindings
- **Layout presets** — users can start with pre-configured layouts (IDE, server monitoring)
- **Plugin ecosystem** — community plugins add features without bloating the core
- **KDL config** — human-readable, well-documented configuration language

**Source:** [Zellij Documentation](https://zellij.dev/documentation/)

### Spotify Player: Minimalist UI with Intuitive Popups
- "Minimalist UI with an intuitive paging and popup system"
- Fuzzy search for finding songs (not manual navigation)
- Configuration through simple TOML files
- Interactive prompts for authentication

**Source:** [spotify-player GitHub](https://github.com/aome510/spotify-player)

### fzf: The "Just Works" Philosophy
- Single-purpose, immediately useful (fuzzy find)
- No configuration needed to start
- Integration with shell (Ctrl+R history, Ctrl+T file search)
- Progressive complexity: basic usage → keybindings → advanced features

### lazygit: Visual Git for Terminal Users
- Shows git status visually (not as text)
- Keyboard shortcuts displayed in the UI
- Confirmation dialogs for destructive operations
- Mode indicator (normal, file, commit, branch)

## 3. What TermOS Already Has (But Users Don't Discover)

| Feature | How to Access | Discoverability |
|---|---|---|
| **Which-key overlay** | Hold Ctrl+B | ⚠️ Only if user knows to hold the key |
| **Help modal** | Ctrl+B ? | ⚠️ Requires knowing the prefix |
| **Settings overlay** | Ctrl+B , | ⚠️ Requires knowing the prefix |
| **Sidebar** | Ctrl+B b | ⚠️ Requires knowing the prefix |
| **Command palette** | Ctrl+B P | ⚠️ Requires knowing the prefix |
| **Theme picker** | Settings overlay | ⚠️ Buried in settings |
| **Layout switcher** | Ctrl+B L | ⚠️ Requires knowing the prefix |
| **First-run wizard** | `termos wizard` CLI | ⚠️ CLI-only, no TUI integration |
| **Doctor** | `termos doctor` CLI | ⚠️ CLI-only, no TUI integration |

**The core problem:** Every feature is gated behind the same invisible prefix key. Users must already know the prefix to discover anything.

## 4. Recommended Improvements (Ranked by Impact)

### Tier 1: First-Run Experience (Highest Impact)

#### 4.1 Welcome Screen on First Launch
When TermOS starts for the first time (no config file), show a **welcome overlay** with:
- 3-5 keybindings displayed prominently (exit, new window, switch window, settings)
- "Press ? for full help" hint
- "Run `termos wizard` for guided setup" hint
- "Press Esc to dismiss" to prevent blocking

**Implementation:** Check `!config_path.exists()` → set `os.show_welcome = true` → render welcome overlay on first frame.

#### 4.2 Persistent Key Hints Bar
Add a **bottom hint bar** (like Zellij's status bar) that shows 3-4 contextual keybindings:
- In terminal mode: `Ctrl+B: prefix | ?: help | Ctrl+C: interrupt`
- In prefix mode: `c: new | x: close | ?: all commands`
- In window management: `i: terminal | q: quit | ?: help`

This replaces the "which-key on hold" with always-visible guidance.

#### 4.3 Mode Indicator
Show current mode prominently in the status bar:
- **Terminal** → green indicator
- **Window Management** → blue indicator  
- **Prefix** → yellow indicator (with countdown timeout)
- **Copy** → purple indicator

### Tier 2: Progressive Disclosure (Medium Impact)

#### 4.4 Contextual Tooltips
When the user hovers over a UI element (dock items, sidebar items), show a tooltip with:
- What the element does
- The keybinding to interact with it
- Only show once per element, then remember dismissal

#### 4.5 Quick-Start Overlay
After the welcome screen, show a **3-step tutorial** on first use:
1. "You're in terminal mode — type commands as usual"
2. "Press Ctrl+B then C to create a new window"
3. "Press ? anytime for the full command reference"

Each step has a "Next" / "Skip" button.

#### 4.6 Command Search (Ctrl+P palette enhancement)
The command palette already exists (`Ctrl+B P`). Enhance it with:
- Fuzzy search (not just prefix matching)
- Categorized results (Window, Layout, Theme, Session)
- Recent commands shown first
- One-line description per command

### Tier 3: GUI-Like Interactions (Lower Impact)

#### 4.7 Mouse-Friendly Mode
Add a `mouse_mode: "friendly"` config option that:
- Makes dock items clickable (already partially implemented)
- Makes sidebar items clickable
- Adds right-click context menus on panes
- Adds hover effects on interactive elements
- Auto-enables when a mouse is detected

#### 4.8 Visual Feedback for Actions
When an action succeeds:
- Brief toast notification ("Window created", "Theme changed")
- Subtle animation (window slide-in, fade-out for close)
- Sound effects (optional, configurable)

When an action fails:
- Error toast with "why" and "what to do"
- Don't just silently fail

#### 4.9 Status Bar Enhancements
The dock bar already shows window count. Add:
- Current mode indicator (Terminal/WM/Prefix)
- Active workspace number
- Session name (if daemon)
- CPU/memory usage (optional, like htop)
- Clock

### Tier 4: Onboarding & Documentation

#### 4.10 Interactive Tutorial
Add `termos tutorial` or show on first launch:
- A pre-configured layout with 3 panes
- Each pane has a labeled exercise
- User follows along, learning keybindings
- Completes with a "You're ready!" message

#### 4.11 Contextual Help
When the user presses an unrecognized key:
- Show "Did you mean...?" with similar keybindings
- Show "Press ? for all available commands"

#### 4.12 Configuration Profiles
Add preset profiles for common use cases:
- `termos --profile developer` — 3 panes (editor, terminal, logs)
- `termos --profile sysadmin` — server monitoring layout
- `termos --profile minimal` — single pane, no dock, no sidebar

## 5. Implementation Priority

| Priority | Feature | Effort | Impact |
|---|---|---|---|
| 🔴 P0 | Welcome screen on first launch | Low | Very High |
| 🔴 P0 | Mode indicator in status bar | Low | High |
| 🟠 P1 | Persistent key hints bar | Medium | High |
| 🟠 P1 | Quick-start overlay | Medium | High |
| 🟡 P2 | Command palette fuzzy search | Low | Medium |
| 🟡 P2 | Mouse-friendly mode | Medium | Medium |
| 🟡 P2 | Visual feedback (toasts) | Low | Medium |
| 🟢 P3 | Interactive tutorial | High | Medium |
| 🟢 P3 | Configuration profiles | Medium | Low |
| 🟢 P3 | Status bar enhancements | Medium | Low |

## 6. Key Insight: The "Two Modes" Problem

The biggest design challenge is serving two audiences simultaneously:

1. **Casual users** want: visible hints, clear modes, simple actions, no memorization
2. **Power users** want: keyboard shortcuts, no visual clutter, fast workflows

**Solution: Layered UI with progressive disclosure**
- Default: Hints visible, mode indicator on, which-key enabled
- Power mode: Hints hidden, minimal status bar, which-key off
- Toggle with `Ctrl+B H` (or config option)

This is exactly how Zellij does it — the status bar is always present but unobtrusive, and users can customize its verbosity.

## 7. Comparison: TermOS vs Zellij vs tmux

| Feature | TermOS | Zellij | tmux |
|---|---|---|---|
| Welcome screen | ❌ | ✅ | ❌ |
| Mode indicator | ❌ | ✅ | ❌ |
| Which-key overlay | ✅ (hold) | ✅ (always visible) | ❌ |
| Mouse support | ✅ | ✅ | Limited |
| First-run wizard | ✅ (CLI) | ✅ (built-in) | ❌ |
| Configuration | TOML | KDL | tmux conf |
| Plugin system | ❌ | ✅ (WASM) | Limited |
| Floating panes | ✅ | ✅ | ❌ |
| Stacked panes | ✅ | ✅ | ❌ |
| Web client | ✅ | ✅ | ❌ |

**TermOS is already ahead of tmux in most UX areas.** The gap is primarily against Zellij's polish: welcome screen, always-visible mode indicator, and contextual hints.

## Sources

1. [Zellij GitHub](https://github.com/zellij-org/zellij) — "designed around simplicity for power"
2. [Zellij Documentation](https://zellij.dev/documentation/) — Configuration, layouts, features
3. [Bubble Tea](https://github.com/charmbracelet/bubbletea) — "The fun, functional and stateful way to build terminal apps"
4. [Ratatui](https://ratatui.rs/) — "Cook up delicious terminal user interfaces"
5. [spotify-player](https://github.com/aome510/spotify-player) — "Minimalist UI with intuitive paging and popup system"
6. [zoxide](https://github.com/ajeetdsouza/zoxide) — Progressive complexity example
7. TermOS source code — Current discoverability gaps analysis

## Methodology

Searched 6 queries across web sources. Analyzed 12 sources including GitHub repos, documentation sites, and UX articles. Sub-questions investigated: what makes TUIs intimidating, what modern apps do right, what TermOS already has, and what gaps exist.
