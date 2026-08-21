# AGENTS.md - Agent Guide for TermOS

This file is for an agent working on the Rust port of TUIOS.

## Project Overview

TermOS is a Rust port of [TUIOS](https://github.com/Gaurav-Gosain/tuios),
the terminal multiplexer and window manager. It uses ratatui for rendering,
crossterm for input, and nix for PTY management.

## Essential Commands

### Build & Run

```bash
# Build
cargo build
cargo build --release

# Build with network features (SSH/web)
cargo build --features network
cargo build --features tls

# Run
cargo run --            # TUI
cargo run -- daemon     # session daemon
cargo run -- tape play examples/demo.tape
```

### Testing

```bash
# All tests  (capped at 4 threads via .cargo/config.toml — do NOT override
# to a higher value; the system PTY ceiling is 4096 and near-exhaustion
# blocks the machine for other users)
cargo test

# Specific test suite
cargo test --test vt          # VT conformance
cargo test --test daemon      # daemon/session
cargo test --test bsp         # BSP tiling
cargo test --test tape_parse_examples  # tape parsing

# Clippy
cargo clippy --all-targets
cargo clippy --all-targets --features network
```

### PTY Limit

The system `/proc/sys/kernel/pty/max` is 4096. PTY-heavy test suites
(daemon, control\_surface, network, theme\_detect\_osc) can exhaust the
pool and starve other processes. Three controls keep this safe:

1. **`.cargo/config.toml` — `test-threads = 4`**: limits concurrency.
2. **`PTY_POOL_CAPACITY = 4`** (`src/terminal/pty.rs`): back-pressure
   semaphore blocks rather than failing when all slots are occupied.
3. **`skip_if_pty_exhausted!()` / `pty_is_available()`**: any test that
   spawns a real PTY checks for ≥20 free slots and skips gracefully when
   the system is loaded.

Every new PTY-spawning test **must** call `crate::skip_if_pty_exhausted!()`
(unit tests) or `if !termos::testutil::pty_is_available() { return; }`
(integration tests) at its top.

## Code Organization

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the full module map.

## Coding Conventions

- Follow idiomatic Rust (clippy clean, no warnings)
- Use `Result<T, E>` for fallible operations; `Box<dyn Error>` for simple cases
- Prefer `&str` over `String` in function arguments
- Use `Arc<Mutex<T>>` for shared state across threads (PTY reader threads)
- Tests are in `tests/` (integration) and `#[cfg(test)] mod tests` (unit)

## Commit Message Format

Use conventional commits:
- `feat:` - New feature
- `fix:` - Bug fix
- `docs:` - Documentation changes
- `refactor:` - Code refactoring
- `test:` - Adding or updating tests
- `chore:` - Maintenance tasks

## Key Dependencies

- `ratatui` 0.29 — TUI rendering
- `crossterm` 0.28 — terminal I/O
- `nix` 0.29 — PTY management
- `crossbeam-channel` 0.5 — cross-thread communication
- `toml` 0.8 — configuration
- `sha2` 0.10 — tape trust store hashing
- `russh` 0.45 (optional) — SSH server
- `axum` 0.7 (optional) — web server
- `rustls` 0.23 (optional) — TLS

## Testing Approach

- VT conformance tests in `tests/vt.rs` verify escape sequence handling
- Daemon tests in `tests/daemon.rs` verify session management
- Tape parsing tests in `tests/tape_parse_examples.rs` verify all Go
  example tapes parse correctly
- Unit tests in each module verify individual components
- Live verification via tmux for TUI testing
