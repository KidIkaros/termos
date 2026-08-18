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

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::unbounded;

    #[test]
    fn remote_sink_new() {
        let (tx, _rx) = unbounded();
        let sink = RemoteSink::new("w1", tx);
        assert_eq!(sink.window, "w1");
    }

    #[test]
    fn remote_sink_new_into_string() {
        let (tx, _rx) = unbounded();
        let sink = RemoteSink::new(String::from("w2"), tx);
        assert_eq!(sink.window, "w2");
    }

    #[test]
    fn remote_sink_write_sends_input_message() {
        let (tx, rx) = unbounded();
        let sink = RemoteSink::new("w1", tx);
        sink.write(b"hello");
        let msg = rx.try_recv().unwrap();
        match msg {
            Message::Input { window, data } => {
                assert_eq!(window, "w1");
                assert_eq!(data, b"hello");
            }
            other => panic!("expected Input message, got {:?}", other),
        }
    }

    #[test]
    fn remote_sink_resize_sends_resize_message() {
        let (tx, rx) = unbounded();
        let sink = RemoteSink::new("w1", tx);
        sink.resize(WinSize { cols: 120, rows: 40 });
        let msg = rx.try_recv().unwrap();
        match msg {
            Message::Resize { window, cols, rows } => {
                assert_eq!(window, "w1");
                assert_eq!(cols, 120);
                assert_eq!(rows, 40);
            }
            other => panic!("expected Resize message, got {:?}", other),
        }
    }

    #[test]
    fn remote_sink_clone_sends_to_same_channel() {
        let (tx, rx) = unbounded();
        let sink1 = RemoteSink::new("w1", tx.clone());
        let sink2 = sink1.clone();
        sink2.write(b"cloned");
        let msg = rx.try_recv().unwrap();
        match msg {
            Message::Input { window, data } => {
                assert_eq!(window, "w1");
                assert_eq!(data, b"cloned");
            }
            other => panic!("expected Input message, got {:?}", other),
        }
    }

    #[test]
    fn remote_sink_multiple_writes() {
        let (tx, rx) = unbounded();
        let sink = RemoteSink::new("w1", tx);
        sink.write(b"first");
        sink.write(b"second");
        sink.write(b"third");
        assert_eq!(rx.len(), 3);
    }
}
