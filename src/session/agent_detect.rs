//! Foreground-process agent detection — ported from Go TUIOS
//! `internal/session/agent_detect.go`.
//!
//! Detects AI-agent CLIs running in a pane by reading `/proc/<pid>/stat`,
//! `/proc/<pid>/cmdline`, and `/proc/<pid>/exe`. The foreground process group
//! leader of the PTY's controlling terminal is resolved via `tcgetpgrp`, and
//! its comm/argv/exe are matched against a built-in list plus any configured
//! extras. Wrapper interpreters (node, python, npx, …) are resolved through to
//! the actual binary named in their arguments.

use std::collections::HashSet;
use std::os::unix::io::RawFd;
use std::path::Path;

/// Built-in agent binary names the detector recognises. Users extend this list
/// via config; they do not replace it. Matching is on the binary's base name,
/// so a full path resolves the same.
pub const DEFAULT_AGENT_BINARIES: &[&str] = &[
    "claude",
    "claude-code",
    "codex",
    "aider",
    "cursor-agent",
    "opencode",
    "goose",
    "crush",
    "gemini",
    "amp",
    "droid",
    "cline",
    "kilocode",
    "auggie",
    "octofriend",
    "qwen",
];

/// Interpreters and launchers that run an agent as a script rather than being
/// the agent themselves. When the foreground process is one of these, the
/// detector also inspects the command-line arguments.
pub const WRAPPER_INTERPRETERS: &[&str] = &[
    "node", "nodejs", "deno", "bun", "python", "python2", "python3", "uv", "uvx", "npx", "pnpm",
    "yarn", "bunx", "sh", "bash", "zsh", "fish", "env",
];

/// Script extensions stripped from an argv base name before matching.
const SCRIPT_EXTENSIONS: &[&str] = &[".js", ".mjs", ".cjs", ".ts", ".py"];

/// Login shells whose names are noise in a row label.
const LOGIN_SHELLS: &[&str] = &[
    "sh", "bash", "zsh", "fish", "dash", "ksh", "csh", "tcsh", "nu", "xonsh", "elvish", "pwsh",
    "powershell", "cmd",
];

/// Three descriptions of the same foreground process. None is reliable on its
/// own, so the detector needs all of them.
#[derive(Debug, Clone, Default)]
pub struct ProcessInfo {
    /// `/proc/<pid>/comm`: the process name, truncated at 15 chars, rewritable.
    pub comm: String,
    /// `/proc/<pid>/cmdline`: the full command line, NUL-separated.
    pub cmdline: Vec<String>,
    /// `/proc/<pid>/exe`: the resolved executable, empty when unreadable.
    pub exe: String,
}

/// A matched agent: which name matched, which binary, and where it came from.
#[derive(Debug, Clone)]
pub struct AgentMatch {
    /// The harness id when matched via the registry, otherwise the binary name.
    pub name: String,
    /// The binary base name that matched.
    pub binary: String,
    /// Where the match came from.
    pub source: MatchSource,
}

/// Where an `AgentMatch` came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchSource {
    /// Matched the built-in or configured name list.
    Builtin,
    /// Matched via the harness manifest registry.
    Registry,
}

/// Reduce a comm value or argv token to the base name used for matching:
/// no directory, no trailing NUL, no login-shell `-` prefix, no script
/// extension, lowercased.
pub fn agent_base_name(s: &str) -> String {
    let s = s.trim().trim_end_matches('\0');
    if s.is_empty() {
        return String::new();
    }
    let base = Path::new(s)
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| s.to_string());
    let base = base.strip_prefix('-').unwrap_or(&base);
    let lower = base.to_lowercase();
    for ext in SCRIPT_EXTENSIONS {
        if let Some(before) = lower.strip_suffix(ext) {
            return before.to_string();
        }
    }
    lower
}

/// Whether a base name is a wrapper interpreter.
pub fn is_wrapper_name(base: &str) -> bool {
    WRAPPER_INTERPRETERS.contains(&base)
}

