# Tape Scripting

Tapes are reproducible terminal automation scripts for TermOS. They record
and replay keyboard input, window management actions, and mode changes.

## File Format

A tape file is a plain-text list of commands, one per line. Comments start
with `#`. The file may include a header with metadata:

```
# tape: my-tape
# version: 1
# recorded: 2024-01-15T10:30:00Z
DisableAnimations

EnableTiling
WindowManagementMode
Sleep 200ms
TerminalMode
Type "echo hello"
Enter

EnableAnimations
```

## Commands

### Input Commands

| Command | Description |
|---------|-------------|
| `Type "text"` | Type a string (quoted, with escape support) |
| `Enter` | Press Enter |
| `Space` | Press Space |
| `Backspace` | Press Backspace |
| `Delete` | Press Delete |
| `Tab` | Press Tab |
| `Escape` | Press Escape |
| `Up` / `Down` / `Left` / `Right` | Arrow keys |
| `Home` / `End` | Home / End keys |
| `PageUp` / `PageDown` | Page Up / Page Down |
| `Key "ctrl+c"` | A named key combination |

### Window Management

| Command | Description |
|---------|-------------|
| `NewWindow` | Create a new window |
| `CloseWindow` | Close the focused window |
| `FocusNext` / `FocusPrev` | Cycle focus |
| `SplitHorizontal` / `SplitVertical` | Split the focused pane |
| `RotateSplit` | Rotate the split direction |
| `WindowManagementMode` | Enter window-management mode |
| `TerminalMode` | Enter terminal mode |

### Workspace

| Command | Description |
|---------|-------------|
| `Workspace <N>` | Switch to workspace N (1-9) |

### Timing and Control

| Command | Description |
|---------|-------------|
| `Sleep <duration>` | Wait (e.g. `Sleep 200ms`, `Sleep 1.5s`) |
| `DisableAnimations` | Suppress animation transitions |
| `EnableAnimations` | Re-enable animations |

## Recording

Press `Ctrl+B`, `T`, `r` to start recording. A recording indicator appears
in the dock. Press `Ctrl+B`, `T`, `s` to stop and save.

The recorder:
- Accumulates consecutive typed characters into one `Type` command
- Records explicit key/action commands
- Preserves delays above a threshold as `Sleep` commands
- Emits a valid tape file with animation suppression/restoration

## Trust

Tapes from other sources must be trusted before playback. The trust store
is keyed by (canonical path, SHA-256 content hash):

- `tape play` prompts for untrusted tapes: `y` to trust and run, `n` to
  abort, `d` to deny permanently
- Trusted tapes run without prompting
- Denial is by path alone; trust is by path + content
- Tampering with a trusted tape reverts it to untrusted

## Project Tapes

A `.tuios.tape` file in the current directory is a "project tape". Press
`Ctrl+B`, `T`, `t` to discover and review it. The review overlay shows the
tape's path and hash; `y` trusts and plays it.

## Remote Execution

`tape exec -s <SESSION> <FILE>` sends a tape to a running session via the
daemon. The daemon broadcasts each command to the session's attached
clients, which execute them in their event loop.
