//! Structured debug logging for the daemon — ported from Go TUIOS
//! `internal/session/debug.go`.
//!
//! A leveled protocol logger with a ring buffer of recent entries. The level
//! gates what is printed; every entry is stored in the buffer regardless, so
//! `get_log_entries` can surface recent history even when logging is off.

use std::io::{self, Write};
use std::sync::{Mutex, OnceLock, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// The verbosity of protocol logging.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum DebugLevel {
    /// Disable all debug output.
    #[default]
    Off,
    /// Log only errors.
    Errors,
    /// Log connection events and errors.
    Basic,
    /// Log all messages except high-frequency PTY I/O.
    Messages,
    /// Log everything including PTY I/O.
    Verbose,
    /// Log full payload hex dumps.
    Trace,
}

impl DebugLevel {
    /// The wire spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Errors => "errors",
            Self::Basic => "basic",
            Self::Messages => "messages",
            Self::Verbose => "verbose",
            Self::Trace => "trace",
        }
    }
}

impl std::fmt::Display for DebugLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Parse a string into a `DebugLevel`. Unknown input maps to `Off`.
pub fn parse_debug_level(s: &str) -> DebugLevel {
    match s.to_ascii_lowercase().as_str() {
        "off" | "0" | "" => DebugLevel::Off,
        "errors" | "error" | "1" => DebugLevel::Errors,
        "basic" | "2" => DebugLevel::Basic,
        "messages" | "message" | "msg" | "3" => DebugLevel::Messages,
        "verbose" | "4" => DebugLevel::Verbose,
        "trace" | "5" | "all" => DebugLevel::Trace,
        _ => DebugLevel::Off,
    }
}

/// One stored log entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    /// Unix-millis timestamp.
    pub timestamp: i64,
    /// The level name (`"errors"`, `"basic"`, …).
    pub level: String,
    /// The formatted message.
    pub message: String,
}

/// A ring buffer for storing recent log entries.
pub struct LogBuffer {
    entries: Vec<Option<LogEntry>>,
    size: usize,
    head: usize,
    count: usize,
}

impl LogBuffer {
    /// Create a new buffer with the given capacity.
    pub fn new(capacity: usize) -> Self {
        let cap = capacity.max(1);
        Self {
            entries: (0..cap).map(|_| None).collect(),
            size: cap,
            head: 0,
            count: 0,
        }
    }

    /// Add an entry to the buffer.
    pub fn add(&mut self, level: &str, message: &str) {
        let entry = LogEntry {
            timestamp: now_millis(),
            level: level.to_string(),
            message: message.to_string(),
        };
        self.entries[self.head] = Some(entry);
        self.head = (self.head + 1) % self.size;
        if self.count < self.size {
            self.count += 1;
        }
    }

    /// All entries in chronological order.
    pub fn get_all(&self) -> Vec<LogEntry> {
        if self.count == 0 {
            return vec![];
        }
        let start = (self.head + self.size - self.count) % self.size;
        (0..self.count)
            .map(|i| {
                self.entries[(start + i) % self.size]
                    .clone()
                    .unwrap_or_else(|| LogEntry {
                        timestamp: 0,
                        level: String::new(),
                        message: String::new(),
                    })
            })
            .collect()
    }

    /// The last `n` entries in chronological order.
    pub fn get_last(&self, n: usize) -> Vec<LogEntry> {
        let n = n.min(self.count);
        if n == 0 {
            return vec![];
        }
        let start = (self.head + self.size - n) % self.size;
        (0..n)
            .map(|i| {
                self.entries[(start + i) % self.size]
                    .clone()
                    .unwrap_or_else(|| LogEntry {
                        timestamp: 0,
                        level: String::new(),
                        message: String::new(),
                    })
            })
            .collect()
    }

    /// Clear the buffer.
    pub fn clear(&mut self) {
        self.head = 0;
        self.count = 0;
        for e in &mut self.entries {
            *e = None;
        }
    }
}

/// Current millis since the epoch, or 0 on clock failure.
fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// --- Global state -----------------------------------------------------------

static CURRENT_LEVEL: RwLock<DebugLevel> = RwLock::new(DebugLevel::Off);

static LOG_BUFFER: OnceLock<Mutex<LogBuffer>> = OnceLock::new();

fn buffer() -> &'static Mutex<LogBuffer> {
    LOG_BUFFER.get_or_init(|| Mutex::new(LogBuffer::new(1000)))
}

/// The output sink. Defaults to stderr.
static OUTPUT: Mutex<Option<Box<dyn Write + Send + Sync>>> = Mutex::new(None);

/// Set the global debug level.
pub fn set_debug_level(level: DebugLevel) {
    *CURRENT_LEVEL.write().unwrap() = level;
}

/// The current global debug level.
pub fn get_debug_level() -> DebugLevel {
    *CURRENT_LEVEL.read().unwrap()
}

/// Redirect debug output to `w`. Pass `None` to restore stderr.
pub fn set_debug_output(w: Option<Box<dyn Write + Send + Sync>>) {
    *OUTPUT.lock().unwrap() = w;
}

