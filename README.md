# TermOS

A Rust port of [TUIOS](https://github.com/Gaurav-Gosain/tuios), the terminal
multiplexer and window manager.

## Features

- **Modal TUI**: vim-like window-management and terminal modes with
  tmux-style leader prefixes (`Ctrl+B`)
- **BSP tiling**: binary space partition layout with master-stack and
  scrolling modes
- **Multi-workspace**: up to 9 workspaces with `Alt+1-9` switching
- **VT emulation**: full ANSI/VT100 parser with scrollback, alternate
  screen, OSC 52 clipboard, and mouse support
- **Tape scripting**: record and replay terminal automation with a
  secure per-machine trust store
- **Graphics passthrough**: Kitty graphics protocol and Sixel forwarding
  with placement tracking (no rasterization)
- **Session daemon**: Unix-socket daemon for session persistence and
  multi-client attach
- **Network modes** (optional): SSH server (russh) and web terminal
  (axum + xterm.js) behind the `network` cargo feature
- **Hooks**: shell-command lifecycle hooks (TOML config)
- **Agent state**: `--skill` mode for AI agents driving TermOS panes

## Quick Start

```bash
# Build
cargo build --release

# Run the TUI
./target/release/tuios

# Start a session daemon
./target/release/tuios daemon &

# Create and attach a session
./target/release/tuios run my-session
./target/release/tuios attach my-session

# Play a tape
./target/release/tuios tape play examples/demo.tape
```

## Network Modes

```bash
# Build with network support
cargo build --release --features network

# SSH server (requires a host key)
ssh-keygen -t ed25519 -f ~/.ssh/tuios_host_key -N ""
./target/release/tuios --network ssh --host-key ~/.ssh/tuios_host_key

# Web terminal
./target/release/tuios --network web --addr 0.0.0.0:8080
```

## Documentation

- [Architecture](docs/ARCHITECTURE.md)
- [CLI Reference](docs/CLI_REFERENCE.md)
- [Tape Scripting](docs/TAPE_SCRIPTING.md)
- [Graphics Passthrough](docs/GRAPHICS.md)
- [Daemon & Sessions](docs/DAEMON.md)

## Testing

```bash
cargo test                # 240 tests
cargo clippy --all-targets
cargo test --test vt      # VT conformance
cargo test --test daemon  # session management
```

## License

MIT
