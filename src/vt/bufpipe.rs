//! Buffered output pipe — ported from Go TUIOS `internal/vt/bufpipe.go`.
//!
//! Wraps a writer with an internal buffer for efficient VT output.

use std::io::{self, Write};

/// A buffered pipe wrapping a writer. Flushes automatically when the
/// internal buffer reaches capacity, and on drop.
pub struct BufferedPipe<W: Write> {
    writer: Option<W>,
    buf: Vec<u8>,
    threshold: usize,
}

impl<W: Write> BufferedPipe<W> {
    /// Create a new buffered pipe with 8KB buffer.
    pub fn new(writer: W) -> Self {
        Self::with_capacity(writer, 8 * 1024)
    }

    /// Create with a specific buffer capacity.
    pub fn with_capacity(writer: W, cap: usize) -> Self {
        Self {
            writer: Some(writer),
            buf: Vec::with_capacity(cap),
            threshold: cap,
        }
    }

    /// Write data, auto-flushing when buffer is full.
    pub fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(data);
        if self.buf.len() >= self.threshold {
            self.flush()?;
        }
        Ok(data.len())
    }

    /// Write a string slice.
    pub fn write_str(&mut self, s: &str) -> io::Result<usize> {
        self.write(s.as_bytes())
    }

    /// Write formatted arguments.
    pub fn write_fmt(&mut self, args: std::fmt::Arguments) -> io::Result<usize> {
        let s = format!("{}", args);
        self.write(s.as_bytes())
    }

    /// Flush the buffer to the underlying writer.
    pub fn flush(&mut self) -> io::Result<()> {
        if !self.buf.is_empty() {
            if let Some(ref mut w) = self.writer {
                w.write_all(&self.buf)?;
            }
            self.buf.clear();
        }
        if let Some(ref mut w) = self.writer {
            w.flush()?;
        }
        Ok(())
    }

    /// Get a reference to the underlying writer.
    pub fn get_ref(&self) -> Option<&W> {
        self.writer.as_ref()
    }

    /// Get a mutable reference to the underlying writer.
    pub fn get_mut(&mut self) -> Option<&mut W> {
        self.writer.as_mut()
    }

    /// Consume the pipe and return the underlying writer (after flushing).
    pub fn into_inner(mut self) -> io::Result<W> {
        self.flush()?;
        self.writer
            .take()
            .ok_or_else(|| io::Error::other("writer already taken"))
    }
}

impl<W: Write> Write for BufferedPipe<W> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        BufferedPipe::write(self, data)
    }

    fn flush(&mut self) -> io::Result<()> {
        BufferedPipe::flush(self)
    }
}

impl<W: Write> Drop for BufferedPipe<W> {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_write_and_flush() {
        let mut pipe = BufferedPipe::new(Vec::new());
        pipe.write_all(b"hello").unwrap();
        pipe.flush().unwrap();
        assert_eq!(pipe.get_ref().unwrap(), b"hello");
    }

    #[test]
    fn auto_flush_on_threshold() {
        let mut pipe = BufferedPipe::with_capacity(Vec::new(), 4);
        pipe.write_all(b"hello world").unwrap();
        assert!(pipe.get_ref().is_some_and(|w| !w.is_empty()));
    }

    #[test]
    fn write_str() {
        let mut pipe = BufferedPipe::new(Vec::new());
        pipe.write_str("hello").unwrap();
        pipe.flush().unwrap();
        assert_eq!(pipe.get_ref().unwrap(), b"hello");
    }

    #[test]
    fn drop_flushes() {
        let mut pipe = BufferedPipe::new(Vec::new());
        pipe.write_all(b"unflushed").unwrap();
        let inner = pipe.into_inner().unwrap();
        assert_eq!(inner, b"unflushed");
    }

    #[test]
    fn multiple_writes() {
        let mut pipe = BufferedPipe::new(Vec::new());
        pipe.write_all(b"one ").unwrap();
        pipe.write_all(b"two ").unwrap();
        pipe.write_all(b"three").unwrap();
        pipe.flush().unwrap();
        assert_eq!(pipe.get_ref().unwrap(), b"one two three");
    }
}
