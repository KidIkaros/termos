# Contributing to TermOS

Thank you for considering contributing to TermOS! This guide will help you get started.

TermOS is a Rust port of [TUIOS](https://github.com/Gaurav-Gosain/tuios), the terminal multiplexer and window manager.

## Ways to Contribute

- **Bug Reports**: Use the [bug report template](https://github.com/Gaurav-Gosain/tuios/issues/new?template=bug_report.yml)
- **Feature Requests**: Use the [feature request template](https://github.com/Gaurav-Gosain/tuios/issues/new?template=feature_request.yml)
- **Code Contributions**: Submit pull requests for bug fixes, features, or improvements
- **Documentation**: Improve or expand documentation in `docs/` or README
- **Testing**: Test on different platforms and report issues

**Have questions?** Use [GitHub Discussions](https://github.com/Gaurav-Gosain/tuios/discussions).

---

## Development Setup

### Prerequisites

- **Rust** (stable, latest stable toolchain recommended)
- A terminal with true color support
- Git

### Quick Start

```bash
# Clone the repository
git clone https://github.com/Gaurav-Gosain/tuios.git
cd termos

# Build from source
cargo build

# Run
cargo run

# Run tests
cargo test
```

---

## Project Structure

```
termos/
├── src/
│   ├── main.rs             # CLI entry point
│   ├── lib.rs              # Library root
│   ├── app/                # Window manager and core orchestration
│   ├── config/             # Configuration and keybindings
│   ├── graphics/           # Graphics/ANSI rendering
│   ├── hooks/              # Session event hooks
│   ├── keys.rs             # Key event handling
│   ├── layout/             # Window layout and tiling (BSP)
│   ├── network/            # SSH and web server (optional features)
│   ├── session/            # Daemon session management
│   ├── tape/               # Tape scripting automation
│   ├── terminal/           # Terminal window management
│   ├── ui/                 # UI components
│   └── vt/                 # Terminal emulation (ANSI/VT100)
├── docs/                   # Documentation
├── tests/                  # Integration tests
├── benches/                # Benchmarks
├── examples/               # Tape script examples
└── fuzz/                   # Fuzzing targets
```

See [ARCHITECTURE.md](docs/ARCHITECTURE.md) for detailed technical documentation.

---

## Making Changes

### Before You Start

1. **Check existing issues** to see if someone is already working on it
2. **Open an issue** to discuss major changes before implementing
3. **Fork the repository** and create a new branch for your changes

### Code Guidelines

- Follow idiomatic Rust (clippy clean, no warnings)
- Run `cargo fmt` before committing
- Ensure tests pass: `cargo test`
- Keep commits focused and atomic
- Write clear commit messages

### Pull Request Process

1. **Create a branch** from `main`:
   ```bash
   git checkout -b feat/your-feature-name
   ```

2. **Make your changes** and commit:
   ```bash
   git add .
   git commit -m "feat: add your feature description"
   ```

3. **Test thoroughly** across platforms if possible (see [Testing](#testing) below)

4. **Push and create a PR**:
   ```bash
   git push origin feat/your-feature-name
   ```

5. **Fill out the PR template** with:
   - What the PR does
   - Motivation/context
   - Key changes
   - How to verify
   - Platform(s) tested

### Commit Message Format

Use conventional commit prefixes:
- `feat:` - New feature
- `fix:` - Bug fix
- `docs:` - Documentation changes
- `refactor:` - Code refactoring
- `test:` - Adding or updating tests
- `chore:` - Maintenance tasks

Examples:
```
feat: add configurable dockbar position
fix: panic when closing last window on Linux
docs: update keybindings reference
```

---

## Testing

### Cross-Platform Testing

TermOS supports multiple platforms. If possible, test your changes on:

**Platforms:**
- Linux (x86_64, arm64)
- macOS / Darwin (arm64, x86_64)

**You don't need to test everything** - but mention what you tested in your PR.

### Running Tests

```bash
# Run all tests
cargo test

# Run specific test suite
cargo test --test vt          # VT conformance
cargo test --test daemon      # daemon/session
cargo test --test bsp         # BSP tiling
cargo test --test tape_parse_examples  # tape parsing

# Run with clippy
cargo clippy --all-targets
cargo clippy --all-targets --features network
```

### Manual Testing Checklist

When testing UI/UX changes:
- [ ] Create/close multiple windows
- [ ] Switch between workspaces
- [ ] Test tiling mode
- [ ] Test copy mode navigation
- [ ] Verify keybindings work as expected
- [ ] Check terminal output rendering
- [ ] Test mouse interactions

---

## Documentation

When contributing, consider updating:

- **README.md** - For user-facing features
- **docs/KEYBINDINGS.md** - For new keybindings
- **docs/CONFIGURATION.md** - For configuration options
- **docs/CLI_REFERENCE.md** - For CLI flags/commands
- **docs/ARCHITECTURE.md** - For architectural changes

---

## Code Review Process

1. Maintainer will review your PR
2. Address any requested changes
3. Once approved, your PR will be merged
4. Your contribution will be included in the next release

---

## Release Process

TermOS uses automated releases via GitHub Actions:
- Releases are tagged (e.g., `v0.3.4`)
- Binaries are built for all platforms
- Package managers are updated automatically

You don't need to worry about this as a contributor - the maintainer handles releases.

---

## Getting Help

- **Questions**: [GitHub Discussions](https://github.com/Gaurav-Gosain/tuios/discussions)
- **Bugs**: [Bug Report Template](https://github.com/Gaurav-Gosain/tuios/issues/new?template=bug_report.yml)
- **Features**: [Feature Request Template](https://github.com/Gaurav-Gosain/tuios/issues/new?template=feature_request.yml)

---

## Code of Conduct

Be respectful, constructive, and collaborative. We're all here to make TermOS better.

---

**Thank you for contributing to TermOS!**
