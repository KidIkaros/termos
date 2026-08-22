//! Widget system — TermOS as a mission-control dashboard.
//!
//! Every widget implements the `Widget` trait: it owns its state, updates on a
//! configurable interval, and renders into an allocated rectangle.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────┐
//! │ WidgetRegistry                                   │
//! │  ├─ System: CpuWidget, MemWidget, DiskWidget …  │
//! │  ├─ Dev:     GitWidget, BuildWidget …            │
//! │  └─ Utility: ClockWidget, NotesWidget …          │
//! │                                                   │
//! │ ┌──────────┐ ┌──────────┐ ┌──────────┐          │
//! │ │ Widget A │ │ Widget B │ │ Widget C │  ← grid  │
//! │ └──────────┘ └──────────┘ └──────────┘          │
//! └─────────────────────────────────────────────────┘
//! ```

pub mod layout;
pub mod registry;
pub mod system;
pub mod dev;
pub mod utility;

pub use registry::{WidgetRegistry, WidgetMeta, WidgetKind};
pub use layout::{WidgetLayout, WidgetSlot};

use ratatui::layout::Rect;
use ratatui::Frame;

/// Core widget trait. Every dashboard widget implements this.
pub trait Widget: Send + Sync {
    /// Unique identifier (e.g. "cpu", "git_status", "clock").
    fn id(&self) -> &str;

    /// Display name shown in the UI.
    fn name(&self) -> &str;

    /// Widget category for grouping.
    fn kind(&self) -> WidgetKind;

    /// How often this widget should refresh (in milliseconds).
    fn refresh_interval_ms(&self) -> u64 {
        1000
    }

    /// Update internal state. Called periodically based on `refresh_interval_ms`.
    fn tick(&mut self);

    /// Render the widget into the given area.
    fn render(&self, f: &mut Frame, area: Rect);

    /// Minimum width (in columns) this widget needs.
    fn min_width(&self) -> u16 {
        20
    }

    /// Minimum height (in rows) this widget needs.
    fn min_height(&self) -> u16 {
        5
    }

    /// Whether this widget can be focused for interaction.
    fn is_interactive(&self) -> bool {
        false
    }

    /// Handle input when this widget is focused. Returns true if consumed.
    fn handle_key(&mut self, _key: &crossterm::event::KeyEvent) -> bool {
        false
    }
}

/// A snapshot of widget state for the status bar or overlay.
#[derive(Debug, Clone)]
pub struct WidgetSnapshot {
    pub id: String,
    pub name: String,
    pub summary: String,
    pub detail: Option<String>,
    pub kind: WidgetKind,
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    struct TestWidget {
        id: String,
        tick_count: usize,
    }

    impl Widget for TestWidget {
        fn id(&self) -> &str {
            &self.id
        }
        fn name(&self) -> &str {
            "Test Widget"
        }
        fn kind(&self) -> WidgetKind {
            WidgetKind::System
        }
        fn tick(&mut self) {
            self.tick_count += 1;
        }
        fn render(&self, _f: &mut Frame, _area: Rect) {}
    }

    #[test]
    fn widget_trait_works() {
        let mut w = TestWidget {
            id: "test".into(),
            tick_count: 0,
        };
        assert_eq!(w.id(), "test");
        assert_eq!(w.name(), "Test Widget");
        assert_eq!(w.tick_count, 0);
        w.tick();
        assert_eq!(w.tick_count, 1);
    }

    #[test]
    fn widget_render_does_not_panic() {
        let w = TestWidget {
            id: "test".into(),
            tick_count: 0,
        };
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                w.render(f, Rect::new(0, 0, 80, 24));
            })
            .unwrap();
    }
}
