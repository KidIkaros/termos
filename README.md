# TermOS

A Rust port of [TUIOS](https://github.com/Gaurav-Gosain/tuios), the terminal
multiplexer and window manager. Built with ratatui, crossterm, and nix.

## Features

- **Modal TUI**: vim-like window-management and terminal modes with
  tmux-style leader prefixes (`Ctrl+B`)
- **BSP tiling**: binary space partition layout with master-stack and
  scrolling modes
- **Multi-workspace**: up to 9 workspaces with `Alt+1-9` switching
- **VT emulation**: full ANSI/VT100 parser with scrollback, alternate
  screen, OSC 52 clipboard, and mouse support
- **Vim copy mode**: char search (`f`/`F`/`t`/`T`), word motions
  (`w`/`b`/`e`), regex search (`/`/`?`/`n`/`N`), visual selection (`v`/`V`)
- **Mouse interaction**: border-drag resize, double/triple-click select
  (word/line), auto-copy on release
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
- **Config hot-reload**: edit `config.toml` and changes apply live
- **Custom themes**: load themes from `~/.config/termos/themes/*.json`
- **CLI overrides**: `--theme`, `--border-style`, `--ascii-only`,
  `--no-which-key` flags
- **Help modal**: press `?` in window management mode for keybindings
- **Sound cues**: agent alert sounds via system audio player

## Installation

### From source

```bash
cargo build --release
# Binary at target/release/termos
```

### Install script

```bash
curl -fsSL https://raw.githubusercontent.com/Gaurav-Gosain/tuios/main/install.sh | bash
```

### Docker

```bash
docker build -t termos .
docker run -it termos
```

## Quick Start

```bash
# Run the TUI
./target/release/termos

# With CLI overrides
./target/release/termos --theme dracula --ascii-only

# Start a session daemon
./target/release/termos daemon &

# Create and attach a session
./target/release/termos run my-session
./target/release/termos attach my-session

# Play a tape
./target/release/termos tape play examples/demo.tape
```

## Network Modes

```bash
# Build with network support
cargo build --release --features network

# SSH server (requires a host key)
ssh-keygen -t ed25519 -f ~/.ssh/termos_host_key -N ""
./target/release/termos --network ssh --host-key ~/.ssh/termos_host_key

# Web terminal
./target/release/termos --network web --addr 0.0.0.0:8080
```

## Documentation

- [Architecture](docs/ARCHITECTURE.md)
- [Keybindings](docs/KEYBINDINGS.md)
- [Configuration](docs/CONFIGURATION.md)
- [CLI Reference](docs/CLI_REFERENCE.md)
- [Tape Scripting](docs/TAPE_SCRIPTING.md)
- [Graphics Passthrough](docs/GRAPHICS.md)
- [Daemon & Sessions](docs/DAEMON.md)
- [Sessions](docs/SESSIONS.md)
- [Multi-Client](docs/MULTI_CLIENT.md)
- [Hooks](docs/HOOKS.md)
- [Themes](docs/THEMES.md)
- [BSP Tiling](docs/BSP_TILING.md)
- [Layout Modes](docs/LAYOUT_MODES.md)
- [Agent State](docs/AGENT_STATE.md)
- [Tape Recording](docs/TAPE_RECORDING.md)
- [Project Tapes](docs/PROJECT_TAPES.md)
- [Control Protocol](docs/protocol.md)
- [Rehydration](docs/REHYDRATION.md)
- [Web Terminal](docs/WEB.md)
- [Contributing](CONTRIBUTING.md)

## Testing

```bash
cargo test                          # 275 tests
cargo test --features network       # with network tests
cargo clippy --all-targets -- -D warnings
cargo test --test vt                # VT conformance
cargo test --test daemon            # session management
cargo bench --no-run                # benchmarks compile
cargo +nightly fuzz build           # fuzz targets
```

## License

MIT
