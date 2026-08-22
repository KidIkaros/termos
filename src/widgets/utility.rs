//! Utility widgets — clock, notes, clipboard, quick actions.

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use std::time::{SystemTime, UNIX_EPOCH};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use super::{Widget, WidgetKind};

// ---------------------------------------------------------------------------
// Clock Widget
// ---------------------------------------------------------------------------

/// Shows current time and date.
pub struct ClockWidget {
    time_str: String,
    date_str: String,
}

impl ClockWidget {
    pub fn new() -> Self {
        Self {
            time_str: String::new(),
            date_str: String::new(),
        }
    }
}

impl Default for ClockWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for ClockWidget {
    fn id(&self) -> &str {
        "clock"
    }
    fn name(&self) -> &str {
        "Clock"
    }
    fn kind(&self) -> WidgetKind {
        WidgetKind::Utility
    }
    fn refresh_interval_ms(&self) -> u64 {
        1000
    }
    fn tick(&mut self) {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        // Simple time formatting without chrono
        let hours = (now / 3600) % 24;
        let minutes = (now / 60) % 60;
        let seconds = now % 60;
        self.time_str = format!("{hours:02}:{minutes:02}:{seconds:02}");
        self.date_str = format!("Day {}/{}", now / 86400 + 1, now % 86400 / 3600);
    }

    fn render(&self, f: &mut Frame, area: Rect) {
        let lines = vec![
            Line::from(Span::styled(
                &self.time_str,
                Style::default().fg(Color::Cyan),
            )),
            Line::from(Span::raw(&self.date_str)),
        ];
        let block = Block::default().title(" Clock ").borders(Borders::ALL);
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
// Notes Widget
// ---------------------------------------------------------------------------

/// A simple scratchpad for quick notes. Interactive: you can type into it.
pub struct NotesWidget {
    lines: Vec<String>,
    cursor_row: usize,
    cursor_col: usize,
    dirty: bool,
}

impl NotesWidget {
    pub fn new() -> Self {
        Self {
            lines: vec![String::new()],
            cursor_row: 0,
            cursor_col: 0,
            dirty: false,
        }
    }
}

impl Default for NotesWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for NotesWidget {
    fn id(&self) -> &str {
        "notes"
    }
    fn name(&self) -> &str {
        "Notes"
    }
    fn kind(&self) -> WidgetKind {
        WidgetKind::Utility
    }
    fn is_interactive(&self) -> bool {
        true
    }

    fn tick(&mut self) {
        // Notes don't auto-refresh; they update on input only.
    }

    fn render(&self, f: &mut Frame, area: Rect) {
        let lines: Vec<Line> = self
            .lines
            .iter()
            .enumerate()
            .map(|(i, l)| {
                let style = if i == self.cursor_row {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                };
                Line::styled(l.clone(), style)
            })
            .collect();
        let block = Block::default()
            .title(format!(" Notes [{} lines] ", self.lines.len()))
            .borders(Borders::ALL);
        let paragraph = Paragraph::new(lines).block(block);
        f.render_widget(paragraph, area);
    }

    fn handle_key(&mut self, key: &crossterm::event::KeyEvent) -> bool {
        use crossterm::event::{KeyCode, KeyModifiers};
        match key.code {
            KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::NONE)
                || key.modifiers.contains(KeyModifiers::SHIFT) =>
            {
                if let Some(line) = self.lines.get_mut(self.cursor_row) {
                    line.insert(self.cursor_col, c);
                    self.cursor_col += 1;
                    self.dirty = true;
                }
                true
            }
            KeyCode::Enter => {
                let rest = if let Some(line) = self.lines.get(self.cursor_row) {
                    line[self.cursor_col..].to_string()
                } else {
                    String::new()
                };
                if let Some(line) = self.lines.get_mut(self.cursor_row) {
                    line.truncate(self.cursor_col);
                }
                self.cursor_row += 1;
                self.cursor_col = 0;
                self.lines.insert(self.cursor_row, rest);
                self.dirty = true;
                true
            }
            KeyCode::Backspace => {
                if self.cursor_col > 0 {
                    if let Some(line) = self.lines.get_mut(self.cursor_row) {
                        self.cursor_col -= 1;
                        line.remove(self.cursor_col);
                        self.dirty = true;
                    }
                } else if self.cursor_row > 0 {
                    let current = self.lines.remove(self.cursor_row);
                    self.cursor_row -= 1;
                    self.cursor_col = self.lines[self.cursor_row].len();
                    self.lines[self.cursor_row].push_str(&current);
                    self.dirty = true;
                }
                true
            }
            KeyCode::Left => {
                if self.cursor_col > 0 {
                    self.cursor_col -= 1;
                }
                true
            }
            KeyCode::Right => {
                if let Some(line) = self.lines.get(self.cursor_row) {
                    if self.cursor_col < line.len() {
                        self.cursor_col += 1;
                    }
                }
                true
            }
            KeyCode::Up => {
                if self.cursor_row > 0 {
                    self.cursor_row -= 1;
                    let len = self.lines[self.cursor_row].len();
                    self.cursor_col = self.cursor_col.min(len);
                }
                true
            }
            KeyCode::Down => {
                if self.cursor_row + 1 < self.lines.len() {
                    self.cursor_row += 1;
                    let len = self.lines[self.cursor_row].len();
                    self.cursor_col = self.cursor_col.min(len);
                }
                true
            }
            _ => false,
        }
    }

    fn min_width(&self) -> u16 {
        20
    }
    fn min_height(&self) -> u16 {
        5
    }
}

// ---------------------------------------------------------------------------
// Clipboard Widget
// ---------------------------------------------------------------------------

/// Shows clipboard history (last N entries).
pub struct ClipboardWidget {
    entries: Vec<String>,
    selected: usize,
    max_entries: usize,
}

impl ClipboardWidget {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            selected: 0,
            max_entries: 20,
        }
    }
}

