# Contributing to TermOS

Thank you for considering contributing to TermOS! This guide will help you get started.

## Ways to Contribute

- **Bug Reports**: Open an issue with a clear description and reproduction steps
- **Feature Requests**: Open an issue describing the feature and use case
- **Code Contributions**: Submit pull requests for bug fixes, features, or improvements
- **Documentation**: Improve or expand documentation in `docs/` or README
- **Testing**: Test on different platforms and report issues

## Development Setup

### Prerequisites

- **Rust 1.70+** (required for building)
- A terminal with true color support
- Git

### Quick Start

```bash
# Clone the repository
git clone https://github.com/Gaurav-Gosain/tuios.git
cd termos

# Build
cargo build

# Run
cargo run

# Run tests
cargo test

# Run with network features (SSH/web)
cargo build --features network
cargo build --features tls

# Clippy
cargo clippy --all-targets --all-features -- -D warnings

# Release build
cargo build --release
```

### Nix

```bash
nix develop    # Enter development shell
nix build      # Build package
nix run        # Run directly
```

## Code Organization

See [docs/ARCHITECTURE.md](ARCHITECTURE.md) for the full module map.

Key modules:
- `src/app/` — Core window manager, message pump, rendering
- `src/vt/` — VT terminal emulation (parser, emulator, screen, scrollback)
- `src/session/` — Daemon, session management, protocol
- `src/layout/` — BSP tiling
- `src/network/` — SSH server, web server, TLS
- `src/config/` — Configuration, keybindings, themes
- `src/tape/` — Tape scripting automation
- `src/graphics/` — Kitty/sixel graphics passthrough

## Coding Conventions

- Follow idiomatic Rust (clippy clean, no warnings)
- Use `Result<T, E>` for fallible operations
- Prefer `&str` over `String` in function arguments
- Use `Arc<Mutex<T>>` for shared state across threads
- Tests in `tests/` (integration) and `#[cfg(test)] mod tests` (unit)

## Commit Message Format

Use conventional commits:
- `feat:` - New feature
- `fix:` - Bug fix
- `docs:` - Documentation changes
- `refactor:` - Code refactoring
- `test:` - Adding or updating tests
- `chore:` - Maintenance tasks

## Testing

```bash
# All tests
cargo test

# Specific test suite
cargo test --test vt          # VT conformance
cargo test --test daemon      # daemon/session
cargo test --test bsp         # BSP tiling
cargo test --test proptest_vt # VT property tests
cargo test --test proptest_bsp # BSP property tests

# All features
cargo test --all-features
```

## Release Process

Releases are automated via GitHub Actions:
- Tag format: `v*.*.*` (e.g. `v0.1.0`)
- Builds for Linux (macOS/Windows where supported)
- Published as GitHub Releases
