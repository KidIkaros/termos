//! Dock session control buttons — the icons at the bar's right-hand end.
//! Ported from Go TUIOS `internal/app/dock_session_buttons.go`.

use crate::config::constants;

/// The narrowest dock that carries the controls at all.
const DOCK_SESSION_ICON_MIN_WIDTH: usize = 34;

/// Names one of the dock's session controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockSessionAction {
    /// Close the session and every process in it.
    Close,
    /// Detach this client, leaving the session running.
    Detach,
    /// Rename the current session.
    Rename,
}

/// A hit rectangle for a session control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DockSessionHit {
    /// Inclusive left x.
    pub x0: i32,
    /// Exclusive right x.
    pub x1: i32,
    /// The dock row y.
    pub y: i32,
    /// Which control covers this rect.
    pub action: DockSessionAction,
}

/// A control's glyph, following the configured glyph set.
pub fn dock_session_icon(action: DockSessionAction, ascii_only: bool) -> &'static str {
    match action {
        DockSessionAction::Close => dock_icon_close_session(ascii_only),
        DockSessionAction::Detach => dock_icon_leave_running(ascii_only),
        DockSessionAction::Rename => dock_icon_rename(ascii_only),
    }
}

fn dock_icon_leave_running(ascii_only: bool) -> &'static str {
    if ascii_only { "d" } else { "\u{f08b}" }
}

fn dock_icon_close_session(ascii_only: bool) -> &'static str {
    if ascii_only { "X" } else { "\u{f011}" }
}

fn dock_icon_rename(ascii_only: bool) -> &'static str {
    if ascii_only { "R" } else { "\u{f02b}" }
}

/// Whether the dock is wide enough to carry the controls.
pub fn dock_session_controls_fit(render_width: usize) -> bool {
    render_width >= DOCK_SESSION_ICON_MIN_WIDTH
}

/// The ordered list of session control buttons and their icons.
pub fn dock_session_buttons(ascii_only: bool) -> Vec<(DockSessionAction, &'static str)> {
    let _ = constants::dock_separator(ascii_only);
    vec![
        (DockSessionAction::Rename, dock_session_icon(DockSessionAction::Rename, ascii_only)),
        (DockSessionAction::Detach, dock_session_icon(DockSessionAction::Detach, ascii_only)),
        (DockSessionAction::Close, dock_session_icon(DockSessionAction::Close, ascii_only)),
    ]
}

/// The hover label for a control.
pub fn dock_session_label(action: DockSessionAction) -> &'static str {
    match action {
        DockSessionAction::Detach => "Leave running",
        DockSessionAction::Close => "Close session",
        DockSessionAction::Rename => "Rename session",
    }
}

/// The total width the session controls strip occupies.
pub fn dock_session_strip_width(ascii_only: bool) -> usize {
    let buttons = dock_session_buttons(ascii_only);
    let mut w = 0;
    for (i, (_, icon)) in buttons.iter().enumerate() {
        if i > 0 { w += 1; }
        w += 2 + icon.chars().count();
    }
    w
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controls_fit_wide() {
        assert!(dock_session_controls_fit(40));
    }

    #[test]
    fn controls_fit_narrow() {
        assert!(!dock_session_controls_fit(20));
    }

    #[test]
    fn controls_fit_boundary() {
        assert!(dock_session_controls_fit(DOCK_SESSION_ICON_MIN_WIDTH));
        assert!(!dock_session_controls_fit(DOCK_SESSION_ICON_MIN_WIDTH - 1));
    }

    #[test]
    fn icon_close() {
        assert_eq!(dock_session_icon(DockSessionAction::Close, true), "X");
        assert_ne!(dock_session_icon(DockSessionAction::Close, false), "X");
    }

    #[test]
    fn icon_detach() {
        assert_eq!(dock_session_icon(DockSessionAction::Detach, true), "d");
        assert_ne!(dock_session_icon(DockSessionAction::Detach, false), "d");
    }

    #[test]
    fn icon_rename() {
        assert_eq!(dock_session_icon(DockSessionAction::Rename, true), "R");
        assert_ne!(dock_session_icon(DockSessionAction::Rename, false), "R");
    }

    #[test]
    fn buttons_returns_three() {
        let buttons = dock_session_buttons(true);
        assert_eq!(buttons.len(), 3);
    }

    #[test]
    fn buttons_contains_all_actions() {
        let buttons = dock_session_buttons(true);
        let actions: Vec<_> = buttons.iter().map(|(a, _)| *a).collect();
        assert!(actions.contains(&DockSessionAction::Close));
        assert!(actions.contains(&DockSessionAction::Detach));
        assert!(actions.contains(&DockSessionAction::Rename));
    }

    #[test]
    fn label_close() {
        assert_eq!(dock_session_label(DockSessionAction::Close), "Close session");
    }

    #[test]
    fn label_detach() {
        assert_eq!(dock_session_label(DockSessionAction::Detach), "Leave running");
    }

    #[test]
    fn label_rename() {
        assert_eq!(dock_session_label(DockSessionAction::Rename), "Rename session");
    }

    #[test]
    fn strip_width_nonzero() {
        assert!(dock_session_strip_width(true) > 0);
    }

    #[test]
    fn strip_width_ascii_vs_nerd() {
        assert!(dock_session_strip_width(true) > 0);
        assert!(dock_session_strip_width(false) > 0);
    }
}
