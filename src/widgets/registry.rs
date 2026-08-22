//! Widget registry — manages the collection of available and active widgets.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::{Widget, WidgetSnapshot};

/// Widget categories for grouping in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum WidgetKind {
    /// System monitoring: CPU, memory, disk, network, processes.
    System,
    /// Development: git status, build state, test results, dependency audit.
    Dev,
    /// Utilities: clock, calendar, notes, clipboard, quick actions.
    Utility,
    /// Custom user-defined widgets (shell commands, API calls, etc.).
    Custom,
}

impl WidgetKind {
    pub fn label(&self) -> &'static str {
        match self {
            WidgetKind::System => "System",
            WidgetKind::Dev => "Dev",
            WidgetKind::Utility => "Utility",
            WidgetKind::Custom => "Custom",
        }
    }
}

/// Metadata about a widget for display in the widget picker.
#[derive(Debug, Clone)]
pub struct WidgetMeta {
    pub id: String,
    pub name: String,
    pub kind: WidgetKind,
    pub description: String,
    pub min_width: u16,
    pub min_height: u16,
    pub refresh_ms: u64,
}

/// The widget registry holds all active widgets and manages their lifecycle.
pub struct WidgetRegistry {
    /// Active widgets, keyed by ID.
    widgets: HashMap<String, Box<dyn Widget>>,
    /// Last tick time per widget, for interval-based refresh.
    last_tick: HashMap<String, Instant>,
    /// Widget layout configuration.
    layout: super::layout::WidgetLayout,
    /// Focused widget ID (for interactive widgets).
    focused: Option<String>,
}

impl WidgetRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            widgets: HashMap::new(),
            last_tick: HashMap::new(),
            layout: super::layout::WidgetLayout::new(),
            focused: None,
        }
    }

    /// Register a widget. Replaces any existing widget with the same ID.
    pub fn register(&mut self, widget: Box<dyn Widget>) {
        let id = widget.id().to_string();
        self.last_tick.insert(id.clone(), Instant::now());
        self.widgets.insert(id, widget);
    }

    /// Unregister a widget by ID.
    pub fn unregister(&mut self, id: &str) -> Option<Box<dyn Widget>> {
        self.last_tick.remove(id);
        self.widgets.remove(id)
    }

    /// Get a reference to a widget by ID.
    pub fn get(&self, id: &str) -> Option<&dyn Widget> {
        self.widgets.get(id).map(|w| w.as_ref())
    }

    /// Get a mutable reference to a widget by ID.

    /// Tick all widgets whose refresh interval has elapsed.
    pub fn tick_all(&mut self) {
        let now = Instant::now();
        for (id, widget) in &mut self.widgets {
            let last = self.last_tick.get(id).copied().unwrap_or(now);
            let interval = Duration::from_millis(widget.refresh_interval_ms());
            if now.duration_since(last) >= interval {
                widget.tick();
                self.last_tick.insert(id.clone(), now);
            }
        }
    }

    /// Get snapshots of all widgets for status bar display.
    pub fn snapshots(&self) -> Vec<WidgetSnapshot> {
        self.widgets
            .values()
            .map(|w| WidgetSnapshot {
                id: w.id().to_string(),
                name: w.name().to_string(),
                summary: String::new(), // Widgets fill this in render
                detail: None,
                kind: w.kind(),
            })
            .collect()
    }

    /// List all registered widget metadata.
    pub fn list_meta(&self) -> Vec<WidgetMeta> {
        self.widgets
            .values()
            .map(|w| WidgetMeta {
                id: w.id().to_string(),
                name: w.name().to_string(),
                kind: w.kind(),
                description: String::new(),
                min_width: w.min_width(),
                min_height: w.min_height(),
                refresh_ms: w.refresh_interval_ms(),
            })
            .collect()
    }

    /// Get widgets filtered by kind.
    pub fn by_kind(&self, kind: WidgetKind) -> Vec<&dyn Widget> {
        self.widgets
            .values()
            .filter(|w| w.kind() == kind)
            .map(|w| w.as_ref())
            .collect()
    }

    /// Set the focused widget.
    pub fn set_focused(&mut self, id: Option<String>) {
        self.focused = id;
    }

    /// Get the focused widget ID.
    pub fn focused(&self) -> Option<&str> {
        self.focused.as_deref()
    }

    /// Get mutable access to the layout.
    pub fn layout_mut(&mut self) -> &mut super::layout::WidgetLayout {
        &mut self.layout
    }

    /// Get the layout.
    pub fn layout(&self) -> &super::layout::WidgetLayout {
        &self.layout
    }

    /// Number of registered widgets.
    pub fn len(&self) -> usize {
        self.widgets.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.widgets.is_empty()
    }

    /// IDs of all registered widgets.
    pub fn ids(&self) -> Vec<&str> {
        self.widgets.keys().map(|s| s.as_str()).collect()
    }
}

impl Default for WidgetRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;

    struct DummyWidget {
        id: String,
        kind: WidgetKind,
        tick_count: usize,
    }

    impl Widget for DummyWidget {
        fn id(&self) -> &str {
            &self.id
        }
        fn name(&self) -> &str {
            &self.id
        }
        fn kind(&self) -> WidgetKind {
            self.kind
        }
        fn tick(&mut self) {
            self.tick_count += 1;
        }
        fn render(&self, _f: &mut ratatui::Frame, _area: Rect) {}
    }

    fn make_widget(id: &str, kind: WidgetKind) -> Box<DummyWidget> {
        Box::new(DummyWidget {
            id: id.to_string(),
            kind,
            tick_count: 0,
        })
    }

    #[test]
    fn register_and_get() {
        let mut reg = WidgetRegistry::new();
        reg.register(make_widget("cpu", WidgetKind::System));
        assert!(reg.get("cpu").is_some());
        assert!(reg.get("mem").is_none());
    }

    #[test]
    fn unregister() {
        let mut reg = WidgetRegistry::new();
        reg.register(make_widget("cpu", WidgetKind::System));
        assert!(reg.unregister("cpu").is_some());
        assert!(reg.get("cpu").is_none());
    }

    #[test]
    fn by_kind_filter() {
        let mut reg = WidgetRegistry::new();
        reg.register(make_widget("cpu", WidgetKind::System));
        reg.register(make_widget("mem", WidgetKind::System));
        reg.register(make_widget("clock", WidgetKind::Utility));

        let sys = reg.by_kind(WidgetKind::System);
        assert_eq!(sys.len(), 2);
        let util = reg.by_kind(WidgetKind::Utility);
        assert_eq!(util.len(), 1);
    }

    #[test]
    fn len_and_is_empty() {
        let mut reg = WidgetRegistry::new();
        assert!(reg.is_empty());
        reg.register(make_widget("cpu", WidgetKind::System));
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn ids_returns_all() {
        let mut reg = WidgetRegistry::new();
        reg.register(make_widget("a", WidgetKind::System));
        reg.register(make_widget("b", WidgetKind::Dev));
        let mut ids = reg.ids();
        ids.sort();
        assert_eq!(ids, vec!["a", "b"]);
    }
}
