//! Overlay mouse routing — ported from Go TUIOS `internal/app/overlay_mouse.go`.
//!
//! Routes mouse events to the topmost overlay panel, handling:
//! - Click-away dismissal
//! - Click-to-raise
//! - Drag initiation and motion
//! - Tab switching
//! - Row selection/activation
//! - Wheel scrolling

use crate::app::overlay_hit::{
    overlay_hit_at, overlay_hit_by_kind, raise_overlay, set_overlay_offset,
    OVERLAY_KIND_ORDER,
};
use crate::app::Os;

/// Check whether any overlay is currently open and has recorded hit geometry.
pub fn overlay_active(os: &Os) -> bool {
    !os.overlay_hits.is_empty()
}

/// Check whether an overlay drag is in progress.
pub fn overlay_drag_active(os: &Os) -> bool {
    os.overlay_drag.active
}

/// Route a mouse click to the topmost overlay. Returns (consumed, should_activate).
pub fn overlay_mouse_click(os: &mut Os, x: i32, y: i32, right: bool) -> (bool, bool) {
    // Find the topmost overlay at this point.
    let hit = match overlay_hit_at(&os.overlay_hits, x, y) {
        Some(h) => h.clone(),
        None => {
            // Click outside any overlay — dismiss the topmost one (click-away).
            if let Some(top_kind) = os.overlay_z_order.last() {
                let kind = top_kind.clone();
                close_overlay(os, &kind);
                return (true, false);
            }
            return (false, false);
        }
    };

    // Click inside an overlay — raise it to the top.
    os.overlay_z_order = raise_overlay(&os.overlay_z_order, &hit.kind);

    let (lx, ly) = hit.to_local(x, y);

    if right {
        // Right-click on chrome starts a drag.
        if ly < 3 {
            os.overlay_drag.start(&hit.kind, lx, ly);
            return (true, false);
        }
        return (true, false);
    }

    // Left-click on chrome (title bar area) starts a drag.
    if ly < 2 {
        os.overlay_drag.start(&hit.kind, lx, ly);
        return (true, false);
    }

    // Check tab hits (if the panel has tabs, they're at the top of the body).
    if ly == 2 {
        // Tab area — consume the click.
        return (true, false);
    }

    // Check body row hits.
    for row in &hit.rows {
        if lx >= row.rect.x0
            && lx < row.rect.x1
            && ly >= row.rect.y0
            && ly < row.rect.y1
        {
            return overlay_row_click(os, &hit.kind, row.idx, lx, ly);
        }
    }

    // Click inside the panel but not on any interactive element.
    (true, false)
}

/// Route mouse motion to the overlay system. Returns true if consumed.
pub fn overlay_mouse_motion(os: &mut Os, x: i32, y: i32) -> bool {
    if os.overlay_drag.active {
        // Moving a dragged overlay — update its offset.
        let kind = os.overlay_drag.kind.clone();
        // Find the panel hit for this kind to get its origin.
        if let Some(h) = overlay_hit_by_kind(&os.overlay_hits, &kind) {
            let new_x = x - h.origin_x - os.overlay_drag.offset_x;
            let new_y = y - h.origin_y - os.overlay_drag.offset_y;
            set_overlay_offset(&mut os.overlay_offsets, &kind, new_x, new_y);
        }
        return true;
    }
    // Hover — consume if over a panel.
    overlay_hit_at(&os.overlay_hits, x, y).is_some()
}

/// End any in-progress overlay drag.
pub fn overlay_mouse_release(os: &mut Os) {
    os.overlay_drag.end();
}

/// Route a mouse wheel event to the topmost overlay. Returns true if consumed.
pub fn overlay_mouse_wheel(os: &mut Os, x: i32, y: i32, up: bool) -> bool {
    if !overlay_active(os) {
        return false;
    }
    // Only scroll if the cursor is over an overlay.
    if overlay_hit_at(&os.overlay_hits, x, y).is_none() {
        return false;
    }

    let delta = wheel_delta(up) as usize;

    // Route wheel to the topmost overlay that supports scrolling.
    let top_kind = match os.overlay_z_order.last() {
        Some(k) => k.clone(),
        None => return false,
    };

    match top_kind.as_str() {
        "palette" => {
            if up {
                os.palette_selected = os.palette_selected.saturating_sub(delta);
            } else {
                os.palette_selected = os.palette_selected.saturating_add(delta);
            }
            true
        }
        "themepicker" => {
            if up {
                os.theme_picker_selected = os.theme_picker_selected.saturating_sub(delta);
            } else {
                os.theme_picker_selected = os.theme_picker_selected.saturating_add(delta);
            }
            true
        }
        "switcher" => {
            if up {
                os.switcher_selected = os.switcher_selected.saturating_sub(delta);
            } else {
                os.switcher_selected = os.switcher_selected.saturating_add(delta);
            }
            true
        }
        _ => true, // Consume wheel over any overlay to prevent pane scrolling.
    }
}

