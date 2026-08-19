//! End-to-end test of the OSC 11 light/dark theme detection.
//!
//! Plays the host terminal for the real `termos` binary inside a PTY: the
//! startup OSC 11 query (`ESC ] 11 ; ?`) is answered with a LIGHT background,
//! and the test asserts the dockbar paints catppuccin-latte. It then drives
//! the palette "Re-detect light/dark theme" command through real keystrokes,
//! answers the second query with a DARK background, and asserts the dockbar
//! swaps to catppuccin-mocha with a "dark detected" notification.
//!
//! This exercises the full path that unit tests can't: the query bytes over a
//! real PTY, the raw-mode reply read from stdin, and the live theme swap.

use std::ffi::CString;
use std::io::{Read, Write};
use std::os::fd::{AsFd, AsRawFd, FromRawFd, IntoRawFd};

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use nix::poll::{poll, PollFd, PollFlags};
use nix::pty::{forkpty, ForkptyResult, Winsize};
use nix::sys::signal::{killpg, Signal};
use nix::unistd::{dup, execve, Pid};

/// The OSC 11 query the TUI sends to learn the terminal's background.
const QUERY: &[u8] = b"\x1b]11;?\x07";
/// A light background answer (catppuccin-latte-ish off-white).
const LIGHT_ANSWER: &[u8] = b"\x1b]11;rgb:efef/e9e9/d9d9\x07";
/// A dark background answer (catppuccin-mocha-ish near-black).
const DARK_ANSWER: &[u8] = b"\x1b]11;rgb:1e1e/2e2e/2e2e\x07";
/// catppuccin-latte ansi[0] `#DCE0E8` — the dock background when light.
const LATTE_DOCK_BG: &[u8] = b"48;2;220;224;232";
/// catppuccin-mocha ansi[0] `#1E1E2E` — the dock background when dark.
const MOCHA_DOCK_BG: &[u8] = b"48;2;30;30;46";
/// Substring of the re-detect notification text.
const DARK_DETECTED_NOTICE: &[u8] = b"dark detected";

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// Poll the accumulated TUI output for `needle` until the deadline.
fn wait_for(buf: &Arc<Mutex<Vec<u8>>>, needle: &[u8], timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if contains(&buf.lock().unwrap(), needle) {
            return true;
        }
        thread::sleep(Duration::from_millis(50));
    }
    false
}

/// Build the child's environment: inherited, minus COLORFGBG (so the OSC
/// answer is the only detection signal), plus isolated XDG dirs.
fn child_env(config: &Path, state: &Path, data: &Path) -> Vec<CString> {
    let overrides = [
        ("XDG_CONFIG_HOME", config.display().to_string()),
        ("XDG_STATE_HOME", state.display().to_string()),
        ("XDG_DATA_HOME", data.display().to_string()),
        ("TERM", "xterm-256color".into()),
        ("COLORTERM", "truecolor".into()),
        ("SHELL", "/bin/bash".into()),
    ];
    let mut env: Vec<CString> = std::env::vars()
        .filter(|(k, _)| {
            k != "COLORFGBG" && !overrides.iter().any(|(ok, _)| ok == k)
        })
        .map(|(k, v)| CString::new(format!("{k}={v}")).unwrap())
        .collect();
    for (k, v) in overrides {
        env.push(CString::new(format!("{k}={v}")).unwrap());
    }
    env
}

/// Kill the TUI's process group on drop so a failed assertion cannot leak
/// the child (and its shell) into the test run.
struct ChildGuard {
    pid: Pid,
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = killpg(self.pid, Signal::SIGKILL);
        let _ = nix::sys::wait::waitpid(self.pid, None);
    }
}

