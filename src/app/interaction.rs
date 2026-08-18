//! Interaction features — ported from Go TUIOS `internal/app/` interaction files.
//!
//! Contains:
//! - Pointer shape (OSC 22) for cursor changes over borders/corners
//! - Resize deferral for efficient resizing during gestures
//! - Hold mode for momentary window-management mode
//! - Click-to-open for file path detection
//! - Host notifications (OSC 9) for desktop alerts
//! - Notification jump for click-to-jump
//! - Keyboard enhancements (Kitty protocol)
//! - Mac option advice for macOS Option key
//! - Separator gap for configurable pane gaps
//! - Tick stats for render performance
//! - Crash logging
//! - Post-render writer for graphics passthrough

use std::collections::HashMap;
use std::io::{self, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

// ─── Pointer Shape (OSC 22) ──────────────────────────────────────────────

/// CSS cursor shape names for OSC 22.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PointerShape {
    #[default]
    Default,
    Grab,
    Grabbing,
    EwResize,
    NsResize,
    NwseResize,
    NeswResize,
}

impl PointerShape {
    /// The CSS cursor name used in the OSC 22 sequence.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Grab => "grab",
            Self::Grabbing => "grabbing",
            Self::EwResize => "ew-resize",
            Self::NsResize => "ns-resize",
            Self::NwseResize => "nwse-resize",
            Self::NeswResize => "nesw-resize",
        }
    }
}

/// Write an OSC 22 sequence to change the mouse pointer shape.
/// Tracks the last shape to avoid redundant writes.
pub fn set_pointer_shape<W: Write>(
    w: &mut W,
    shape: PointerShape,
    current: &mut PointerShape,
) -> io::Result<()> {
    if shape == *current {
        return Ok(());
    }
    *current = shape;
    write!(w, "\x1b]22;{}\x1b\\", shape.as_str())?;
    w.flush()
}

/// Reset the pointer to default.
pub fn reset_pointer_shape<W: Write>(w: &mut W, current: &mut PointerShape) -> io::Result<()> {
    set_pointer_shape(w, PointerShape::Default, current)
}

/// Determine the pointer shape for a position over a window border.
/// Returns `None` if the position is not on a border/corner.
pub fn pointer_shape_for_border(
    x: i32,
    y: i32,
    win_x: i32,
    win_y: i32,
    win_w: i32,
    win_h: i32,
    border_off: i32,
) -> PointerShape {
    if border_off == 0 {
        return PointerShape::Default;
    }
    let on_left = x == win_x;
    let on_right = x == win_x + win_w - 1;
    let on_top = y == win_y;
    let on_bottom = y == win_y + win_h - 1;

    // Corners → diagonal resize
    if (on_left && on_top) || (on_right && on_bottom) {
        return PointerShape::NwseResize;
    }
    if (on_right && on_top) || (on_left && on_bottom) {
        return PointerShape::NeswResize;
    }
    // Vertical edges → horizontal resize
    if on_left || on_right {
        return PointerShape::EwResize;
    }
    // Top border → grab (title bar)
    if on_top {
        return PointerShape::Grab;
    }
    // Bottom border → vertical resize
    if on_bottom {
        return PointerShape::NsResize;
    }
    PointerShape::Default
}

// ─── Resize Deferral ─────────────────────────────────────────────────────

/// How long after the last resize event the deferral still counts as live.
pub const RESIZE_DEFERRAL_TIMEOUT: Duration = Duration::from_millis(500);

/// Tracks deferred resize state to avoid expensive PTY resizes during gestures.
#[derive(Debug, Default)]
pub struct ResizeDeferral {
    pub resizing: bool,
    pub last_pointer_at: Option<Instant>,
    pub viewport_resizing: bool,
    pub viewport_resize_at: Option<Instant>,
    pub viewport_resize_gen: u64,
    pending: HashMap<String, (i32, i32)>,
}

impl ResizeDeferral {
    /// Create a new tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a retile happening now should defer the expensive half.
    /// Has a side effect: finding the deferral stale ends it and drains.
    pub fn active(&mut self) -> bool {
        if !self.resizing && !self.viewport_resizing {
            return false;
        }
        let now = Instant::now();
        let mut fresh = false;
        if self.resizing {
            if let Some(t) = self.last_pointer_at {
                if now.duration_since(t) <= RESIZE_DEFERRAL_TIMEOUT {
                    fresh = true;
                }
            }
        }
        if !fresh && self.viewport_resizing {
            if let Some(t) = self.viewport_resize_at {
                if now.duration_since(t) <= RESIZE_DEFERRAL_TIMEOUT {
                    fresh = true;
                }
            }
        }
        if fresh {
            return true;
        }
        self.end();
        false
    }