/// Compute the scroll delta for a wheel event.
fn wheel_delta(_up: bool) -> i32 {
    1
}

/// Handle a click on a body row of an overlay panel.
fn overlay_row_click(
    os: &mut Os,
    kind: &str,
    idx: usize,
    _lx: i32,
    _ly: i32,
) -> (bool, bool) {
    match kind {
        "palette" => {
            os.palette_selected = idx;
            os.activate_palette();
            (true, true)
        }
        "themepicker" => {
            os.theme_picker_selected = idx;
            os.apply_selected_theme();
            (true, true)
        }
        "switcher" => {
            os.switcher_selected = idx;
            os.activate_switcher();
            (true, true)
        }
        _ => (true, false),
    }
}

/// Close an overlay by kind, resetting the corresponding state field.
pub fn close_overlay(os: &mut Os, kind: &str) {
    match kind {
        "palette" => os.palette_open = false,
        "switcher" => os.switcher_open = false,
        "help" => os.help_open = false,
        "settings" => os.settings_open = false,
        "themepicker" => os.theme_picker_open = false,
        "accent" => os.accent_picker_open = false,
        "debug" => os.debug_overlay_open = false,
        "quit" => {
            os.show_quit_confirmation = false;
            os.quit_menu = None;
        }
        "sessionclose" => os.session_close = None,
        "aggregate" => os.aggregate_open = false,
        "tapemanager" => os.tape_manager_open = false,
        "scrollback" => os.scrollback_mode = false,
        "rename" => os.rename_dialog = None,
        "projecttape" => os.project_tape_pending = None,
        _ => {}
    }
    os.overlay_z_order.retain(|k| k != kind);
}

