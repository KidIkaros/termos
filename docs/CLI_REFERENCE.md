# CLI Reference

## Synopsis

```
tuios [OPTIONS] [COMMAND]
```

## Commands

### `tuios` (default)

Start the interactive TUI window manager.

```
tuios [--debug] [--config <FILE>] [--skill]
```

- `--debug`: enable debug logging
- `--config <FILE>`: load a custom config file
- `--skill`: print the agent skill sheet (recipes for driving TermOS from
  inside a pane) and exit

### `tuios daemon`

Run the session daemon (Unix socket server).

```
tuios daemon [--socket <PATH>]
```

### `tuios attach <SESSION>`

Attach to a running session.

```
tuios attach <SESSION> [--socket <PATH>]
```

### `tuios run <SESSION>`

Create a new session with a default window.

```
tuios run <SESSION> [--socket <PATH>]
```

### `tuios exec <SESSION> -- <COMMAND>`

Run a command in a new window on the given session.

```
tuios exec <SESSION> -- <COMMAND...>
```

### `tuios tape play <FILE>`

Play a tape script. Untrusted tapes prompt for trust approval.

```
tuios tape play <FILE>
```

### `tuios tape exec -s <SESSION> <FILE>`

Execute a tape script on a running session via the daemon.

```
tuios tape exec -s <SESSION> <FILE>
```

### `tuios tape list`

List saved tapes.

### `tuios tape show <NAME>`

Print a tape's contents. Accepts `foo` or `foo.tape`.

### `tuios tape delete <NAME>`

Delete a saved tape. Accepts `foo` or `foo.tape`.

### `tuios tape dir`

Print the tape storage directory.

### `tuios resurrect [SESSION]`

Restore saved session(s) from disk. With a name, restores that session;
without, restores all saved sessions.

### `tuios start-server`

Alias for `tuios daemon`.

### `tuios kill-server`

Stop the running daemon.

### `tuios session-info [SESSION]`

Show session details: window count, creation time, attached/restored status.

### `tuios list-windows [SESSION]`

Show window count for a session.

### `tuios set-session-name <SESSION> <NAME>`

Set a session's display label.

### `tuios set-session-accent <SESSION> <ACCENT>`

Set a session's accent color.

### `tuios logs`

Show the daemon log.

### `tuios layout <list|delete|dir>`

Manage saved layouts:
- `list`: list saved layout names
- `delete <NAME>`: delete a saved layout
- `dir`: print the layout storage directory

### `tuios config <show|path|edit|reset|validate>`

Config management:
- `show`: print the current config
- `path`: print the config file path
- `edit`: open the config in `$EDITOR`/`$VISUAL` (default: vi)
- `reset`: write the default config to the config path
- `validate`: check config for errors and warnings

### `tuios keybinds <list|describe>`

Keybind reference:
- `list`: list all keybindings
- `describe <ACTION>`: describe a specific action or key

## Network Modes (requires `--features network`)

### `tuios --network ssh --host-key <PATH> [--addr <ADDR>]`

Run the SSH server. Each connection gets a fresh TermOS session.

### `tuios --network web [--addr <ADDR>]`

Run the web terminal server (xterm.js + WebSocket).

## Environment

- `TERMOS_SOCKET`: override the daemon socket path
- `XDG_DATA_HOME`: tape storage and trust store location
- `XDG_STATE_HOME`: session state location
- `TERM_PROGRAM`: detected for graphics capability probing
- `KITTY_WINDOW_ID`, `GHOSTTY_RESOURCES_DIR`, etc.: terminal detection
