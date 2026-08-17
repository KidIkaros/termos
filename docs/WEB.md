# Web Terminal Mode

**Security Notice:** The web terminal functionality is provided as a separate feature, gated behind the `network` Cargo feature, to isolate the web server from the main TermOS binary. This prevents the web server from being used as a potential backdoor.

TermOS can be accessed through any modern web browser when built with the `network` feature.

> **Ported from TUIOS** (https://github.com/Gaurav-Gosain/tuios) — the upstream Go project provides a separate `tuios-web` binary. TermOS implements the same functionality as a Cargo feature using `axum` and `rustls`.

## Table of Contents

- [Building](#building)
- [Overview](#overview)
- [Quick Start](#quick-start)
- [Features](#features)
- [Architecture](#architecture)
- [Configuration](#configuration)
- [Transport Protocols](#transport-protocols)
- [Rendering](#rendering)
- [Performance](#performance)
- [Security](#security)
- [Troubleshooting](#troubleshooting)

---

## Building

The web terminal requires the `network` feature (and `tls` for HTTPS):

```bash
# Build with web/SSH support
cargo build --features network

# Build with TLS support for HTTPS
cargo build --features network,tls

# Run the web server
cargo run --features network -- web

# Run with TLS
cargo run --features network,tls -- web --auto-tls
```

Without the `network` feature, the `web` subcommand is not available.

---

## Overview

The `web` subcommand starts a web server that serves a full TermOS experience in the browser. It is powered by `axum` for HTTP/WebSocket serving and `rustls` for TLS.

**Key technologies:**
- **xterm.js** for terminal emulation (served as static assets)
- **WebGL/Canvas** for hardware-accelerated rendering
- **WebSocket** for real-time communication
- **JetBrains Mono Nerd Font** for proper icon rendering

> **Note:** The upstream Go project uses the [sip library](https://github.com/Gaurav-Gosain/sip) for web terminal serving. See [SIP_LIBRARY.md](SIP_LIBRARY.md) for details on the upstream architecture. TermOS implements equivalent functionality in Rust using `axum` and `rustls`.

## Quick Start

```bash
# Start web server on default port (7681)
cargo run --features network -- web

# Open in browser
open http://localhost:7681

# With custom port
cargo run --features network -- web --port 8080

# With TermOS flags forwarded
cargo run --features network -- web --theme dracula --show-keys
```

## Features

- **Full TermOS Experience**: All TermOS features work in the browser
- **WebGL Rendering**: GPU-accelerated terminal rendering for smooth 60fps
- **WebSocket Communication**: Real-time bidirectional communication
- **Bundled Nerd Fonts**: No client-side font installation required
- **Settings Panel**: Configure renderer and font size
- **Mouse Support**: Full mouse interaction with cell-based optimization
- **Auto-Reconnect**: Automatic reconnection with exponential backoff
- **Read-Only Mode**: View-only sessions for demonstrations

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                        Browser                               │
├─────────────────────────────────────────────────────────────┤
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────────┐  │
│  │  xterm.js   │◄──►│ terminal.js │◄──►│   WebSocket     │  │
│  │  (WebGL)    │    │  (client)   │    │   (transport)   │  │
│  └─────────────┘    └─────────────┘    └────────┬────────┘  │
└─────────────────────────────────────────────────┼───────────┘
                                                  │
                                    ┌─────────────┴─────────────┐
                                    │     WebSocket (TCP:7681)  │
                                    └─────────────┬─────────────┘
                                                  │
┌─────────────────────────────────────────────────┼───────────┐
│                     Server                      │           │
├─────────────────────────────────────────────────┼───────────┤
│  ┌──────────────┐    ┌──────────────┐    ┌─────┴─────┐     │
│  │ HTTP Server  │    │  TLS (rustls)│    │  Session  │     │
│  │  (axum)      │    │   (optional) │    │  Manager  │     │
│  │  :7681       │    └──────────────┘    └─────┬─────┘     │
│  └──────────────┘                               │           │
│                                          ┌─────┴─────┐     │
│                                          │    PTY    │     │
│                                          │ (TermOS)  │     │
│                                          └───────────┘     │
└─────────────────────────────────────────────────────────────┘
```

### Data Flow

1. **Client → Server**: Keyboard/mouse input sent as binary WebSocket messages
2. **Server → Client**: Terminal output streamed with message batching
3. **Framing**: WebSocket frames preserve message boundaries

### Message Protocol

| Type | Code | Direction | Description |
|------|------|-----------|-------------|
| Input | `0` | C→S | Keyboard/mouse input |
| Output | `1` | S→C | Terminal output data |
| Resize | `2` | C→S | Terminal size change |
| Ping | `3` | C→S | Keep-alive ping |
| Pong | `4` | S→C | Keep-alive response |
| Title | `5` | S→C | Window title update |
| Options | `6` | S→C | Session configuration |
| Close | `7` | S→C | Session ended |

## Configuration

### Command Line Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--port` | `7681` | HTTP server port |
| `--host` | `localhost` | Server bind address |
| `--read-only` | `false` | Disable client input |
| `--max-connections` | `0` | Max concurrent sessions (0=unlimited) |
| `--cert` | | TLS certificate (PEM); serves HTTPS |
| `--key` | | TLS private key (PEM); required with `--cert` |
| `--auto-tls` | `false` | Serve HTTPS from a self-signed certificate TermOS generates and keeps |
| `--cert-dir` | | Where `--auto-tls` keeps its keypair (default: `termos` in your user config dir) |
| `--cert-host` | | Extra DNS name or IP in the `--auto-tls` certificate (repeatable) |
| `--cert-days` | `0` | Days an `--auto-tls` certificate is valid for (0 = 365) |
| `--insecure` | `false` | Serve a non-loopback host unencrypted |
| `--touch` | `auto` | Whether a client is driven by a finger: `auto`, `on`, `off` |
| `--default-session` | | Default session name for all connections |
| `--ephemeral` | `false` | Disable daemon mode (sessions don't persist) |

### Daemon Mode (Default)

By default, the web server connects to the TermOS daemon for persistent sessions:

```bash
# Start web server with daemon mode (default)
cargo run --features network -- web

# All clients share a specific session
cargo run --features network -- web --default-session shared

# Disable daemon mode (standalone sessions)
cargo run --features network -- web --ephemeral
```

**Benefits of daemon mode:**
- Sessions persist when browser tabs close
- Multiple browsers/tabs can view the same session
- State (windows, workspaces) preserved across reconnections
- Integrates with `cargo run -- ls`, `cargo run -- attach`, and other session commands

**Multi-client behavior:**
- Terminal size uses minimum of all connected client dimensions
- State changes broadcast to all clients in real-time
- Clients notified when others join/leave

### TermOS Flags

All TermOS flags are forwarded to the spawned instance:

```bash
# Theme and appearance
cargo run --features network -- web --theme nord --border-style rounded

# Debug mode
cargo run --features network -- web --debug --show-keys

# ASCII-only mode
cargo run --features network -- web --ascii-only

# Disable animations for instant transitions
cargo run --features network -- web --no-animations
```

### Client Settings

Click the gear button in the browser to access:

- **Renderer**: Auto, WebGL, Canvas, or DOM
- **Font Size**: 10-24px

Settings are persisted in localStorage.

### On a phone

A touch device gets a key bar, carrying the TermOS chord row over the keys a
phone keyboard does not have, and a touch layer on the terminal: a tap is a
click, a long press is a right click, and a press, hold and drag is a drag.

TermOS widens two gestures for a finger, because a cell is about 8px across and
18px tall:

- **A pane division** can be grabbed from the columns either side of it, not
  just the one it is drawn in.
- **A long press on a pane** opens the pane menu even while you are typing in
  it. That menu is the finger-sized way to close, zoom, rename or split, since
  the title bar's own buttons are one row tall. A pointer reaches the same menu
  with ctrl or shift held, as before.

Neither changes anything for a pointer. Whether a client is a finger is decided
from the connection's user agent, which is a guess: the client does not put the
answer on the wire, and Safari on an iPad asking for the desktop site has no
answer at all. `--touch on` and `--touch off` settle it by hand.

## Transport Protocols

### WebSocket (Primary)

- **Port**: Same as HTTP (default: 7681)
- **Protocol**: WebSocket over TCP
- **Benefits**: Universal browser support, reliable framing
- **Used by**: All browsers

TermOS uses WebSocket as its primary transport. The upstream Go project also
supports WebTransport (HTTP/3 over QUIC), but TermOS currently implements
WebSocket only for simplicity and broad compatibility.

### TLS (Optional)

When built with the `tls` feature, TermOS can serve HTTPS/WSS:

```bash
# Self-signed certificate (auto-generated)
cargo run --features network,tls -- web --host 0.0.0.0 --auto-tls

# Custom certificate
cargo run --features network,tls -- web --host 0.0.0.0 --cert cert.pem --key key.pem
```

## Rendering

### WebGL (Default)

GPU-accelerated rendering using xterm.js WebGL addon:
- Smooth 60fps scrolling and updates
- Lower CPU usage
- Hardware-accelerated text rendering

### Canvas (Fallback)

2D canvas rendering:
- Good performance on most devices
- Used when WebGL unavailable or context lost

### DOM (Fallback)

Standard DOM-based rendering:
- Most compatible option
- Higher CPU usage
- Used when Canvas addon unavailable

## Performance

### Server Optimizations

- **Buffer Pools**: Reusable buffers reduce allocation pressure
- **Direct Streaming**: No intermediate buffering for PTY output
- **Structured Logging**: Configurable log levels via `--debug`

### Client Optimizations

- **requestAnimationFrame Batching**: Terminal writes batched per frame
- **Mouse Deduplication**: Only sends events when cell position changes
- **Pre-allocated Buffers**: Reusable send/receive buffers
- **Cached DOM Elements**: No repeated querySelector calls

### Typical Performance

| Metric | Value |
|--------|-------|
| Latency (local) | <5ms |
| Latency (LAN) | <20ms |
| Mouse events filtered | 80-95% |
| Memory (per session) | ~10MB |

## Security

### Certificate Handling

For development, TermOS can generate a self-signed certificate (requires `tls` feature):
- Valid for 365 days by default (configurable via `--cert-days`)
- Hash provided via `/cert-hash` endpoint
- No browser certificate warning needed for WebSocket Secure

### Binding a LAN address (reaching the server from a phone)

`--host localhost` keeps traffic inside the machine, so it needs no
certificate. Any other host is on a network, where an unencrypted terminal
means every keystroke is readable by anyone else on it, so TermOS
refuses that bind until you say which way you want it:

```bash
# Over HTTPS, from a certificate TermOS generates on first use and keeps
cargo run --features network,tls -- web --host 192.168.1.31 --auto-tls

# Over HTTPS, from a certificate you already have
cargo run --features network,tls -- web --host 192.168.1.31 --cert cert.pem --key key.pem

# In clear text, on a network you trust and no other
cargo run --features network -- web --host 192.168.1.31 --insecure
```

`--auto-tls` uses a keypair TermOS manages for this user, in `termos` inside
your user config directory. It signs for `localhost`, this machine's hostname
and `hostname.local`, and every non-loopback address on every interface, so the
LAN address you actually type works. `--cert-host` adds names only your
router's DNS knows. A certificate that stops covering the address being bound,
which is what a moved DHCP lease looks like, is regenerated rather than served
into a name mismatch the browser will not let you click through.

The certificate signs for itself, so **the first visit from any browser shows a
warning**: "Your connection is not private", `NET::ERR_CERT_AUTHORITY_INVALID`,
or "Potential Security Risk Ahead". That is expected. Choose Advanced, then
Proceed. The connection is encrypted either way; what the browser cannot do is
vouch for who is on the other end. To stop seeing it, copy the `.crt` to the
device and install it as a trusted certificate: on Android under Settings,
Encryption & credentials, Install a certificate, CA certificate; on iOS open
the file, install the profile, then enable it under About, Certificate Trust
Settings. TermOS prints all of this the first time it generates one.

```bash
cargo run --features network,tls -- web cert            # where it is, what it covers, when it expires, its fingerprint
cargo run --features network,tls -- web cert new        # generate one (--force to replace an existing one)
cargo run --features network,tls -- web cert rm --force # delete it
cargo run --features network,tls -- web cert path       # just the path, for a unit file (--key for the key's)
```

No command in this group asks a question, and neither does the refusal above,
so a systemd unit or a container gets the same behaviour and the same exit code
as a shell does.

The private key is written `0600` inside a `0700` directory, and its path is
printed by `cargo run --features network,tls -- web cert path --key` and nowhere else.

### Production Recommendations

1. Use a reverse proxy (nginx, Caddy) with proper TLS
2. Set `--host 127.0.0.1` and proxy external traffic
3. Use `--max-connections` to limit resource usage
4. Consider `--read-only` for public demos

### CORS

All origins allowed by default. For production, configure allowed origins in the server config.

## Troubleshooting

### WebSocket Not Connecting

1. Check browser console for errors
2. Verify the server is running (`curl http://localhost:7681`)
3. Check firewall rules for the configured port
4. Verify TLS configuration if using HTTPS

### Blank Terminal

1. Check browser console for errors
2. Verify fonts loaded (`document.fonts.check()`)
3. Try switching renderer in settings
4. Check if TermOS process started (server logs)

### High Latency

1. Check network conditions
2. Use WebGL renderer for smoother updates
3. Check server CPU usage
4. Reduce number of active windows

### Session Not Closing

If pressing `q` doesn't close the web session:
1. Server sends `MsgClose` when PTY exits
2. Check for browser console errors
3. Verify session cleanup in server logs

### Debug Mode

```bash
# Enable verbose logging
cargo run --features network -- web --debug
```

Server logs include:
- Connection attempts and session lifecycle
- Bytes sent/received per session
- Terminal resize events
- Error details

---

## Related Documentation

- [CLI Reference](CLI_REFERENCE.md) - Complete command reference
- [Configuration](CONFIGURATION.md) - TOML configuration options
- [Keybindings](KEYBINDINGS.md) - Keyboard shortcuts
- [Architecture](ARCHITECTURE.md) - Technical architecture
- [SIP_LIBRARY.md](SIP_LIBRARY.md) - Upstream Go sip library reference
- [MULTI_CLIENT.md](MULTI_CLIENT.md) - Multi-client session behavior