/// Whether a base name is a login shell.
pub fn is_login_shell(base: &str) -> bool {
    LOGIN_SHELLS.contains(&base)
}

/// The label a pane earns from what it is running: the base name of the
/// foreground process, or empty when that is just a shell.
pub fn foreground_command(info: &ProcessInfo, running: bool, shell: &str) -> String {
    if !running {
        return String::new();
    }
    let mut name = String::new();
    if !info.cmdline.is_empty() {
        name = agent_base_name(&info.cmdline[0]);
    }
    if name.is_empty() {
        name = agent_base_name(&info.comm);
    }
    if name.is_empty() || name == shell || is_login_shell(&name) {
        return String::new();
    }
    name
}

/// Detect the foreground process of the controlling terminal for `pty_fd`.
///
/// Uses `tcgetpgrp` to find the foreground process group leader, then reads
/// `/proc/<pid>/stat`, `/proc/<pid>/cmdline`, and `/proc/<pid>/exe`. Returns
/// `None` on non-Linux platforms, when procfs is unavailable, or when the
/// process has exited.
pub fn detect_foreground_process(pty_fd: RawFd) -> Option<ProcessInfo> {
    let pid = tcgetpgrp(pty_fd)?;
    if pid <= 0 {
        return None;
    }
    let info = ProcessInfo {
        comm: read_comm(pid),
        cmdline: read_cmdline(pid),
        exe: read_exe(pid),
    };
    if info.comm.is_empty() && info.cmdline.is_empty() {
        return None;
    }
    Some(info)
}

/// `tcgetpgrp` wrapper. Returns the foreground process group id, or `None`
/// on failure or non-Linux platforms.
#[cfg(target_os = "linux")]
fn tcgetpgrp(fd: RawFd) -> Option<i32> {
    // SAFETY: tcgetpgrp is a simple libc call that reads the foreground pgrp
    // of a terminal. The fd is valid because the caller owns it.
    let pgrp = unsafe { nix::libc::tcgetpgrp(fd) };
    if pgrp < 0 {
        None
    } else {
        Some(pgrp)
    }
}

#[cfg(not(target_os = "linux"))]
fn tcgetpgrp(_fd: RawFd) -> Option<i32> {
    None
}

/// Read `/proc/<pid>/comm`, trimmed, or empty on error.
#[cfg(target_os = "linux")]
fn read_comm(pid: i32) -> String {
    let path = format!("/proc/{}/comm", pid);
    match std::fs::read_to_string(&path) {
        Ok(s) => s.trim().to_string(),
        Err(_) => String::new(),
    }
}

#[cfg(not(target_os = "linux"))]
fn read_comm(_pid: i32) -> String {
    String::new()
}

/// Read `/proc/<pid>/cmdline`, NUL-separated arguments, or empty on error.
#[cfg(target_os = "linux")]
fn read_cmdline(pid: i32) -> Vec<String> {
    let path = format!("/proc/{}/cmdline", pid);
    let data = match std::fs::read(&path) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };
    if data.is_empty() {
        return Vec::new();
    }
    let trimmed = data.split(|&b| b == 0).filter(|s| !s.is_empty());
    trimmed
        .map(|s| String::from_utf8_lossy(s).to_string())
        .collect()
}

#[cfg(not(target_os = "linux"))]
fn read_cmdline(_pid: i32) -> Vec<String> {
    Vec::new()
}

/// Read `/proc/<pid>/exe` (the symlink target), stripping the
/// ` (deleted)` suffix, or empty on error.
#[cfg(target_os = "linux")]
fn read_exe(pid: i32) -> String {
    let path = format!("/proc/{}/exe", pid);
    match std::fs::read_link(&path) {
        Ok(target) => {
            let s = target.to_string_lossy().to_string();
            s.strip_suffix(" (deleted)").unwrap_or(&s).to_string()
        }
        Err(_) => String::new(),
    }
}

#[cfg(not(target_os = "linux"))]
fn read_exe(_pid: i32) -> String {
    String::new()
}