    /// End the deferral and return what was pending.
    pub fn end(&mut self) -> Vec<(String, i32, i32)> {
        self.resizing = false;
        self.viewport_resizing = false;
        self.pending
            .drain()
            .map(|(id, (w, h))| (id, w, h))
            .collect()
    }

    /// Record a pending resize for a window.
    pub fn pending(&mut self, window_id: &str, width: i32, height: i32) {
        self.pending.insert(window_id.to_string(), (width, height));
    }

    /// Mark a pointer-driven resize as active.
    pub fn start_pointer_resize(&mut self) {
        self.resizing = true;
        self.last_pointer_at = Some(Instant::now());
    }

    /// Mark a viewport resize as active.
    pub fn start_viewport_resize(&mut self) {
        self.viewport_resizing = true;
        self.viewport_resize_at = Some(Instant::now());
        self.viewport_resize_gen += 1;
    }

    /// Refresh the pointer timestamp.
    pub fn touch_pointer(&mut self) {
        self.last_pointer_at = Some(Instant::now());
    }

    /// Refresh the viewport timestamp.
    pub fn touch_viewport(&mut self) {
        self.viewport_resize_at = Some(Instant::now());
    }

    /// Pending viewport resize generation and whether there is one.
    pub fn pending_viewport(&self) -> (u64, bool) {
        (self.viewport_resize_gen, self.viewport_resizing)
    }
}

// ─── Hold Mode ───────────────────────────────────────────────────────────

/// Hold mode: a momentary window-management mode activated by holding a key.
/// While held, keys are routed to window management instead of the terminal.
#[derive(Debug, Default)]
pub struct HoldMode {
    active: bool,
    started_at: Option<Instant>,
}

impl HoldMode {
    /// Create a new hold mode tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Activate hold mode.
    pub fn start(&mut self) {
        self.active = true;
        self.started_at = Some(Instant::now());
    }

    /// Deactivate hold mode.
    pub fn end(&mut self) {
        self.active = false;
        self.started_at = None;
    }

    /// Whether hold mode is currently active.
    pub fn active(&self) -> bool {
        self.active
    }

    /// How long hold mode has been active.
    pub fn duration(&self) -> Option<Duration> {
        self.started_at.map(|t| t.elapsed())
    }
}

// ─── Click-to-Open ───────────────────────────────────────────────────────

/// Detect file paths in terminal output that can be clicked to open.
pub fn detect_file_paths(text: &str) -> Vec<(usize, String)> {
    let mut results = Vec::new();
    let mut chars = text.char_indices().peekable();

    while let Some((start, c)) = chars.next() {
        if c == '/' || (c == '.' && chars.peek().is_some_and(|(_, c)| *c == '/')) {
            // Collect a path-like sequence
            let mut path = String::new();
            path.push(c);
            while let Some(&(_, ch)) = chars.peek() {
                if ch.is_alphanumeric()
                    || ch == '/'
                    || ch == '.'
                    || ch == '-'
                    || ch == '_'
                    || ch == '+'
                    || ch == ':'
                {
                    path.push(ch);

                    chars.next();
                } else {
                    break;
                }
            }
            // Must look like a real path: at least one more / or a file extension
            if path.len() > 2
                && (path.matches('/').count() >= 2
                    || (path.contains('.') && path.matches('/').count() >= 1))
            {
                results.push((start, path));
            }
        }
    }
    results
}

// ─── Host Notifications (OSC 9) ──────────────────────────────────────────

/// Desktop notification via OSC 9.
pub fn osc9_notify<W: Write>(w: &mut W, message: &str) -> io::Result<()> {
    write!(w, "\x1b]9;{}\x1b\\", message)?;
    w.flush()
}

/// OSC 777 notification (urxnterm).
pub fn osc777_notify<W: Write>(w: &mut W, title: &str, message: &str) -> io::Result<()> {
    write!(w, "\x1b]777;notify;{};{}\x1b\\", title, message)?;
    w.flush()
}

/// OSC 1337 notification (iTerm2).
pub fn osc1337_notify<W: Write>(w: &mut W, title: &str, message: &str) -> io::Result<()> {
    let payload = format!("{}|{}", title, message);
    let b64 = base64_encode(payload.as_bytes());
    write!(w, "\x1b]1337;Notification={}\x1b\\", b64)?;
    w.flush()
}

