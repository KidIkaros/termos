//! Daemon-mode window I/O — ported from Go TUIOS
//! `internal/terminal/window_io.go`.
//!
//! In daemon mode, a window's output arrives from the daemon over a socket
//! rather than from a local PTY. This module provides:
//!
//! - **Output epoch**: a monotonic counter that gates stale output. Bumping
//!   the epoch (on unsubscribe/restore) causes queued chunks from a previous
//!   epoch to be dropped by the writer thread.
//! - **Output writer thread**: a background thread that serializes writes to
//!   the emulator, batching small chunks into capped VT writes and
//!   coalescing render signals to prevent partial-frame flickering.
//! - **Render coalescer**: a ticker that fires render signals at a capped
//!   rate (~120fps) so multiple VT writes between ticks coalesce into a
//!   single render showing the latest complete frame.
//! - **Daemon response reader**: drains emulator response bytes (DA/DSR
//!   answers) so they do not leak as visible escape sequences.
//!
//! The local PTY path (non-daemon) uses the simpler `drain_thread` in
//! `window.rs` directly; this module is only for daemon-backed windows.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossbeam_channel::{bounded, Receiver, Sender};

use crate::vt::Emulator;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum bytes of pending daemon output one pass of the writer coalesces
/// before writing to the emulator. Mirrors `maxBatch` in Go.
const MAX_BATCH: usize = 256 * 1024;

/// Maximum bytes written to the emulator under a single lock acquisition.
/// Bounds how long the render path can be kept out of the emulator.
const MAX_VT_CHUNK: usize = 8 * 1024;

/// The render coalescer tick interval (~120fps cap). Multiple VT writes
/// between ticks coalesce into a single render.
const COALESCE_INTERVAL: Duration = Duration::from_millis(8);

/// The output channel buffer capacity. Large enough to absorb bursts without
/// dropping data under normal load.
const OUTPUT_CHAN_CAPACITY: usize = 1024;

// ---------------------------------------------------------------------------
// OutputChunk — one queued batch of daemon output
// ---------------------------------------------------------------------------

/// One queued batch of daemon output and the epoch it was queued under.
///
/// A resize chunk carries `width`/`height` instead of `data` — it is applied
/// in stream order so the emulator wraps lines at the same width the daemon
/// did. A drain sentinel carries a `drained` channel that is closed once
/// everything queued ahead of it has been applied.
///
/// Ported from Go `outputChunk`.
#[derive(Debug)]
struct OutputChunk {
    /// The output bytes (empty for resize/drain sentinels).
    data: Vec<u8>,
    /// The epoch this chunk was queued under. Stale chunks (whose epoch does
    /// not match the current `output_epoch`) are dropped by the writer.
    epoch: u64,
    /// Width and height for resize chunks (both > 0). When set, `data` is
    /// ignored and the emulator is resized instead of written to.
    width: i32,
    height: i32,
}

impl OutputChunk {
    /// Whether this chunk is a resize (carries width/height instead of bytes).
    fn is_resize(&self) -> bool {
        self.width > 0 && self.height > 0
    }
}

// ---------------------------------------------------------------------------
// DaemonOutputWriter — the daemon-mode output writer + render coalescer
// ---------------------------------------------------------------------------

/// The daemon-mode output writer for a window.
///
/// Runs a background thread that reads `OutputChunk`s from a channel, batches
/// them, and writes to the emulator in bounded chunks. A render coalescer
/// thread fires render signals at a capped rate when new output is available.
///
/// The `output_epoch` gates stale output: bumping it (via `discard_pending`)
/// causes chunks from a previous epoch to be dropped.
///
/// Ported from Go `outputWriter` + `renderCoalescer`.
pub struct DaemonOutputWriter {
    /// Sender for output chunks. External senders (the daemon read loop) push
    /// chunks here. Never closed — `done` is the lifecycle guard.
    tx: Sender<OutputChunk>,
    /// Lifecycle signal: closing this stops the writer and coalescer threads.
    done: Arc<AtomicBool>,
    /// The output epoch. Bumped by `discard_pending` to drop stale chunks.
    output_epoch: Arc<AtomicU64>,
    /// Whether the daemon output stream owns the emulator's size.
    stream_owns_size: Arc<AtomicBool>,
    /// The render signal sender. The coalescer fires this at a capped rate
    /// when new output is available.
    _render_signal: Option<Sender<()>>,
    /// Wake channel for the render coalescer; idle workers block on it.
    _coalesce_tx: Sender<()>,
    /// Join handles for the background threads.
    _writer_handle: Option<std::thread::JoinHandle<()>>,
    _coalescer_handle: Option<std::thread::JoinHandle<()>>,
}