/// Match a foreground process against the built-in list plus configured extras.
///
/// The built-in list and the `agent_binaries` config list are merged into a
/// case-insensitive set. Matching checks `comm`, `exe`, and the arguments of
/// wrapper interpreters. Returns `Some(AgentMatch)` when an agent is detected.
pub fn match_agent(info: &ProcessInfo, agent_binaries: &[String]) -> Option<AgentMatch> {
    let names: HashSet<String> = DEFAULT_AGENT_BINARIES
        .iter()
        .map(|s| s.to_string())
        .chain(
            agent_binaries
                .iter()
                .map(|s| s.trim().to_lowercase())
                .filter(|s| !s.is_empty()),
        )
        .collect();

    // comm or exe base name directly matches.
    let comm_base = agent_base_name(&info.comm);
    let exe_base = agent_base_name(&info.exe);
    if !comm_base.is_empty() && names.contains(&comm_base) {
        return Some(AgentMatch {
            name: comm_base.clone(),
            binary: comm_base,
            source: MatchSource::Builtin,
        });
    }
    if !exe_base.is_empty() && names.contains(&exe_base) {
        return Some(AgentMatch {
            name: exe_base.clone(),
            binary: exe_base,
            source: MatchSource::Builtin,
        });
    }

    // The install path names the agent even when the binary does not (e.g.
    // Claude Code's ".../share/claude/versions/2.1.222").
    if !info.exe.is_empty() && arg_names_agent(&info.exe, &names) {
        // Find which component matched.
        if let Some(name) = path_component_match(&info.exe, &names) {
            return Some(AgentMatch {
                name: name.clone(),
                binary: name,
                source: MatchSource::Builtin,
            });
        }
    }

    // An interpreter is a stand-in for the script it runs. Either name can be
    // the interpreter: a wrapper script sets comm while exe stays "node", and
    // a renamed process does the reverse.
    let comm_is_wrapper = comm_base.is_empty() || is_wrapper_name(&comm_base);
    let exe_is_wrapper = !exe_base.is_empty() && is_wrapper_name(&exe_base);
    if !comm_is_wrapper && !exe_is_wrapper {
        return None;
    }

    // Scan each argument's path components for a known agent name.
    for arg in &info.cmdline {
        if arg_names_agent(arg, &names) {
            if let Some(name) = path_component_match(arg, &names) {
                return Some(AgentMatch {
                    name: name.clone(),
                    binary: name,
                    source: MatchSource::Builtin,
                });
            }
        }
    }

    None
}

/// Whether any path component of a single argv token, reduced to a base name,
/// is a known agent name.
fn arg_names_agent(arg: &str, names: &HashSet<String>) -> bool {
    let arg = arg.trim_end_matches('\0');
    if arg.is_empty() {
        return false;
    }
    for comp in arg.split('/') {
        if comp.is_empty() {
            continue;
        }
        let base = agent_base_name(comp);
        if !base.is_empty() && names.contains(&base) {
            return true;
        }
    }
    false
}

