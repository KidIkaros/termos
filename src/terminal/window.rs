//! The terminal window — a PTY + emulator pair, with an I/O thread that
//! drains PTY output into the emulator and an input path that encodes keys
//! and writes them to the PTY.
//!
//! ## Concurrency
//!
//! The `emulator` is wrapped in `Arc<Mutex<Emulator>>` because the PTY reader
//! thread writes to it while the render path reads it. The `geometry` snapshot
//! and `pending_resize` are behind `Mutex` for the same reason: the layout
//! goroutine writes them while other threads read. The `output_buffer` is
//! only touched from the UI thread (the one that calls `write` / `flush_output`),
//! so it does not need a lock — but `flush_output` takes the emulator lock when
//! draining.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::terminal::pty::{spawn_pty, PtyHandle, PtySink, WinSize};
use crate::vt::Emulator;

// ---------------------------------------------------------------------------
// Geometry snapshot
// ---------------------------------------------------------------------------

/// A self-consistent copy of the window's on-screen geometry, published by the
/// thread that owns layout and read by callbacks (kitty/sixel passthrough) that
/// must not touch the live fields while the layout loop mutates them.
///
/// Ported from Go TUIOS `GeometrySnapshot`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Geometry {
    /// Screen-space X of the outer rectangle.
    pub x: i32,
    /// Screen-space Y of the outer rectangle.
    pub y: i32,
    /// Outer width in cells.
    pub width: i32,
    /// Outer height in cells.
    pub height: i32,
    /// Cells per border edge (0 when tiled, 1 when bordered).
    pub border_offset: i32,
    /// Cursor column (0-indexed) at snapshot time.
    pub cursor_x: i32,
    /// Cursor row (0-indexed) at snapshot time.
    pub cursor_y: i32,
}

impl Geometry {
    /// The drawable content width (outer minus borders).
    pub fn content_width(&self) -> i32 {
        (self.width - 2 * self.border_offset).max(1)
    }

    /// The drawable content height (outer minus borders).
    pub fn content_height(&self) -> i32 {
        (self.height - 2 * self.border_offset).max(1)
    }
}

// ---------------------------------------------------------------------------
// Output batching constants
// ---------------------------------------------------------------------------

/// Maximum bytes of pending PTY output that one pass of `flush_output`
/// coalesces before writing to the emulator.
const MAX_BATCH: usize = 256 * 1024;

/// Maximum bytes written to the emulator under a single lock acquisition.
/// Bounds how long the render path can be kept out of the emulator by a pane's
/// own output — a latency knob, not a throughput one.
const MAX_VT_CHUNK: usize = 8 * 1024;

// ---------------------------------------------------------------------------
// Window
// ---------------------------------------------------------------------------

/// Extra spawn options beyond the base interactive shell.
#[derive(Debug, Clone, Copy, Default)]
pub struct SpawnOptions<'a> {
    /// Run `sh -c <command>` instead of an interactive shell (command pane).
    pub command: Option<&'a str>,
    /// `start_suspended`: hold the child stopped until the first Enter.
    pub suspended: bool,
}

impl SpawnOptions<'_> {
    /// The default interactive shell window.
    pub const fn shell() -> Self {
        SpawnOptions { command: None, suspended: false }
    }
}

/// Retained raw VT lines used by the renderer between emulator updates.
#[derive(Debug, Clone)]
pub struct RenderCache {
    pub width: i32,
    pub height: i32,
    pub viewport: usize,
    pub lines: Vec<Vec<crate::vt::cell::StyledChar>>,
}

/// A terminal window: one shell session in a pane.
pub struct Window {
    pub id: String,
    pub title: String,
    pub emulator: Arc<Mutex<Emulator>>,
    writer: Option<Box<dyn PtySink>>,
    handle: Option<PtyHandle>,
    /// Whether the PTY reader is draining (set false when backgrounded).
    reading: Arc<AtomicBool>,
    /// Whether the shell exited.
    pub exited: bool,
    /// The fixed command this pane runs (`sh -c <command>`), or `None` for
    /// an interactive shell. Command panes show their exit status and re-run
    /// with Enter.
    pub command: Option<String>,
    /// `start_suspended`: the child is held with SIGSTOP until the pane is
    /// manually triggered (first Enter sends SIGCONT).
    pub suspended: bool,
    /// The command's exit status once it has finished (`None` while running).
    pub exit_code: Option<i32>,
    /// The last size applied, so a same-size resize is a no-op.
    last_size: Option<WinSize>,
    /// Whether the window is zoomed.
    pub zoomed: bool,
    /// Whether the window is minimized (hidden from tiling, shown as a dock icon).
    pub minimized: bool,
    /// Pre-zoom position and size, saved when zooming in.
    pub pre_zoom_x: i32,
    pub pre_zoom_y: i32,
    pub pre_zoom_width: i32,
    pub pre_zoom_height: i32,
    /// The window's agent state, wire spelling (`none` when not reporting).
    pub agent_state: String,
    /// The free-text note the last agent report carried.
    pub agent_message: String,
    /// The harness id the last agent report named.
    pub agent_harness: String,
    /// The typed agent state (parallel to `agent_state` wire string).
    pub agent_state_typed: Option<crate::session::agent_state::AgentState>,
    /// The agent message (typed, parallel to `agent_message`).
    pub agent_message_opt: Option<String>,
    /// The agent harness id (typed, parallel to `agent_harness`).
    pub agent_harness_opt: Option<String>,
    /// When the agent state was last set.
    pub agent_state_at: Option<std::time::Instant>,
    /// The foreground command running in the pane (base name, or empty).
    pub foreground_cmd: Option<String>,
    /// Memoised shell working-directory read (per-window).
    pub cwd_cache: super::cwd::CwdCache,

    // --- Phase 5: Terminal window I/O improvements ---

    /// Geometry snapshot for cross-thread readers (passthrough callbacks).
    /// Published by `publish_geometry`, read by `last_geometry`.
    geometry: Mutex<Option<Geometry>>,

    /// Pending output buffer for coalescing small PTY writes into larger
    /// chunks. Only touched from the thread that calls `write` / `flush_output`
    /// (the UI thread), so it does not need its own lock.
    output_buffer: Vec<u8>,

    /// A deferred resize: when resize events arrive in rapid succession, the
    /// final size is stored here and announced once via `flush_resize`.
    /// `Some` means a resize is pending; `None` means nothing deferred.
    pending_resize: Mutex<Option<WinSize>>,
    /// Dirty flag: set by the drain thread when PTY output arrives,
    /// cleared by the render path after painting.
    dirty: Arc<AtomicBool>,
    /// Retained raw VT lines. The renderer still composites them every frame,
    /// but only rebuilds the styled line snapshot when content changes.
    pub(crate) render_cache: Mutex<Option<RenderCache>>,

    /// Cell pixel dimensions for XTWINOPS / TIOCGWINSZ pixel reporting.
    /// `0` means unknown — pixel size is not reported in that case.
    cell_pixel_width: u16,
    cell_pixel_height: u16,

    // --- Phase G: Terminal completion ---