impl Default for ClipboardWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for ClipboardWidget {
    fn id(&self) -> &str {
        "clipboard"
    }
    fn name(&self) -> &str {
        "Clipboard"
    }
    fn kind(&self) -> WidgetKind {
        WidgetKind::Utility
    }
    fn refresh_interval_ms(&self) -> u64 {
        2000
    }
    fn tick(&mut self) {
        // Read clipboard via xclip/xsel
        #[cfg(target_os = "linux")]
        if let Ok(output) = std::process::Command::new("xclip")
            .args(["-selection", "clipboard", "-o"])
            .output()
        {
            let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !text.is_empty()
                && self.entries.last().map(|s| s.as_str()) != Some(&text)
            {
                self.entries.push(text);
                if self.entries.len() > self.max_entries {
                    self.entries.remove(0);
                }
            }
        }
    }

    fn render(&self, f: &mut Frame, area: Rect) {
        let lines: Vec<Line> = if self.entries.is_empty() {
            vec![Line::from(Span::styled(
                "  (empty)",
                Style::default().fg(Color::DarkGray),
            ))]
        } else {
            self.entries
                .iter()
                .rev()
                .enumerate()
                .take(area.height as usize - 2)
                .map(|(i, entry)| {
                    let preview: String = entry.chars().take(30).collect();
                    let style = if i == self.selected {
                        Style::default().fg(Color::Yellow)
                    } else {
                        Style::default()
                    };
                    Line::styled(format!("  {preview}"), style)
                })
                .collect()
        };
        let block = Block::default()
            .title(format!(" Clipboard ({}) ", self.entries.len()))
            .borders(Borders::ALL);
        let paragraph = Paragraph::new(lines).block(block);
        f.render_widget(paragraph, area);
    }

    fn min_width(&self) -> u16 {
        20
    }
    fn min_height(&self) -> u16 {
        5
    }
}

