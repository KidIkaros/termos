//! Sixel graphics passthrough — forwards DCS `8;...;q` sequences to the host
//! terminal, offsetting the cursor position so the image lands inside the
//! pane. Ported from TUIOS `internal/app/sixel_passthrough.go`.
//!
//! Sixel is simpler than Kitty: there's no image-id remap, and the image is
//! placed at the cursor position. TermOS positions the cursor at the pane's
//! top-left (using CUP) before forwarding, then restores it.
//!
//! Unlike Kitty, sixel has no per-image delete command, so hiding a placement
//! requires overwriting the image area with spaces (erased characters).

use std::collections::HashMap;
use std::io::Write;
use std::sync::Mutex;

use super::capability::Capabilities;
use super::placement::WindowPositionInfo;

/// A sixel image placement in a guest window. Ported from Go's
/// `SixelPassthroughPlacement`.
#[derive(Debug, Clone)]
pub struct SixelPassthroughPlacement {
    /// The window this placement belongs to.
    pub window_id: u32,
    /// Absolute line in scrollback where the image starts.
    pub absolute_line: i32,
    /// Column position in the guest terminal at placement time.
    pub guest_x: i32,
    /// Row position in the guest terminal at placement time.
    pub guest_y: i32,

    // Image dimensions
    /// Pixel width.
    pub width: i32,
    /// Pixel height.
    pub height: i32,
    /// Number of terminal rows the image occupies.
    pub rows: i32,
    /// Number of terminal columns the image occupies.
    pub cols: i32,

    // Host terminal position (calculated during refresh)
    pub host_x: i32,
    pub host_y: i32,

    /// Whether the placement is currently hidden.
    pub hidden: bool,

    // Track if currently placed and at what position (to avoid re-rendering every frame)
    pub placed_at_x: i32,
    pub placed_at_y: i32,
    pub is_placed: bool,

    // Clipping state
    pub clip_top: i32,
    pub clip_bottom: i32,
    pub clip_left: i32,
    pub clip_right: i32,

    /// The raw sixel data for re-rendering.
    pub raw_sequence: Vec<u8>,

    /// True if placed while alternate screen was active.
    pub placed_on_alt_screen: bool,
}

impl SixelPassthroughPlacement {
    /// Create a new sixel placement record.
    pub fn new(window_id: u32, absolute_line: i32, guest_x: i32, guest_y: i32) -> Self {
        Self {
            window_id,
            absolute_line,
            guest_x,
            guest_y,
            width: 0,
            height: 0,
            rows: 0,
            cols: 0,
            host_x: 0,
            host_y: 0,
            hidden: true,
            placed_at_x: 0,
            placed_at_y: 0,
            is_placed: false,
            clip_top: 0,
            clip_bottom: 0,
            clip_left: 0,
            clip_right: 0,
            raw_sequence: Vec::new(),
            placed_on_alt_screen: false,
        }
    }
}

/// Parameters for a sixel forward command, grouping the many fields into a
/// single struct to keep the API clean.
#[derive(Debug, Clone)]
pub struct SixelCommandInfo {
    /// Column position in the guest terminal at placement time.
    pub cursor_x: i32,
    /// Row position in the guest terminal at placement time.
    pub cursor_y: i32,
    /// Absolute scrollback line where the image starts.
    pub abs_line: i32,
    /// Whether the alt screen was active at placement time.
    pub is_alt_screen: bool,
    /// Host cell pixel width (for row/col calculation).
    pub cell_width: i32,
    /// Host cell pixel height (for row/col calculation).
    pub cell_height: i32,
    /// Image pixel width.
    pub width: i32,
    /// Image pixel height.
    pub height: i32,
    /// The raw sixel data (DCS body after `q`).
    pub raw_sequence: Vec<u8>,
}

