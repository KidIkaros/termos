//! The terminal window — a PTY + emulator pair, with an I/O thread that
//! drains PTY output into the emulator and an input path that encodes keys
//! and writes them to the PTY.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::terminal::pty::{spawn_pty, PtyHandle, PtySink, WinSize};
use crate::vt::Emulator;

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
    /// The last size applied, so a same-size resize is a no-op.
    last_size: Option<WinSize>,
}

impl Window {
    pub fn spawn(
        id: impl Into<String>,
        title: impl Into<String>,
        size: WinSize,
        shell: &str,
        command: Option<&str>,
        wake: Box<dyn Fn() + Send + 'static>,
    ) -> Result<Self, crate::terminal::pty::PtyError> {
        let argv: Vec<String> = match command {
            Some(cmd) => vec!["sh".to_string(), "-c".to_string(), cmd.to_string()],
            None => vec![shell.to_string()],
        };

        let (writer, handle, reader) = spawn_pty(size, &argv, wake)?;
        let emulator = Arc::new(Mutex::new(Emulator::new(size.cols as i32, size.rows as i32)));

        let emu_clone = Arc::clone(&emulator);
        std::thread::spawn(move || drain_thread(reader.rx, emu_clone));

        Ok(Self {
            id: id.into(),
            title: title.into(),
            emulator,
            writer: Some(Box::new(writer)),
            handle: Some(handle),
            reading: Arc::new(AtomicBool::new(true)),
            exited: false,
            last_size: None,
        })
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
        let emulator = Arc::new(Mutex::new(Emulator::new(size.cols as i32, size.rows as i32)));
        let emu_clone = Arc::clone(&emulator);
        std::thread::spawn(move || drain_thread(output, emu_clone));
        Self {
            id: id.into(),
            title: title.into(),
            emulator,
            writer: Some(sink),
            handle: None,
            reading: Arc::new(AtomicBool::new(true)),
            exited: false,
            last_size: None,
        }
    }

    /// Create a window without a PTY (used in tests and daemon restore).
    pub fn without_pty(id: impl Into<String>, title: impl Into<String>, size: WinSize) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            emulator: Arc::new(Mutex::new(Emulator::new(size.cols as i32, size.rows as i32))),
            writer: None,
            handle: None,
            reading: Arc::new(AtomicBool::new(true)),
            exited: false,
            last_size: None,
        }
    }

    /// Write encoded bytes to the PTY.
    pub fn write(&self, data: &[u8]) {
        if let Some(writer) = &self.writer {
            writer.write(data);
        }
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
        }
        if let Ok(mut emu) = self.emulator.lock() {
            emu.resize(size.cols as i32, size.rows as i32);
        }
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
    }
}

fn drain_thread(rx: crossbeam_channel::Receiver<Vec<u8>>, emulator: Arc<Mutex<Emulator>>) {
    while let Ok(chunk) = rx.recv() {
        if let Ok(mut emu) = emulator.lock() {
            emu.write(&chunk);
        }
    }
}
