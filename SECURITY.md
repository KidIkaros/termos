# Security Policy

## Supported Versions

TermOS is under active development. Security fixes are applied to the latest
`main` branch only; there are no separate maintenance release lines yet.

| Version | Supported          |
|---------|--------------------|
| latest main branch | ✅ Supported |
| older commits / tags | ❌ Not supported |

## Reporting a Vulnerability

**Do not open a public GitHub issue for security vulnerabilities.**

Instead, please report vulnerabilities privately so we can triage and fix
them before public disclosure.

- **Email**: Send a description of the vulnerability, reproduction steps, and
  any relevant proof-of-concept to the maintainer at
  **security@example.com** (replace with the project's real security contact
  once one is established).
- **Encryption**: If you use PGP, request the maintainer's public key first
  and encrypt your report.
- **GitHub Security Advisories**: You may also use GitHub's private
  vulnerability reporting feature on the repository's Security tab.

Please include the following in your report:

1. A clear description of the issue and its impact.
2. Steps to reproduce (minimal example if possible).
3. The TermOS version or commit hash you tested.
4. Your operating system and architecture.
5. Any suggested remediation.

## Response Timeline

| Stage | Target |
|-------|--------|
| Acknowledgment of report | Within 48 hours |
| Initial assessment / triage | Within 5 business days |
| Fix or mitigation released | Within 30 days of acknowledgment |
| Public disclosure | After a fix is released, or 30 days after acknowledgment if no fix is ready (coordinated with reporter) |

We will keep you informed of progress throughout the process and credit
reporters in release notes unless you prefer to remain anonymous.

## Scope

This policy covers **TermOS**, the Rust port of TUIOS, including:

- The `termos` binary (TUI multiplexer, session daemon, tape player).
- The `termos` library crate.
- Optional network features (SSH server via `russh`, web terminal via `axum`)
  enabled with `--features network` or `--features tls`.
- The installation script (`install.sh`) and release artifacts.

### Out of scope

- The upstream **Go** project [TUIOS](https://github.com/Gaurav-Gosain/tuios)
  — vulnerabilities in the Go codebase should be reported to that project's
  maintainers directly.
- Vulnerabilities in third-party dependencies (report upstream to the
  relevant crate). We will still appreciate a heads-up so we can track and
  bump the affected dependency promptly.
- Issues that require already-compromised access to the user's machine or
  root privileges to exploit.

## Security Considerations

TermOS spawns shell processes via PTYs and, in daemon mode, listens on a
Unix socket. When network features are enabled, it also opens SSH and/or
HTTP/WebSocket listeners. Operators should:

- Run the daemon socket with appropriate filesystem permissions (it defaults
  to `$XDG_RUNTIME_DIR/termos/`).
- Never expose the SSH or web server to untrusted networks without
  authentication and TLS.
- Review tape scripts before adding them to the trust store (`termos tape`).
