//! Sixel graphics passthrough — forwards DCS `8;...;q` sequences to the host
//! terminal, offsetting the cursor position so the image lands inside the
//! pane. Ported from TUIOS `internal/app/sixel_passthrough.go`.
//!
//! Sixel is simpler than Kitty: there's no image-id remap, and the image is
//! placed at the cursor position. TUIOS positions the cursor at the pane's
//! top-left (using CUP) before forwarding, then restores it.

use std::io::Write;
use std::sync::Mutex;

use super::capability::Capabilities;

/// Sixel passthrough state.
pub struct SixelPassthrough {
    host_out: Mutex<Box<dyn Write + Send>>,
    enabled: bool,
}

impl SixelPassthrough {
    pub fn new(_caps: Capabilities, host_out: Box<dyn Write + Send>) -> Self {
        let enabled = _caps.sixel;
        Self {
            host_out: Mutex::new(host_out),
            enabled,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Forward a raw Sixel stream (the bytes between `DCS 8;...;q` and `ST`)
    /// to the host, positioning it at `(pane_x, pane_y)` in cells.
    pub fn forward(&self, pane_x: u32, pane_y: u32, sixel_data: &[u8]) -> std::io::Result<()> {
        if !self.enabled {
            return Ok(());
        }
        let mut out = self.host_out.lock().unwrap();
        // Save cursor, move to pane origin, write the Sixel DCS, restore.
        // CSI s = save, CSI u = restore (DECSC/DECRC).
        write!(out, "\x1b[s")?;
        write!(out, "\x1b[{};{}H", pane_y + 1, pane_x + 1)?;
        out.write_all(b"\x1bP8")?;
        // The sixel data may include its own DCS header bytes; the caller
        // passes only the payload after `q`.
        out.write_all(b";q")?;
        out.write_all(sixel_data)?;
        out.write_all(b"\x1b\\")?;
        write!(out, "\x1b[u")?;
        out.flush()?;
        Ok(())
    }

    /// Clear all Sixel images on the host (a full-screen erase is the only
    /// portable way; Sixel has no per-image delete).
    pub fn clear_all(&self) -> std::io::Result<()> {
        if !self.enabled {
            return Ok(());
        }
        // ESC [ 2 J clears the screen; the host terminal drops Sixel pixels
        // in the cleared region.
        let mut out = self.host_out.lock().unwrap();
        out.write_all(b"\x1b[2J")?;
        out.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_caps(sixel: bool) -> Capabilities {
        Capabilities {
            kitty: false,
            sixel,
            host: super::super::capability::HostTerminal::WezTerm,
            inside_multiplexer: false,
        }
    }

    #[test]
    fn disabled_is_noop() {
        let sp = SixelPassthrough::new(test_caps(false), Box::new(std::io::sink()));
        assert!(!sp.is_enabled());
        sp.forward(0, 0, b"~!1~").unwrap();
    }

    #[test]
    fn forward_positions_and_wraps() {
        use std::sync::Arc;
        use std::sync::Mutex as StdMutex;
        let buf: Arc<StdMutex<Vec<u8>>> = Arc::new(StdMutex::new(Vec::new()));
        struct SharedWriter(Arc<StdMutex<Vec<u8>>>);
        impl Write for SharedWriter {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(b);
                Ok(b.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let sp = SixelPassthrough::new(test_caps(true), Box::new(SharedWriter(buf.clone())));
        sp.forward(10, 5, b"~!1~").unwrap();
        let buf_inner = buf.lock().unwrap();
        let s = String::from_utf8_lossy(&buf_inner);
        assert!(s.starts_with("\x1b[s"), "got: {s:?}");
        assert!(s.contains("\x1b[6;11H"), "got: {s:?}");
        assert!(s.contains("\x1bP8;q~!1~\x1b\\"), "got: {s:?}");
        assert!(s.ends_with("\x1b[u"), "got: {s:?}");
    }
}
