//! Kitty graphics protocol passthrough — forwards APC G sequences to the
//! host terminal, rewriting image ids and placement coordinates so images
//! follow their pane. Ported from TUIOS `internal/app/kitty_passthrough.go`
//! and `internal/app/kitty_passthrough_forward.go`.
//!
//! The Kitty graphics protocol uses APC (Application Program Command):
//!   `\x1b_G <key=value;...> <payload> \x1b\`  (or `\x9b` for ST)
//!
//! TermOS rewrites:
//!   - `i=<id>` to the host-side id (per-window remap)
//!   - placement coordinates (`x=`, `y=`) to absolute screen positions
//!   - and forwards the payload bytes verbatim (no decode/re-encode).

use std::io::Write;
use std::sync::Mutex;

use super::capability::Capabilities;
use super::placement::PlacementStore;

/// True if a graphics payload looks like an echoed kitty protocol response
/// rather than real image data. Matched against the RAW wire payload (the
/// base64 text between `;` and the APC terminator), NOT the decoded bytes.
///
/// Shape: `^(OK|E[A-Z]{2,}(:.*)?)$` with a hard length cap so a legitimate
/// (necessarily longer, mixed-case) base64 payload cannot match. The
/// error name must be at least 3 chars (`E` + 2 uppercase) to avoid
/// colliding with 2-char base64 chunks like `EN`.
pub fn is_kitty_response(payload: &str) -> bool {
    let bytes = payload.as_bytes();
    if bytes.is_empty() || bytes.len() > 256 {
        return false;
    }
    if payload == "OK" {
        return true;
    }
    if bytes[0] != b'E' {
        return false;
    }
    let mut i = 1;
    while i < bytes.len() && bytes[i].is_ascii_uppercase() {
        i += 1;
    }
    if i < 3 {
        return false; // need at least E + 2 uppercase letters
    }
    if i == bytes.len() {
        return true;
    }
    bytes[i] == b':'
}

/// Kitty graphics passthrough state.
pub struct KittyPassthrough {
    caps: Capabilities,
    /// The host terminal's output stream (stdout or the SSH/web channel).
    host_out: Mutex<Box<dyn Write + Send>>,
    /// Per-window placement and id-remap store.
    placements: Mutex<PlacementStore>,
    /// Whether passthrough is enabled (host supports kitty graphics).
    enabled: bool,
}

