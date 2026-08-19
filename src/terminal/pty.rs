//! PTY management — Layer 1 of the terminal.
//!
//! Opens a pseudoterminal, forks a shell child process attached to the slave
//! end, and exposes the master fd as a pair of `PtyReader` / `PtyWriter`
//! handles that can be handed to separate threads.
//!
//! This is the same kernel path the workspace's GPU terminal uses:
//!
//!   posix_openpt(O_RDWR | O_NOCTTY)
//!     → grantpt / unlockpt
//!     → ptsname  (get slave device path)
//!     → fork
//!       child:  open slave, setsid, dup2 → stdin/stdout/stderr, execve(shell)
//!       parent: owns master fd
//!
//! # Fork safety
//!
//! `spawn_pty` can be called from a multithreaded process (the daemon), so
//! everything the child needs — argv, the full environment, the slave path,
//! the working directory — is prepared **before** `fork`. The child then runs
//! only async-signal-safe calls (`setsid`, `open`, `dup2`, `chdir`, `execve`).
//! Allocating or calling `setenv` after `fork` can deadlock the child on a
//! malloc/environ lock another thread held at the fork instant.

use std::{
    ffi::CString,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use crossbeam_channel::{unbounded, Receiver, Sender};
use nix::fcntl::open as nix_open;
use nix::{
    fcntl::{fcntl, FcntlArg, OFlag},
    libc,
    pty::{grantpt, posix_openpt, ptsname, unlockpt, PtyMaster},
    sys::stat::Mode,
    unistd::{close, dup2, fork, setsid, ForkResult, Pid},
};

/// A window size in cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WinSize {
    pub cols: u16,
    pub rows: u16,
}

/// Wake callback invoked by the reader thread after each chunk of PTY data is
/// sent on the channel, so the event loop wakes immediately instead of polling.
pub type WakeCallback = Box<dyn Fn() + Send + 'static>;

/// Errors that can arise during PTY setup.
#[derive(Debug)]
pub enum PtyError {
    Nix(nix::Error),
    NulError(std::ffi::NulError),
}

impl From<nix::Error> for PtyError {
    fn from(e: nix::Error) -> Self {
        PtyError::Nix(e)
    }
}
impl From<std::ffi::NulError> for PtyError {
    fn from(e: std::ffi::NulError) -> Self {
        PtyError::NulError(e)
    }
}

impl std::fmt::Display for PtyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PtyError::Nix(e) => write!(f, "nix error: {e}"),
            PtyError::NulError(e) => write!(f, "nul error: {e}"),
        }
    }
}

impl std::error::Error for PtyError {}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// The writer half — send bytes toward the shell.
pub struct PtyWriter {
    file: std::fs::File,
}

impl PtyWriter {
    /// Write raw bytes (already encoded key sequences) to the master fd.
    pub fn write(&self, data: &[u8]) {
        let fd = self.file.as_raw_fd();
        let mut written = 0;
        while written < data.len() {
            let result = unsafe {
                libc::write(
                    fd,
                    data[written..].as_ptr() as *const libc::c_void,
                    data.len() - written,
                )
            };
            if result > 0 {
                written += result as usize;
            } else if result < 0 && nix::errno::Errno::last() == nix::errno::Errno::EINTR {
                continue;
            } else {
                log::debug!("PTY write stopped after {written}/{} bytes", data.len());
                break;
            }
        }
    }

    /// Resize the terminal window (TIOCSWINSZ ioctl).
    pub fn resize(&self, size: WinSize) {
        let ws = libc::winsize {
            ws_row: size.rows,
            ws_col: size.cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        unsafe {
            libc::ioctl(self.file.as_raw_fd(), libc::TIOCSWINSZ, &ws);
        }
    }

    /// Set the PTY window size including pixel dimensions (TIOCSWINSZ ioctl).
    /// This enables applications like `kitten icat` to query the terminal
    /// size in pixels via XTWINOPS / TIOCGWINSZ.
    pub fn set_pixel_size(&self, cols: u16, rows: u16, xpixel: u16, ypixel: u16) {
        let ws = libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: xpixel,
            ws_ypixel: ypixel,
        };
        unsafe {
            libc::ioctl(self.file.as_raw_fd(), libc::TIOCSWINSZ, &ws);
        }
    }

    /// The raw file descriptor of the PTY master. Used for `tcgetpgrp` to
    /// detect the foreground process group.
    pub fn raw_fd(&self) -> std::os::unix::io::RawFd {
        self.file.as_raw_fd()
    }
}

