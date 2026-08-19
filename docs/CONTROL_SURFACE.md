# Control Surface — Scripting TermOS

The daemon speaks a public, line-delimited JSON protocol over a Unix
socket. Everything the TUI does interactively can be driven headlessly:
create sessions and windows, send input, capture and tail output, wait for
conditions, and block until a pane exits. This is the surface CI pipelines
and AI agents (`--skill` mode) drive.

## The socket

```
TERMOS_SOCKET            # explicit override
$XDG_RUNTIME_DIR/termos/termos.sock
/tmp/tuios-<uid>.sock    # fallback
```

The same socket accepts two flavours, auto-detected by the first byte:

- **Binary protocol** — length-prefixed JSON `Message` frames (used by the
  TUI client).
- **Verb protocol** — one JSON request per line, one JSON response per
  line. This document covers the verb protocol; it is what `termos action`,
  `termos subscribe`, and `termos block-until-exit` speak.

Start the daemon with `termos daemon` (or `termos start-server`). All
commands below fail with a clear message if it is not running.

## Generic verb call

```
termos action <verb> [key=value ...] [--json]
```

`key=value` pairs become string parameters. The response is one JSON object;
without `--json` it is printed compactly, with `--json` pretty-printed.

```bash
termos action new-session name=ci shell=/bin/sh
termos action new-window session=ci shell=/bin/sh --json
termos action send-text session=ci window=w0 text='echo hello\r'
termos action capture-pane session=ci window=w0
```

`termos list-verbs [verb]` prints every verb with its parameter schema,
accepted values, and an example. `termos list-verbs --json` emits the full
machine-readable catalog.

## Verbs

| Verb | Parameters | Returns |
|---|---|---|
| `hello` | `client`, `version`, `protocol` | version handshake |
| `list-sessions` | — | `sessions: [SessionInfo]` |
| `new-session` | `name`*, `shell` | `session: SessionInfo` |
| `session-info` | `session` | session detail |
| `list-windows` | `session`* | `windows: [WindowInfo]` |
| `new-window` | `session`*, `shell`, `workspace` | `window: WindowInfo` (**id** = script target) |
| `close-window` | `session`*, `window` | `closed: true` |
| `send-text` | `session`*, `window`, `text` | `sent: text` |
| `send-keys` | `session`*, `window`, `keys` | `sent: text` |
| `capture-pane` | `session`*, `window` | `window`, `content` |
| `resize` | `session`*, `window`, `cols`, `rows` | `resized: true` |
| `wait-for` | `session`*, `window`, `pattern`, `timeout` | `window`, `matched` |
| `block-until-exit` | `session`*, `window`, `timeout` | `window`, `exit_code`, `success` |
| `subscribe` | `session`*, `window` | streamed output (see below) |
| `kill-session` | `session`* | `killed: name` |
| `set-session-name` / `set-session-accent` | `session`*, `name`/`accent` | rename / accent |
| `set-workspace-name` | `session`*, `workspace`, `name` | workspace label |
| `get-agent-state` / `set-agent-state` | `session`*, `window`, ... | agent state |
| `diagnose` | — | daemon health report |

`session` and `window` are optional everywhere a single target can be
inferred: a missing `session` defaults to the only session, and a missing
`window` to the session's most recently active window. Windows resolve by
id, by exact/prefix title, or by omission. `*` = resolved this way.

Errors are envelope objects with a stable code:

```json
{"error": {"code": "window_not_found", "message": "window 'nope' not found"}}
```

Codes: `unknown_verb`, `invalid_params`, `session_not_found`,
`window_not_found`, `timeout`, `command_failed`, `unknown_verb`, `io`.

## The scripted workflow

### 1. Create a session and capture the window id

```bash
termos action new-session name=ci shell=/bin/sh
termos action new-window session=ci shell=/bin/sh
# {"window":{"id":"w1", ...}}
```

Every creation verb returns the new entity's id — script targeting never
guesses.

### 2. Drive and observe

```bash
termos action send-text session=ci window=w1 text='cargo test\r'
termos action wait-for session=ci window=w1 pattern='test result: ok' timeout=120000
termos action capture-pane session=ci window=w1
```

`wait-for` polls the pane's output ring until the regex matches or the
timeout elapses — the building block for "wait until the command finishes".

### 3. Structured queries

```bash
termos ls --json                  # sessions only
termos ls --json -W               # sessions + every window with geometry
termos action get-window session=ci window=w1  # detail
termos action diagnose            # daemon health
```

### 4. Tail output and wait for exit

```bash
# Stream a pane's output until its shell exits (plain tail, or --json events)
termos subscribe -s ci -w w1
termos subscribe -s ci -w w1 --json

# Block until the pane exits; report the exit code
termos block-until-exit -s ci -w w1 --timeout 120000
# exit code 0  → pane exited 0

# Failure-aware variant for retry loops: exit 0 when the pane fails
termos block-until-exit -s ci -w w1 --failure
```

`block-until-exit` exit status: `0` condition met, `1` condition not met,
`2` timeout/error. `--success` (default) matches exit code 0; `--failure`
matches any non-zero exit. `--timeout 0` waits forever.

`subscribe` streams one JSON line per output chunk and a final
`{"closed": true}` when the window exits or is closed. Plain mode prints
just the data (so `termos subscribe` behaves like `tail -f`).

## Raw protocol

A script can talk to the socket directly — one request line, one response
line:

```bash
printf '%s\n' '{"verb":"list-sessions","params":{}}' \
  | socat - UNIX-CONNECT:"$TERMOS_SOCKET"
```

```json
{"result": {"sessions": [{"name":"ci","windows":1,"attached":false}]}}
```

## Examples

### CI: run a command and gate on its result

```bash
termos action new-session name=ci shell=/bin/sh
WID=$(termos action new-window session=ci shell=/bin/sh | jq -r .window.id)
termos action send-text session=ci window=$WID text='make test\r'
termos action wait-for session=ci window=$WID pattern='PASS|FAIL' timeout=300000
termos block-until-exit -s ci -w $WID --timeout 300000 || echo "build pane failed"
termos action capture-pane session=ci window=$WID
```

### Retry loop until a command succeeds

```bash
while true; do
  WID=$(termos action new-window session=ci shell=/bin/sh | jq -r .window.id)
  termos action send-text session=ci window=$WID text='./flaky.sh\r'
  if termos block-until-exit -s ci -w $WID --success; then
    echo "flaky.sh passed"; break
  fi
done
```

### Agent driver (`--skill` mode uses the same protocol)

```bash
termos action new-session name=agent shell=/bin/sh
WID=$(termos action new-window session=agent shell=/bin/sh | jq -r .window.id)
termos action send-text session=agent window=$WID text='cargo run --release\r'
termos subscribe -s agent -w $WID --json      # watch the build
termos action set-agent-state session=agent window=$WID state=working message=compiling harness=my-agent
```

## Testing

`cargo test --test control_surface` drives a real daemon over a temp socket
end-to-end: session creation, ID capture, input/capture/wait, subscribe
streaming, exit-status reporting, and error envelopes. `cargo test --test
daemon` covers the binary protocol and lifecycle.