// ---------------------------------------------------------------------------
// Quick Actions Widget
// ---------------------------------------------------------------------------

/// A launcher for quick shell commands and scripts.
pub struct ActionsWidget {
    actions: Vec<ActionEntry>,
    selected: usize,
}

#[derive(Debug, Clone)]
pub struct ActionEntry {
    pub label: String,
    pub command: String,
    pub icon: String,
}

impl ActionsWidget {
    pub fn new() -> Self {
        Self {
            actions: vec![
                ActionEntry { label: "Update system".into(), command: "sudo apt update".into(), icon: "⟳".into() },
                ActionEntry { label: "Git pull".into(), command: "git pull".into(), icon: "⬇".into() },
                ActionEntry { label: "Run tests".into(), command: "cargo test".into(), icon: "✓".into() },
                ActionEntry { label: "Format code".into(), command: "cargo fmt".into(), icon: "¶".into() },
                ActionEntry { label: "Disk usage".into(), command: "df -h".into(), icon: "חלוקת".into() },
            ],
            selected: 0,
        }
    }
}

impl Default for ActionsWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for ActionsWidget {
    fn id(&self) -> &str {
        "actions"
    }
    fn name(&self) -> &str {
        "Actions"
    }
    fn kind(&self) -> WidgetKind {
        WidgetKind::Utility
    }
    fn is_interactive(&self) -> bool {
        true
    }

    fn tick(&mut self) {}

    fn render(&self, f: &mut Frame, area: Rect) {
        let lines: Vec<Line> = self
            .actions
            .iter()
            .enumerate()
            .map(|(i, action)| {
                let style = if i == self.selected {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                };
                Line::styled(
                    format!("  {} {}", action.icon, action.label),
                    style,
                )
            })
            .collect();
        let block = Block::default()
            .title(" Quick Actions ")
            .borders(Borders::ALL);
        let paragraph = Paragraph::new(lines).block(block);
        f.render_widget(paragraph, area);
    }

    fn handle_key(&mut self, key: &crossterm::event::KeyEvent) -> bool {
        use crossterm::event::KeyCode;
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if self.selected > 0 {
                    self.selected -= 1;
                }
                true
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.selected + 1 < self.actions.len() {
                    self.selected += 1;
                }
                true
            }
            KeyCode::Enter => {
                // Execute the selected action
                if let Some(action) = self.actions.get(self.selected) {
                    let _ = std::process::Command::new("sh")
                        .args(["-c", &action.command])
                        .spawn();
                }
                true
            }
            _ => false,
        }
    }

    fn min_width(&self) -> u16 {
        20
    }
    fn min_height(&self) -> u16 {
        5
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn clock_widget_basics() {
        let mut w = ClockWidget::new();
        assert_eq!(w.id(), "clock");
        w.tick();
        assert!(!w.time_str.is_empty());
    }

    #[test]
    fn notes_widget_basics() {
        let mut w = NotesWidget::new();
        assert_eq!(w.id(), "notes");
        assert!(w.is_interactive());
    }

    #[test]
    fn clipboard_widget_basics() {
        let mut w = ClipboardWidget::new();
        assert_eq!(w.id(), "clipboard");
        w.tick(); // May fail if xclip not installed
    }

    #[test]
    fn actions_widget_basics() {
        let mut w = ActionsWidget::new();
        assert_eq!(w.id(), "actions");
        assert!(w.is_interactive());
        assert_eq!(w.actions.len(), 5);
    }

    #[test]
    fn render_does_not_panic() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let w = ClockWidget::new();
                w.render(f, Rect::new(0, 0, 30, 5));
                let n = NotesWidget::new();
                n.render(f, Rect::new(0, 5, 30, 8));
                let a = ActionsWidget::new();
                a.render(f, Rect::new(0, 13, 30, 8));
            })
            .unwrap();
    }
}