/// Return the first path component (as a base name) that matches a known agent.
fn path_component_match(arg: &str, names: &HashSet<String>) -> Option<String> {
    let arg = arg.trim_end_matches('\0');
    for comp in arg.split('/') {
        if comp.is_empty() {
            continue;
        }
        let base = agent_base_name(comp);
        if !base.is_empty() && names.contains(&base) {
            return Some(base);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_name_strips_path() {
        assert_eq!(agent_base_name("/usr/bin/aider"), "aider");
    }

    #[test]
    fn base_name_strips_login_prefix() {
        assert_eq!(agent_base_name("-bash"), "bash");
    }

    #[test]
    fn base_name_lowercases() {
        assert_eq!(agent_base_name("Aider"), "aider");
    }

    #[test]
    fn base_name_strips_extension() {
        assert_eq!(agent_base_name("aider.js"), "aider");
        assert_eq!(agent_base_name("aider.py"), "aider");
    }

    #[test]
    fn base_name_strips_nul() {
        assert_eq!(agent_base_name("aider\0"), "aider");
    }

    #[test]
    fn match_agent_by_comm() {
        let info = ProcessInfo {
            comm: "claude".to_string(),
            cmdline: vec!["claude".to_string()],
            exe: "/usr/bin/claude".to_string(),
        };
        let m = match_agent(&info, &[]).unwrap();
        assert_eq!(m.binary, "claude");
        assert_eq!(m.source, MatchSource::Builtin);
    }

    #[test]
    fn match_agent_by_exe() {
        let info = ProcessInfo {
            comm: "claude".to_string(),
            cmdline: vec![],
            exe: "/usr/share/claude/versions/2.1.222".to_string(),
        };
        let m = match_agent(&info, &[]).unwrap();
        assert_eq!(m.binary, "claude");
    }

    #[test]
    fn match_agent_wrapper_node() {
        let info = ProcessInfo {
            comm: "node".to_string(),
            cmdline: vec![
                "node".to_string(),
                "/usr/lib/node_modules/@anthropic-ai/claude-code/cli.js".to_string(),
            ],
            exe: "/usr/bin/node".to_string(),
        };
        let m = match_agent(&info, &[]).unwrap();
        assert_eq!(m.binary, "claude-code");
    }

    #[test]
    fn match_agent_wrapper_npx() {
        let info = ProcessInfo {
            comm: "npx".to_string(),
            cmdline: vec!["npx".to_string(), "opencode".to_string()],
            exe: "/usr/bin/npx".to_string(),
        };
        let m = match_agent(&info, &[]).unwrap();
        assert_eq!(m.binary, "opencode");
    }

    #[test]
    fn match_agent_no_match_for_non_wrapper() {
        let info = ProcessInfo {
            comm: "bash".to_string(),
            cmdline: vec!["bash".to_string(), "--norc".to_string()],
            exe: "/usr/bin/bash".to_string(),
        };
        assert!(match_agent(&info, &[]).is_none());
    }

    #[test]
    fn match_agent_extra_config_names() {
        let info = ProcessInfo {
            comm: "myagent".to_string(),
            cmdline: vec!["myagent".to_string()],
            exe: "/usr/bin/myagent".to_string(),
        };
        let extras = vec!["myagent".to_string()];
        let m = match_agent(&info, &extras).unwrap();
        assert_eq!(m.binary, "myagent");
    }

    #[test]
    fn match_agent_case_insensitive_extras() {
        let info = ProcessInfo {
            comm: "MyAgent".to_string(),
            cmdline: vec!["MyAgent".to_string()],
            exe: String::new(),
        };
        let extras = vec!["myagent".to_string()];
        let m = match_agent(&info, &extras).unwrap();
        assert_eq!(m.binary, "myagent");
    }

    #[test]
    fn foreground_command_strips_shell() {
        let info = ProcessInfo {
            comm: "bash".to_string(),
            cmdline: vec!["bash".to_string()],
            exe: "/usr/bin/bash".to_string(),
        };
        assert_eq!(foreground_command(&info, true, "bash"), "");
    }

    #[test]
    fn foreground_command_returns_name() {
        let info = ProcessInfo {
            comm: "claude".to_string(),
            cmdline: vec!["claude".to_string()],
            exe: "/usr/bin/claude".to_string(),
        };
        assert_eq!(foreground_command(&info, true, "bash"), "claude");
    }

    #[test]
    fn foreground_command_not_running() {
        let info = ProcessInfo::default();
        assert_eq!(foreground_command(&info, false, "bash"), "");
    }

    #[test]
    fn is_wrapper_name_checks() {
        assert!(is_wrapper_name("node"));
        assert!(is_wrapper_name("python3"));
        assert!(is_wrapper_name("npx"));
        assert!(!is_wrapper_name("claude"));
        assert!(!is_wrapper_name("aider"));
    }

    #[test]
    fn is_login_shell_checks() {
        assert!(is_login_shell("bash"));
        assert!(is_login_shell("zsh"));
        assert!(!is_login_shell("claude"));
    }
}
