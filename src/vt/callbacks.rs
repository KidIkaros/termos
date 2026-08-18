//! VT emulator callbacks — ported from Go TUIOS `internal/vt/callbacks.go`.
//!
//! A set of optional callbacks fired by the emulator on specific events
//! (bell, title change, alt-screen switch, cursor changes, etc.).

/// RGB color triple.
pub type Rgb = (u8, u8, u8);

/// A callback that receives a string slice.
pub type StrCallback = Box<dyn Fn(&str) + Send + Sync>;

/// A callback that receives two position tuples.
pub type PositionCallback = Box<dyn Fn((i32, i32), (i32, i32)) + Send + Sync>;

/// Terminal cursor style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorStyle {
    Default,
    BlinkingBlock,
    SteadyBlock,
    BlinkingUnderline,
    SteadyUnderline,
    BlinkingBar,
    SteadyBar,
}

/// A set of optional callbacks for VT emulator events.
///
/// Each callback is `Option<Box<dyn Fn>>`, set via the builder methods.
/// Unset callbacks are no-ops when fired.
pub struct Callbacks {
    pub bell: Option<Box<dyn Fn() + Send + Sync>>,
    pub title: Option<StrCallback>,
    pub icon_name: Option<StrCallback>,
    pub alt_screen: Option<Box<dyn Fn(bool) + Send + Sync>>,
    pub cursor_position: Option<PositionCallback>,
    pub cursor_visibility: Option<Box<dyn Fn(bool) + Send + Sync>>,
    pub cursor_style: Option<Box<dyn Fn(CursorStyle, bool) + Send + Sync>>,
    pub cursor_color: Option<Box<dyn Fn(Option<Rgb>) + Send + Sync>>,
    pub mouse_mode_changed: Option<Box<dyn Fn() + Send + Sync>>,
    pub selection: Option<StrCallback>,
}

impl Callbacks {
    /// Create with all callbacks unset.
    pub fn new() -> Self {
        Self {
            bell: None,
            title: None,
            icon_name: None,
            alt_screen: None,
            cursor_position: None,
            cursor_visibility: None,
            cursor_style: None,
            cursor_color: None,
            mouse_mode_changed: None,
            selection: None,
        }
    }

    /// Set the bell callback.
    pub fn with_bell(mut self, f: impl Fn() + Send + Sync + 'static) -> Self {
        self.bell = Some(Box::new(f));
        self
    }

    /// Set the title callback.
    pub fn with_title(mut self, f: impl Fn(&str) + Send + Sync + 'static) -> Self {
        self.title = Some(Box::new(f));
        self
    }

    /// Set the icon name callback.
    pub fn with_icon_name(mut self, f: impl Fn(&str) + Send + Sync + 'static) -> Self {
        self.icon_name = Some(Box::new(f));
        self
    }

    /// Set the alt-screen callback.
    pub fn with_alt_screen(mut self, f: impl Fn(bool) + Send + Sync + 'static) -> Self {
        self.alt_screen = Some(Box::new(f));
        self
    }

    /// Set the cursor position callback.
    pub fn with_cursor_position(
        mut self,
        f: impl Fn((i32, i32), (i32, i32)) + Send + Sync + 'static,
    ) -> Self {
        self.cursor_position = Some(Box::new(f));
        self
    }

    /// Set the cursor visibility callback.
    pub fn with_cursor_visibility(mut self, f: impl Fn(bool) + Send + Sync + 'static) -> Self {
        self.cursor_visibility = Some(Box::new(f));
        self
    }

    /// Set the cursor style callback.
    pub fn with_cursor_style(
        mut self,
        f: impl Fn(CursorStyle, bool) + Send + Sync + 'static,
    ) -> Self {
        self.cursor_style = Some(Box::new(f));
        self
    }

    /// Set the cursor color callback.
    pub fn with_cursor_color(mut self, f: impl Fn(Option<Rgb>) + Send + Sync + 'static) -> Self {
        self.cursor_color = Some(Box::new(f));
        self
    }