/// Sixel passthrough state.
pub struct SixelPassthrough {
    host_out: Mutex<Box<dyn Write + Send>>,
    enabled: bool,
    /// Per-window placements: window_id → Vec of placements.
    placements: Mutex<HashMap<u32, Vec<SixelPassthroughPlacement>>>,
    /// Pending sixel output to be written.
    pending_output: Mutex<Vec<u8>>,
}

impl SixelPassthrough {
    pub fn new(_caps: Capabilities, host_out: Box<dyn Write + Send>) -> Self {
        let enabled = _caps.sixel;
        Self {
            host_out: Mutex::new(host_out),
            enabled,
            placements: Mutex::new(HashMap::new()),
            pending_output: Mutex::new(Vec::new()),
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

    /// Store a sixel placement for later rendering during refresh. Ported
    /// from Go's `ForwardCommand`.
    pub fn forward_command(&self, window_id: u32, cmd: &SixelCommandInfo) {
        if !self.enabled {
            return;
        }

        // Calculate rows and columns from pixel dimensions.
        let rows = if cmd.cell_height > 0 {
            (cmd.height + cmd.cell_height - 1) / cmd.cell_height
        } else {
            1
        };
        let cols = if cmd.cell_width > 0 {
            (cmd.width + cmd.cell_width - 1) / cmd.cell_width
        } else {
            1
        };

        let mut placements = self.placements.lock().unwrap();
        let window_placements = placements.entry(window_id).or_default();

        // Check for existing placement at the same position with same dimensions
        // (shell redraws can re-emit the same sixel).
        for existing in window_placements.iter_mut() {
            if existing.absolute_line == cmd.abs_line
                && existing.guest_x == cmd.cursor_x
                && existing.width == cmd.width
                && existing.height == cmd.height
            {
                existing.raw_sequence = cmd.raw_sequence.clone();
                existing.is_placed = false; // Force re-render
                return;
            }
        }

        let mut placement =
            SixelPassthroughPlacement::new(window_id, cmd.abs_line, cmd.cursor_x, cmd.cursor_y);
        placement.width = cmd.width;
        placement.height = cmd.height;
        placement.rows = rows;
        placement.cols = cols;
        placement.raw_sequence = cmd.raw_sequence.clone();
        placement.placed_on_alt_screen = cmd.is_alt_screen;
        window_placements.push(placement);
    }

    /// Clear all placements for a window.
    pub fn clear_window(&self, window_id: u32) {
        self.placements.lock().unwrap().remove(&window_id);
    }

    /// Remove placements that were made on the alt screen. Called when
    /// transitioning from alt screen to normal screen. Ported from Go's
    /// `ClearAltScreenPlacements`.
    pub fn clear_alt_screen_placements(&self, window_id: u32) {
        let mut placements = self.placements.lock().unwrap();
        if let Some(window_placements) = placements.get_mut(&window_id) {
            window_placements.retain(|p| !p.placed_on_alt_screen);
        }
    }

    /// Remove placements that have scrolled past a certain line. Ported from
    /// Go's `ClearScrolledOut`. Used for memory management.
    pub fn clear_scrolled_out(&self, window_id: u32, min_line: i32) {
        let mut placements = self.placements.lock().unwrap();
        if let Some(window_placements) = placements.get_mut(&window_id) {
            window_placements.retain(|p| p.absolute_line + p.rows > min_line);
        }
    }

    /// The total number of placements across all windows.
    pub fn placement_count(&self) -> usize {
        self.placements
            .lock()
            .unwrap()
            .values()
            .map(|v| v.len())
            .sum()
    }

    /// True if any window has placements.
    pub fn has_placements(&self) -> bool {
        self.placements
            .lock()
            .unwrap()
            .values()
            .any(|v| !v.is_empty())
    }

    /// Hide a sixel placement by overwriting the image area with spaces.
    /// Unlike Kitty graphics, sixel has no delete command, so we must
    /// actively clear the area. Ported from Go's `hidePlacement`.
    fn hide_placement(p: &mut SixelPassthroughPlacement, output: &mut Vec<u8>) {
        if p.is_placed && p.rows > 0 {
            output.extend_from_slice(b"\x1b7"); // Save cursor
            for row in 0..p.rows {
                let _ = write!(output, "\x1b[{};{}H", p.placed_at_y + row + 1, p.placed_at_x + 1);
                let _ = write!(output, "\x1b[{}X", p.cols); // Erase N characters
            }
            output.extend_from_slice(b"\x1b8"); // Restore cursor
        }
        p.hidden = true;
        p.is_placed = false;
    }

    /// Write a sixel image to the host terminal at the placement's position.
    /// The raw sixel data is passed through without re-encoding. Ported from
    /// Go's `placeSixel`.
    fn place_sixel(p: &SixelPassthroughPlacement, output: &mut Vec<u8>) {
        if p.raw_sequence.is_empty() {
            return;
        }
        output.extend_from_slice(b"\x1b7"); // Save cursor
        let _ = write!(output, "\x1b[{};{}H", p.host_y + 1, p.host_x + 1);
        output.extend_from_slice(b"\x1bP"); // DCS
        output.extend_from_slice(&p.raw_sequence);
        output.extend_from_slice(b"\x1b\\"); // ST
        output.extend_from_slice(b"\x1b8"); // Restore cursor
    }

    /// Refresh all sixel placements: update visibility and positions for all
    /// placements. Called during each render cycle. Ported from Go's
    /// `RefreshAllPlacements`.
    ///
    /// Sixel images can't be pixel-cropped without palette re-quantization,
    /// so images that extend past window boundaries are hidden rather than
    /// clipped.
    pub fn refresh_placements(
        &self,
        get_window_info: &dyn Fn(u32) -> Option<WindowPositionInfo>,
        _cell_height: i32,
        host_height: i32,
    ) {
        if !self.enabled {
            return;
        }

        let mut placements = self.placements.lock().unwrap();
        let mut output = Vec::new();
        let window_ids: Vec<u32> = placements.keys().copied().collect();

        for window_id in window_ids {
            let Some(info) = get_window_info(window_id) else {
                if let Some(window_placements) = placements.get_mut(&window_id) {
                    for p in window_placements.iter_mut() {
                        if !p.hidden {
                            Self::hide_placement(p, &mut output);
                        }
                    }
                }
                continue;
            };

            if !info.visible {
                if let Some(window_placements) = placements.get_mut(&window_id) {
                    for p in window_placements.iter_mut() {
                        if !p.hidden {
                            Self::hide_placement(p, &mut output);
                        }
                    }
                }
                continue;
            }

            // During window manipulation (drag/resize), hide this window's images.
            if info.is_being_manipulated {
                if let Some(window_placements) = placements.get_mut(&window_id) {
                    for p in window_placements.iter_mut() {
                        if !p.hidden {
                            Self::hide_placement(p, &mut output);
                        }
                    }
                }
                continue;
            }

            // Calculate viewport boundaries using content height (exclude borders).
            let content_height = if info.height - 2 * info.content_offset_y > 0 {
                info.height - 2 * info.content_offset_y
            } else {
                info.height
            };
            let viewport_top = info.scrollback_len - info.scroll_offset;
            let viewport_bottom = viewport_top + content_height;

            if let Some(window_placements) = placements.get_mut(&window_id) {
                for p in window_placements.iter_mut() {
                    // Check if placement matches current screen mode.
                    if p.placed_on_alt_screen != info.is_alt_screen {
                        if !p.hidden {
                            Self::hide_placement(p, &mut output);
                        }
                        continue;
                    }

                    // Calculate visibility.
                    let placement_bottom = p.absolute_line + p.rows;
                    let mut any_part_visible =
                        placement_bottom > viewport_top && p.absolute_line < viewport_bottom;

                    // When not scrolled back, also consider images that extend
                    // beyond current scrollback.
                    if !any_part_visible
                        && info.scroll_offset == 0
                        && p.absolute_line >= viewport_top
                    {
                        any_part_visible = true;
                    }

                    if !any_part_visible {
                        if !p.hidden {
                            Self::hide_placement(p, &mut output);
                        }
                        continue;
                    }

                    // Calculate host position.
                    let relative_y = (p.absolute_line - viewport_top).max(0);
                    let host_x = info.window_x + info.content_offset_x + p.guest_x;
                    let host_y = info.window_y + info.content_offset_y + relative_y;

                    // Window content area bounds (in host coordinates).
                    let window_content_bottom = info.window_y + info.height - info.content_offset_y;

                    // Hide if image extends past window content bottom.
                    if host_y + p.rows > window_content_bottom {
                        if !p.hidden {
                            Self::hide_placement(p, &mut output);
                        }
                        continue;
                    }

                    // Hide if image extends past screen bottom (causes scroll feedback).
                    if host_height > 0 && host_y + p.rows >= host_height - 1 {
                        if !p.hidden {
                            Self::hide_placement(p, &mut output);
                        }
                        continue;
                    }

                    // Hide if top is clipped (scrolled partially out of view).
                    if p.absolute_line < viewport_top {
                        if !p.hidden {
                            Self::hide_placement(p, &mut output);
                        }
                        continue;
                    }

                    // Check if position changed — only re-render if needed.
                    let position_changed =
                        !p.is_placed || p.placed_at_x != host_x || p.placed_at_y != host_y;

                    p.host_x = host_x;
                    p.host_y = host_y;

                    if position_changed {
                        Self::place_sixel(p, &mut output);
                        p.placed_at_x = host_x;
                        p.placed_at_y = host_y;
                        p.is_placed = true;
                    }
                    p.hidden = false;
                }
            }
        }

        // Write accumulated output to the host.
        if !output.is_empty() {
            let mut out = self.host_out.lock().unwrap();
            let _ = out.write_all(&output);
            let _ = out.flush();
        }

        // Clean up empty windows.
        placements.retain(|_, v| !v.is_empty());
    }

    /// Hide all sixel placements and queue clear commands. Used during resize
    /// to prevent stale positions.
    pub fn hide_all_placements(&self) {
        let mut placements = self.placements.lock().unwrap();
        let mut output = Vec::new();
        for window_placements in placements.values_mut() {
            for p in window_placements.iter_mut() {
                if !p.hidden {
                    Self::hide_placement(p, &mut output);
                }
            }
        }
        if !output.is_empty() {
            let mut out = self.host_out.lock().unwrap();
            let _ = out.write_all(&output);
            let _ = out.flush();
        }
    }

    /// Flush any pending output to the host terminal.
    pub fn flush_output(&self) -> std::io::Result<()> {
        let mut pending = self.pending_output.lock().unwrap();
        if !pending.is_empty() {
            let mut out = self.host_out.lock().unwrap();
            out.write_all(&pending)?;
            out.flush()?;
            pending.clear();
        }
        Ok(())
    }

    /// Clear all Sixel images on the host (a full-screen erase is the only
    /// portable way; Sixel has no per-image delete).
    pub fn clear_all(&self) -> std::io::Result<()> {
        if !self.enabled {
            return Ok(());
        }
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

    fn shared_writer() -> (
        std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
        Box<dyn Write + Send>,
    ) {
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
        let buf_clone = buf.clone();
        (buf, Box::new(SharedWriter(buf_clone)))
    }

    fn make_window_info(x: i32, y: i32, w: i32, h: i32) -> WindowPositionInfo {
        WindowPositionInfo {
            window_x: x,
            window_y: y,
            width: w,
            height: h,
            visible: true,
            screen_width: 200,
            screen_height: 100,
            window_z: 1,
            ..Default::default()
        }
    }

    fn make_sixel_cmd(
        cursor_x: i32,
        cursor_y: i32,
        abs_line: i32,
        is_alt: bool,
        width: i32,
        height: i32,
        raw: &[u8],
    ) -> SixelCommandInfo {
        SixelCommandInfo {
            cursor_x,
            cursor_y,
            abs_line,
            is_alt_screen: is_alt,
            cell_width: 9,
            cell_height: 20,
            width,
            height,
            raw_sequence: raw.to_vec(),
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
        let (buf, writer) = shared_writer();
        let sp = SixelPassthrough::new(test_caps(true), writer);
        sp.forward(10, 5, b"~!1~").unwrap();
        let buf_inner = buf.lock().unwrap();
        let s = String::from_utf8_lossy(&buf_inner);
        assert!(s.starts_with("\x1b[s"), "got: {s:?}");
        assert!(s.contains("\x1b[6;11H"), "got: {s:?}");
        assert!(s.contains("\x1bP8;q~!1~\x1b\\"), "got: {s:?}");
        assert!(s.ends_with("\x1b[u"), "got: {s:?}");
    }

    #[test]
    fn forward_command_stores_placement() {
        let sp = SixelPassthrough::new(test_caps(true), Box::new(std::io::sink()));
        sp.forward_command(1, &make_sixel_cmd(0, 0, 5, false, 90, 100, b"~!1~"));
        assert_eq!(sp.placement_count(), 1);
        assert!(sp.has_placements());
    }

    #[test]
    fn forward_command_updates_existing() {
        let sp = SixelPassthrough::new(test_caps(true), Box::new(std::io::sink()));
        sp.forward_command(1, &make_sixel_cmd(0, 0, 5, false, 90, 100, b"~!1~"));
        sp.forward_command(1, &make_sixel_cmd(0, 0, 5, false, 90, 100, b"~!2~"));
        assert_eq!(sp.placement_count(), 1);
    }

    #[test]
    fn forward_command_adds_new_for_different_position() {
        let sp = SixelPassthrough::new(test_caps(true), Box::new(std::io::sink()));
        sp.forward_command(1, &make_sixel_cmd(0, 0, 5, false, 90, 100, b"~!1~"));
        sp.forward_command(1, &make_sixel_cmd(10, 5, 15, false, 90, 100, b"~!2~"));
        assert_eq!(sp.placement_count(), 2);
    }

    #[test]
    fn clear_window_removes_placements() {
        let sp = SixelPassthrough::new(test_caps(true), Box::new(std::io::sink()));
        sp.forward_command(1, &make_sixel_cmd(0, 0, 5, false, 90, 100, b"~!1~"));
        sp.forward_command(2, &make_sixel_cmd(0, 0, 5, false, 90, 100, b"~!2~"));
        sp.clear_window(1);
        assert_eq!(sp.placement_count(), 1);
    }

    #[test]
    fn clear_alt_screen_placements_removes_alt() {
        let sp = SixelPassthrough::new(test_caps(true), Box::new(std::io::sink()));
        sp.forward_command(1, &make_sixel_cmd(0, 0, 5, true, 90, 100, b"~!1~"));
        sp.forward_command(1, &make_sixel_cmd(0, 0, 10, false, 90, 100, b"~!2~"));
        assert_eq!(sp.placement_count(), 2);
        sp.clear_alt_screen_placements(1);
        assert_eq!(sp.placement_count(), 1);
    }

    #[test]
    fn clear_scrolled_out_removes_old() {
        let sp = SixelPassthrough::new(test_caps(true), Box::new(std::io::sink()));
        sp.forward_command(1, &make_sixel_cmd(0, 0, 5, false, 90, 100, b"~!1~"));
        sp.forward_command(1, &make_sixel_cmd(0, 0, 50, false, 90, 100, b"~!2~"));
        assert_eq!(sp.placement_count(), 2);
        sp.clear_scrolled_out(1, 20);
        assert_eq!(sp.placement_count(), 1);
    }

    #[test]
    fn refresh_placements_places_visible() {
        let (buf, writer) = shared_writer();
        let sp = SixelPassthrough::new(test_caps(true), writer);
        sp.forward_command(1, &make_sixel_cmd(0, 0, 0, false, 90, 40, b"~!1~"));

        let windows = HashMap::from([(1, make_window_info(5, 5, 80, 24))]);
        sp.refresh_placements(&|wid| windows.get(&wid).cloned(), 20, 100);

        let buf_inner = buf.lock().unwrap();
        let s = String::from_utf8_lossy(&buf_inner);
        assert!(s.contains("\x1bP"), "should emit sixel DCS: {s:?}");
        assert!(s.contains("\x1b7"), "should save cursor: {s:?}");
    }

    #[test]
    fn refresh_placements_hides_invisible_window() {
        let (buf, writer) = shared_writer();
        let sp = SixelPassthrough::new(test_caps(true), writer);
        sp.forward_command(1, &make_sixel_cmd(0, 0, 0, false, 90, 40, b"~!1~"));

        let windows = HashMap::from([(1, make_window_info(5, 5, 80, 24))]);
        sp.refresh_placements(&|wid| windows.get(&wid).cloned(), 20, 100);

        buf.lock().unwrap().clear();
        let mut info = make_window_info(5, 5, 80, 24);
        info.visible = false;
        let windows2 = HashMap::from([(1, info)]);
        sp.refresh_placements(&|wid| windows2.get(&wid).cloned(), 20, 100);

        let buf_inner = buf.lock().unwrap();
        let s = String::from_utf8_lossy(&buf_inner);
        assert!(s.contains("\x1b["), "should emit erase: {s:?}");
    }

    #[test]
    fn refresh_placements_skips_alt_screen_mismatch() {
        let (buf, writer) = shared_writer();
        let sp = SixelPassthrough::new(test_caps(true), writer);
        sp.forward_command(1, &make_sixel_cmd(0, 0, 0, false, 90, 40, b"~!1~"));

        let mut info = make_window_info(5, 5, 80, 24);
        info.is_alt_screen = true;
        let windows = HashMap::from([(1, info)]);
        sp.refresh_placements(&|wid| windows.get(&wid).cloned(), 20, 100);

        let buf_inner = buf.lock().unwrap();
        let s = String::from_utf8_lossy(&buf_inner);
        assert!(
            !s.contains("\x1bP~"),
            "should not place on alt screen mismatch: {s:?}"
        );
    }

    #[test]
    fn refresh_placements_no_replace_when_unchanged() {
        let (buf, writer) = shared_writer();
        let sp = SixelPassthrough::new(test_caps(true), writer);
        sp.forward_command(1, &make_sixel_cmd(0, 0, 0, false, 90, 40, b"~!1~"));

        let windows = HashMap::from([(1, make_window_info(5, 5, 80, 24))]);
        sp.refresh_placements(&|wid| windows.get(&wid).cloned(), 20, 100);
        buf.lock().unwrap().clear();
        sp.refresh_placements(&|wid| windows.get(&wid).cloned(), 20, 100);
        let buf_inner = buf.lock().unwrap();
        let s = String::from_utf8_lossy(&buf_inner);
        assert!(
            !s.contains("\x1bP~"),
            "should not re-place when unchanged: {s:?}"
        );
    }

    #[test]
    fn hide_all_placements_clears_visible() {
        let (buf, writer) = shared_writer();
        let sp = SixelPassthrough::new(test_caps(true), writer);
        sp.forward_command(1, &make_sixel_cmd(0, 0, 0, false, 90, 40, b"~!1~"));

        let windows = HashMap::from([(1, make_window_info(5, 5, 80, 24))]);
        sp.refresh_placements(&|wid| windows.get(&wid).cloned(), 20, 100);

        buf.lock().unwrap().clear();
        sp.hide_all_placements();
        let buf_inner = buf.lock().unwrap();
        let s = String::from_utf8_lossy(&buf_inner);
        assert!(s.contains("\x1b7"), "should emit save cursor: {s:?}");
        assert!(s.contains('X'), "should emit erase char: {s:?}");
    }
}