impl KittyPassthrough {
    /// Create a new passthrough for the given host capabilities and output.
    pub fn new(caps: Capabilities, host_out: Box<dyn Write + Send>) -> Self {
        let enabled = caps.kitty;
        Self {
            caps,
            host_out: Mutex::new(host_out),
            placements: Mutex::new(PlacementStore::new()),
            enabled,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn capabilities(&self) -> Capabilities {
        self.caps
    }

    /// Forward a raw APC G sequence to the host, rewriting the image id and
    /// placement coordinates for the given window. `apc` is the full APC
    /// payload (between `\x1b_G` and the ST terminator), e.g.
    /// `a=T,f=100,s=200;i=1;<base64>`.
    pub fn forward(
        &self,
        window_id: u32,
        pane_x: u32,
        pane_y: u32,
        apc: &str,
    ) -> std::io::Result<()> {
        if !self.enabled {
            return Ok(());
        }
        let rewritten = self.rewrite_apc(window_id, pane_x, pane_y, apc);
        // Track the placement if this is a transmit-and-place or place command.
        self.record_placement_if_any(window_id, pane_x, pane_y, apc);
        let mut out = self.host_out.lock().unwrap();
        out.write_all(b"\x1b_G")?;
        out.write_all(rewritten.as_bytes())?;
        out.write_all(b"\x1b\\")?;
        out.flush()?;
        Ok(())
    }

    /// Record a placement in the store if the APC carries `i=` and is a
    /// transmit/place action (`a=T`, `a=t`, `a=p`, `a=u`).
    fn record_placement_if_any(&self, window_id: u32, pane_x: u32, pane_y: u32, apc: &str) {
        let (params, _) = match apc.find(';') {
            Some(idx) => (&apc[..idx], &apc[idx + 1..]),
            None => (apc, ""),
        };
        let mut guest_id = None;
        let mut is_place = false;
        for p in params.split(',') {
            if let Some(rest) = p.strip_prefix("i=") {
                guest_id = rest.parse::<u32>().ok();
            }
            if let Some(rest) = p.strip_prefix("a=") {
                is_place = matches!(rest, "T" | "t" | "p" | "u");
            }
        }
        if let Some(gid) = guest_id {
            let mut store = self.placements.lock().unwrap();
            let host_id = store.map_id(window_id, gid);
            if is_place {
                store.place(
                    window_id,
                    super::placement::Placement::new(host_id, gid, pane_x, pane_y),
                );
            }
        }
    }

    /// Rewrite an APC payload: remap `i=<id>`, offset `x=`/`y=` by the pane's
    /// absolute position. Returns the rewritten payload (without the
    /// `\x1b_G`/ST wrappers).
    fn rewrite_apc(&self, window_id: u32, pane_x: u32, pane_y: u32, apc: &str) -> String {
        // Split into params and payload on the first `;`.
        let (params, payload) = match apc.find(';') {
            Some(idx) => (&apc[..idx], &apc[idx + 1..]),
            None => (apc, ""),
        };

        // Parse and rewrite params.
        let mut parts: Vec<String> = params
            .split(',')
            .map(|p| self.rewrite_param(window_id, pane_x, pane_y, p))
            .collect();

        // Ensure there's an `i=` if we allocated a host id. (The first
        // transmit for a guest id may not carry `i=`; kitty assigns one and
        // returns it, but for passthrough we proactively assign.)
        // We don't force-add `i=` here because the guest's APC already has it
        // in the common case; the remap in `rewrite_param` handles it.

        let _ = &mut parts; // silence unused mut when no rewrites happen
        let new_params = parts.join(",");
        if payload.is_empty() {
            new_params
        } else {
            format!("{new_params};{payload}")
        }
    }

    fn rewrite_param(&self, window_id: u32, pane_x: u32, pane_y: u32, param: &str) -> String {
        if let Some(rest) = param.strip_prefix("i=") {
            if let Ok(guest_id) = rest.parse::<u32>() {
                let host_id = self.placements.lock().unwrap().map_id(window_id, guest_id);
                return format!("i={host_id}");
            }
        }
        if let Some(rest) = param.strip_prefix("x=") {
            if let Ok(x) = rest.parse::<u32>() {
                return format!("x={}", x + pane_x);
            }
        }
        if let Some(rest) = param.strip_prefix("y=") {
            if let Ok(y) = rest.parse::<u32>() {
                return format!("y={}", y + pane_y);
            }
        }
        param.to_string()
    }

    /// Clear all placements for a window (on close or workspace switch).
    pub fn clear_window(&self, window_id: u32) {
        self.placements.lock().unwrap().clear_window(window_id);
    }

    /// True if a window has any tracked placements.
    pub fn has_placements(&self, window_id: u32) -> bool {
        self.placements.lock().unwrap().has_placements(window_id)
    }

    /// Re-emit placement commands (`a=p`) for all of a window's images at
    /// their new absolute positions. Called after a pane move or resize.
    pub fn refresh_placements(
        &self,
        window_id: u32,
        pane_x: u32,
        pane_y: u32,
    ) -> std::io::Result<()> {
        if !self.enabled {
            return Ok(());
        }
        let store = self.placements.lock().unwrap();
        let placements = store.placements_for(window_id);
        if placements.is_empty() {
            return Ok(());
        }
        let mut out = self.host_out.lock().unwrap();
        for p in placements {
            // Delete the old placement, then re-place at the new position.
            // d=p deletes only the placement, not the image data.
            write!(out, "\x1b_Ga=d,d=p,i={}\x1b\\", p.host_image_id)?;
            // a=p places an already-transmitted image at the cursor position.
            // Move the cursor to the pane-relative position first.
            let abs_x = pane_x + p.x;
            let abs_y = pane_y + p.y;
            write!(out, "\x1b[{};{}H", abs_y + 1, abs_x + 1)?;
            write!(out, "\x1b_Ga=p,i={}\x1b\\", p.host_image_id)?;
        }
        out.flush()?;
        Ok(())
    }

    /// Clear everything (host reset).
    pub fn clear_all(&self) -> std::io::Result<()> {
        self.placements.lock().unwrap().clear_all();
        if !self.enabled {
            return Ok(());
        }
        // Kitty delete-all: `\x1b_Ga=d\x1b\\`
        let mut out = self.host_out.lock().unwrap();
        out.write_all(b"\x1b_Ga=d\x1b\\")?;
        out.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_caps(kitty: bool) -> Capabilities {
        Capabilities {
            kitty,
            sixel: false,
            host: if kitty {
                super::super::capability::HostTerminal::Kitty
            } else {
                super::super::capability::HostTerminal::Unknown
            },
            inside_multiplexer: false,
        }
    }

    #[test]
    fn response_detection() {
        assert!(is_kitty_response("OK"));
        assert!(is_kitty_response("ENOENT"));
        assert!(is_kitty_response("EINVAL:bad params"));
        assert!(!is_kitty_response(""));
        assert!(!is_kitty_response("EN"));
        assert!(!is_kitty_response("E"));
        assert!(is_kitty_response("ENOENT"));
        assert!(!is_kitty_response("Hello"));
        // A real base64 payload is too long and mixed-case.
        assert!(!is_kitty_response(&"A".repeat(300)));
    }

    #[test]
    fn disabled_passthrough_is_noop() {
        let kp = KittyPassthrough::new(test_caps(false), Box::new(std::io::sink()));
        assert!(!kp.is_enabled());
        // Forwarding is a no-op.
        kp.forward(1, 0, 0, "a=T,f=100;i=1;AAAA").unwrap();
    }

    #[test]
    fn forward_remaps_id_and_offsets() {
        let kp = KittyPassthrough::new(test_caps(true), Box::new(std::io::sink()));
        // First forward allocates host id 1 for guest id 1.
        let rewritten = kp.rewrite_apc(1, 10, 5, "a=T,f=100,i=1,x=0,y=0;AAAA");
        assert!(rewritten.contains("i=1"), "got: {rewritten}");
        assert!(rewritten.contains("x=10"), "got: {rewritten}");
        assert!(rewritten.contains("y=5"), "got: {rewritten}");
        // A second guest id gets host id 2.
        let rewritten2 = kp.rewrite_apc(1, 10, 5, "a=T,i=2;BBBB");
        assert!(rewritten2.contains("i=2"), "got: {rewritten2}");
        // Same guest id reuses the same host id.
        let rewritten3 = kp.rewrite_apc(1, 10, 5, "a=p,i=1;CCCC");
        assert!(rewritten3.contains("i=1"), "got: {rewritten3}");
    }

    #[test]
    fn forward_preserves_payload() {
        let kp = KittyPassthrough::new(test_caps(true), Box::new(std::io::sink()));
        let rewritten = kp.rewrite_apc(1, 0, 0, "a=T,f=100;AAAA==");
        assert!(rewritten.ends_with(";AAAA=="), "got: {rewritten}");
    }

    #[test]
    fn forward_without_payload() {
        let kp = KittyPassthrough::new(test_caps(true), Box::new(std::io::sink()));
        let rewritten = kp.rewrite_apc(1, 0, 0, "a=q,i=1");
        assert!(!rewritten.contains(';'), "got: {rewritten}");
    }

    #[test]
    fn clear_window_drops_placements() {
        let kp = KittyPassthrough::new(test_caps(true), Box::new(std::io::sink()));
        kp.forward(1, 0, 0, "a=T,i=1;AAAA").unwrap();
        assert!(kp.has_placements(1));
        kp.clear_window(1);
        assert!(!kp.has_placements(1));
    }

    #[test]
    fn refresh_placements_re_emits_at_new_position() {
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
        let kp = KittyPassthrough::new(test_caps(true), Box::new(SharedWriter(buf.clone())));
        // Place an image at (0,0).
        kp.forward(1, 0, 0, "a=T,i=1;AAAA").unwrap();
        // Refresh at (10, 5).
        kp.refresh_placements(1, 10, 5).unwrap();
        let buf_inner = buf.lock().unwrap();
        let s = String::from_utf8_lossy(&buf_inner);
        // Should contain a delete-placement and a re-place at the new position.
        assert!(s.contains("a=d,d=p,i=1"), "got: {s:?}");
        assert!(s.contains("a=p,i=1"), "got: {s:?}");
        // The CUP should target the new position (row 6, col 11 = 1-based).
        assert!(s.contains("\x1b[6;11H"), "got: {s:?}");
    }

    #[test]
    fn refresh_placements_noop_when_empty() {
        let kp = KittyPassthrough::new(test_caps(true), Box::new(std::io::sink()));
        // No placements — should be a no-op.
        kp.refresh_placements(1, 10, 5).unwrap();
        assert!(!kp.has_placements(1));
    }
}