#[test]
fn osc11_queries_drive_auto_theme_and_live_swap() {
    // Isolated config with `theme = "auto"` (defaults pair latte/mocha).
    let config_dir = tempfile::tempdir().unwrap();
    let state_dir = tempfile::tempdir().unwrap();
    let data_dir = tempfile::tempdir().unwrap();
    let config_path = config_dir.path().join("termos").join("config.toml");
    std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
    let mut cfg = termos::config::UserConfig::default_config();
    cfg.appearance.theme = "auto".into();
    let toml = toml::to_string_pretty(&cfg).unwrap();
    std::fs::write(&config_path, toml).unwrap();

    let bin = CString::new(env!("CARGO_BIN_EXE_termos")).unwrap();
    let argv = [bin.clone()];
    let env = child_env(config_dir.path(), state_dir.path(), data_dir.path());

    // Fork a PTY with a real window size (the TUI renders 0x0 without one).
    let winsize = Winsize { ws_row: 24, ws_col: 80, ws_xpixel: 0, ws_ypixel: 0 };
    let mut forked = None;
    for _ in 0..15 {
        match unsafe { forkpty(Some(&winsize), None) } {
            Ok(f) => {
                forked = Some(f);
                break;
            }
            Err(e) if e == nix::errno::Errno::ENOSPC || e == nix::errno::Errno::EIO => {
                // PTY pressure on loaded machines; retry briefly.
                thread::sleep(Duration::from_millis(150));
            }
            Err(e) => panic!("forkpty failed: {e}"),
        }
    }
    let (mut master, child_pid) = match forked.expect("forkpty kept failing (PTY pressure)") {
        ForkptyResult::Parent { child, master } => {
            (unsafe { std::fs::File::from_raw_fd(master.into_raw_fd()) }, child)
        }
        ForkptyResult::Child => {
            // execve never returns on success.
            let _ = execve(&bin, &argv, &env);
            std::process::exit(127);
        }
    };
    let _guard = ChildGuard { pid: child_pid };

    // Reader thread: accumulate the TUI's output and answer each OSC 11
    // query: the first gets LIGHT, later ones get DARK.
    let reader_fd = dup(master.as_raw_fd()).unwrap();
    let reader_file = unsafe { std::fs::File::from_raw_fd(reader_fd) };
    let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
    let queries = Arc::new(AtomicUsize::new(0));
    let buf_t = Arc::clone(&buf);
    let queries_t = Arc::clone(&queries);
    thread::spawn(move || {
        let mut file = reader_file;
        let mut chunk = [0u8; 8192];
        loop {
            let mut fds = [PollFd::new(file.as_fd(), PollFlags::POLLIN)];
            match poll(&mut fds, 50u16) {
                Ok(0) => continue,
                Ok(_) => {}
                Err(_) => break,
            }
            match file.read(&mut chunk) {
                Ok(0) => break, // child closed
                Ok(n) => {
                    let mut b = buf_t.lock().unwrap();
                    b.extend_from_slice(&chunk[..n]);
                    while let Some(idx) = b.windows(QUERY.len()).position(|w| w == QUERY) {
                        let n = queries_t.fetch_add(1, Ordering::SeqCst) + 1;
                        let answer = if n == 1 { LIGHT_ANSWER } else { DARK_ANSWER };
                        file.write_all(answer).unwrap();
                        b.drain(..idx + QUERY.len());
                    }
                }
                Err(_) => break,
            }
        }
    });

    // 1. Startup detection: light answer → latte dockbar.
    assert!(
        wait_for(&buf, LATTE_DOCK_BG, Duration::from_secs(20)),
        "startup did not apply the light theme (expected catppuccin-latte dock)"
    );

    // 2. Drive the palette through real keystrokes: leader (Ctrl+B) + P,
    //    filter, Enter. Each step waits for the UI to catch up.
    master.write_all(b"\x02P").unwrap();
    assert!(
        wait_for(&buf, b"Commands", Duration::from_secs(5)),
        "palette did not open after leader+P"
    );
    master.write_all(b"re-detect").unwrap();
    assert!(
        wait_for(&buf, b"Re-detect light/dark theme", Duration::from_secs(5)),
        "palette filter did not surface the re-detect command"
    );
    master.write_all(b"\r").unwrap();

    // 3. Re-detect: the second query is answered dark → live swap to mocha.
    assert!(
        wait_for(&buf, MOCHA_DOCK_BG, Duration::from_secs(20)),
        "re-detect did not swap to the dark theme (expected catppuccin-mocha dock)"
    );
    assert!(
        wait_for(&buf, DARK_DETECTED_NOTICE, Duration::from_secs(2)),
        "the 'dark detected' notification was not shown"
    );
    assert_eq!(
        queries.load(Ordering::SeqCst),
        2,
        "expected exactly two OSC 11 queries (startup + re-detect)"
    );
}
