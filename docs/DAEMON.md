# Daemon & session model — design

Phase 2 introduces the persistent-session architecture the remaining phases
build on. The key architectural choice, taken faithfully from the Go project:

> **The daemon owns the PTYs; the client owns the emulator and renderer.**

The daemon never parses VT or renders. It holds each session's shell processes
and multiplexes raw bytes: client input is forwarded to the PTY, PTY output is
streamed back to attached clients, which feed their own emulator and paint the
screen. This means the entire existing `vt`/`app`/`render` stack runs
unchanged on the client; only the `Window`'s I/O half is swapped from a local
PTY to a remote one.

## Session model

- `Session { id, name, created_at }` — `name` is the unique, user-facing
  identity every switch/kill/persist operation addresses; `id` is a UUID used
  internally and on the wire.
- A session owns zero or more windows, each a spawned PTY (`shell`, `cwd`,
  `cols`/`rows`) plus its workspace. The daemon tracks window→workspace so a
  client can rebuild the layout when it attaches.
- `Manager` holds sessions by name and by id, generates `session-N` names, and
  validates names (see `ValidateSessionName`: non-empty, no path separators,
  no whitespace, length-bounded).

## Transport

Unix domain socket (SOCK_STREAM) at:

- `$TUIOS_SOCKET` if set,
- else `$XDG_RUNTIME_DIR/tuios/tuios.sock`,
- else `/tmp/tuios-<uid>.sock`.

Framing: `u32` big-endian length prefix followed by a JSON payload. JSON (not
gob) is the wire format — it is one of the Go codec's two formats, it is
readable/debuggable, and it removes the need for a bespoke binary codec.

## Messages

Tagged enum, serialized with serde (`#[serde(tag = "type")]`):

| Message | Direction | Purpose |
|---------|-----------|---------|
| `Hello { name }` | C→D | handshake |
| `Welcome { version, sessions }` | D→C | handshake reply + session list |
| `List` | C→D | request session list |
| `ListResult { sessions }` | D→C | session list |
| `New { name, shell }` | C→D | create a session (+first window) |
| `Attach { name }` | C→D | attach; daemon starts streaming |
| `Attached { windows }` | D→C | attach ack + window list |
| `Detach` | C→D | detach (daemon stops streaming) |
| `Kill { name }` | C→D | kill a session |
| `NewWindow { shell, workspace }` | C→D | spawn a window in the session |
| `CloseWindow { id }` | C→D | kill a window's PTY |
| `WindowAdded { window }` | D→C | a window was spawned (broadcast) |
| `WindowClosed { window }` | D→C | a window was closed (broadcast) |
| `Input { window, data }` | C→D | forward bytes to a PTY |
| `Resize { window, cols, rows }` | C→D | resize a PTY |
| `PtyOutput { window, data }` | D→C | PTY output chunk |
| `PtyClosed { window }` | D→C | a window's shell exited |
| `Error { message }` | D→C | error reply |

## Attach lifecycle

1. Client connects, sends `Hello`, receives `Welcome`.
2. Client sends `Attach { name }`; the daemon registers a subscriber in the
   session's broadcast hub and replies `Attached { windows }`. Multiple
   clients may attach to the same session.
3. Each live window pumps its PTY output into the session's broadcast hub,
   which fans `PtyOutput`/`PtyClosed` frames out to every attached client; the
   client feeds them to its emulator and paints.
4. Client forwards keys as `Input` frames and layout changes as `Resize`;
   window spawn/close is requested via `NewWindow`/`CloseWindow` and announced
   to all clients via `WindowAdded`/`WindowClosed`.
5. `Detach` (or disconnect) removes the subscriber; the session's PTYs keep
   running.

## Persistence ("resurrection")

Sessions are saved as JSON in `$XDG_STATE_HOME/tuios/sessions/<name>.json`
(Windows/parts only — the shells to respawn and their workspaces). On daemon
start the saved sessions are restored (shells respawned, marked `restored`).
An explicit `kill` removes the state file so it does not resurrect.

## CLI

- `tuios daemon` — run the daemon in the foreground (or `--start` to fork).
- `tuios run [name]` — start daemon if needed, create/attach a session, run the TUI.
- `tuios attach <name>` — attach the TUI to an existing session.
- `tuios list` / `tuios ls` — list sessions.
- `tuios kill <name>` — kill a session.
- (default) — legacy single-process mode, unchanged.
