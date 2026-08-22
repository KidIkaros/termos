//! Development widgets — git, build, test status.

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use super::{Widget, WidgetKind};

// ---------------------------------------------------------------------------
// Git Status Widget
// ---------------------------------------------------------------------------

/// Shows git repo status: branch, modified/added/deleted/staged files.
pub struct GitWidget {
    branch: String,
    modified: usize,
    added: usize,
    deleted: usize,
    staged: usize,
    untracked: usize,
    ahead: usize,
    behind: usize,
    dirty: bool,
}

impl GitWidget {
    pub fn new() -> Self {
        Self {
            branch: String::new(),
            modified: 0,
            added: 0,
            deleted: 0,
            staged: 0,
            untracked: 0,
            ahead: 0,
            behind: 0,
            dirty: false,
        }
    }
}

impl Default for GitWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for GitWidget {
    fn id(&self) -> &str {
        "git"
    }
    fn name(&self) -> &str {
        "Git"
    }
    fn kind(&self) -> WidgetKind {
        WidgetKind::Dev
    }
    fn refresh_interval_ms(&self) -> u64 {
        5000
    }
    fn tick(&mut self) {
        // Run git status --porcelain -b
        if let Ok(output) = std::process::Command::new("git")
            .args(["status", "--porcelain", "-b"])
            .output()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            self.modified = 0;
            self.added = 0;
            self.deleted = 0;
            self.staged = 0;
            self.untracked = 0;
            self.ahead = 0;
            self.behind = 0;

            for line in stdout.lines() {
                if self.branch.is_empty() {
                    // First line: ## branch...upstream [ahead N, behind M]
                    if let Some(b) = line.strip_prefix("## ") {
                        self.branch = b.split_whitespace().next().unwrap_or("???").to_string();
                        if let Some(ahead) = line.find("ahead ") {
                            let rest = &line[ahead + 6..];
                            if let Some(n) = rest.split(|c: char| !c.is_ascii_digit()).next() {
                                self.ahead = n.parse().unwrap_or(0);
                            }
                        }
                        if let Some(behind) = line.find("behind ") {
                            let rest = &line[behind + 7..];
                            if let Some(n) = rest.split(|c: char| !c.is_ascii_digit()).next() {
                                self.behind = n.parse().unwrap_or(0);
                            }
                        }
                        continue;
                    }
                }
                let status = &line[..2.min(line.len())];
                match status {
                    "M " | "M" => self.modified += 1,
                    "A " | "A" => self.added += 1,
                    "D " | "D" => self.deleted += 1,
                    "MM" | "AM" | "DM" => {
                        self.staged += 1;
                        self.modified += 1;
                    }
                    "??" => self.untracked += 1,
                    _ => {}
                }
            }
            self.dirty = self.modified + self.added + self.deleted + self.staged > 0;
        }
    }

    fn render(&self, f: &mut Frame, area: Rect) {
        let branch_color = if self.dirty {
            Color::Yellow
        } else {
            Color::Green
        };
        let mut lines = vec![Line::from(vec![
            Span::styled(" ", Style::default()),
            Span::styled(&self.branch, Style::default().fg(branch_color)),
        ])];

        if self.staged > 0 {
            lines.push(Line::from(Span::styled(
                format!("  staged:  {}", self.staged),
                Style::default().fg(Color::Green),
            )));
        }
        if self.modified > 0 {
            lines.push(Line::from(Span::styled(
                format!("  changed: {}", self.modified),
                Style::default().fg(Color::Yellow),
            )));
        }
        if self.deleted > 0 {
            lines.push(Line::from(Span::styled(
                format!("  deleted: {}", self.deleted),
                Style::default().fg(Color::Red),
            )));
        }
        if self.untracked > 0 {
            lines.push(Line::from(Span::styled(
                format!("  new:     {}", self.untracked),
                Style::default().fg(Color::Blue),
            )));
        }
        if self.ahead > 0 {
            lines.push(Line::from(Span::styled(
                format!("  ahead:   {}", self.ahead),
                Style::default().fg(Color::Cyan),
            )));
        }
        if self.behind > 0 {
            lines.push(Line::from(Span::styled(
                format!("  behind:  {}", self.behind),
                Style::default().fg(Color::Magenta),
            )));
        }
        if !self.dirty && self.ahead == 0 && self.behind == 0 {
            lines.push(Line::from(Span::styled(
                "  clean",
                Style::default().fg(Color::DarkGray),
            )));
        }

        let block = Block::default().title(" Git ").borders(Borders::ALL);
        let paragraph = Paragraph::new(lines).block(block);
        f.render_widget(paragraph, area);
    }

    fn min_width(&self) -> u16 {
        18
    }
    fn min_height(&self) -> u16 {
        4
    }
}