/// Log a message at the given level. The entry is always stored in the ring
/// buffer; it is printed only when the current level is at least `level`.
pub fn protocol_log(level: DebugLevel, message: &str) {
    buffer().lock().unwrap().add(level.as_str(), message);
    if get_debug_level() >= level {
        let mut guard = OUTPUT.lock().unwrap();
        let line = format!("[TUIOS] {}\n", message);
        if let Some(w) = guard.as_mut() {
            let _ = w.write_all(line.as_bytes());
            let _ = w.flush();
        } else {
            let _ = io::stderr().write_all(line.as_bytes());
        }
    }
}

/// Log an error message.
pub fn log_error(msg: &str) {
    protocol_log(DebugLevel::Errors, &format!("[ERROR] {}", msg));
}

/// Log a basic connection event.
pub fn log_basic(msg: &str) {
    protocol_log(DebugLevel::Basic, msg);
}

/// Return the last `n` log entries. `n <= 0` returns all entries.
pub fn get_log_entries(n: i32) -> Vec<LogEntry> {
    if n <= 0 {
        buffer().lock().unwrap().get_all()
    } else {
        buffer().lock().unwrap().get_last(n as usize)
    }
}

/// Clear the global log buffer.
pub fn clear_log_buffer() {
    buffer().lock().unwrap().clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_debug_level_variants() {
        assert_eq!(parse_debug_level("off"), DebugLevel::Off);
        assert_eq!(parse_debug_level("OFF"), DebugLevel::Off);
        assert_eq!(parse_debug_level("errors"), DebugLevel::Errors);
        assert_eq!(parse_debug_level("1"), DebugLevel::Errors);
        assert_eq!(parse_debug_level("basic"), DebugLevel::Basic);
        assert_eq!(parse_debug_level("messages"), DebugLevel::Messages);
        assert_eq!(parse_debug_level("msg"), DebugLevel::Messages);
        assert_eq!(parse_debug_level("verbose"), DebugLevel::Verbose);
        assert_eq!(parse_debug_level("trace"), DebugLevel::Trace);
        assert_eq!(parse_debug_level("all"), DebugLevel::Trace);
        assert_eq!(parse_debug_level("garbage"), DebugLevel::Off);
        assert_eq!(parse_debug_level(""), DebugLevel::Off);
    }

    #[test]
    fn debug_level_display() {
        assert_eq!(DebugLevel::Off.to_string(), "off");
        assert_eq!(DebugLevel::Errors.to_string(), "errors");
        assert_eq!(DebugLevel::Trace.to_string(), "trace");
    }

    #[test]
    fn debug_level_ordering() {
        assert!(DebugLevel::Trace > DebugLevel::Verbose);
        assert!(DebugLevel::Verbose > DebugLevel::Messages);
        assert!(DebugLevel::Messages > DebugLevel::Basic);
        assert!(DebugLevel::Basic > DebugLevel::Errors);
        assert!(DebugLevel::Errors > DebugLevel::Off);
    }

    #[test]
    fn log_buffer_add_and_get_all() {
        let mut buf = LogBuffer::new(4);
        buf.add("errors", "first");
        buf.add("basic", "second");
        let all = buf.get_all();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].message, "first");
        assert_eq!(all[1].message, "second");
        assert_eq!(all[0].level, "errors");
    }

    #[test]
    fn log_buffer_wraps_around() {
        let mut buf = LogBuffer::new(3);
        buf.add("errors", "a");
        buf.add("errors", "b");
        buf.add("errors", "c");
        buf.add("errors", "d"); // overwrites "a"
        let all = buf.get_all();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].message, "b");
        assert_eq!(all[2].message, "d");
    }

    #[test]
    fn log_buffer_get_last() {
        let mut buf = LogBuffer::new(10);
        buf.add("errors", "a");
        buf.add("errors", "b");
        buf.add("errors", "c");
        let last2 = buf.get_last(2);
        assert_eq!(last2.len(), 2);
        assert_eq!(last2[0].message, "b");
        assert_eq!(last2[1].message, "c");
        // More than stored returns all.
        let last10 = buf.get_last(10);
        assert_eq!(last10.len(), 3);
    }

    #[test]
    fn log_buffer_clear() {
        let mut buf = LogBuffer::new(4);
        buf.add("errors", "x");
        buf.clear();
        assert!(buf.get_all().is_empty());
        assert!(buf.get_last(5).is_empty());
    }

    #[test]
    fn log_buffer_empty_get_all() {
        let buf = LogBuffer::new(4);
        assert!(buf.get_all().is_empty());
    }

    #[test]
    fn set_and_get_debug_level() {
        set_debug_level(DebugLevel::Messages);
        assert_eq!(get_debug_level(), DebugLevel::Messages);
        set_debug_level(DebugLevel::Off);
        assert_eq!(get_debug_level(), DebugLevel::Off);
    }

    #[test]
    fn protocol_log_stores_in_buffer() {
        clear_log_buffer();
        protocol_log(DebugLevel::Errors, "test error");
        let entries = get_log_entries(0);
        assert!(entries.iter().any(|e| e.message == "test error"));
    }

    #[test]
    fn log_error_prefixes() {
        clear_log_buffer();
        log_error("boom");
        let entries = get_log_entries(0);
        let e = entries.iter().find(|e| e.message.contains("boom")).unwrap();
        assert!(e.message.contains("[ERROR]"));
    }

    #[test]
    fn get_log_entries_negative_returns_all() {
        clear_log_buffer();
        log_basic("one");
        log_basic("two");
        let all = get_log_entries(-1);
        assert!(all.len() >= 2);
    }
}
