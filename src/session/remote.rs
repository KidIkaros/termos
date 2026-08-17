//! The remote window I/O half: a `PtySink` that encodes input and resize as
//! protocol messages instead of writing to a local PTY.

use crossbeam_channel::Sender;

use crate::terminal::pty::{PtySink, WinSize};

use super::protocol::Message;

/// A `PtySink` for a daemon-backed window. `write`/`resize` become
/// `Input`/`Resize` messages sent to the socket writer thread.
#[derive(Clone)]
pub struct RemoteSink {
    window: String,
    tx: Sender<Message>,
}

impl RemoteSink {
    pub fn new(window: impl Into<String>, tx: Sender<Message>) -> Self {
        Self {
            window: window.into(),
            tx,
        }
    }
}

impl PtySink for RemoteSink {
    fn write(&self, data: &[u8]) {
        let _ = self.tx.send(Message::Input {
            window: self.window.clone(),
            data: data.to_vec(),
        });
    }

    fn resize(&self, size: WinSize) {
        let _ = self.tx.send(Message::Resize {
            window: self.window.clone(),
            cols: size.cols,
            rows: size.rows,
        });
    }
}