/// Return the list of currently open overlay kinds.
pub fn open_overlay_kinds(os: &Os) -> Vec<String> {
    let mut kinds: Vec<String> = Vec::new();
    if os.palette_open { kinds.push("palette".into()); }
    if os.switcher_open { kinds.push("switcher".into()); }
    if os.help_open { kinds.push("help".into()); }
    if os.settings_open { kinds.push("settings".into()); }
    if os.theme_picker_open { kinds.push("themepicker".into()); }
    if os.accent_picker_open { kinds.push("accent".into()); }
    if os.debug_overlay_open { kinds.push("debug".into()); }
    if os.show_quit_confirmation || os.quit_menu.is_some() { kinds.push("quit".into()); }
    if os.session_close.is_some() { kinds.push("sessionclose".into()); }
    if os.aggregate_open { kinds.push("aggregate".into()); }
    if os.tape_manager_open { kinds.push("tapemanager".into()); }
    if os.scrollback_mode { kinds.push("scrollback".into()); }
    if os.rename_dialog.is_some() { kinds.push("rename".into()); }
    if os.project_tape_pending.is_some() { kinds.push("projecttape".into()); }
    kinds.sort_by_key(|k| {
        OVERLAY_KIND_ORDER.iter().position(|&o| o == k.as_str()).unwrap_or(usize::MAX)
    });
    kinds
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::overlay_hit::{OverlayDragState, OverlayPanelHit};
    use crate::ui::overlay::Geometry;

    fn make_os() -> Os {
        let mut os = Os::new(crate::config::userconfig::UserConfig::default());
        os.overlay_hits = vec![];
        os.overlay_z_order = vec![];
        os.overlay_offsets = vec![];
        os.overlay_drag = OverlayDragState::default();
        os
    }

    fn make_hit(kind: &str, x: i32, y: i32, w: i32, h: i32, z: i32) -> OverlayPanelHit {
        OverlayPanelHit {
            kind: kind.to_string(),
            origin_x: x,
            origin_y: y,
            z,
            geo: Geometry {
                width: w,
                height: h,
                title_bar: crate::ui::overlay::Rect::default(),
                tabs: vec![],
                tab_prev: crate::ui::overlay::Rect::default(),
                tab_next: crate::ui::overlay::Rect::default(),
                body_x: 0,
                body_y: 0,
                inner_width: w,
                hints: vec![],
            },
            rows: vec![],
        }
    }

    #[test]
    fn overlay_active_false_when_no_hits() {
        let os = make_os();
        assert!(!overlay_active(&os));
    }

    #[test]
    fn overlay_active_true_when_hits_present() {
        let mut os = make_os();
        os.overlay_hits = vec![make_hit("help", 0, 0, 40, 10, 100)];
        assert!(overlay_active(&os));
    }

    #[test]
    fn overlay_drag_active_false_by_default() {
        let os = make_os();
        assert!(!overlay_drag_active(&os));
    }

    #[test]
    fn overlay_drag_active_true_when_dragging() {
        let mut os = make_os();
        os.overlay_drag.active = true;
        assert!(overlay_drag_active(&os));
    }

    #[test]
    fn close_overlay_clears_state() {
        let mut os = make_os();
        os.palette_open = true;
        os.overlay_z_order = vec!["palette".into()];
        close_overlay(&mut os, "palette");
        assert!(!os.palette_open);
        assert!(os.overlay_z_order.is_empty());
    }

    #[test]
    fn close_overlay_unknown_kind_is_noop() {
        let mut os = make_os();
        close_overlay(&mut os, "nonexistent");
    }

    #[test]
    fn open_overlay_kinds_lists_open_overlays() {
        let mut os = make_os();
        os.palette_open = true;
        os.help_open = true;
        let kinds = open_overlay_kinds(&os);
        assert!(kinds.contains(&"palette".to_string()));
        assert!(kinds.contains(&"help".to_string()));
    }

    #[test]
    fn open_overlay_kinds_empty_when_none_open() {
        let os = make_os();
        assert!(open_overlay_kinds(&os).is_empty());
    }

    #[test]
    fn overlay_mouse_click_outside_dismisses_topmost() {
        let mut os = make_os();
        os.palette_open = true;
        os.overlay_z_order = vec!["palette".into()];
        os.overlay_hits = vec![make_hit("palette", 10, 10, 20, 5, 100)];
        let (consumed, _) = overlay_mouse_click(&mut os, 0, 0, false);
        assert!(consumed);
        assert!(!os.palette_open);
    }

    #[test]
    fn overlay_mouse_click_inside_raises_overlay() {
        let mut os = make_os();
        os.palette_open = true;
        os.help_open = true;
        os.overlay_z_order = vec!["palette".into(), "help".into()];
        os.overlay_hits = vec![
            make_hit("palette", 0, 0, 20, 5, 100),
            make_hit("help", 5, 5, 20, 5, 101),
        ];
        let (consumed, _) = overlay_mouse_click(&mut os, 10, 7, false);
        assert!(consumed);
        assert_eq!(os.overlay_z_order.last().map(|s| s.as_str()), Some("help"));
    }

    #[test]
    fn overlay_mouse_click_on_chrome_starts_drag() {
        let mut os = make_os();
        os.help_open = true;
        os.overlay_z_order = vec!["help".into()];
        os.overlay_hits = vec![make_hit("help", 10, 10, 30, 10, 100)];
        let (consumed, _) = overlay_mouse_click(&mut os, 15, 11, false);
        assert!(consumed);
        assert!(os.overlay_drag.active);
        assert_eq!(os.overlay_drag.kind, "help");
    }

    #[test]
    fn overlay_mouse_release_ends_drag() {
        let mut os = make_os();
        os.overlay_drag.active = true;
        os.overlay_drag.kind = "help".into();
        overlay_mouse_release(&mut os);
        assert!(!os.overlay_drag.active);
    }

    #[test]
    fn overlay_mouse_motion_when_not_dragging_consumes_over_panel() {
        let mut os = make_os();
        os.overlay_hits = vec![make_hit("help", 10, 10, 30, 10, 100)];
        assert!(overlay_mouse_motion(&mut os, 20, 15));
        assert!(!overlay_mouse_motion(&mut os, 0, 0));
    }

    #[test]
    fn overlay_mouse_wheel_consumed_over_overlay() {
        let mut os = make_os();
        os.palette_open = true;
        os.overlay_z_order = vec!["palette".into()];
        os.overlay_hits = vec![make_hit("palette", 10, 10, 30, 10, 100)];
        assert!(overlay_mouse_wheel(&mut os, 20, 15, true));
    }

    #[test]
    fn overlay_row_click_palette_selects() {
        let mut os = make_os();
        os.palette_open = true;
        let (consumed, activated) = overlay_row_click(&mut os, "palette", 0, 0, 0);
        assert!(consumed);
        assert!(activated);
    }

    #[test]
    fn overlay_row_click_themepicker_selects() {
        let mut os = make_os();
        os.theme_picker_open = true;
        let (consumed, activated) = overlay_row_click(&mut os, "themepicker", 0, 0, 0);
        assert!(consumed);
        assert!(activated);
    }

    #[test]
    fn overlay_row_click_switcher_selects() {
        let mut os = make_os();
        os.switcher_open = true;
        let (consumed, activated) = overlay_row_click(&mut os, "switcher", 0, 0, 0);
        assert!(consumed);
        assert!(activated);
    }

    #[test]
    fn overlay_row_click_unknown_kind_consumes() {
        let mut os = make_os();
        let (consumed, activated) = overlay_row_click(&mut os, "unknown", 0, 0, 0);
        assert!(consumed);
        assert!(!activated);
    }

    #[test]
    fn wheel_delta_returns_positive() {
        assert_eq!(wheel_delta(true), 1);
        assert_eq!(wheel_delta(false), 1);
    }
}
