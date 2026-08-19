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

### `tuios ssh [--host <HOST>] [--port <PORT>] [--key-path <PATH>] [--read-only]`

Run the SSH server. Each connection gets a fresh TermOS session.

- `--host` (default `localhost`), `--port` (default `2222`)
- `--key-path <PATH>`: host private key (required)
- `--read-only`: guests see output but their input is dropped (observer mode)

### `tuios web [--host <HOST>] [--port <PORT>] [--token <TOKEN>] [--read-only] [--max-connections N] [--touch auto|on|off] [--cert <PEM> --key <PEM> | --auto-tls]`

Run the web terminal server (xterm.js + WebSocket).

- `--host` (default `127.0.0.1`), `--port` (default `8080`)
- `--token <TOKEN>`: require an access token. Loopback binds without a token
  stay open; non-loopback binds always require a token (or refuse to start
  without TLS). The token is presented as `?token=` on the page or socket URL.
- `--read-only`: guests see output but their input is dropped (observer mode)
- `--max-connections <N>`: cap concurrent WebSocket clients (0 = unlimited)
- `--touch auto|on|off`: touch detection for the mobile key bar
- `--cert <PEM> --key <PEM>`: serve HTTPS with explicit certificate/key files
- `--auto-tls`: generate a self-signed certificate for the bind host (stored
  under the data dir) and serve HTTPS

TLS is required for any non-loopback bind (`check_transport_security` refuses
plain HTTP off localhost).

## Environment

- `TERMOS_SOCKET`: override the daemon socket path
- `XDG_DATA_HOME`: tape storage and trust store location
- `XDG_STATE_HOME`: session state location
- `TERM_PROGRAM`: detected for graphics capability probing
- `KITTY_WINDOW_ID`, `GHOSTTY_RESOURCES_DIR`, etc.: terminal detection