/// The write half of a window's I/O: send input and resize toward the shell.
/// A local PTY implements it directly; a remote (daemon-backed) window
/// implements it by sending protocol messages.
pub trait PtySink: Send {
    fn write(&self, data: &[u8]);
    fn resize(&self, size: WinSize);
    /// Set pixel dimensions on the PTY (TIOCSWINSZ with xpixel/ypixel).
    /// Default is a no-op — remote sinks forward resize as a protocol message
    /// that does not carry pixel dimensions.
    fn set_pixel_size(&self, _cols: u16, _rows: u16, _xpixel: u16, _ypixel: u16) {}
}

impl PtySink for PtyWriter {
    fn write(&self, data: &[u8]) {
        PtyWriter::write(self, data);
    }

    fn resize(&self, size: WinSize) {
        PtyWriter::resize(self, size);
    }

    fn set_pixel_size(&self, cols: u16, rows: u16, xpixel: u16, ypixel: u16) {
        PtyWriter::set_pixel_size(self, cols, rows, xpixel, ypixel);
    }
}

/// Handle to the child shell process. Sends SIGHUP and reaps on drop.
/// Also retains a dup of the PTY master fd for `tcgetpgrp` queries
/// (foreground-process detection) that need the controlling terminal.
pub struct PtyHandle {
    child_pid: Pid,
    master_fd: Option<std::os::fd::OwnedFd>,
}

impl PtyHandle {
    fn new(child_pid: Pid, master_fd: std::os::fd::OwnedFd) -> Self {
        PtyHandle {
            child_pid,
            master_fd: Some(master_fd),
        }
    }

    pub fn pid(&self) -> i32 {
        self.child_pid.as_raw()
    }

    /// The raw fd of the PTY master, for `tcgetpgrp` / foreground-process
    /// detection. Returns `None` after `close()` or for remote windows.
    pub fn master_fd(&self) -> Option<std::os::unix::io::RawFd> {
        self.master_fd.as_ref().map(|fd| fd.as_raw_fd())
    }

    /// Drop the master fd, closing the PTY master. Called on window close.
    pub fn close(&mut self) {
        self.master_fd.take();
    }
}

impl Drop for PtyHandle {
    fn drop(&mut self) {
        // Send SIGHUP to the child process group (shell + its children).
        unsafe {
            libc::kill(self.child_pid.as_raw(), libc::SIGHUP);
        }
        // Reap the child to avoid zombies.
        let _ = nix::sys::wait::waitpid(self.child_pid, None);
    }
}

/// The reader half — a channel of output chunks fed by a background thread.
pub struct PtyReader {
    pub rx: Receiver<Vec<u8>>,
    pub reading: Arc<AtomicBool>,
}

/// Resolve a program name to an absolute path via PATH lookup, in the parent,
/// so the post-fork child only ever calls async-signal-safe functions.
/// Absolute paths and names containing '/' pass through unchanged.
fn resolve_program_path(name: &str) -> String {
    if name.contains('/') {
        return name.to_string();
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return candidate.to_string_lossy().into_owned();
            }
        }
    }
    name.to_string()
}

/// Build the child's full environment **before** fork, so the child never
/// touches the malloc/environ locks after fork. Mirrors the old child-side
/// logic: inherit the parent environ, force TERM / COLORTERM / TERM_PROGRAM
/// (so guest tools like chafa, yazi, kitten icat pick the right output
/// format), then apply caller-supplied overrides (e.g. TERMOS_ENV).
fn build_child_env(extra_env: &[(String, String)]) -> Vec<CString> {
    use std::os::unix::ffi::OsStrExt;
    let mut env: Vec<(Vec<u8>, Vec<u8>)> = std::env::vars_os()
        .map(|(k, v)| (k.as_bytes().to_vec(), v.as_bytes().to_vec()))
        .collect();
    let mut set = |k: &str, v: &str| {
        if let Some(e) = env.iter_mut().find(|(ek, _)| ek == k.as_bytes()) {
            e.1 = v.as_bytes().to_vec();
        } else {
            env.push((k.as_bytes().to_vec(), v.as_bytes().to_vec()));
        }
    };
    set("TERM", "xterm-256color");
    set("COLORTERM", "1");
    let term_program =
        std::env::var("TERMOS_TERM_PROGRAM").unwrap_or_else(|_| "TermOS".to_string());
    set("TERM_PROGRAM", &term_program);
    let term_program_version =
        std::env::var("TERMOS_TERM_PROGRAM_VERSION").unwrap_or_else(|_| "0.1.0".to_string());
    set("TERM_PROGRAM_VERSION", &term_program_version);
    for (k, v) in extra_env {
        set(k, v);
    }
    env.into_iter()
        .map(|(mut k, v)| {
            k.push(b'=');
            k.extend(v);
            CString::new(k).expect("environment contains a NUL byte")
        })
        .collect()
}