    /// Whether the window is in scrollback viewing mode.
    scrollback_mode: bool,
    /// The scrollback offset (0 = live screen, positive = scrolled back).
    scrollback_offset: usize,
    /// The last announced content width (for resize deduplication).
    announced_width: i32,
    /// The last announced content height (for resize deduplication).
    announced_height: i32,
    /// Whether resize announcements are held (deferred until released).
    announce_held: bool,
    /// Whether the window is in copy mode.
    copy_mode: bool,
    /// Whether copy mode was entered implicitly (via scroll/drag gesture).
    copy_mode_implicit: bool,
    /// Copy mode cursor X position.
    copy_cursor_x: i32,
    /// Copy mode cursor Y position.
    copy_cursor_y: i32,
    /// Copy mode scroll offset.
    copy_scroll_offset: usize,
    /// Whether the window is tiled (no individual borders).
    pub tiled: bool,
    /// The daemon output writer (set for daemon-mode windows).
    daemon_writer: Option<Arc<super::window_io::DaemonOutputWriter>>,
}

impl Window {
    pub fn spawn(
        id: impl Into<String>,
        title: impl Into<String>,
        size: WinSize,
        shell: &str,
        opts: SpawnOptions<'_>,
        wake: Box<dyn Fn() + Send + 'static>,
        extra_env: &[(String, String)],
    ) -> Result<Self, crate::terminal::pty::PtyError> {
        let SpawnOptions { command, suspended } = opts;
        let argv: Vec<String> = match command {
            Some(cmd) => vec!["sh".to_string(), "-c".to_string(), cmd.to_string()],
            None => vec![shell.to_string()],
        };

        let (writer, handle, reader) = spawn_pty(size, &argv, wake, extra_env, None, suspended)?;
        let emulator = Arc::new(Mutex::new(Emulator::new(
            size.cols as i32,
            size.rows as i32,
        )));

        let emu_clone = Arc::clone(&emulator);
        let dirty = Arc::new(AtomicBool::new(true));
        let dirty_clone = Arc::clone(&dirty);
        std::thread::spawn(move || drain_thread(reader.rx, emu_clone, dirty_clone));

        // A suspended command pane starts blank; write a hint into the
        // emulator so the pane explains itself until Enter triggers it.
        if suspended {
            if let Some(cmd) = command {
                if let Ok(mut emu) = emulator.lock() {
                    emu.write(
                        format!("\r\n[suspended] press Enter to run: {cmd}\r\n")
                            .as_bytes(),
                    );
                }
            }
        }

        let win = Self {
            id: id.into(),
            title: title.into(),
            emulator,
            writer: Some(Box::new(writer)),
            handle: Some(handle),
            reading: Arc::new(AtomicBool::new(true)),
            exited: false,
            command: command.map(|c| c.to_string()),
            suspended,
            exit_code: None,
            agent_state: String::new(),
            agent_message: String::new(),
            agent_harness: String::new(),
            agent_state_typed: None,
            agent_message_opt: None,
            agent_harness_opt: None,
            agent_state_at: None,
            foreground_cmd: None,
            cwd_cache: super::cwd::CwdCache::new(),
            last_size: None,
            zoomed: false,
            minimized: false,
            pre_zoom_x: 0,
            pre_zoom_y: 0,
            pre_zoom_width: 0,
            pre_zoom_height: 0,
            geometry: Mutex::new(None),
            output_buffer: Vec::new(),
            pending_resize: Mutex::new(None),
            dirty,
            render_cache: Mutex::new(None),
            cell_pixel_width: 0,
            cell_pixel_height: 0,
            scrollback_mode: false,
            scrollback_offset: 0,
            announced_width: 0,
            announced_height: 0,
            announce_held: false,
            copy_mode: false,
            copy_mode_implicit: false,
            copy_cursor_x: 0,
            copy_cursor_y: 0,
            copy_scroll_offset: 0,
            tiled: false,
            daemon_writer: None,
        };
        // Publish the initial geometry before the PTY reader starts, so
        // callbacks always have a snapshot to read.
        win.publish_geometry(0, 0, size.cols as i32, size.rows as i32, 0);
        Ok(win)
    }

    /// Create a window backed by a remote PTY: input goes to `sink` (which
    /// sends protocol messages) and output arrives on `output` (fed by the
    /// client's socket reader thread).
    pub fn remote(
        id: impl Into<String>,
        title: impl Into<String>,
        size: WinSize,
        sink: Box<dyn PtySink>,
        output: crossbeam_channel::Receiver<Vec<u8>>,
    ) -> Self {
        let emulator = Arc::new(Mutex::new(Emulator::new(
            size.cols as i32,
            size.rows as i32,
        )));
        let emu_clone = Arc::clone(&emulator);
        let dirty = Arc::new(AtomicBool::new(true));
        let dirty_clone = Arc::clone(&dirty);
        std::thread::spawn(move || drain_thread(output, emu_clone, dirty_clone));
        let win = Self {
            id: id.into(),
            title: title.into(),
            emulator,
            writer: Some(sink),
            handle: None,
            reading: Arc::new(AtomicBool::new(true)),
            exited: false,
            command: None,
            suspended: false,
            exit_code: None,
            agent_state: String::new(),
            agent_message: String::new(),
            agent_harness: String::new(),
            agent_state_typed: None,
            agent_message_opt: None,
            agent_harness_opt: None,
            agent_state_at: None,
            foreground_cmd: None,
            cwd_cache: super::cwd::CwdCache::new(),
            last_size: None,
            zoomed: false,
            minimized: false,
            pre_zoom_x: 0,
            pre_zoom_y: 0,
            pre_zoom_width: 0,
            pre_zoom_height: 0,
            geometry: Mutex::new(None),
            output_buffer: Vec::new(),
            pending_resize: Mutex::new(None),
            dirty,
            render_cache: Mutex::new(None),
            cell_pixel_width: 0,
            cell_pixel_height: 0,
            scrollback_mode: false,
            scrollback_offset: 0,
            announced_width: 0,
            announced_height: 0,
            announce_held: false,
            copy_mode: false,
            copy_mode_implicit: false,
            copy_cursor_x: 0,
            copy_cursor_y: 0,
            copy_scroll_offset: 0,
            tiled: false,
            daemon_writer: None,
        };
        win.publish_geometry(0, 0, size.cols as i32, size.rows as i32, 0);
        win
    }

    /// Create a window without a PTY (used in tests and daemon restore).
    pub fn without_pty(id: impl Into<String>, title: impl Into<String>, size: WinSize) -> Self {
        let win = Self {
            id: id.into(),
            title: title.into(),
            emulator: Arc::new(Mutex::new(Emulator::new(
                size.cols as i32,
                size.rows as i32,
            ))),
            writer: None,
            handle: None,
            reading: Arc::new(AtomicBool::new(true)),
            exited: false,
            command: None,
            suspended: false,
            exit_code: None,
            agent_state: String::new(),
            agent_message: String::new(),
            agent_harness: String::new(),
            agent_state_typed: None,
            agent_message_opt: None,
            agent_harness_opt: None,
            agent_state_at: None,
            foreground_cmd: None,
            cwd_cache: super::cwd::CwdCache::new(),
            last_size: None,
            zoomed: false,
            minimized: false,
            pre_zoom_x: 0,
            pre_zoom_y: 0,
            pre_zoom_width: 0,
            pre_zoom_height: 0,
            geometry: Mutex::new(None),
            output_buffer: Vec::new(),
            pending_resize: Mutex::new(None),
            dirty: Arc::new(AtomicBool::new(true)),
            render_cache: Mutex::new(None),
            cell_pixel_width: 0,
            cell_pixel_height: 0,
            scrollback_mode: false,
            scrollback_offset: 0,
            announced_width: 0,
            announced_height: 0,
            announce_held: false,
            copy_mode: false,
            copy_mode_implicit: false,
            copy_cursor_x: 0,
            copy_cursor_y: 0,
            copy_scroll_offset: 0,
            tiled: false,
            daemon_writer: None,
        };
        win.publish_geometry(0, 0, size.cols as i32, size.rows as i32, 0);
        win
    }

