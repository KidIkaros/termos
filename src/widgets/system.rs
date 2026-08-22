//! System monitoring widgets.

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, Paragraph};
use ratatui::Frame;

use super::{Widget, WidgetKind};

// ---------------------------------------------------------------------------
// CPU Widget
// ---------------------------------------------------------------------------

/// Displays CPU usage as a gauge with a sparkline history.
pub struct CpuWidget {
    /// Per-core usage percentages (0.0–100.0).
    _cores: Vec<f64>,
    /// Rolling history for sparkline (last 60 samples).
    history: Vec<f64>,
    /// Overall usage.
    overall: f64,
}

impl CpuWidget {
    pub fn new() -> Self {
        Self {
            _cores: Vec::new(),
            history: Vec::with_capacity(60),
            overall: 0.0,
        }
    }
}

impl Default for CpuWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for CpuWidget {
    fn id(&self) -> &str {
        "cpu"
    }
    fn name(&self) -> &str {
        "CPU"
    }
    fn kind(&self) -> WidgetKind {
        WidgetKind::System
    }
    fn refresh_interval_ms(&self) -> u64 {
        2000
    }
    fn tick(&mut self) {
        // Read /proc/stat on Linux
        #[cfg(target_os = "linux")]
        {
            if let Ok(stat) = std::fs::read_to_string("/proc/stat") {
                let mut total_idle = 0u64;
                let mut total = 0u64;
                for line in stat.lines() {
                    if let Some(rest) = line.strip_prefix("cpu") {
                        let nums: Vec<u64> = rest
                            .split_whitespace()
                            .filter_map(|s| s.parse().ok())
                            .collect();
                        if nums.len() >= 4 {
                            let idle = nums[3];
                            let line_total: u64 = nums.iter().sum();
                            total_idle += idle;
                            total += line_total;
                        }
                    }
                }
                if total > 0 {
                    self.overall = ((total - total_idle) as f64 / total as f64) * 100.0;
                    self.history.push(self.overall);
                    if self.history.len() > 60 {
                        self.history.remove(0);
                    }
                }
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            // Fallback: simulate
            self.overall = 0.0;
        }
    }

    fn render(&self, f: &mut Frame, area: Rect) {
        let _label = format!("{:.1}%", self.overall);
        let color = if self.overall > 80.0 {
            Color::Red
        } else if self.overall > 50.0 {
            Color::Yellow
        } else {
            Color::Green
        };
        let gauge = Gauge::default()
            .block(Block::default().title(" CPU ").borders(Borders::ALL))
            .gauge_style(Style::default().fg(color))
            .ratio((self.overall / 100.0) as f64);
        f.render_widget(gauge, area);
    }

    fn min_width(&self) -> u16 {
        20
    }
    fn min_height(&self) -> u16 {
        3
    }
}

// ---------------------------------------------------------------------------
// Memory Widget
// ---------------------------------------------------------------------------

/// Displays memory usage with used/total and a gauge.
pub struct MemWidget {
    used_bytes: u64,
    total_bytes: u64,
    swap_used: u64,
    swap_total: u64,
}

impl MemWidget {
    pub fn new() -> Self {
        Self {
            used_bytes: 0,
            total_bytes: 1,
            swap_used: 0,
            swap_total: 0,
        }
    }
}

impl Default for MemWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for MemWidget {
    fn id(&self) -> &str {
        "mem"
    }
    fn name(&self) -> &str {
        "Memory"
    }
    fn kind(&self) -> WidgetKind {
        WidgetKind::System
    }
    fn refresh_interval_ms(&self) -> u64 {
        3000
    }
    fn tick(&mut self) {
        #[cfg(target_os = "linux")]
        {
            if let Ok(meminfo) = std::fs::read_to_string("/proc/meminfo") {
                for line in meminfo.lines() {
                    if let Some(rest) = line.strip_prefix("MemTotal:") {
                        self.total_bytes = parse_kb(rest) * 1024;
                    } else if let Some(rest) = line.strip_prefix("MemAvailable:") {
                        let available = parse_kb(rest) * 1024;
                        self.used_bytes = self.total_bytes.saturating_sub(available);
                    } else if let Some(rest) = line.strip_prefix("SwapTotal:") {
                        self.swap_total = parse_kb(rest) * 1024;
                    } else if let Some(rest) = line.strip_prefix("SwapFree:") {
                        let free = parse_kb(rest) * 1024;
                        self.swap_used = self.swap_total.saturating_sub(free);
                    }
                }
            }
        }
    }