    /// Set the mouse mode changed callback.
    pub fn with_mouse_mode_changed(mut self, f: impl Fn() + Send + Sync + 'static) -> Self {
        self.mouse_mode_changed = Some(Box::new(f));
        self
    }

    /// Set the selection (OSC 52 clipboard) callback.
    pub fn with_selection(mut self, f: impl Fn(&str) + Send + Sync + 'static) -> Self {
        self.selection = Some(Box::new(f));
        self
    }

    /// Fire the bell callback if set.
    pub fn fire_bell(&self) {
        if let Some(ref f) = self.bell {
            f();
        }
    }

    /// Fire the title callback if set.
    pub fn fire_title(&self, title: &str) {
        if let Some(ref f) = self.title {
            f(title);
        }
    }

    /// Fire the icon name callback if set.
    pub fn fire_icon_name(&self, name: &str) {
        if let Some(ref f) = self.icon_name {
            f(name);
        }
    }

    /// Fire the alt-screen callback if set.
    pub fn fire_alt_screen(&self, active: bool) {
        if let Some(ref f) = self.alt_screen {
            f(active);
        }
    }

    /// Fire the cursor position callback if set.
    pub fn fire_cursor_position(&self, old: (i32, i32), new: (i32, i32)) {
        if let Some(ref f) = self.cursor_position {
            f(old, new);
        }
    }

    /// Fire the cursor visibility callback if set.
    pub fn fire_cursor_visibility(&self, visible: bool) {
        if let Some(ref f) = self.cursor_visibility {
            f(visible);
        }
    }

    /// Fire the cursor style callback if set.
    pub fn fire_cursor_style(&self, style: CursorStyle, blinking: bool) {
        if let Some(ref f) = self.cursor_style {
            f(style, blinking);
        }
    }

    /// Fire the cursor color callback if set.
    pub fn fire_cursor_color(&self, color: Option<Rgb>) {
        if let Some(ref f) = self.cursor_color {
            f(color);
        }
    }

    /// Fire the mouse mode changed callback if set.
    pub fn fire_mouse_mode_changed(&self) {
        if let Some(ref f) = self.mouse_mode_changed {
            f();
        }
    }

    /// Fire the selection callback if set.
    pub fn fire_selection(&self, data: &str) {
        if let Some(ref f) = self.selection {
            f(data);
        }
    }
}

impl Default for Callbacks {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    use std::sync::Mutex;

    #[test]
    fn fire_bell_when_set() {
        let count = Arc::new(AtomicU32::new(0));
        let c = count.clone();
        let cb = Callbacks::new().with_bell(move || {
            c.fetch_add(1, Ordering::Relaxed);
        });
        cb.fire_bell();
        cb.fire_bell();
        assert_eq!(count.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn fire_bell_when_unset_is_noop() {
        let cb = Callbacks::new();
        cb.fire_bell();
    }

    #[test]
    fn fire_title() {
        let captured = Arc::new(Mutex::new(String::new()));
        let c = captured.clone();
        let cb = Callbacks::new().with_title(move |t| {
            *c.lock().unwrap() = t.to_string();
        });
        cb.fire_title("hello");
        assert_eq!(&*captured.lock().unwrap(), "hello");
    }

    #[test]
    fn fire_alt_screen() {
        let flag = Arc::new(AtomicU32::new(0));
        let f = flag.clone();
        let cb = Callbacks::new().with_alt_screen(move |active| {
            f.store(if active { 1 } else { 0 }, Ordering::Relaxed);
        });
        cb.fire_alt_screen(true);
        assert_eq!(flag.load(Ordering::Relaxed), 1);
        cb.fire_alt_screen(false);
        assert_eq!(flag.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn fire_cursor_color() {
        let captured = Arc::new(Mutex::new(None));
        let c = captured.clone();
        let cb = Callbacks::new().with_cursor_color(move |color| {
            *c.lock().unwrap() = color;
        });
        cb.fire_cursor_color(Some((255, 0, 0)));
        assert_eq!(*captured.lock().unwrap(), Some((255, 0, 0)));
        cb.fire_cursor_color(None);
        assert_eq!(*captured.lock().unwrap(), None);
    }
}