    /// Write encoded bytes to the PTY.
    pub fn write(&self, data: &[u8]) {
        if let Some(writer) = &self.writer {
            writer.write(data);
        }
    }

    /// Check if the window has new content since the last render.
    pub fn is_dirty(&self) -> bool {
        self.dirty.load(Ordering::Relaxed)
    }

    /// Clear the dirty flag after rendering.
    pub fn clear_dirty(&self) {
        self.dirty.store(false, Ordering::Relaxed);
    }

    /// Resize the PTY and the emulator (a no-op when the size is unchanged).
    /// Returns true when a new size was applied after the initial sizing —
    /// i.e. a real resize, not the first layout application. Used by the
    /// after-resize hook so it does not fire for every window at startup.
    pub fn resize(&mut self, size: WinSize) -> bool {
        if self.last_size == Some(size) {
            return false;
        }
        let changed = self.last_size.is_some();
        self.last_size = Some(size);
        if let Some(writer) = &self.writer {
            writer.resize(size);
            // Report pixel dimensions when known.
            if self.cell_pixel_width > 0 && self.cell_pixel_height > 0 {
                let xpixel = size.cols.saturating_mul(self.cell_pixel_width);
                let ypixel = size.rows.saturating_mul(self.cell_pixel_height);
                writer.set_pixel_size(size.cols, size.rows, xpixel, ypixel);
            }
        }
        if let Ok(mut emu) = self.emulator.lock() {
            emu.resize(size.cols as i32, size.rows as i32);
        }
        self.dirty.store(true, Ordering::Release);
        if let Ok(mut cache) = self.render_cache.lock() {
            *cache = None;
        }
        // Update the geometry snapshot with the new size.
        self.publish_geometry(
            self.last_geometry().map(|g| g.x).unwrap_or(0),
            self.last_geometry().map(|g| g.y).unwrap_or(0),
            size.cols as i32,
            size.rows as i32,
            self.last_geometry().map(|g| g.border_offset).unwrap_or(0),
        );
        changed
    }

    pub fn set_reading(&self, reading: bool) {
        self.reading.store(reading, Ordering::Release);
    }

    pub fn pid(&self) -> Option<i32> {
        self.handle.as_ref().map(|h| h.pid())
    }

    pub fn close(&mut self) {
        self.exited = true;
        // Flush any pending output before teardown.
        let _ = self.flush_output();
        // Flush any pending resize.
        let _ = self.flush_resize();
        // Close the PTY master fd held by the handle.
        if let Some(handle) = &mut self.handle {
            handle.close();
        }
    }

    /// Whether this window is a command pane whose command has finished.
    pub fn can_rerun(&self) -> bool {
        self.command.is_some() && self.exit_code.is_some()
    }

    /// Non-blocking exit check for command panes: reaps the child with
    /// `waitpid(WNOHANG)` and records its status. Call periodically from the
    /// UI thread. Returns `true` when the exit status changed.
    pub fn poll_exit(&mut self) -> bool {
        if self.command.is_none() || self.exit_code.is_some() {
            return false;
        }
        let Some(handle) = &self.handle else {
            return false;
        };
        let pid = nix::unistd::Pid::from_raw(handle.pid());
        use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
        match waitpid(pid, Some(WaitPidFlag::WNOHANG)) {
            Ok(WaitStatus::Exited(_, code)) => {
                self.exit_code = Some(code);
                self.exited = true;
                handle.reaped_flag().store(true, Ordering::Release);
                true
            }
            Ok(WaitStatus::Signaled(_, sig, _)) => {
                // Mirrors the shell convention: 128 + signal number.
                self.exit_code = Some(128 + sig as i32);
                self.exited = true;
                handle.reaped_flag().store(true, Ordering::Release);
                true
            }
            Ok(WaitStatus::Stopped(_, _) | WaitStatus::Continued(_)) | Ok(_) => false,
            Err(nix::errno::Errno::ECHILD) => {
                // Already reaped elsewhere; treat as finished with unknown
                // status so the pane doesn't look perpetually alive.
                self.exited = true;
                self.exit_code = self.exit_code.or(Some(-1));
                true
            }
            Err(_) => false,
        }
    }