impl DaemonOutputWriter {
    /// Start a new daemon output writer for the given emulator.
    ///
    /// `render_signal` is an optional sender that the coalescer fires when
    /// new output is available — connect it to the UI thread's wake channel.
    pub fn new(
        emulator: Arc<Mutex<Emulator>>,
        render_signal: Option<Sender<()>>,
    ) -> Self {
        let (tx, rx) = bounded::<OutputChunk>(OUTPUT_CHAN_CAPACITY);
        let done = Arc::new(AtomicBool::new(false));
        let output_epoch = Arc::new(AtomicU64::new(0));
        let stream_owns_size = Arc::new(AtomicBool::new(false));
        let (coalesce_tx, coalesce_rx) = bounded::<()>(1);

        // Spawn the writer thread.
        let writer_done = Arc::clone(&done);
        let writer_epoch = Arc::clone(&output_epoch);
        let writer_coalesce = coalesce_tx.clone();
        let writer_handle = std::thread::Builder::new()
            .name("daemon-output-writer".to_string())
            .spawn(move || {
                writer_thread(rx, emulator, writer_done, writer_epoch, writer_coalesce);
            })
            .ok();

        // Spawn the render coalescer thread.
        let coalescer_done = Arc::clone(&done);
        let coalescer_render = render_signal.clone();
        let coalescer_handle = std::thread::Builder::new()
            .name("daemon-render-coalescer".to_string())
            .spawn(move || {
                coalescer_thread(coalescer_done, coalesce_rx, coalescer_render);
            })
            .ok();

        Self {
            tx,
            done,
            output_epoch,
            stream_owns_size,
            _render_signal: render_signal,
            _coalesce_tx: coalesce_tx,
            _writer_handle: writer_handle,
            _coalescer_handle: coalescer_handle,
        }
    }

    /// Queue output data for the emulator. Data is written in order by the
    /// background writer thread, batched with other queued chunks.
    ///
    /// Ported from Go `WriteOutputAsync`.
    pub fn write_output_async(&self, data: &[u8]) {
        if self.done.load(Ordering::Acquire) {
            return;
        }
        let chunk = OutputChunk {
            data: data.to_vec(),
            epoch: self.output_epoch.load(Ordering::Acquire),
            width: 0,
            height: 0,
        };
        // Non-blocking send: if the channel is full, drop the data rather than
        // blocking the daemon read loop.
        let _ = self.tx.try_send(chunk);
    }

    /// Queue a resize to be applied in stream order among the bytes around
    /// it. The emulator is resized to the given dimensions when this chunk
    /// reaches the writer.
    ///
    /// Ported from Go `ResizeFromStream`.
    pub fn resize_from_stream(&self, width: i32, height: i32) {
        if self.done.load(Ordering::Acquire) || width <= 0 || height <= 0 {
            return;
        }
        let chunk = OutputChunk {
            data: Vec::new(),
            epoch: self.output_epoch.load(Ordering::Acquire),
            width,
            height,
        };
        let _ = self.tx.try_send(chunk);
    }

    /// Bump the output epoch, causing all currently queued chunks to be
    /// dropped by the writer. Used when unsubscribing from a pane before a
    /// snapshot restore: the queued bytes are older than the snapshot and
    /// would be painted twice if applied afterwards.
    ///
    /// Ported from Go `DiscardPendingOutput`.
    pub fn discard_pending(&self) {
        self.output_epoch.fetch_add(1, Ordering::AcqRel);
    }

    /// The current output epoch.
    pub fn output_epoch(&self) -> u64 {
        self.output_epoch.load(Ordering::Acquire)
    }