    fn render(&self, f: &mut Frame, area: Rect) {
        let pct = if self.total_bytes > 0 {
            (self.used_bytes as f64 / self.total_bytes as f64) * 100.0
        } else {
            0.0
        };
        let used_mb = self.used_bytes / (1024 * 1024);
        let total_mb = self.total_bytes / (1024 * 1024);
        let label = format!("{used_mb}/{total_mb} MB ({pct:.1}%)");

        let color = if pct > 85.0 {
            Color::Red
        } else if pct > 60.0 {
            Color::Yellow
        } else {
            Color::Green
        };

        let gauge = Gauge::default()
            .block(Block::default().title(" Memory ").borders(Borders::ALL))
            .label(label)
            .gauge_style(Style::default().fg(color))
            .ratio((pct / 100.0) as f64);
        f.render_widget(gauge, area);
    }

    fn min_width(&self) -> u16 {
        24
    }
    fn min_height(&self) -> u16 {
        3
    }
}

// ---------------------------------------------------------------------------
// Disk Widget
// ---------------------------------------------------------------------------

/// Shows disk usage for mounted filesystems.
pub struct DiskWidget {
    entries: Vec<DiskEntry>,
}

#[derive(Debug, Clone)]
struct DiskEntry {
    mount: String,
    total_gb: f64,
    used_gb: f64,
}

impl DiskWidget {
    pub fn new() -> Self {
        Self { entries: Vec::new() }
    }
}

impl Default for DiskWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for DiskWidget {
    fn id(&self) -> &str {
        "disk"
    }
    fn name(&self) -> &str {
        "Disk"
    }
    fn kind(&self) -> WidgetKind {
        WidgetKind::System
    }
    fn refresh_interval_ms(&self) -> u64 {
        10_000 // Check every 10s
    }
    fn tick(&mut self) {
        self.entries.clear();
        // Use df command for cross-platform compatibility
        if let Ok(output) = std::process::Command::new("df")
            .args(["-h", "--output=source,size,used,avail,target"])
            .output()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines().skip(1) {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 5 && parts[0].starts_with("/dev/") {
                    let mount = parts[4].to_string();
                    let total = parse_size_gb(parts[1]);
                    let used = parse_size_gb(parts[2]);
                    if total > 0.0 {
                        self.entries.push(DiskEntry { mount, total_gb: total, used_gb: used });
                    }
                }
            }
        }
    }

    fn render(&self, f: &mut Frame, area: Rect) {
        let mut lines = Vec::new();
        for entry in &self.entries {
            let pct = if entry.total_gb > 0.0 {
                (entry.used_gb / entry.total_gb) * 100.0
            } else {
                0.0
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{:8}", entry.mount),
                    Style::default().fg(Color::Cyan),
                ),
                Span::raw(format!(
                    " {:6.1}/{:.1} GB ({:.0}%)",
                    entry.used_gb, entry.total_gb, pct
                )),
            ]));
        }
        let block = Block::default().title(" Disk ").borders(Borders::ALL);
        let paragraph = Paragraph::new(lines).block(block);
        f.render_widget(paragraph, area);
    }

    fn min_width(&self) -> u16 {
        28
    }
    fn min_height(&self) -> u16 {
        3
    }
}

// ---------------------------------------------------------------------------
// Network Widget
// ---------------------------------------------------------------------------

/// Shows network interface stats (bytes in/out).
pub struct NetWidget {
    rx_bytes: u64,
    tx_bytes: u64,
    prev_rx: u64,
    prev_tx: u64,
    rx_rate: f64,
    tx_rate: f64,
}

impl NetWidget {
    pub fn new() -> Self {
        Self {
            rx_bytes: 0,
            tx_bytes: 0,
            prev_rx: 0,
            prev_tx: 0,
            rx_rate: 0.0,
            tx_rate: 0.0,
        }
    }
}