    /// If this is a suspended command pane, resume it with SIGCONT and return
    /// `true`. The first Enter in the pane calls this.
    pub fn resume_if_suspended(&mut self) -> bool {
        if !self.suspended || self.command.is_none() {
            return false;
        }
        let Some(handle) = &self.handle else {
            return false;
        };
        let pid = handle.pid();
        if pid > 0 {
            let _ = nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), nix::sys::signal::Signal::SIGCONT);
        }
        self.suspended = false;
        true
    }

    /// Re-run a finished command pane: spawn a fresh PTY for the same command
    /// and swap in the new writer/handle/emulator. Keeps the window's id,
    /// title and `command`.
    pub fn restart(
        &mut self,
        size: WinSize,
        wake: Box<dyn Fn() + Send + 'static>,
        extra_env: &[(String, String)],
    ) -> Result<(), crate::terminal::pty::PtyError> {
        let Some(cmd) = self.command.clone() else {
            return Err(crate::terminal::pty::PtyError::Nix(nix::errno::Errno::EINVAL));
        };
        let argv = vec!["sh".to_string(), "-c".to_string(), cmd.clone()];
        let (writer, handle, reader) =
            spawn_pty(size, &argv, wake, extra_env, None, false)?;
        let emulator = Arc::new(Mutex::new(Emulator::new(
            size.cols as i32,
            size.rows as i32,
        )));
        let emu_clone = Arc::clone(&emulator);
        let dirty = Arc::new(AtomicBool::new(true));
        let dirty_clone = Arc::clone(&dirty);
        std::thread::spawn(move || drain_thread(reader.rx, emu_clone, dirty_clone));

        // Replacing `handle` drops the old one; its child already exited so
        // the reaped flag skips the kill.
        self.emulator = emulator;
        self.dirty = dirty;
        if let Ok(mut cache) = self.render_cache.lock() {
            *cache = None;
        }
        self.writer = Some(Box::new(writer));
        self.handle = Some(handle);
        self.exited = false;
        self.exit_code = None;
        self.suspended = false;
        self.last_size = Some(size);
        self.publish_geometry(0, 0, size.cols as i32, size.rows as i32, 0);
        Ok(())
    }

    /// The shell's working directory, or empty when unknown.
    pub fn cwd(&self) -> String {
        self.cwd_cache.get(self.pid().unwrap_or(0))
    }

    // -----------------------------------------------------------------------
    // Geometry snapshot
    // -----------------------------------------------------------------------

    /// Publish the current geometry for cross-thread readers. Captures the
    /// window's position, size, border offset, and cursor location into a
    /// snapshot that callbacks can read without touching the live fields.
    ///
    /// No-ops when the geometry is unchanged since the last publish.
    pub fn publish_geometry(
        &self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        border_offset: i32,
    ) {
        // Read the cursor position from the emulator (0,0 if locked).
        let (cursor_x, cursor_y) = match self.emulator.lock() {
            Ok(emu) => {
                let pos = emu.cursor_position();
                (pos.x, pos.y)
            }
            Err(_) => (0, 0),
        };

        let new_geom = Geometry {
            x,
            y,
            width,
            height,
            border_offset,
            cursor_x,
            cursor_y,
        };

        if let Ok(mut guard) = self.geometry.lock() {
            if let Some(ref existing) = *guard {
                if *existing == new_geom {
                    return;
                }
            }
            *guard = Some(new_geom);
        }
    }

    /// The most recently published geometry snapshot. Returns a zero
    /// `Geometry` when none has been published yet.
    pub fn last_geometry(&self) -> Option<Geometry> {
        self.geometry.lock().ok().and_then(|g| *g)
    }

    // -----------------------------------------------------------------------
    // Output batching / coalescing
    // -----------------------------------------------------------------------

    /// Queue bytes for the PTY output buffer. Small writes are coalesced into
    /// larger chunks for efficiency — the emulator parses the whole batch at
    /// once in `flush_output`, which reduces lock acquisitions and parser
    /// overhead for high-frequency output.
    ///
    /// The buffer is drained by `flush_output`. Callers that want immediate
    /// delivery should call `flush_output` after queuing.
    pub fn queue_output(&mut self, data: &[u8]) {
        self.output_buffer.extend_from_slice(data);
        // Auto-flush when the buffer reaches the batch cap.
        if self.output_buffer.len() >= MAX_BATCH {
            let _ = self.flush_output();
        }
    }

    /// Flush the pending output buffer to the emulator, coalescing all queued
    /// bytes into a single write. The batch is written in bounded chunks
    /// (`MAX_VT_CHUNK`) so the render path is not starved by a pane's own
    /// output — each chunk releases the emulator lock between writes.
    ///
    /// Returns the number of bytes written.
    pub fn flush_output(&mut self) -> usize {
        if self.output_buffer.is_empty() {
            return 0;
        }
        let batch = std::mem::take(&mut self.output_buffer);
        let total = batch.len();

        // Write in bounded chunks to avoid holding the emulator lock for too
        // long on a large batch.
        let Ok(mut emu) = self.emulator.lock() else {
            // Put the data back if we could not lock.
            self.output_buffer = batch;
            return 0;
        };

        for chunk in batch.chunks(MAX_VT_CHUNK) {
            emu.write(chunk);
        }

        total
    }

    // -----------------------------------------------------------------------
    // Deferred resize
    // -----------------------------------------------------------------------

    /// Defer a resize: store the target size without announcing it to the PTY
    /// or emulator. When resize events arrive in rapid succession (e.g. during
    /// a mouse drag), only the final size needs to be announced. Call
    /// `flush_resize` once the sequence settles to apply the last size.
    pub fn defer_resize(&self, size: WinSize) {
        if let Ok(mut guard) = self.pending_resize.lock() {
            *guard = Some(size);
        }
    }

    /// Apply the deferred resize if one is pending. Announces the final size
    /// to the PTY and emulator, then clears the pending state. Returns `true`
    /// when a resize was applied, `false` when nothing was pending.
    pub fn flush_resize(&mut self) -> bool {
        let size = match self.pending_resize.lock() {
            Ok(mut guard) => guard.take(),
            Err(_) => None,
        };
        let Some(size) = size else {
            return false;
        };
        self.resize(size)
    }

    /// Whether a deferred resize is pending.
    pub fn has_pending_resize(&self) -> bool {
        self.pending_resize
            .lock()
            .map(|g| g.is_some())
            .unwrap_or(false)
    }

    // -----------------------------------------------------------------------
    // Foreground-process detection
    // -----------------------------------------------------------------------

    /// Detect the foreground process running in this pane's PTY. Returns the
    /// process info (comm, cmdline, exe) read from `/proc`, or `None` when the
    /// foreground process group cannot be determined (non-Linux, no PTY, or
    /// the process has exited).
    ///
    /// Uses `tcgetpgrp` on the PTY master fd to find the foreground process
    /// group leader, then reads `/proc/<pid>/stat`, `/proc/<pid>/cmdline`,
    /// and `/proc/<pid>/exe` via the `agent_detect` module.
    pub fn foreground_process_info(&self) -> Option<crate::session::agent_detect::ProcessInfo> {
        let fd = self.handle.as_ref()?.master_fd()?;
        crate::session::agent_detect::detect_foreground_process(fd)
    }

    /// The foreground process name (base name), or empty when only a shell is
    /// running or the process cannot be determined. Convenience wrapper around
    /// `foreground_process_info` and `agent_detect::foreground_command`.
    pub fn foreground_process_name(&self) -> String {
        let Some(info) = self.foreground_process_info() else {
            return String::new();
        };
        // A shell is "running" when the PTY has a foreground process group.
        let running = self
            .handle
            .as_ref()
            .and_then(|h| h.master_fd())
            .is_some();
        let shell = self
            .title
            .split_whitespace()
            .next()
            .unwrap_or("sh")
            .to_lowercase();
        crate::session::agent_detect::foreground_command(&info, running, &shell)
    }

    // -----------------------------------------------------------------------
    // Terminal reset
    // -----------------------------------------------------------------------

    /// Reset the terminal to a clean state. Sends RIS (ESC c) to the emulator
    /// — which resets the screen, modes, and cursor — then clears all graphics
    /// state (kitty image placements, sixel buffers). Also flushes any pending
    /// output so the reset takes effect immediately.
    pub fn reset_terminal(&mut self) {
        // Flush pending output first so the reset is not overwritten by
        // stale buffered data.
        let _ = self.flush_output();

        // Send RIS (ESC c) — full reset — to the emulator.
        if let Ok(mut emu) = self.emulator.lock() {
            emu.write(b"\x1bc");
            // Clear graphics state (kitty/sixel) that the emulator's RIS
            // handler does not clear on its own.
            emu.clear_graphics();
        }
    }

    // -----------------------------------------------------------------------
    // PTY pixel dimensions
    // -----------------------------------------------------------------------

    /// Set the cell pixel dimensions for this window. This is used to report
    /// accurate pixel dimensions to child processes via TIOCGWINSZ, which
    /// enables graphics protocols (kitty icat, sixel) to size images.
    ///
    /// When both dimensions are non-zero, the PTY is immediately updated with
    /// the current window size in pixels.
    pub fn set_cell_pixel_dimensions(&mut self, cell_width: u16, cell_height: u16) {
        self.cell_pixel_width = cell_width;
        self.cell_pixel_height = cell_height;

        // Update the PTY immediately if we have a known size.
        if cell_width > 0 && cell_height > 0 {
            if let (Some(writer), Some(size)) = (&self.writer, self.last_size) {
                let xpixel = size.cols.saturating_mul(cell_width);
                let ypixel = size.rows.saturating_mul(cell_height);
                writer.set_pixel_size(size.cols, size.rows, xpixel, ypixel);
            }
        }
    }

    /// The configured cell pixel width, or 0 when unknown.
    pub fn cell_pixel_width(&self) -> u16 {
        self.cell_pixel_width
    }

    /// The configured cell pixel height, or 0 when unknown.
    pub fn cell_pixel_height(&self) -> u16 {
        self.cell_pixel_height
    }

    // -----------------------------------------------------------------------
    // Phase G: Scrollback / copy mode
    // -----------------------------------------------------------------------

    /// Whether the window is in scrollback viewing mode.
    pub fn scrollback_mode(&self) -> bool {
        self.scrollback_mode
    }

    /// The current scrollback offset (0 = live screen).
    pub fn scrollback_offset(&self) -> usize {
        self.scrollback_offset
    }

    /// The number of lines in the scrollback buffer (lock-free, try-lock).
    pub fn scrollback_len(&self) -> usize {
        match self.emulator.try_lock() {
            Ok(emu) => emu.scrollback_len(),
            Err(_) => 0,
        }
    }

    /// Clear the scrollback buffer.
    pub fn clear_scrollback(&self) {
        if let Ok(mut emu) = self.emulator.lock() {
            emu.clear_scrollback();
        }
    }

    /// Set the maximum number of scrollback lines.
    pub fn set_scrollback_max_lines(&self, max: usize) {
        if let Ok(mut emu) = self.emulator.lock() {
            emu.set_scrollback_max_lines(max);
        }
    }

    /// Enter scrollback viewing mode, starting at the bottom (most recent).
    pub fn enter_scrollback_mode(&mut self) {
        self.scrollback_mode = true;
        self.scrollback_offset = 0;
    }

    /// Exit scrollback viewing mode.
    pub fn exit_scrollback_mode(&mut self) {
        self.scrollback_mode = false;
        self.scrollback_offset = 0;
    }

    /// Scroll up (back in history) by `lines` lines in scrollback mode.
    pub fn scroll_up(&mut self, lines: usize) {
        if !self.scrollback_mode {
            return;
        }
        let max = self.scrollback_len();
        self.scrollback_offset = (self.scrollback_offset + lines).min(max);
    }

    /// Scroll down (toward live output) by `lines` lines in scrollback mode.
    pub fn scroll_down(&mut self, lines: usize) {
        if !self.scrollback_mode {
            return;
        }
        self.scrollback_offset = self.scrollback_offset.saturating_sub(lines);
        if self.scrollback_offset == 0 {
            self.exit_scrollback_mode();
        }
    }

    // -----------------------------------------------------------------------
    // Copy mode
    // -----------------------------------------------------------------------

    /// Whether copy mode is active (including implicit sessions).
    pub fn in_copy_mode(&self) -> bool {
        self.copy_mode
    }

    /// Whether copy mode should present itself as a mode (not implicit).
    pub fn copy_mode_visible(&self) -> bool {
        self.copy_mode && !self.copy_mode_implicit
    }

    /// Whether copy mode is active only because a scroll/drag gesture needed it.
    pub fn in_implicit_copy_mode(&self) -> bool {
        self.copy_mode && self.copy_mode_implicit
    }

    /// Enter vim-style copy mode.
    pub fn enter_copy_mode(&mut self) {
        self.copy_mode = true;
        self.copy_mode_implicit = false;
        self.copy_cursor_x = 0;
        self.copy_cursor_y = self.last_geometry().map(|g| g.height / 2).unwrap_or(0);
        self.copy_scroll_offset = 0;
        self.scrollback_offset = 0;
    }

    /// Enter copy mode implicitly (via mouse wheel / scrollbar drag).
    pub fn enter_copy_mode_implicit(&mut self) {
        self.enter_copy_mode();
        self.copy_mode_implicit = true;
    }

    /// Exit copy mode and return to live terminal mode.
    pub fn exit_copy_mode(&mut self) {
        self.copy_mode = false;
        self.copy_mode_implicit = false;
        self.copy_scroll_offset = 0;
        self.scrollback_offset = 0;
    }

    /// Copy mode cursor position.
    pub fn copy_cursor(&self) -> (i32, i32) {
        (self.copy_cursor_x, self.copy_cursor_y)
    }

    /// Set the copy mode cursor position.
    pub fn set_copy_cursor(&mut self, x: i32, y: i32) {
        self.copy_cursor_x = x;
        self.copy_cursor_y = y;
    }

    /// Copy mode scroll offset.
    pub fn copy_scroll_offset(&self) -> usize {
        self.copy_scroll_offset
    }

    /// Set the copy mode scroll offset.
    pub fn set_copy_scroll_offset(&mut self, offset: usize) {
        self.copy_scroll_offset = offset;
        self.scrollback_offset = offset;
    }

    // -----------------------------------------------------------------------
    // Phase G: Geometry extras
    // -----------------------------------------------------------------------

    /// Convert screen coordinates to terminal-relative coordinates.
    /// Returns `(term_x, term_y, inside)` where `inside` is true when the
    /// coordinates fall within the content area.
    ///
    /// Ported from Go `ScreenToTerminal`.
    pub fn screen_to_terminal(&self, screen_x: i32, screen_y: i32) -> (i32, i32, bool) {
        let geom = self.last_geometry().unwrap_or_default();
        let off = geom.border_offset;
        let term_x = screen_x - geom.x - off;
        let term_y = screen_y - geom.y - off;
        let inside = term_x >= 0
            && term_y >= 0
            && term_x < geom.content_width()
            && term_y < geom.content_height();
        (term_x, term_y, inside)
    }

    /// The last announced content width (what the guest was told).
    pub fn announced_width(&self) -> i32 {
        self.announced_width
    }

    /// The last announced content height (what the guest was told).
    pub fn announced_height(&self) -> i32 {
        self.announced_height
    }

    /// Record the emulator size the guest already believes it has, so a later
    /// resize to the same size does not re-announce it.
    ///
    /// Ported from Go `SeedAnnouncedSize`.
    pub fn seed_announced_size(&mut self, width: i32, height: i32) {
        self.announced_width = width;
        self.announced_height = height;
    }

    /// Stop resize from telling the guest anything until
    /// [`release_announcements`] is called.
    ///
    /// Ported from Go `HoldAnnouncements`.
    pub fn hold_announcements(&mut self) {
        self.announce_held = true;
    }

    /// End a hold and send the size the pane settled at if it differs from
    /// what the guest already has.
    ///
    /// Ported from Go `ReleaseAnnouncements`.
    pub fn release_announcements(&mut self) {
        if !self.announce_held {
            return;
        }
        self.announce_held = false;
        // If the announced size differs from the last applied size, re-announce.
        let geom = self.last_geometry().unwrap_or_default();
        let cw = geom.content_width();
        let ch = geom.content_height();
        if self.announced_width != cw || self.announced_height != ch {
            self.announced_width = cw;
            self.announced_height = ch;
            if let Some(writer) = &self.writer {
                if let Some(size) = self.last_size {
                    writer.resize(size);
                }
            }
        }
    }

    /// Set whether the window is tiled (no individual borders).
    pub fn set_tiled(&mut self, tiled: bool) {
        self.tiled = tiled;
    }

    /// The border offset: 0 for tiled windows, 1 otherwise.
    pub fn border_offset(&self) -> i32 {
        if self.tiled {
            0
        } else {
            1
        }
    }

    // -----------------------------------------------------------------------
    // Phase G: Unix SIGWINCH
    // -----------------------------------------------------------------------

    /// Send SIGWINCH to the child process to notify it of a resize. The
    /// kernel PTY size is already updated by `resize`; this signal tells the
    /// shell to query the new size via `ioctl(TIOCGWINSZ)` and redraw.
    ///
    /// Ported from Go `TriggerRedraw`.
    pub fn trigger_redraw(&self) {
        if let Some(pid) = self.pid() {
            // SAFETY: kill is a simple libc call. SIGWINCH is a benign signal.
            unsafe {
                nix::libc::kill(pid, nix::libc::SIGWINCH);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Phase G: Cleanup — reset terminal on exit
    // -----------------------------------------------------------------------

    /// Reset the host terminal to a clean state on application exit.
    /// Sends RIS, disables mouse tracking, shows cursor, exits alt screen,
    /// and resets all text attributes.
    ///
    /// Ported from Go `ResetTerminal` (cleanup.go).
    pub fn reset_host_terminal() {
        use std::io::Write;
        let mut stdout = std::io::stdout();
        let _ = stdout.write_all(
            b"\x1bc\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1004l\x1b[?1006l\x1b[?25h\x1b[?47l\x1b[0m\r\n",
        );
        let _ = stdout.flush();
    }

    // -----------------------------------------------------------------------
    // Phase G: Daemon output writer
    // -----------------------------------------------------------------------

    /// Set the daemon output writer for this window (daemon mode only).
    pub fn set_daemon_writer(&mut self, writer: Arc<super::window_io::DaemonOutputWriter>) {
        self.daemon_writer = Some(writer);
    }

    /// The daemon output writer, if set.
    pub fn daemon_writer(&self) -> Option<&Arc<super::window_io::DaemonOutputWriter>> {
        self.daemon_writer.as_ref()
    }

    /// Signal that new output is available. Used by the diff protocol and
    /// the daemon output writer to wake the render path.
    pub fn signal_new_output(&self) {
        self.dirty.store(true, Ordering::Release);
    }
}

// ---------------------------------------------------------------------------
// PTY reader drain thread
// ---------------------------------------------------------------------------

fn drain_thread(rx: crossbeam_channel::Receiver<Vec<u8>>, emulator: Arc<Mutex<Emulator>>, dirty: Arc<AtomicBool>) {
    // Batch coalescing on the reader side: collect multiple chunks into a
    // single buffer before taking the emulator lock, to reduce lock
    // acquisitions under high output rates.
    let mut batch: Vec<u8> = Vec::with_capacity(MAX_BATCH);

    while let Ok(chunk) = rx.recv() {
        batch.extend_from_slice(&chunk);

        // Try to drain more chunks without blocking.
        while batch.len() < MAX_BATCH {
            match rx.try_recv() {
                Ok(more) => batch.extend_from_slice(&more),
                Err(_) => break,
            }
        }

        // Write in bounded chunks so the render path is not starved.
        if let Ok(mut emu) = emulator.lock() {
            for chunk in batch.chunks(MAX_VT_CHUNK) {
                emu.write(chunk);
            }
        }
        // Mark the window as having new content.
        dirty.store(true, Ordering::Relaxed);
        batch.clear();
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometry_content_size_with_borders() {
        let geom = Geometry {
            x: 10,
            y: 20,
            width: 80,
            height: 24,
            border_offset: 1,
            cursor_x: 0,
            cursor_y: 0,
        };
        assert_eq!(geom.content_width(), 78);
        assert_eq!(geom.content_height(), 22);
    }

    #[test]
    fn geometry_content_size_tiled() {
        let geom = Geometry {
            x: 0,
            y: 0,
            width: 80,
            height: 24,
            border_offset: 0,
            cursor_x: 0,
            cursor_y: 0,
        };
        assert_eq!(geom.content_width(), 80);
        assert_eq!(geom.content_height(), 24);
    }

    #[test]
    fn geometry_content_size_clamped_to_one() {
        let geom = Geometry {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
            border_offset: 1,
            cursor_x: 0,
            cursor_y: 0,
        };
        assert_eq!(geom.content_width(), 1);
        assert_eq!(geom.content_height(), 1);
    }

    #[test]
    fn geometry_default_is_zero() {
        let geom = Geometry::default();
        assert_eq!(geom.x, 0);
        assert_eq!(geom.y, 0);
        assert_eq!(geom.width, 0);
        assert_eq!(geom.height, 0);
        assert_eq!(geom.border_offset, 0);
        assert_eq!(geom.cursor_x, 0);
        assert_eq!(geom.cursor_y, 0);
    }

    #[test]
    fn publish_and_read_geometry() {
        let win = Window::without_pty("test", "Test", WinSize { cols: 80, rows: 24 });
        win.publish_geometry(10, 20, 100, 50, 1);
        let geom = win.last_geometry().unwrap();
        assert_eq!(geom.x, 10);
        assert_eq!(geom.y, 20);
        assert_eq!(geom.width, 100);
        assert_eq!(geom.height, 50);
        assert_eq!(geom.border_offset, 1);
    }

    #[test]
    fn publish_geometry_is_idempotent() {
        let win = Window::without_pty("test", "Test", WinSize { cols: 80, rows: 24 });
        win.publish_geometry(5, 10, 80, 24, 1);
        let first = win.last_geometry().unwrap();
        // Publish the same geometry again — should be a no-op.
        win.publish_geometry(5, 10, 80, 24, 1);
        let second = win.last_geometry().unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn without_pty_publishes_initial_geometry() {
        let win = Window::without_pty("test", "Test", WinSize { cols: 80, rows: 24 });
        let geom = win.last_geometry().unwrap();
        assert_eq!(geom.width, 80);
        assert_eq!(geom.height, 24);
    }

    #[test]
    fn queue_and_flush_output() {
        let mut win = Window::without_pty("test", "Test", WinSize { cols: 80, rows: 24 });
        win.queue_output(b"hello ");
        win.queue_output(b"world");
        assert_eq!(win.output_buffer.len(), 11);

        let written = win.flush_output();
        assert_eq!(written, 11);
        assert!(win.output_buffer.is_empty());

        // The emulator should have received the data.
        let emu = win.emulator.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(emu.width(), 80);
        assert_eq!(emu.height(), 24);
    }

    #[test]
    fn flush_output_empty_is_zero() {
        let mut win = Window::without_pty("test", "Test", WinSize { cols: 80, rows: 24 });
        assert_eq!(win.flush_output(), 0);
    }

    #[test]
    fn flush_output_coalesces_multiple_writes() {
        let mut win = Window::without_pty("test", "Test", WinSize { cols: 80, rows: 24 });
        for i in 0..10 {
            win.queue_output(format!("line {i}\n").as_bytes());
        }
        let written = win.flush_output();
        assert_eq!(written, win.output_buffer.len() + written); // tautology check
        // Each "line N\n" is 7 bytes, 10 lines = 70 bytes.
        assert_eq!(written, 70);
    }

    #[test]
    fn queue_output_auto_flushes_at_cap() {
        let mut win = Window::without_pty("test", "Test", WinSize { cols: 80, rows: 24 });
        // Queue more than MAX_BATCH bytes to trigger auto-flush.
        let big = vec![b'A'; MAX_BATCH + 1];
        win.queue_output(&big);
        // After auto-flush, the buffer should be empty (all written).
        assert!(win.output_buffer.is_empty());
    }

    #[test]
    fn defer_and_flush_resize() {
        let mut win = Window::without_pty("test", "Test", WinSize { cols: 80, rows: 24 });
        // Defer several sizes in rapid succession.
        win.defer_resize(WinSize { cols: 100, rows: 30 });
        win.defer_resize(WinSize { cols: 120, rows: 40 });
        win.defer_resize(WinSize { cols: 90, rows: 25 });

        assert!(win.has_pending_resize());

        // Flush applies the last deferred size. The first resize from
        // None→90,25 returns false (no previous size to change from),
        // but the pending state is cleared.
        let _ = win.flush_resize();
        assert!(!win.has_pending_resize());

        // A second flush should be a no-op.
        let applied_again = win.flush_resize();
        assert!(!applied_again);

        // Now defer a new size and flush — it should report a change.
        win.defer_resize(WinSize { cols: 60, rows: 15 });
        assert!(win.has_pending_resize());
        let applied = win.flush_resize();
        assert!(applied);
        assert!(!win.has_pending_resize());
    }

    #[test]
    fn remote_window_emulator_path_handles_wide_and_combining() {
        // The web/SSH clients attach to a daemon and feed PTY output into
        // `Window::remote` emulators via a channel + drain_thread. Drive that
        // exact path and verify the wide-char/combining fixes hold end to end:
        // selection text stays clean (no phantom spaces) and line extraction
        // preserves graphemes.
        let (tx, rx) = crossbeam_channel::unbounded();
        struct Sink;
        impl PtySink for Sink {
            fn write(&self, _data: &[u8]) {}
            fn resize(&self, _size: WinSize) {}
        }
        let win = Window::remote("w0", "remote", WinSize { cols: 80, rows: 24 }, Box::new(Sink), rx);

        // Feed wide CJK + combining marks exactly as a daemon PTY would emit.
        tx.send(b"\x1b[2J\x1b[H".to_vec()).unwrap();
        tx.send("\u{4f60}\u{4f60}XX end\n".as_bytes().to_vec()).unwrap();
        // Decomposed Latin: e + U+0301 must stay attached to one cell.
        tx.send("e\u{301}Z\n".as_bytes().to_vec()).unwrap();
        // Drain (bounded wait).
        let emu = win.emulator.clone();
        for _ in 0..100 {
            let ok = emu.lock().map(|e| {
                let t = e.content_line_text(0);
                t.contains("end") && e.content_line_text(1).contains('Z')
            }).unwrap_or(false);
            if ok {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        let emu = emu.lock().unwrap_or_else(|e| e.into_inner());
        // Selection across the whole CJK run: clean, no phantom spaces.
        let text = emu.selection_text(0, 0, 0, 9);
        assert_eq!(text, "\u{4f60}\u{4f60}XX end");
        // Combining mark rides the base char in the extracted line (lines
        // are padded to full width, so trim trailing spaces).
        let line = emu.content_line_text(1);
        assert!(line.trim_end().ends_with("e\u{301}Z"), "got: {line:?}");
        // Grid invariant: no width-0 occupied cells from the remote feed.
        for (i, cell) in emu.content_line(1).iter().enumerate() {
            if let Some(_c) = cell.content {
                assert!(cell.width >= 1, "cell {i} occupied with width 0");
            }
        }
    }

    #[test]
    fn flush_resize_no_pending_is_false() {
        let mut win = Window::without_pty("test", "Test", WinSize { cols: 80, rows: 24 });
        assert!(!win.has_pending_resize());
        assert!(!win.flush_resize());
    }

    #[test]
    fn reset_terminal_clears_output_and_graphics() {
        let mut win = Window::without_pty("test", "Test", WinSize { cols: 80, rows: 24 });
        // Queue some output so we can verify it is flushed.
        win.queue_output(b"some text");
        assert!(!win.output_buffer.is_empty());

        win.reset_terminal();

        // Output buffer should be flushed (empty after reset).
        assert!(win.output_buffer.is_empty());
        // Emulator should still be functional.
        let emu = win.emulator.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(emu.width(), 80);
        assert_eq!(emu.height(), 24);
    }

    #[test]
    fn set_cell_pixel_dimensions_stores_values() {
        let mut win = Window::without_pty("test", "Test", WinSize { cols: 80, rows: 24 });
        assert_eq!(win.cell_pixel_width(), 0);
        assert_eq!(win.cell_pixel_height(), 0);

        win.set_cell_pixel_dimensions(10, 20);
        assert_eq!(win.cell_pixel_width(), 10);
        assert_eq!(win.cell_pixel_height(), 20);
    }

    #[test]
    fn set_cell_pixel_dimensions_zero_is_valid() {
        let mut win = Window::without_pty("test", "Test", WinSize { cols: 80, rows: 24 });
        win.set_cell_pixel_dimensions(0, 0);
        assert_eq!(win.cell_pixel_width(), 0);
        assert_eq!(win.cell_pixel_height(), 0);
    }

    #[test]
    fn foreground_process_info_without_pty_is_none() {
        let win = Window::without_pty("test", "Test", WinSize { cols: 80, rows: 24 });
        assert!(win.foreground_process_info().is_none());
    }

    #[test]
    fn foreground_process_name_without_pty_is_empty() {
        let win = Window::without_pty("test", "Test", WinSize { cols: 80, rows: 24 });
        assert!(win.foreground_process_name().is_empty());
    }

    #[test]
    fn close_flushes_pending_output_and_resize() {
        let mut win = Window::without_pty("test", "Test", WinSize { cols: 80, rows: 24 });
        win.queue_output(b"pending");
        win.defer_resize(WinSize { cols: 100, rows: 30 });
        win.close();
        assert!(win.exited);
        assert!(win.output_buffer.is_empty());
        assert!(!win.has_pending_resize());
    }

    #[test]
    fn resize_updates_geometry_snapshot() {
        let mut win = Window::without_pty("test", "Test", WinSize { cols: 80, rows: 24 });
        // Initial geometry from without_pty.
        let geom0 = win.last_geometry().unwrap();
        assert_eq!(geom0.width, 80);
        assert_eq!(geom0.height, 24);

        // Resize should update the snapshot.
        win.resize(WinSize { cols: 120, rows: 40 });
        let geom1 = win.last_geometry().unwrap();
        assert_eq!(geom1.width, 120);
        assert_eq!(geom1.height, 40);
    }

    #[test]
    fn resize_same_size_is_noop() {
        let mut win = Window::without_pty("test", "Test", WinSize { cols: 80, rows: 24 });
        // First resize from None → Some is not a "changed" resize.
        let first = win.resize(WinSize { cols: 80, rows: 24 });
        assert!(!first);
        // Same size again is a no-op.
        let second = win.resize(WinSize { cols: 80, rows: 24 });
        assert!(!second);
        // Different size is a real change.
        let third = win.resize(WinSize { cols: 100, rows: 30 });
        assert!(third);
    }

    #[test]
    fn drain_thread_coalesces_and_writes() {
        let (tx, rx) = crossbeam_channel::unbounded::<Vec<u8>>();
        let emulator = Arc::new(Mutex::new(Emulator::new(80, 24)));
        let emu_clone = Arc::clone(&emulator);
        let dirty_clone = Arc::new(AtomicBool::new(true));
        let handle = std::thread::spawn(move || drain_thread(rx, emu_clone, dirty_clone));

        // Send several small chunks.
        tx.send(b"hello ".to_vec()).unwrap();
        tx.send(b"world\n".to_vec()).unwrap();
        // Drop the sender to end the drain thread.
        drop(tx);
        handle.join().unwrap();

        // Verify the emulator received the data by checking it has content.
        let emu = emulator.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(emu.width(), 80);
        assert_eq!(emu.height(), 24);
    }

    #[test]
    fn spawned_window_becomes_dirty_for_output_after_initial_frame() {
        crate::skip_if_pty_exhausted!();
        let opts = SpawnOptions {
            command: Some("printf INITIAL; sleep 0.5; printf DIRTY_REPRO"),
            suspended: false,
        };
        let wake = Box::new(|| {}) as Box<dyn Fn() + Send + 'static>;
        let win = Window::spawn(
            "dirty",
            "Dirty flag",
            WinSize { cols: 80, rows: 24 },
            "/bin/sh",
            opts,
            wake,
            &[],
        )
        .expect("PTY should spawn");

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let text = win.emulator.lock().unwrap().render_text();
            if text.contains("INITIAL") {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "initial output never arrived");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }

        // Simulate the renderer completing the initial frame.
        win.clear_dirty();
        assert!(!win.is_dirty());

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let text = win.emulator.lock().unwrap().render_text();
            if text.contains("DIRTY_REPRO") {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "later output never arrived");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }

        assert!(win.is_dirty(), "new PTY output must invalidate the window");
    }

    // --- Phase G tests ---

    #[test]
    fn scrollback_mode_enter_and_exit() {
        let mut win = Window::without_pty("test", "Test", WinSize { cols: 80, rows: 24 });
        assert!(!win.scrollback_mode());
        win.enter_scrollback_mode();
        assert!(win.scrollback_mode());
        assert_eq!(win.scrollback_offset(), 0);
        win.exit_scrollback_mode();
        assert!(!win.scrollback_mode());
    }

    #[test]
    fn scroll_up_and_down() {
        let mut win = Window::without_pty("test", "Test", WinSize { cols: 80, rows: 24 });
        win.enter_scrollback_mode();
        // Without scrollback content, scroll_up is clamped to 0.
        win.scroll_up(10);
        assert_eq!(win.scrollback_offset(), 0);
        // scroll_down at offset 0 exits scrollback mode.
        win.scroll_down(5);
        assert!(!win.scrollback_mode());
    }

    #[test]
    fn copy_mode_enter_and_exit() {
        let mut win = Window::without_pty("test", "Test", WinSize { cols: 80, rows: 24 });
        assert!(!win.in_copy_mode());
        win.enter_copy_mode();
        assert!(win.in_copy_mode());
        assert!(win.copy_mode_visible());
        assert!(!win.in_implicit_copy_mode());
        win.exit_copy_mode();
        assert!(!win.in_copy_mode());
    }

    #[test]
    fn copy_mode_implicit() {
        let mut win = Window::without_pty("test", "Test", WinSize { cols: 80, rows: 24 });
        win.enter_copy_mode_implicit();
        assert!(win.in_copy_mode());
        assert!(!win.copy_mode_visible());
        assert!(win.in_implicit_copy_mode());
        win.exit_copy_mode();
    }

    #[test]
    fn copy_cursor_set_and_get() {
        let mut win = Window::without_pty("test", "Test", WinSize { cols: 80, rows: 24 });
        win.enter_copy_mode();
        win.set_copy_cursor(10, 5);
        assert_eq!(win.copy_cursor(), (10, 5));
    }

    #[test]
    fn screen_to_terminal_inside() {
        let win = Window::without_pty("test", "Test", WinSize { cols: 80, rows: 24 });
        win.publish_geometry(10, 20, 80, 24, 1);
        let (tx, ty, inside) = win.screen_to_terminal(15, 25);
        assert!(inside);
        assert_eq!(tx, 4);
        assert_eq!(ty, 4);
    }

    #[test]
    fn screen_to_terminal_outside() {
        let win = Window::without_pty("test", "Test", WinSize { cols: 80, rows: 24 });
        win.publish_geometry(10, 20, 80, 24, 1);
        let (_tx, _ty, inside) = win.screen_to_terminal(5, 5);
        assert!(!inside);
    }

    #[test]
    fn seed_announced_size() {
        let mut win = Window::without_pty("test", "Test", WinSize { cols: 80, rows: 24 });
        win.seed_announced_size(78, 22);
        assert_eq!(win.announced_width(), 78);
        assert_eq!(win.announced_height(), 22);
    }

    #[test]
    fn hold_and_release_announcements() {
        let mut win = Window::without_pty("test", "Test", WinSize { cols: 80, rows: 24 });
        win.hold_announcements();
        // Release without a change should be a no-op (no panic).
        win.release_announcements();
    }

    #[test]
    fn border_offset_tiled_vs_bordered() {
        let mut win = Window::without_pty("test", "Test", WinSize { cols: 80, rows: 24 });
        assert_eq!(win.border_offset(), 1);
        win.set_tiled(true);
        assert_eq!(win.border_offset(), 0);
    }

    #[test]
    fn trigger_redraw_without_pty_is_safe() {
        let win = Window::without_pty("test", "Test", WinSize { cols: 80, rows: 24 });
        // Should not panic even without a PTY.
        win.trigger_redraw();
    }

    #[test]
    fn daemon_writer_set_and_get() {
        let mut win = Window::without_pty("test", "Test", WinSize { cols: 80, rows: 24 });
        assert!(win.daemon_writer().is_none());
        let emulator = Arc::clone(&win.emulator);
        let writer = Arc::new(super::super::window_io::DaemonOutputWriter::new(emulator, None));
        win.set_daemon_writer(writer);
        assert!(win.daemon_writer().is_some());
    }

    // --- Command panes ---

    fn spawn_cmd(cmd: &str, suspended: bool) -> Window {
        let size = WinSize { cols: 80, rows: 24 };
        // Retry briefly on PTY pressure (this box runs near the pty ceiling).
        let mut last = None;
        for _ in 0..10 {
            let wake = Box::new(|| {}) as Box<dyn Fn() + Send + 'static>;
            let opts = SpawnOptions { command: Some(cmd), suspended };
            match Window::spawn("cp", "cmd", size, "/bin/sh", opts, wake, &[]) {
                Ok(w) => return w,
                Err(e) => {
                    last = Some(e);
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            }
        }
        panic!("spawn kept failing: {last:?}")
    }

    fn wait_text(win: &Window, needle: &str, timeout: std::time::Duration) -> String {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let text = win.emulator.lock().unwrap().render_text();
            if text.contains(needle) {
                return text;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for {needle:?}; text: {text:?}"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[test]
    fn command_pane_captures_exit_code_and_reruns() {
        crate::skip_if_pty_exhausted!();
        let mut win = spawn_cmd("echo FIRST_RUN; exit 7", false);
        wait_text(&win, "FIRST_RUN", std::time::Duration::from_secs(10));

        // Poll until the child is reaped and the exit status recorded.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline && !win.poll_exit() {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(win.exit_code, Some(7), "exit status not captured");
        assert!(win.can_rerun());
        assert!(win.exited);

        // Re-run the same command: fresh emulator, running again.
        let size = WinSize { cols: 80, rows: 24 };
        win.restart(size, Box::new(|| {}) as Box<dyn Fn() + Send + 'static>, &[])
            .expect("restart");
        assert_eq!(win.exit_code, None);
        assert!(!win.exited);
        wait_text(&win, "FIRST_RUN", std::time::Duration::from_secs(10));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline && !win.poll_exit() {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(win.exit_code, Some(7), "exit status after re-run");
    }

    /// The child's process state letter from /proc: `T` = stopped,
    /// `S`/`R` = running, `Z` = zombie, `X` = gone.
    fn proc_state(pid: i32) -> String {
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .and_then(|s| s.split_whitespace().nth(2).map(|c| c.to_string()))
            .unwrap_or_else(|| "gone".into())
    }

    #[test]
    fn suspended_command_pane_waits_for_trigger() {
        crate::skip_if_pty_exhausted!();
        let mut win = spawn_cmd("echo SUSPENDED_OUTPUT", true);
        // The pane explains itself, and the child is genuinely stopped.
        wait_text(&win, "[suspended]", std::time::Duration::from_secs(10));
        assert!(win.suspended);
        assert!(!win.can_rerun());
        let pid = win.pid().expect("child pid");
        assert_eq!(
            proc_state(pid),
            "T",
            "suspended child must be stopped (SIGSTOP), got {:?}",
            proc_state(pid)
        );
        assert!(!win.poll_exit(), "a stopped child must not count as exited");

        // First Enter resumes it; the command then runs and exits cleanly.
        assert!(win.resume_if_suspended());
        assert!(!win.suspended);
        assert!(!win.resume_if_suspended(), "second resume must be a no-op");
        wait_text(&win, "SUSPENDED_OUTPUT", std::time::Duration::from_secs(10));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline && !win.poll_exit() {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(win.exit_code, Some(0), "resumed command should exit 0");
    }
}
