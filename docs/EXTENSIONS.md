# Extension Protocol

TermOS supports extensibility through three mechanisms: **lifecycle hooks**,
**status-line widgets**, and **custom palette actions**. A full WASM plugin
runtime (like Zellij's) is not yet implemented; the protocol is designed so
a WASM layer can slot in later without breaking existing extensions.

## 1. Lifecycle Hooks

Hooks fire asynchronously when events occur. Each hook runs a shell command
with `TERMOS_*` environment variables providing context.

### Configuration

```toml
[hooks]
after-new-window = "notify-send 'TermOS' 'opened $TERMOS_WINDOW_NAME'"
after-focus-change = ["echo $TERMOS_WINDOW_NAME > /tmp/termos-focus", "~/.config/termos/on-focus.sh"]
```

### Available Events

| Event | Fires when |
|---|---|
| `after-new-window` | A new window is created |
| `after-close-window` | A window is closed |
| `after-focus-change` | Focus moves to a different pane |
| `after-workspace-switch` | The active workspace changes |
| `after-attach` | A client attaches to a session |
| `after-detach` | A client detaches from a session |
| `after-layout-change` | The tiling layout changes |
| `after-resize` | The terminal is resized |
| `after-agent-state` | A pane's agent state changes (gated by `[notifications.agent]`) |
| `pane-shell-prompt` | Shell emits OSC 133 `A` (fresh prompt) |
| `pane-command-started` | Shell emits OSC 133 `B` (command started) |
| `pane-command-finished` | Shell emits OSC 133 `D` (command finished) |

### Environment Variables

Every hook receives:

| Variable | Description |
|---|---|
| `TERMOS_EVENT` | The event name (e.g. `after-new-window`) |
| `TERMOS_WINDOW_ID` | The window's unique ID |
| `TERMOS_WINDOW_NAME` | The window's title |
| `TERMOS_WORKSPACE` | The current workspace number (1–9) |
| `TERMOS_SESSION_ID` | The daemon session name (empty in local mode) |

Event-specific:

| Variable | Events | Description |
|---|---|---|
| `TERMOS_PREV_WORKSPACE` | `after-workspace-switch` | Previous workspace number |
| `TERMOS_LAYOUT` | `after-layout-change` | New layout (`bsp`, `master-stack`, etc.) |
| `TERMOS_WIDTH` / `TERMOS_HEIGHT` | `after-resize` | New terminal dimensions |
| `TERMOS_AGENT_STATE` | `after-agent-state` | New agent state |
| `TERMOS_AGENT_PREV_STATE` | `after-agent-state` | Previous agent state |
| `TERMOS_AGENT_HARNESS` | `after-agent-state` | Harness ID |
| `TERMOS_AGENT_MESSAGE` | `after-agent-state` | Free-text message |
| `TERMOS_EXIT_CODE` | `pane-command-finished` | Exit code (`-1` if unknown) |

### Semantics

- Hooks run **asynchronously** in their own threads.
- Output is **discarded** and exit status **ignored** (fire-and-forget).
- Multiple commands per event run in **parallel**.
- Hooks registered via `[notifications.agent] command` are shorthand for
  registering under the `after-agent-state` event.

## 2. Status-Line Widgets

Status widgets run shell commands periodically and render the first line of
stdout in the dock bar's right region.

### Configuration

```toml
[[status_widgets]]
name = "CPU"
command = "top -bn1 | head -3 | tail -1 | awk '{print $2}'"
refresh_ms = 5000

[[status_widgets]]
name = "Memory"
command = "free -h | awk '/Mem:/{print $3\"/\"$2}'"
refresh_ms = 10000
alignment = "right"
```

### Fields

| Field | Type | Default | Description |
|---|---|---|---|
| `name` | string | (required) | Display label in config; key for caching |
| `command` | string | (required) | Shell command to run |
| `refresh_ms` | integer | `0` | Refresh interval in ms (0 = once at startup) |
| `alignment` | string | `"right"` | `"left"`, `"center"`, or `"right"` |

### Rendering

- Widgets are rendered **right-to-left** in the dock bar, separated by `│`.
- Each widget shows its `command` output (first line, trimmed).
- Widgets that fail show `err`.
- The dock bar only has 1 row, so widgets share space with session controls
  and workspace pills.

## 3. Custom Palette Actions

Custom actions add entries to the command palette that run shell commands.

### Configuration

```toml
[[custom_actions]]
name = "Run tests"
command = "cargo test"
category = "Build"

[[custom_actions]]
name = "Git status"
command = "git status"
category = "Git"
```

### Fields

| Field | Type | Default | Description |
|---|---|---|---|
| `name` | string | (required) | Label shown in the palette |
| `command` | string | (required) | Shell command to execute |
| `category` | string | `"Custom"` | Palette grouping category |

### Dispatch

- Custom actions appear in the palette alongside built-in commands.
- They are filtered by the same fuzzy matching as built-in commands.
- When selected, the command runs with the same `TERMOS_*` environment
  variables as hooks (window context of the focused pane).
- Execution is **synchronous** (the palette closes and the command runs
  to completion).

## 4. Extension Protocol (Future)

The extension protocol is designed for future WASM plugin support. The
planned contract:

1. **Event subscription**: Plugins register for hook events.
2. **Rendering contract**: Plugins produce a text/ANSI buffer that the
   dock renderer composites. Status widgets are the current primitive
   version of this.
3. **Verb surface**: Plugins register custom commands (custom actions are
   the current primitive version).
4. **IPC**: Plugins communicate with the host via stdin/stdout (hooks and
   custom actions) or a typed protocol (future WASM).

The current hook + widget + custom action system covers the 80% use case.
WASM plugins will add bidirectional communication, richer rendering, and
persistent state.

## Example: Git Branch in the Dock

```toml
[[status_widgets]]
name = "git-branch"
command = "git rev-parse --abbrev-ref HEAD 2>/dev/null || echo ' detached'"
refresh_ms = 2000
```

## Example: Build Notification

```toml
[hooks]
after-layout-change = "notify-send 'TermOS' 'Layout changed to $TERMOS_LAYOUT'"

[[custom_actions]]
name = "Full build"
command = "cargo build --release 2>&1 | tail -5"
category = "Build"
```