impl Default for NetWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for NetWidget {
    fn id(&self) -> &str {
        "net"
    }
    fn name(&self) -> &str {
        "Network"
    }
    fn kind(&self) -> WidgetKind {
        WidgetKind::System
    }
    fn refresh_interval_ms(&self) -> u64 {
        2000
    }
    fn tick(&mut self) {
        #[cfg(target_os = "linux")]
        {
            if let Ok(stat) = std::fs::read_to_string("/proc/net/dev") {
                let mut rx = 0u64;
                let mut tx = 0u64;
                for line in stat.lines().skip(2) {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 10 {
                        if let (Ok(r), Ok(t)) = (parts[1].parse::<u64>(), parts[9].parse::<u64>())
                        {
                            rx += r;
                            tx += t;
                        }
                    }
                }
                let elapsed = 2.0; // refresh_interval_ms / 1000
                if self.prev_rx > 0 {
                    self.rx_rate = (rx - self.prev_rx) as f64 / elapsed;
                    self.tx_rate = (tx - self.prev_tx) as f64 / elapsed;
                }
                self.prev_rx = self.rx_bytes;
                self.prev_tx = self.tx_bytes;
                self.rx_bytes = rx;
                self.tx_bytes = tx;
            }
        }
    }

    fn render(&self, f: &mut Frame, area: Rect) {
        let rx_str = format_bytes_rate(self.rx_rate);
        let tx_str = format_bytes_rate(self.tx_rate);
        let lines = vec![
            Line::from(vec![
                Span::styled("  ▼ ", Style::default().fg(Color::Green)),
                Span::raw(format!("{rx_str}/s")),
            ]),
            Line::from(vec![
                Span::styled("  ▲ ", Style::default().fg(Color::Red)),
                Span::raw(format!("{tx_str}/s")),
            ]),
        ];
        let block = Block::default().title(" Network ").borders(Borders::ALL);
        let paragraph = Paragraph::new(lines).block(block);
        f.render_widget(paragraph, area);
    }

    fn min_width(&self) -> u16 {
        20
    }
    fn min_height(&self) -> u16 {
        4
    }
}

// ---------------------------------------------------------------------------
// Process Widget
// ---------------------------------------------------------------------------

/// Shows top processes by CPU usage.
pub struct ProcWidget {
    procs: Vec<ProcEntry>,
}

#[derive(Debug, Clone)]
struct ProcEntry {
    pid: u32,
    name: String,
    cpu: f64,
}

impl ProcWidget {
    pub fn new() -> Self {
        Self { procs: Vec::new() }
    }
}