/// Simple base64 encoder (no padding) for notification payloads.
fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        result.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        }
        if chunk.len() > 2 {
            result.push(CHARS[(triple & 0x3F) as usize] as char);
        }
    }
    result
}

// ─── Notification Jump ───────────────────────────────────────────────────

/// A notification with its source window, for click-to-jump.
#[derive(Debug, Clone)]
pub struct NotificationEntry {
    pub window_id: String,
    pub title: String,
    pub message: String,
    pub timestamp: Instant,
}

/// A tracker for notifications that can be jumped to.
#[derive(Debug, Default)]
pub struct NotificationJump {
    entries: Vec<NotificationEntry>,
}

impl NotificationJump {
    /// Create a new tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a notification.
    pub fn add(&mut self, entry: NotificationEntry) {
        self.entries.push(entry);
        // Keep last 100
        if self.entries.len() > 100 {
            self.entries.drain(0..self.entries.len() - 100);
        }
    }

    /// Get all notifications.
    pub fn entries(&self) -> &[NotificationEntry] {
        &self.entries
    }

    /// Clear notifications for a window.
    pub fn clear_for(&mut self, window_id: &str) {
        self.entries.retain(|e| e.window_id != window_id);
    }

    /// Clear all.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Most recent notification for a window.
    pub fn latest_for(&self, window_id: &str) -> Option<&NotificationEntry> {
        self.entries.iter().rev().find(|e| e.window_id == window_id)
    }
}

// ─── Keyboard Enhancements ───────────────────────────────────────────────

/// Kitty keyboard protocol enhancement flags.
#[derive(Debug, Clone, Copy, Default)]
pub struct KeyboardEnhancements {
    pub disambiguate_escape: bool,
    pub report_event_types: bool,
    pub report_alternate_keys: bool,
    pub report_all_keys_as_escapes: bool,
    pub report_associated_text: bool,
}

impl KeyboardEnhancements {
    /// Build the CSI > u sequence to set the flags.
    pub fn set_flags_sequence(&self) -> String {
        let mut flags = 0u32;
        if self.disambiguate_escape {
            flags |= 1;
        }
        if self.report_event_types {
            flags |= 2;
        }
        if self.report_alternate_keys {
            flags |= 4;
        }
        if self.report_all_keys_as_escapes {
            flags |= 8;
        }
        if self.report_associated_text {
            flags |= 16;
        }
        format!("\x1b[>{flags}u")
    }

    /// The sequence to pop all flags (restore base mode).
    pub fn pop_flags_sequence() -> &'static str {
        "\x1b[<u"
    }
}

// ─── Mac Option Advice ───────────────────────────────────────────────────

/// Detect whether the host terminal is on macOS and the Option key is
/// configured as a meta key (vs. composing accented characters).
pub fn mac_option_advice(term_program: &str) -> Option<&'static str> {
    if term_program == "Apple_Terminal" {
        Some("In Terminal.app, set \"Use Option as Meta key\" in Preferences → Profiles → Keyboard for alt keybindings to work.")
    } else if term_program == "iTerm.app" {
        Some("In iTerm2, set \"Option key sends\" to \"Esc+\" in Preferences → Profiles → Keys for alt keybindings to work.")
    } else {
        None
    }
}

// ─── Separator Gap ───────────────────────────────────────────────────────

/// Configurable gap between panes, in cells.
#[derive(Debug, Clone, Copy, Default)]
pub struct SeparatorGap {
    pub horizontal: i32,
    pub vertical: i32,
}

impl SeparatorGap {
    /// Create with the given horizontal and vertical gaps.
    pub fn new(h: i32, v: i32) -> Self {
        Self {
            horizontal: h.max(0),
            vertical: v.max(0),
        }
    }

    /// Total horizontal gap between two side-by-side panes.
    pub fn h_gap(&self) -> i32 {
        self.horizontal
    }

    /// Total vertical gap between two stacked panes.
    pub fn v_gap(&self) -> i32 {
        self.vertical
    }
}

// ─── Tick Stats ──────────────────────────────────────────────────────────

/// Render performance statistics.
#[derive(Debug, Default)]
pub struct TickStats {
    frame_count: AtomicU64,
    render_time_ns: AtomicU64,
    last_frame_at: Option<Instant>,
}