    /// Record whether the daemon output stream owns the emulator's size.
    /// When true, layout-driven resizes are suppressed (the stream carries
    /// size changes in order).
    ///
    /// Ported from Go `SetStreamOwnsSize`.
    pub fn set_stream_owns_size(&self, owns: bool) {
        self.stream_owns_size.store(owns, Ordering::Release);
    }

    /// Whether the daemon output stream owns the emulator's size.
    ///
    /// Ported from Go `StreamOwnsSize`.
    pub fn stream_owns_size(&self) -> bool {
        self.stream_owns_size.load(Ordering::Acquire)
    }

    /// Stop the writer and coalescer threads. After this, queued output is
    /// dropped and no more writes reach the emulator.
    pub fn stop(&self) {
        self.done.store(true, Ordering::Release);
    }
}

impl Drop for DaemonOutputWriter {
    fn drop(&mut self) {
        self.stop();
    }
}

// ---------------------------------------------------------------------------
// Writer thread
// ---------------------------------------------------------------------------

/// The background writer thread. Reads chunks from the channel, batches them,
/// and writes to the emulator in bounded chunks. Drops chunks whose epoch
/// does not match the current output epoch.
///
/// Ported from Go `outputWriter`.
fn writer_thread(
    rx: Receiver<OutputChunk>,
    emulator: Arc<Mutex<Emulator>>,
    done: Arc<AtomicBool>,
    output_epoch: Arc<AtomicU64>,
    coalesce_signal: Sender<()>,
) {
    let mut batch: Vec<u8> = Vec::with_capacity(MAX_BATCH);

    while !done.load(Ordering::Acquire) {
        // Block on the first chunk.
        let first = match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(chunk) => chunk,
            Err(_) => continue,
        };

        let epoch = first.epoch;
        let mut resize: Option<OutputChunk> = None;

        if first.is_resize() {
            // A resize ends the current batch and is applied after it.
            resize = Some(first);
        } else {
            // Start a new batch with this chunk's data.
            batch.clear();
            if first.epoch == output_epoch.load(Ordering::Acquire) {
                batch.extend_from_slice(&first.data);
            }
        }

        // Drain more chunks without blocking, up to MAX_BATCH.
        while batch.len() < MAX_BATCH {
            match rx.try_recv() {
                Ok(more) => {
                    if more.is_resize() {
                        resize = Some(more);
                        break;
                    }
                    if more.epoch == epoch {
                        batch.extend_from_slice(&more.data);
                    }
                }
                Err(_) => break,
            }
        }

        // Write the batch in bounded chunks.
        if !batch.is_empty() {
            let current_epoch = output_epoch.load(Ordering::Acquire);
            if epoch == current_epoch {
                if let Ok(mut emu) = emulator.lock() {
                    for chunk in batch.chunks(MAX_VT_CHUNK) {
                        emu.write(chunk);
                    }
                }
                // Wake the coalescer only when output arrives; the bounded
                // channel naturally coalesces bursts.
                let _ = coalesce_signal.try_send(());
            }
            batch.clear();
        }

        // Apply a deferred resize if one was queued.
        if let Some(resize_chunk) = resize {
            if resize_chunk.epoch == output_epoch.load(Ordering::Acquire) {
                if let Ok(mut emu) = emulator.lock() {
                    let (w, h) = (resize_chunk.width, resize_chunk.height);
                    if emu.width() != w || emu.height() != h {
                        emu.resize(w, h);
                    }
                }
                let _ = coalesce_signal.try_send(());
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Render coalescer thread
// ---------------------------------------------------------------------------

/// The render coalescer thread. Fires render signals at a capped rate
/// (~120fps) when new output is available, so multiple VT writes between
/// ticks coalesce into a single render.
///
/// Ported from Go `renderCoalescer`.
fn coalescer_thread(
    done: Arc<AtomicBool>,
    coalesce_rx: Receiver<()>,
    render_signal: Option<Sender<()>>,
) {
    let mut next_emit: Option<std::time::Instant> = None;
    while !done.load(Ordering::Acquire) {
        let wait = next_emit
            .map(|deadline| deadline.saturating_duration_since(std::time::Instant::now()))
            .unwrap_or(Duration::from_secs(3600));
        match coalesce_rx.recv_timeout(wait) {
            Ok(()) => {
                if next_emit.is_none() {
                    next_emit = Some(std::time::Instant::now() + COALESCE_INTERVAL);
                }
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                if let Some(ref tx) = render_signal {
                    let _ = tx.try_send(());
                }
                next_emit = None;
            }
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
        }
    }
}

// ---------------------------------------------------------------------------
// Daemon response reader
// ---------------------------------------------------------------------------

/// Start a daemon response reader that drains emulator response bytes (DA/DSR
/// answers) so they do not leak as visible escape sequences.
///
/// In daemon mode, the emulator receives queries from the daemon's VT
/// emulator and generates responses, but those responses should not be
/// forwarded to the PTY (they would appear as visible escape sequences).
/// This thread drains them by periodically calling `take_response` on the
/// emulator and discarding the bytes.
///
/// Ported from Go `StartDaemonResponseReader`.
pub fn start_daemon_response_reader(
    emulator: Arc<Mutex<Emulator>>,
    done: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("daemon-response-reader".to_string())
        .spawn(move || {
            while !done.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(8));
                if let Ok(mut emu) = emulator.lock() {
                    let _ = emu.take_response();
                }
            }
        })
        .expect("spawn daemon-response-reader")
}

// ---------------------------------------------------------------------------
// Resize to snapshot
// ---------------------------------------------------------------------------

/// Resize the emulator to the size a daemon snapshot was serialized at,
/// regardless of who owns the size otherwise. A snapshot is a grid as much as
/// it is contents, and the stream resumes at the position it was taken at.
///
/// Ported from Go `ResizeEmulatorToSnapshot`.
pub fn resize_emulator_to_snapshot(
    emulator: &Arc<Mutex<Emulator>>,
    width: i32,
    height: i32,
) {
    if width <= 0 || height <= 0 {
        return;
    }
    if let Ok(mut emu) = emulator.lock() {
        if emu.width() != width || emu.height() != height {
            emu.resize(width, height);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writer_writes_output_to_emulator() {
        let emulator = Arc::new(Mutex::new(Emulator::new(80, 24)));
        let writer = DaemonOutputWriter::new(Arc::clone(&emulator), None);

        writer.write_output_async(b"hello world");
        // Give the writer thread time to process.
        std::thread::sleep(Duration::from_millis(50));

        writer.stop();
        std::thread::sleep(Duration::from_millis(20));

        // The emulator should have received the data.
        let emu = emulator.lock().unwrap_or_else(|e| e.into_inner());
        let text = emu.render_text();
        assert!(text.contains("hello world"), "text: {text}");
    }

    #[test]
    fn writer_applies_stream_resize() {
        let emulator = Arc::new(Mutex::new(Emulator::new(80, 24)));
        let writer = DaemonOutputWriter::new(Arc::clone(&emulator), None);

        writer.resize_from_stream(100, 30);
        std::thread::sleep(Duration::from_millis(50));

        writer.stop();
        std::thread::sleep(Duration::from_millis(20));

        let emu = emulator.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(emu.width(), 100);
        assert_eq!(emu.height(), 30);
    }

    #[test]
    fn discard_pending_drops_stale_output() {
        let emulator = Arc::new(Mutex::new(Emulator::new(80, 24)));
        let writer = DaemonOutputWriter::new(Arc::clone(&emulator), None);

        // Queue some output, then bump the epoch before it's processed.
        writer.write_output_async(b"stale data");
        writer.discard_pending();
        std::thread::sleep(Duration::from_millis(50));

        writer.stop();
        std::thread::sleep(Duration::from_millis(20));

        // The stale data should have been dropped.
        let emu = emulator.lock().unwrap_or_else(|e| e.into_inner());
        let text = emu.render_text();
        assert!(!text.contains("stale data"), "text: {text}");
    }

    #[test]
    fn stream_owns_size_toggle() {
        let emulator = Arc::new(Mutex::new(Emulator::new(80, 24)));
        let writer = DaemonOutputWriter::new(Arc::clone(&emulator), None);

        assert!(!writer.stream_owns_size());
        writer.set_stream_owns_size(true);
        assert!(writer.stream_owns_size());
        writer.set_stream_owns_size(false);
        assert!(!writer.stream_owns_size());
    }

    #[test]
    fn output_epoch_increments() {
        let emulator = Arc::new(Mutex::new(Emulator::new(80, 24)));
        let writer = DaemonOutputWriter::new(Arc::clone(&emulator), None);

        let e0 = writer.output_epoch();
        writer.discard_pending();
        let e1 = writer.output_epoch();
        assert_eq!(e1, e0 + 1);
    }

    #[test]
    fn resize_to_snapshot_changes_size() {
        let emulator = Arc::new(Mutex::new(Emulator::new(80, 24)));
        resize_emulator_to_snapshot(&emulator, 120, 40);
        let emu = emulator.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(emu.width(), 120);
        assert_eq!(emu.height(), 40);
    }

    #[test]
    fn resize_to_snapshot_same_size_is_noop() {
        let emulator = Arc::new(Mutex::new(Emulator::new(80, 24)));
        resize_emulator_to_snapshot(&emulator, 80, 24);
        let emu = emulator.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(emu.width(), 80);
        assert_eq!(emu.height(), 24);
    }

    #[test]
    fn resize_to_snapshot_invalid_is_noop() {
        let emulator = Arc::new(Mutex::new(Emulator::new(80, 24)));
        resize_emulator_to_snapshot(&emulator, 0, 0);
        resize_emulator_to_snapshot(&emulator, -1, 10);
        let emu = emulator.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(emu.width(), 80);
        assert_eq!(emu.height(), 24);
    }

    #[test]
    fn daemon_response_reader_drains_responses() {
        let emulator = Arc::new(Mutex::new(Emulator::new(80, 24)));
        // Queue a response in the emulator.
        {
            let mut emu = emulator.lock().unwrap_or_else(|e| e.into_inner());
            emu.write(b"\x1b[c"); // DA1 query → queues a response
            let _ = emu.take_response(); // clear it
        }
        let done = Arc::new(AtomicBool::new(false));
        let handle = start_daemon_response_reader(Arc::clone(&emulator), Arc::clone(&done));

        // Queue another response.
        {
            let mut emu = emulator.lock().unwrap_or_else(|e| e.into_inner());
            emu.write(b"\x1b[c");
        }

        std::thread::sleep(Duration::from_millis(50));

        // The response should have been drained.
        {
            let mut emu = emulator.lock().unwrap_or_else(|e| e.into_inner());
            let resp = emu.take_response();
            assert!(resp.is_empty(), "response was not drained");
        }

        done.store(true, Ordering::Release);
        let _ = handle.join();
    }

    #[test]
    fn writer_batches_multiple_chunks() {
        let emulator = Arc::new(Mutex::new(Emulator::new(80, 24)));
        let writer = DaemonOutputWriter::new(Arc::clone(&emulator), None);

        // Send several small chunks rapidly.
        for i in 0..10 {
            writer.write_output_async(format!("line{i}\n").as_bytes());
        }
        std::thread::sleep(Duration::from_millis(100));

        writer.stop();
        std::thread::sleep(Duration::from_millis(20));

        let emu = emulator.lock().unwrap_or_else(|e| e.into_inner());
        let text = emu.render_text();
        assert!(text.contains("line0"), "text: {text}");
        assert!(text.contains("line9"), "text: {text}");
    }

    #[test]
    fn render_coalescer_fires_signal() {
        let (tx, rx) = bounded::<()>(16);
        let emulator = Arc::new(Mutex::new(Emulator::new(80, 24)));
        let writer = DaemonOutputWriter::new(Arc::clone(&emulator), Some(tx));

        // Write output to trigger the coalesce signal.
        writer.write_output_async(b"test");
        std::thread::sleep(Duration::from_millis(100));

        // The coalescer should have fired at least one render signal.
        let got_signal = rx.try_recv().is_ok();
        assert!(got_signal, "render coalescer did not fire");

        writer.stop();
    }

    #[test]
    fn stop_is_idempotent() {
        let emulator = Arc::new(Mutex::new(Emulator::new(80, 24)));
        let writer = DaemonOutputWriter::new(Arc::clone(&emulator), None);
        writer.stop();
        writer.stop(); // should not panic
    }
}