impl Default for ProcWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for ProcWidget {
    fn id(&self) -> &str {
        "proc"
    }
    fn name(&self) -> &str {
        "Processes"
    }
    fn kind(&self) -> WidgetKind {
        WidgetKind::System
    }
    fn refresh_interval_ms(&self) -> u64 {
        5000
    }
    fn tick(&mut self) {
        // Parse /proc for top processes — simplified
        self.procs.clear();
        #[cfg(target_os = "linux")]
        {
            use std::collections::HashMap;
            let mut cpus: HashMap<u32, (String, f64, f64)> = HashMap::new();
            if let Ok(dir) = std::fs::read_dir("/proc") {
                for entry in dir.flatten() {
                    let name = entry.file_name();
                    if let Some(pid_str) = name.to_str() {
                        if let Ok(pid) = pid_str.parse::<u32>() {
                            let stat_path = format!("/proc/{pid}/stat");
                            let comm_path = format!("/proc/{pid}/comm");
                            if let (Ok(stat), Ok(comm)) = (
                                std::fs::read_to_string(&stat_path),
                                std::fs::read_to_string(&comm_path),
                            ) {
                                // Parse CPU from stat (fields 14,15 = utime, stime)
                                let fields: Vec<&str> = stat.split_whitespace().collect();
                                if fields.len() > 21 {
                                    if let (Ok(utime), Ok(stime)) = (
                                        fields[13].parse::<f64>(),
                                        fields[14].parse::<f64>(),
                                    ) {
                                        let cpu = (utime + stime) / 100.0;
                                        cpus.insert(pid, (comm.trim().to_string(), cpu, 0.0));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            let mut procs: Vec<ProcEntry> = cpus
                .into_iter()
                .map(|(pid, (name, cpu, _mem))| ProcEntry {
                    pid,
                    name,
                    cpu,
                })
                .collect();
            procs.sort_by(|a, b| b.cpu.partial_cmp(&a.cpu).unwrap_or(std::cmp::Ordering::Equal));
            procs.truncate(8);
            self.procs = procs;
        }
    }

    fn render(&self, f: &mut Frame, area: Rect) {
        let mut lines = vec![Line::from(Span::styled(
            " PID   CPU%  NAME",
            Style::default().fg(Color::DarkGray),
        ))];
        for p in &self.procs {
            lines.push(Line::from(vec![
                Span::raw(format!(" {:5} ", p.pid)),
                Span::styled(
                    format!("{:5.1} ", p.cpu),
                    Style::default().fg(if p.cpu > 50.0 {
                        Color::Red
                    } else {
                        Color::Green
                    }),
                ),
                Span::raw(format!("{:.12}", p.name)),
            ]));
        }
        let block = Block::default().title(" Processes ").borders(Borders::ALL);
        let paragraph = Paragraph::new(lines).block(block);
        f.render_widget(paragraph, area);
    }

    fn min_width(&self) -> u16 {
        24
    }
    fn min_height(&self) -> u16 {
        6
    }
}

fn parse_size_gb(s: &str) -> f64 {
    let s = s.trim();
    if let Some(v) = s.strip_suffix('T') {
        v.parse::<f64>().unwrap_or(0.0) * 1024.0
    } else if let Some(v) = s.strip_suffix('G') {
        v.parse::<f64>().unwrap_or(0.0)
    } else if let Some(v) = s.strip_suffix('M') {
        v.parse::<f64>().unwrap_or(0.0) / 1024.0
    } else if let Some(v) = s.strip_suffix('K') {
        v.parse::<f64>().unwrap_or(0.0) / (1024.0 * 1024.0)
    } else {
        s.parse::<f64>().unwrap_or(0.0) / (1024.0 * 1024.0)
    }
}

fn parse_kb(rest: &str) -> u64 {
    rest.split_whitespace()
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// Format bytes/sec as human-readable.
fn format_bytes_rate(bps: f64) -> String {
    if bps >= 1_048_576.0 {
        format!("{:.1} MB", bps / 1_048_576.0)
    } else if bps >= 1024.0 {
        format!("{:.1} KB", bps / 1024.0)
    } else {
        format!("{:.0} B", bps)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn cpu_widget_basics() {
        let mut w = CpuWidget::new();
        assert_eq!(w.id(), "cpu");
        assert_eq!(w.kind(), WidgetKind::System);
        w.tick(); // Should not panic
    }

    #[test]
    fn mem_widget_basics() {
        let mut w = MemWidget::new();
        assert_eq!(w.id(), "mem");
        w.tick();
    }

    #[test]
    fn disk_widget_basics() {
        let mut w = DiskWidget::new();
        assert_eq!(w.id(), "disk");
        w.tick();
    }

    #[test]
    fn net_widget_basics() {
        let mut w = NetWidget::new();
        assert_eq!(w.id(), "net");
        w.tick();
    }

    #[test]
    fn proc_widget_basics() {
        let mut w = ProcWidget::new();
        assert_eq!(w.id(), "proc");
        w.tick();
    }

    #[test]
    fn render_does_not_panic() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        let widgets: Vec<Box<dyn Widget>> = vec![
            Box::new(CpuWidget::new()),
            Box::new(MemWidget::new()),
            Box::new(DiskWidget::new()),
            Box::new(NetWidget::new()),
            Box::new(ProcWidget::new()),
        ];

        terminal
            .draw(|f| {
                let area = Rect::new(0, 0, 40, 6);
                for (i, w) in widgets.iter().enumerate() {
                    let y = (i as u16) * 6;
                    if y + 6 <= 24 {
                        w.render(f, Rect::new(0, y, 40, 6));
                    }
                }
            })
            .unwrap();
    }

    #[test]
    fn format_bytes_rate_test() {
        assert_eq!(format_bytes_rate(500.0), "500 B");
        assert_eq!(format_bytes_rate(1500.0), "1.5 KB");
        assert_eq!(format_bytes_rate(2_097_152.0), "2.0 MB");
    }
}
