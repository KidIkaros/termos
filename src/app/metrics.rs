//! Metrics pipeline for status widgets.
//!
//! Collects and aggregates metrics from panes, sessions, and the daemon.
//! Based on the metrics monitoring design from Chapter 20 of System Design Interview.
//!
//! Metrics flow:
//! 1. **Collect** → each pane emits metrics periodically
//! 2. **Aggregate** → roll up into per-session and global summaries
//! 3. **Expose** → status widgets query the aggregated data

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A single metric data point.
#[derive(Debug, Clone)]
pub struct MetricPoint {
    pub name: String,
    pub value: f64,
    pub unit: MetricUnit,
    pub timestamp: Instant,
}

/// Metric units for display formatting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricUnit {
    Bytes,
    BytesPerSecond,
    Count,
    Percent,
    Duration,
    None,
}

impl MetricUnit {
    pub fn suffix(&self) -> &'static str {
        match self {
            MetricUnit::Bytes => "B",
            MetricUnit::BytesPerSecond => "B/s",
            MetricUnit::Count => "",
            MetricUnit::Percent => "%",
            MetricUnit::Duration => "ms",
            MetricUnit::None => "",
        }
    }
}

/// Per-pane metrics snapshot.
#[derive(Debug, Clone)]
pub struct PaneMetrics {
    pub pane_id: i32,
    pub bytes_read: u64,
    pub bytes_written: u64,
    pub read_rate: f64,    // bytes/sec
    pub write_rate: f64,   // bytes/sec
    pub cpu_percent: f64,
    pub uptime: Duration,
    pub lines_scrolled: u64,
    pub timestamp: Instant,
}

impl Default for PaneMetrics {
    fn default() -> Self {
        Self {
            pane_id: 0,
            bytes_read: 0,
            bytes_written: 0,
            read_rate: 0.0,
            write_rate: 0.0,
            cpu_percent: 0.0,
            uptime: Duration::ZERO,
            lines_scrolled: 0,
            timestamp: Instant::now(),
        }
    }
}

/// Aggregated session metrics.
#[derive(Debug, Clone, Default)]
pub struct SessionMetrics {
    pub pane_count: usize,
    pub total_bytes_read: u64,
    pub total_bytes_written: u64,
    pub active_panes: usize,
    pub uptime: Duration,
}

/// Global daemon metrics.
#[derive(Debug, Clone, Default)]
pub struct DaemonMetrics {
    pub session_count: usize,
    pub total_ptys: usize,
    pub active_ptys: usize,
    pub uptime: Duration,
    pub memory_usage: u64,
}

/// The metrics collector aggregates data from panes and sessions.
pub struct MetricsCollector {
    pane_metrics: Arc<Mutex<HashMap<i32, PaneMetrics>>>,
    start_time: Instant,
}

impl MetricsCollector {
    pub fn new() -> Self {
        Self {
            pane_metrics: Arc::new(Mutex::new(HashMap::new())),
            start_time: Instant::now(),
        }
    }

    /// Update metrics for a specific pane.
    pub fn update_pane(&self, pane_id: i32, bytes_read: u64, bytes_written: u64) {
        let mut metrics = self.pane_metrics.lock().unwrap();
        let entry = metrics.entry(pane_id).or_insert_with(|| PaneMetrics {
            pane_id,
            ..Default::default()
        });

        let now = Instant::now();
        let elapsed = now.duration_since(entry.timestamp).as_secs_f64();

        if elapsed > 0.0 {
            entry.read_rate = (bytes_read - entry.bytes_read) as f64 / elapsed;
            entry.write_rate = (bytes_written - entry.bytes_written) as f64 / elapsed;
        }

        entry.bytes_read = bytes_read;
        entry.bytes_written = bytes_written;
        entry.timestamp = now;
    }

    /// Remove metrics for a closed pane.
    pub fn remove_pane(&self, pane_id: i32) {
        let mut metrics = self.pane_metrics.lock().unwrap();
        metrics.remove(&pane_id);
    }

    /// Get metrics for a specific pane.
    pub fn pane_metrics(&self, pane_id: i32) -> Option<PaneMetrics> {
        let metrics = self.pane_metrics.lock().unwrap();
        metrics.get(&pane_id).cloned()
    }

    /// Get aggregated session metrics.
    pub fn session_metrics(&self) -> SessionMetrics {
        let metrics = self.pane_metrics.lock().unwrap();
        SessionMetrics {
            pane_count: metrics.len(),
            total_bytes_read: metrics.values().map(|m| m.bytes_read).sum(),
            total_bytes_written: metrics.values().map(|m| m.bytes_written).sum(),
            active_panes: metrics
                .values()
                .filter(|m| m.read_rate > 0.0 || m.write_rate > 0.0)
                .count(),
            uptime: self.start_time.elapsed(),
        }
    }

    /// Get formatted metrics for status bar display.
    pub fn format_for_status_bar(&self) -> String {
        let session = self.session_metrics();
        let up = format_duration(session.uptime);
        let total = session.total_bytes_read + session.total_bytes_written;
        let total_fmt = format_bytes(total);
        format!(
            "{} panes | {} total | up {}",
            session.pane_count, total_fmt, up
        )
    }

    /// Reset all metrics.
    pub fn reset(&self) {
        let mut metrics = self.pane_metrics.lock().unwrap();
        metrics.clear();
    }
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

/// Format bytes into human-readable string.
pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

/// Format duration into human-readable string.
pub fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    }
}

/// Format bytes/sec into human-readable string.
pub fn format_rate(bytes_per_sec: f64) -> String {
    if bytes_per_sec >= 1_048_576.0 {
        format!("{:.1} MB/s", bytes_per_sec / 1_048_576.0)
    } else if bytes_per_sec >= 1024.0 {
        format!("{:.1} KB/s", bytes_per_sec / 1024.0)
    } else {
        format!("{:.0} B/s", bytes_per_sec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_bytes_test() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1_048_576), "1.0 MB");
        assert_eq!(format_bytes(1_073_741_824), "1.0 GB");
    }

    #[test]
    fn format_duration_test() {
        assert_eq!(format_duration(Duration::from_secs(30)), "30s");
        assert_eq!(format_duration(Duration::from_secs(90)), "1m 30s");
        assert_eq!(format_duration(Duration::from_secs(3661)), "1h 1m");
    }

    #[test]
    fn format_rate_test() {
        assert_eq!(format_rate(500.0), "500 B/s");
        assert_eq!(format_rate(1500.0), "1.5 KB/s");
        assert_eq!(format_rate(2_097_152.0), "2.0 MB/s");
    }

    #[test]
    fn pane_metrics_update() {
        let collector = MetricsCollector::new();
        collector.update_pane(1, 1000, 500);
        let metrics = collector.pane_metrics(1).unwrap();
        assert_eq!(metrics.bytes_read, 1000);
        assert_eq!(metrics.bytes_written, 500);
    }

    #[test]
    fn session_metrics_aggregation() {
        let collector = MetricsCollector::new();
        collector.update_pane(1, 1000, 500);
        collector.update_pane(2, 2000, 1000);

        let session = collector.session_metrics();
        assert_eq!(session.pane_count, 2);
        assert_eq!(session.total_bytes_read, 3000);
        assert_eq!(session.total_bytes_written, 1500);
    }

    #[test]
    fn remove_pane_cleans_up() {
        let collector = MetricsCollector::new();
        collector.update_pane(1, 1000, 500);
        assert!(collector.pane_metrics(1).is_some());

        collector.remove_pane(1);
        assert!(collector.pane_metrics(1).is_none());
    }
}