// ---------------------------------------------------------------------------
// Build Status Widget
// ---------------------------------------------------------------------------

/// Shows cargo build status: last build time, errors, warnings.
pub struct BuildWidget {
    last_build_ok: Option<bool>,
    errors: usize,
    warnings: usize,
    build_time_ms: u64,
    last_check: std::time::Instant,
}

impl BuildWidget {
    pub fn new() -> Self {
        Self {
            last_build_ok: None,
            errors: 0,
            warnings: 0,
            build_time_ms: 0,
            last_check: std::time::Instant::now(),
        }
    }
}

impl Default for BuildWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for BuildWidget {
    fn id(&self) -> &str {
        "build"
    }
    fn name(&self) -> &str {
        "Build"
    }
    fn kind(&self) -> WidgetKind {
        WidgetKind::Dev
    }
    fn refresh_interval_ms(&self) -> u64 {
        30_000 // Check every 30s (build is slow)
    }
    fn tick(&mut self) {
        let start = std::time::Instant::now();
        if let Ok(output) = std::process::Command::new("cargo")
            .args(["check", "--message-format=short"])
            .output()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let combined = format!("{stdout}{stderr}");
            self.errors = combined.lines().filter(|l| l.contains("error")).count();
            self.warnings = combined.lines().filter(|l| l.contains("warning")).count();
            self.last_build_ok = Some(output.status.success());
            self.build_time_ms = start.elapsed().as_millis() as u64;
            self.last_check = std::time::Instant::now();
        }
    }

    fn render(&self, f: &mut Frame, area: Rect) {
        let (status_text, status_color) = match self.last_build_ok {
            Some(true) => ("OK", Color::Green),
            Some(false) => ("FAIL", Color::Red),
            None => ("—", Color::DarkGray),
        };

        let mut lines = vec![Line::from(vec![
            Span::styled(" ", Style::default()),
            Span::styled(status_text, Style::default().fg(status_color)),
            Span::raw(format!("  ({}ms)", self.build_time_ms)),
        ])];

        if self.errors > 0 {
            lines.push(Line::from(Span::styled(
                format!("  {e} errors", e = self.errors),
                Style::default().fg(Color::Red),
            )));
        }
        if self.warnings > 0 {
            lines.push(Line::from(Span::styled(
                format!("  {w} warnings", w = self.warnings),
                Style::default().fg(Color::Yellow),
            )));
        }

        let block = Block::default().title(" Build ").borders(Borders::ALL);
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

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn git_widget_basics() {
        let mut w = GitWidget::new();
        assert_eq!(w.id(), "git");
        assert_eq!(w.kind(), WidgetKind::Dev);
        w.tick(); // May not be in a git repo
    }

    #[test]
    fn build_widget_basics() {
        let mut w = BuildWidget::new();
        assert_eq!(w.id(), "build");
        assert_eq!(w.kind(), WidgetKind::Dev);
    }

    #[test]
    fn render_does_not_panic() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let w = GitWidget::new();
                w.render(f, Rect::new(0, 0, 30, 8));
                let b = BuildWidget::new();
                b.render(f, Rect::new(0, 8, 30, 6));
            })
            .unwrap();
    }
}
