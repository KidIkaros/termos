# Driving TUIOS from a pane

This document is for an **agent running inside a TUIOS pane** (or any program
that wants to drive a TUIOS session programmatically). It is embedded in the
binary and printed by `tuios --skill`.

## Detecting TUIOS

Every pane spawned by TUIOS gets these environment variables:

- `TUIOS_ENV=1` — you are inside TUIOS.
- `TUIOS_SESSION_ID=<name>` — the session this pane belongs to.
- `TUIOS_WINDOW_ID=<id>` — this pane's window id.

## Addressing panes

Every verb below targets a window. A target is a window **id** (`w0`) or a
**title** (exact, then prefix). Omit it to target the session's most recently
active window (the pane you are typing in, from the daemon's point of view).

When more than one session exists, pass `-s <session>`; with exactly one
session it is implied.

## Verbs

Run `tuios list-verbs` for the authoritative list (it matches this build).

### send-keys — press keys in a pane

```
tuios send-keys [-s session] [-w window] <key> [key...]
```

Keys are named like a terminal emulator's: `enter`, `space`, `tab`, `esc`,
`backspace`, `delete`, `up`/`down`/`left`/`right`, `home`, `end`,
`pageup`/`pagedown`, single characters (`a`, `Z`, `1`), and modifiers
(`ctrl+b`, `alt+x`, `shift+tab`, `ctrl+alt+x`).

```sh
tuios send-keys "ctrl+b" "c"        # new window
tuios send-keys "ls" enter          # type a command and run it
```

### send-text — type raw text into a pane

```
tuios send-text [-s session] [-w window] <text>
```

Writes the text as-is (no key-name interpretation, no trailing newline).

```sh
tuios send-text "cargo test"
tuios send-text "pass"
```

### capture-pane — read a pane's recent output

```
tuios capture-pane [-s session] [-w window]
```

Prints the last 64 KiB of the pane's raw output. The daemon keeps the ring
itself, so this works with no client attached.

```sh
tuios capture-pane
```

### wait-for — wait until output matches a regex

```
tuios wait-for [-s session] [-w window] <regex> [timeout_ms]
```

Polls the pane's output until the regex matches or the timeout (default
5000 ms) passes. Prints `matched` or `timeout`; exits 0 either way.

```sh
tuios wait-for "cargo test" 30000   # wait for the test run to start
tuios wait-for "\\$"                # wait for a shell prompt
```

### set-agent-state / get-agent-state — report pane state

Agents report their state so TUIOS can surface which panes need a human.
States: `none`, `working`, `needs_input`, `idle`, `done`, `errored`.

```
tuios set-agent-state <state> [-s session] [-w window] [-m message] [--harness H]
tuios get-agent-state [-s session] [-w window]
```

```sh
tuios set-agent-state working -m "running the test suite"
tuios set-agent-state needs_input -m "waiting for approval"
tuios set-agent-state done
```

`[notifications.agent]` in the config decides what TUIOS does with these
transitions (dock message, terminal notification, sound, shell command).

## Best practices

- Prefer `wait-for` over fixed sleeps: it is deterministic.
- Check `TUIOS_ENV` before assuming the verb surface exists.
- The message text passed to `set-agent-state -m` is free text; it travels as
  an environment variable, never as a shell command.