/// Write `prefix` + the decimal `errno` + a newline to `fd` without
/// allocating — async-signal-safe, for use after fork where malloc may be
/// deadlocked. The child has no CString/format machinery available.
unsafe fn write_errno_msg(fd: libc::c_int, prefix: &[u8], errno: i32) {
    let mut buf = [0u8; 96];
    let mut n = 0;
    for &b in prefix {
        if n < buf.len() {
            buf[n] = b;
            n += 1;
        }
    }
    let mut digits = [0u8; 12];
    let mut d = 0;
    let mut v = errno.unsigned_abs();
    if v == 0 {
        digits[d] = b'0';
        d += 1;
    }
    while v > 0 && d < digits.len() {
        digits[d] = b'0' + (v % 10) as u8;
        v /= 10;
        d += 1;
    }
    if errno < 0 && n < buf.len() {
        buf[n] = b'-';
        n += 1;
    }
    while d > 0 && n < buf.len() {
        d -= 1;
        buf[n] = digits[d];
        n += 1;
    }
    if n < buf.len() {
        buf[n] = b'\n';
        n += 1;
    }
    let _ = libc::write(fd, buf.as_ptr() as *const libc::c_void, n);
}

/// Open a PTY, fork a shell, and return the writer/handle plus an output
/// channel that a reader thread feeds. `argv[0]` is the program to exec;
/// `extra_env` is set in the child's environment before exec (after TERM /
/// COLORTERM, which always win).
pub fn spawn_pty(
    size: WinSize,
    argv: &[String],
    wake: WakeCallback,
    extra_env: &[(String, String)],
    cwd: Option<&str>,
) -> Result<(PtyWriter, PtyHandle, PtyReader), PtyError> {
    // 1. Open master.
    let master: PtyMaster = posix_openpt(OFlag::O_RDWR | OFlag::O_NOCTTY)?;
    grantpt(&master)?;
    unlockpt(&master)?;

    let slave_name = unsafe { ptsname(&master)? };

    // Set master to non-blocking for the reader thread.
    let flags = fcntl(master.as_raw_fd(), FcntlArg::F_GETFL)?;
    let flags = OFlag::from_bits_truncate(flags) | OFlag::O_NONBLOCK;
    fcntl(master.as_raw_fd(), FcntlArg::F_SETFL(flags))?;

    let (tx, rx): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = unbounded();

    // ---- prepare everything the child will need, before fork ----
    // The child must only run async-signal-safe calls: allocating (CString,
    // Vec) or calling setenv after fork can deadlock on a lock another thread
    // held at the fork instant.
    let slave_path = CString::new(slave_name.as_str())?;
    let program = resolve_program_path(&argv[0]);
    let program_c = CString::new(program.as_str())?;
    let argv_c: Vec<CString> = argv
        .iter()
        .map(|s| CString::new(s.as_str()).unwrap())
        .collect();
    let env_c = build_child_env(extra_env);
    let argv_p: Vec<*const libc::c_char> = argv_c
        .iter()
        .map(|c| c.as_ptr())
        .chain(std::iter::once(std::ptr::null()))
        .collect();
    let envp: Vec<*const libc::c_char> = env_c
        .iter()
        .map(|c| c.as_ptr())
        .chain(std::iter::once(std::ptr::null()))
        .collect();
    let dir_c = match cwd {
        Some(dir) => Some(CString::new(dir)?),
        None => None,
    };

    // 2. Fork.
    let fork_result = unsafe { fork()? };

    match fork_result {
        ForkResult::Child => {
            // ---- child process: async-signal-safe calls only ----
            let _ = setsid();

            let slave_fd = nix_open(slave_path.as_c_str(), OFlag::O_RDWR, Mode::empty())?;

            dup2(slave_fd, libc::STDIN_FILENO)?;
            dup2(slave_fd, libc::STDOUT_FILENO)?;
            dup2(slave_fd, libc::STDERR_FILENO)?;

            if slave_fd > 2 {
                let _ = close(slave_fd);
            }

            unsafe {
                libc::close(master.as_raw_fd());
            }

            let ws = libc::winsize {
                ws_row: size.rows,
                ws_col: size.cols,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };
            unsafe {
                libc::ioctl(libc::STDIN_FILENO, libc::TIOCSWINSZ, &ws);
            }

            // Start in the requested directory; bail loudly on failure rather
            // than silently running in the wrong cwd.
            if let Some(dir_c) = &dir_c {
                if unsafe { libc::chdir(dir_c.as_ptr()) } != 0 {
                    let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
                    unsafe {
                        write_errno_msg(2, b"chdir failed, errno ", errno);
                        libc::_exit(1);
                    }
                }
            }

            // exec the program — never returns on success.
            if unsafe { libc::execve(program_c.as_ptr(), argv_p.as_ptr(), envp.as_ptr()) } != 0 {
                let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
                unsafe {
                    write_errno_msg(2, b"termos: execve failed, errno ", errno);
                    libc::_exit(126);
                }
            }
            unreachable!()
        }

        ForkResult::Parent { child } => {
            // ---- parent process ----
            let master_fd = Arc::new(unsafe { OwnedFd::from_raw_fd(master.as_raw_fd()) });
            std::mem::forget(master);

            let writer_raw_fd = unsafe { libc::dup(master_fd.as_raw_fd()) };
            if writer_raw_fd < 0 {
                return Err(nix::errno::Errno::last().into());
            }
            let writer_file = unsafe { std::fs::File::from_raw_fd(writer_raw_fd) };
            let writer_flags = fcntl(writer_file.as_raw_fd(), FcntlArg::F_GETFL)?;
            let writer_flags = OFlag::from_bits_truncate(writer_flags) & !OFlag::O_NONBLOCK;
            fcntl(writer_file.as_raw_fd(), FcntlArg::F_SETFL(writer_flags))?;

            let reader_fd = Arc::clone(&master_fd);
            let reading = Arc::new(AtomicBool::new(true));
            let reading_clone = Arc::clone(&reading);
            std::thread::spawn(move || reader_thread(reader_fd, tx, wake, reading_clone));

            let writer = PtyWriter { file: writer_file };
            // Dup the master fd for the handle so it can answer tcgetpgrp
            // queries without sharing the reader's Arc<OwnedFd>.
            let handle_fd_raw = unsafe { libc::dup(master_fd.as_raw_fd()) };
            if handle_fd_raw < 0 {
                return Err(nix::errno::Errno::last().into());
            }
            let handle_fd = unsafe { OwnedFd::from_raw_fd(handle_fd_raw) };
            let handle = PtyHandle::new(child, handle_fd);
            Ok((writer, handle, PtyReader { rx, reading }))
        }
    }
}

