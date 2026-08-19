//! Clipboard integration — OSC 52 and system clipboard shell-out.
//!
//! Supports two paths:
//! 1. OSC 52: works over SSH, supported by most modern terminals.
//! 2. Shell-out: `xclip`/`wl-copy`/`pbcopy` for local clipboard access.
//!
//! Copy mode yank uses OSC 52 first (works everywhere), with shell-out as
//! fallback for terminals that don't support OSC 52.

use std::io::Write;
use std::process::{Command, Stdio};

/// Copy text to the system clipboard.
///
/// Tries OSC 52 first (by writing to stderr, which terminals capture), then
/// falls back to platform-specific clipboard tools.
pub fn copy(text: &str) {
    // Try OSC 52 to stderr — terminals that support it will capture this.
    let osc52 = osc52_copy_command(text);
    let _ = std::io::stderr().write_all(osc52.as_bytes());

    // Also try shell-out to a platform clipboard tool.
    if let Some(tool) = detect_clipboard_tool() {
        let _ = Command::new(tool)
            .args(clipboard_copy_args(tool))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .and_then(|mut child| {
                let stdin = child.stdin.as_mut();
                if let Some(stdin) = stdin {
                    stdin.write_all(text.as_bytes())?;
                }
                child.wait()
            });
    }
}

/// Paste from the system clipboard.
///
/// Tries platform clipboard tools. OSC 52 paste is not supported (it's a
/// terminal-initiated query, not something we can synchronously read).
pub fn paste() -> Option<String> {
    let tool = detect_clipboard_tool()?;
    let output = Command::new(tool)
        .args(clipboard_paste_args(tool))
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        None
    }
}

/// Build the OSC 52 copy escape sequence for the given text.
pub fn osc52_copy_command(text: &str) -> String {
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
    format!("\x1b]52;c;{encoded}\x07")
}

/// Detect the available clipboard tool on this platform.
fn detect_clipboard_tool() -> Option<&'static str> {
    // Check for tools in order of preference.
    let tools = ["wl-copy", "xclip", "xsel", "pbcopy"];
    tools.iter().find(|&&tool| which(tool).is_some()).copied()
}

/// Arguments for the copy command of each tool.
fn clipboard_copy_args(tool: &str) -> Vec<&'static str> {
    match tool {
        "xclip" => vec!["-selection", "clipboard"],
        "xsel" => vec!["--clipboard", "--input"],
        "wl-copy" | "pbcopy" => vec![],
        _ => vec![],
    }
}

/// Arguments for the paste command of each tool.
fn clipboard_paste_args(tool: &str) -> Vec<&'static str> {
    match tool {
        "xclip" => vec!["-selection", "clipboard", "-o"],
        "xsel" => vec!["--clipboard", "--output"],
        "wl-copy" => vec!["--paste"],
        "pbpaste" => vec![],
        _ => vec![],
    }
}

/// Detect the paste tool (may differ from copy tool on some platforms).
fn _detect_paste_tool() -> Option<&'static str> {
    let tools = ["wl-paste", "xclip", "xsel", "pbpaste"];
    tools.iter().find(|&&tool| which(tool).is_some()).copied()
}

/// Simple `which` implementation — checks if a binary is on PATH.
fn which(bin: &str) -> Option<()> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        if dir.join(bin).is_file() {
            return Some(());
        }
    }
    None
}

/// Open a URL or file path using the platform's default handler.
///
/// On Linux: `xdg-open`. On macOS: `open`. On Windows: `start`.
pub fn open_external(target: &str) {
    let (cmd, args) = open_command();
    let _ = Command::new(cmd)
        .args(args)
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

/// Return the platform's open command and its prefix arguments.
fn open_command() -> (&'static str, Vec<&'static str>) {
    #[cfg(target_os = "linux")]
    {
        ("xdg-open", vec![])
    }
    #[cfg(target_os = "macos")]
    {
        ("open", vec![])
    }
    #[cfg(target_os = "windows")]
    {
        ("cmd", vec!["/C", "start"])
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        ("echo", vec![])
    }
}

/// Detect if a string looks like a URL.
pub fn looks_like_url(s: &str) -> bool {
    let s = s.trim();
    s.starts_with("http://") || s.starts_with("https://") || s.starts_with("ftp://")
}

/// Detect if a string looks like a file path.
pub fn looks_like_path(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() {
        return false;
    }
    // Absolute paths.
    if s.starts_with('/') || s.starts_with("~/") {
        return true;
    }
    // Relative paths with common file extensions.
    if let Some(dot) = s.rfind('.') {
        let ext = &s[dot + 1..];
        if ext.len() <= 5 && ext.chars().all(|c| c.is_alphanumeric()) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc52_copy_command_basic() {
        let cmd = osc52_copy_command("hello");
        assert!(cmd.starts_with("\x1b]52;c;"));
        assert!(cmd.ends_with("\x07"));
        // Base64 of "hello" is "aGVsbG8=".
        assert!(cmd.contains("aGVsbG8="));
    }

    #[test]
    fn osc52_copy_command_empty() {
        let cmd = osc52_copy_command("");
        assert!(cmd.contains(";\x07") || cmd.contains(";c;\x07"));
    }

    #[test]
    fn osc52_copy_command_unicode() {
        let cmd = osc52_copy_command("héllo");
        // Should contain valid base64.
        assert!(cmd.starts_with("\x1b]52;c;"));
    }

    #[test]
    fn clipboard_copy_args_xclip() {
        let args = clipboard_copy_args("xclip");
        assert_eq!(args, vec!["-selection", "clipboard"]);
    }

    #[test]
    fn clipboard_copy_args_wl_copy() {
        let args = clipboard_copy_args("wl-copy");
        assert!(args.is_empty());
    }

    #[test]
    fn clipboard_paste_args_xclip() {
        let args = clipboard_paste_args("xclip");
        assert_eq!(args, vec!["-selection", "clipboard", "-o"]);
    }

    #[test]
    fn looks_like_url_http() {
        assert!(looks_like_url("http://example.com"));
        assert!(looks_like_url("https://example.com/path"));
        assert!(looks_like_url("ftp://example.com"));
    }

    #[test]
    fn looks_like_url_not_url() {
        assert!(!looks_like_url("hello world"));
        assert!(!looks_like_url("/usr/local/bin"));
        assert!(!looks_like_url(""));
    }

    #[test]
    fn looks_like_path_absolute() {
        assert!(looks_like_path("/usr/local/bin"));
        assert!(looks_like_path("~/Documents/file.txt"));
    }

    #[test]
    fn looks_like_path_relative_with_ext() {
        assert!(looks_like_path("file.txt"));
        assert!(looks_like_path("report.pdf"));
    }

    #[test]
    fn looks_like_path_not_path() {
        assert!(!looks_like_path(""));
        assert!(!looks_like_path("hello world"));
        assert!(!looks_like_path("justtext"));
    }
}