impl TickStats {
    /// Create a new stats tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a frame render.
    pub fn record_frame(&mut self, render_time: Duration) {
        self.frame_count.fetch_add(1, Ordering::Relaxed);
        self.render_time_ns
            .fetch_add(render_time.as_nanos() as u64, Ordering::Relaxed);
        self.last_frame_at = Some(Instant::now());
    }

    /// Total frames rendered.
    pub fn frame_count(&self) -> u64 {
        self.frame_count.load(Ordering::Relaxed)
    }

    /// Average render time per frame.
    pub fn avg_render_time(&self) -> Duration {
        let frames = self.frame_count();
        if frames == 0 {
            return Duration::ZERO;
        }
        Duration::from_nanos(self.render_time_ns.load(Ordering::Relaxed) / frames)
    }

    /// Time since the last frame.
    pub fn since_last_frame(&self) -> Option<Duration> {
        self.last_frame_at.map(|t| t.elapsed())
    }
}

// ─── Crash Logging ───────────────────────────────────────────────────────

/// Write a crash report to the given path.
pub fn write_crash_report(path: &std::path::Path, info: &str) -> io::Result<()> {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let report = format!(
        "TermOS crash report\nTimestamp: {timestamp}\nVersion: {}\n\n{info}\n",
        env!("CARGO_PKG_VERSION")
    );
    std::fs::write(path, report)
}

// ─── Post-Render Writer ──────────────────────────────────────────────────

/// A buffer for post-render output (graphics passthrough, OSC sequences
/// that must go after the frame).
#[derive(Debug, Default)]
pub struct PostRenderWriter {
    buffer: Vec<u8>,
}

impl PostRenderWriter {
    /// Create a new post-render buffer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue data to write after the next render.
    pub fn queue(&mut self, data: &[u8]) {
        self.buffer.extend_from_slice(data);
    }

    /// Queue a string to write after the next render.
    pub fn queue_str(&mut self, s: &str) {
        self.queue(s.as_bytes());
    }