fn reader_thread(
    master_fd: Arc<OwnedFd>,
    tx: Sender<Vec<u8>>,
    wake: WakeCallback,
    reading: Arc<AtomicBool>,
) {
    let raw_fd = master_fd.as_raw_fd();
    let mut buf = [0u8; 16384];

    loop {
        if !reading.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(10));
            continue;
        }

        let mut pollfd = libc::pollfd {
            fd: raw_fd,
            events: libc::POLLIN,
            revents: 0,
        };

        let poll_result = unsafe { libc::poll(&mut pollfd, 1, 100) };

        match poll_result {
            n if n > 0 => {
                let n =
                    unsafe { libc::read(raw_fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
                match n {
                    n if n > 0 => {
                        let chunk = buf[..n as usize].to_vec();
                        if tx.send(chunk).is_err() {
                            break;
                        }
                        wake();
                    }
                    0 => break, // EOF — shell exited.
                    _ => {
                        let err = nix::errno::Errno::last();
                        if err == nix::errno::Errno::EAGAIN || err == nix::errno::Errno::EWOULDBLOCK
                        {
                            continue;
                        } else {
                            break; // EIO or other — slave closed.
                        }
                    }
                }
            }
            0 => continue,
            _ => {
                let err = nix::errno::Errno::last();
                if err == nix::errno::Errno::EINTR {
                    continue;
                }
                break;
            }
        }
    }

    log::info!("PTY reader thread exiting");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn spawn_pty_delivers_output() {
        let size = WinSize { cols: 80, rows: 24 };
        let argv = vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            "printf pty-stage1".to_string(),
        ];
        let (writer, handle, reader) =
            spawn_pty(size, &argv, Box::new(|| {}), &[], None).expect("spawn PTY");

        let mut output = Vec::new();
        for _ in 0..4 {
            match reader.rx.recv_timeout(Duration::from_secs(1)) {
                Ok(chunk) => output.extend(chunk),
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => break,
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            }
            if output
                .windows(b"pty-stage1".len())
                .any(|w| w == b"pty-stage1")
            {
                break;
            }
        }

        assert!(output
            .windows(b"pty-stage1".len())
            .any(|w| w == b"pty-stage1"));
        drop(writer);
        drop(handle);
    }
}