    /// Drain and return the buffered data.
    pub fn drain(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.buffer)
    }

    /// Whether there is pending data.
    pub fn pending(&self) -> bool {
        !self.buffer.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_shape_as_str() {
        assert_eq!(PointerShape::Default.as_str(), "default");
        assert_eq!(PointerShape::EwResize.as_str(), "ew-resize");
        assert_eq!(PointerShape::NwseResize.as_str(), "nwse-resize");
    }

    #[test]
    fn pointer_shape_for_border_corners() {
        assert_eq!(
            pointer_shape_for_border(0, 0, 0, 0, 80, 24, 1),
            PointerShape::NwseResize
        );
        assert_eq!(
            pointer_shape_for_border(79, 23, 0, 0, 80, 24, 1),
            PointerShape::NwseResize
        );
        assert_eq!(
            pointer_shape_for_border(79, 0, 0, 0, 80, 24, 1),
            PointerShape::NeswResize
        );
    }

    #[test]
    fn pointer_shape_for_border_edges() {
        assert_eq!(
            pointer_shape_for_border(0, 5, 0, 0, 80, 24, 1),
            PointerShape::EwResize
        );
        assert_eq!(
            pointer_shape_for_border(0, 0, 0, 0, 80, 24, 1),
            PointerShape::NwseResize // corner takes priority
        );
        assert_eq!(
            pointer_shape_for_border(5, 0, 0, 0, 80, 24, 1),
            PointerShape::Grab
        );
        assert_eq!(
            pointer_shape_for_border(5, 23, 0, 0, 80, 24, 1),
            PointerShape::NsResize
        );
    }

    #[test]
    fn pointer_shape_no_border() {
        assert_eq!(
            pointer_shape_for_border(0, 0, 0, 0, 80, 24, 0),
            PointerShape::Default
        );
    }

    #[test]
    fn set_pointer_shape_writes_osc22() {
        let mut buf = Vec::new();
        let mut current = PointerShape::Default;
        set_pointer_shape(&mut buf, PointerShape::Grab, &mut current).unwrap();
        assert_eq!(String::from_utf8(buf).unwrap(), "\x1b]22;grab\x1b\\");
        assert_eq!(current, PointerShape::Grab);
    }

    #[test]
    fn set_pointer_shape_skips_redundant() {
        let mut buf = Vec::new();
        let mut current = PointerShape::Grab;
        set_pointer_shape(&mut buf, PointerShape::Grab, &mut current).unwrap();
        assert!(buf.is_empty());
    }

    #[test]
    fn resize_deferral_active_then_expires() {
        let mut d = ResizeDeferral::new();
        d.start_pointer_resize();
        assert!(d.active());
        // After timeout, should expire
        std::thread::sleep(Duration::from_millis(600));
        assert!(!d.active());
    }

    #[test]
    fn resize_deferral_pending() {
        let mut d = ResizeDeferral::new();
        d.pending("w1", 80, 24);
        d.pending("w2", 120, 40);
        let drained = d.end();
        assert_eq!(drained.len(), 2);
    }

    #[test]
    fn hold_mode_lifecycle() {
        let mut h = HoldMode::new();
        assert!(!h.active());
        h.start();
        assert!(h.active());
        assert!(h.duration().is_some());
        h.end();
        assert!(!h.active());
    }

    #[test]
    fn detect_file_paths_finds_paths() {
        let text = "error in /home/user/project/src/main.rs at line 42";
        let paths = detect_file_paths(text);
        assert!(!paths.is_empty());
        assert!(paths.iter().any(|(_, p)| p.contains("main.rs")));
    }

    #[test]
    fn detect_file_paths_ignores_short() {
        let text = "see /a for details";
        let paths = detect_file_paths(text);
        assert!(paths.is_empty() || paths.iter().all(|(_, p)| p.len() <= 2));
    }

    #[test]
    fn osc9_notify_writes_sequence() {
        let mut buf = Vec::new();
        osc9_notify(&mut buf, "hello").unwrap();
        assert_eq!(String::from_utf8(buf).unwrap(), "\x1b]9;hello\x1b\\");
    }

    #[test]
    fn osc777_notify_writes_sequence() {
        let mut buf = Vec::new();
        osc777_notify(&mut buf, "title", "msg").unwrap();
        assert_eq!(
            String::from_utf8(buf).unwrap(),
            "\x1b]777;notify;title;msg\x1b\\"
        );
    }

    #[test]
    fn notification_jump_add_and_clear() {
        let mut nj = NotificationJump::new();
        nj.add(NotificationEntry {
            window_id: "w1".into(),
            title: "test".into(),
            message: "hello".into(),
            timestamp: Instant::now(),
        });
        assert_eq!(nj.entries().len(), 1);
        nj.clear_for("w1");
        assert!(nj.entries().is_empty());
    }

    #[test]
    fn notification_jump_latest_for() {
        let mut nj = NotificationJump::new();
        nj.add(NotificationEntry {
            window_id: "w1".into(),
            title: "first".into(),
            message: "a".into(),
            timestamp: Instant::now(),
        });
        std::thread::sleep(Duration::from_millis(1));
        nj.add(NotificationEntry {
            window_id: "w1".into(),
            title: "second".into(),
            message: "b".into(),
            timestamp: Instant::now(),
        });
        let latest = nj.latest_for("w1").unwrap();
        assert_eq!(latest.title, "second");
    }

    #[test]
    fn keyboard_enhancements_set_flags() {
        let ke = KeyboardEnhancements {
            disambiguate_escape: true,
            report_event_types: true,
            ..Default::default()
        };
        let seq = ke.set_flags_sequence();
        assert!(seq.contains("\x1b[>"));
        assert!(seq.contains("u"));
        // flags 1 | 2 = 3
        assert!(seq.contains("3"));
    }

    #[test]
    fn mac_option_advice_terminal() {
        assert!(mac_option_advice("Apple_Terminal").is_some());
        assert!(mac_option_advice("iTerm.app").is_some());
        assert!(mac_option_advice("xterm-256color").is_none());
    }

    #[test]
    fn separator_gap() {
        let g = SeparatorGap::new(2, 1);
        assert_eq!(g.h_gap(), 2);
        assert_eq!(g.v_gap(), 1);
    }

    #[test]
    fn tick_stats_record() {
        let mut stats = TickStats::new();
        stats.record_frame(Duration::from_millis(5));
        stats.record_frame(Duration::from_millis(15));
        assert_eq!(stats.frame_count(), 2);
        assert_eq!(stats.avg_render_time(), Duration::from_millis(10));
    }

    #[test]
    fn post_render_writer_queue_and_drain() {
        let mut w = PostRenderWriter::new();
        assert!(!w.pending());
        w.queue_str("hello");
        w.queue(b" world".as_ref());
        assert!(w.pending());
        let drained = w.drain();
        assert_eq!(String::from_utf8(drained).unwrap(), "hello world");
        assert!(!w.pending());
    }

    #[test]
    fn base64_encode_basic() {
        assert_eq!(base64_encode(b"hello"), "aGVsbG8");
    }
}
