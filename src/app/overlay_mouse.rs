//! Overlay mouse routing — ported from Go TUIOS `internal/app/overlay_mouse.go`.
//!
//! Routes mouse events to the topmost overlay panel, handling:
//! - Click-away dismissal
//! - Click-to-raise
//! - Drag initiation and motion
//! - Tab switching (TabPrev/TabNext/Tabs hit testing)
//! - Accent picker cell routing
//! - Row hover selection
//! - Row selection/activation
//! - Wheel scrolling

use crate::app::overlay_hit::{
    overlay_hit_at, overlay_hit_by_kind, raise_overlay, set_overlay_offset,
    OverlayPanelHit, OVERLAY_KIND_ORDER,
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

/// Route a mouse click to the topmost overlay. Returns (consumed, activated).
pub fn overlay_mouse_click(os: &mut Os, x: i32, y: i32, right: bool) -> (bool, bool) {
    // Inline palette hit-testing when overlay_hits isn't populated.
    if os.palette_open && !right {
        if let Some((px, py, pw, _ph, row_ys)) = os.palette_geometry() {
            if x >= px && x < px + pw && y >= py && y < py + 2 + row_ys.len() as i32 {
                // Click on a row — select and activate.
                for (i, &ry) in row_ys.iter().enumerate() {
                    if y == ry {
                        os.palette_selected = i;
                        os.activate_palette();
                        return (true, true);
                    }
                }
                // Click on query line or header — just consume.
                return (true, false);
            }
            // Click outside palette — dismiss.
            os.close_palette();
            return (true, false);
        }
    }

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

    // Right-click anywhere on the panel grabs it for dragging.
    if right {
        os.overlay_drag.start(&hit.kind, lx, ly);
        return (true, false);
    }

    // Left-click on a tab switches section. The strip's overflow arrows step
    // to the neighbouring section.
    if hit.geo.tab_prev.contains(lx, ly) {
        step_overlay_tab(os, &hit.kind, -1);
        return (true, false);
    }
    if hit.geo.tab_next.contains(lx, ly) {
        step_overlay_tab(os, &hit.kind, 1);
        return (true, false);
    }
    for (i, r) in hit.geo.tabs.iter().enumerate() {
        if r.contains(lx, ly) {
            set_overlay_tab(os, &hit.kind, i);
            return (true, false);
        }
    }

    // The accent picker is a field of cells rather than a list of rows, so it
    // routes off its own recorded geometry.
    if hit.kind == "accent" && accent_picker_press(os, &hit, lx, ly) {
        return (true, true);
    }

    // Left-click on a body row selects/activates it.
    for row in &hit.rows {
        if row.rect.contains(lx, ly) {
            return overlay_row_click(os, &hit.kind, row.idx, lx, ly);
        }
    }

    // Left-click on any other part of the panel (title, padding, footer, blank
    // space) grabs it for dragging.
    os.overlay_drag.start(&hit.kind, lx, ly);
    (true, false)
}

/// Route mouse motion to the overlay system. Returns true if consumed.
pub fn overlay_mouse_motion(os: &mut Os, x: i32, y: i32) -> bool {
    if os.overlay_drag.active {
        // Moving a dragged overlay — update its offset.
        let kind = os.overlay_drag.kind.clone();
        if let Some(h) = overlay_hit_by_kind(&os.overlay_hits, &kind) {
            let new_x = x - h.origin_x - os.overlay_drag.offset_x;
            let new_y = y - h.origin_y - os.overlay_drag.offset_y;
            set_overlay_offset(&mut os.overlay_offsets, &kind, new_x, new_y);
        }
        return true;
    }

    if os.overlay_hits.is_empty() {
        return false;
    }
    let hit = match overlay_hit_at(&os.overlay_hits, x, y) {
        Some(h) => h.clone(),
        None => return false,
    };
    let (lx, ly) = hit.to_local(x, y);

    // Accent picker: only a held button paints; bare hover does nothing.
    if hit.kind == "accent" {
        return true;
    }

    // Highlight the row under the cursor (selection only, no activation).
    for row in &hit.rows {
        if row.rect.contains(lx, ly) {
            overlay_row_hover(os, &hit.kind, row.idx);
            break;
        }
    }
    true
}

/// End any in-progress overlay drag.
pub fn overlay_mouse_release(os: &mut Os) {
    os.overlay_drag.end();
}

/// Route a mouse wheel event to the overlay under the cursor (falling back to
/// the topmost overlay). Returns true if consumed.
pub fn overlay_mouse_wheel(os: &mut Os, x: i32, y: i32, up: bool) -> bool {
    if !overlay_active(os) {
        return false;
    }

    // Find the overlay under the cursor, or fall back to the topmost.
    let hit = match overlay_hit_at(&os.overlay_hits, x, y) {
        Some(h) => h.clone(),
        None => {
            // Fall back to the topmost overlay.
            let top_kind = match os.overlay_z_order.last() {
                Some(k) => k.clone(),
                None => return false,
            };
            match overlay_hit_by_kind(&os.overlay_hits, &top_kind) {
                Some(h) => h.clone(),
                None => return false,
            }
        }
    };

    let delta = wheel_delta(up);

    match hit.kind.as_str() {
        "palette" => {
            os.palette_move(delta);
            true
        }
        "themepicker" => {
            os.theme_picker_move(delta);
            true
        }
        "switcher" => {
            os.switcher_move(delta);
            true
        }
        "accent" => {
            os.accent_picker_move(delta);
            true
        }
        _ => true, // Consume wheel over any overlay to prevent pane scrolling.
    }
}

/// Map a wheel direction to a selection delta (matches Go `wheelDelta`).
fn wheel_delta(up: bool) -> i32 {
    if up {
        -1
    } else {
        1
    }
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
        "settings" => {
            os.settings_selected = idx;
            (true, false)
        }
        "quit" => {
            if let Some(menu) = &mut os.quit_menu {
                menu.selected = idx;
            }
            (true, false)
        }
        "sessionclose" => {
            if let Some((ref mut _id, ref mut _label)) = os.session_close {
                // Selection within the session-close dialog.
            }
            (true, false)
        }
        _ => (true, false),
    }
}

/// Move an overlay's selection to the hovered row (selection only, no
/// activation). Mirrors Go `overlayRowHover`.
fn overlay_row_hover(os: &mut Os, kind: &str, idx: usize) {
    match kind {
        "palette" => os.palette_selected = idx,
        "themepicker" => {
            let current = os.theme_picker_selected;
            if idx != current {
                os.theme_picker_move(idx as i32 - current as i32);
            }
        }
        "switcher" => os.switcher_selected = idx,
        "settings" => os.settings_selected = idx,
        "quit" => {
            if let Some(menu) = &mut os.quit_menu {
                menu.selected = idx;
            }
        }
        _ => {}
    }
}

/// Switch the active section tab of an overlay. Mirrors Go `setOverlayTab`.
fn set_overlay_tab(os: &mut Os, kind: &str, _i: usize) {
    match kind {
        "help" => {
            // Help has categories; the Rust port stores the category index.
            // For now, just reset scroll when switching tabs.
            // (Full help-category state would require additional fields.)
        }
        "settings" => {
            os.settings_selected = 0;
        }
        _ => {}
    }
}

/// Move the overlay's active section by delta. Mirrors Go `stepOverlayTab`.
fn step_overlay_tab(os: &mut Os, kind: &str, delta: i32) {
    match kind {
        "settings" => {
            let len = os.settings_rows().len() as i32;
            if len > 0 {
                os.settings_selected =
                    (os.settings_selected as i32 + delta).rem_euclid(len) as usize;
            }
        }
        "help" => {
            // Help currently has one mode-aware section, so there is no
            // secondary tab to switch. Keep the event consumed rather than
            // letting it fall through to the pane beneath.
        }
        _ => {}
    }
}

/// Handle a click on the accent picker overlay. Returns true if handled.
fn accent_picker_press(os: &mut Os, hit: &OverlayPanelHit, lx: i32, ly: i32) -> bool {
    if !os.accent_picker_open {
        return false;
    }
    // Panel::render places body rows after top padding, title, and a blank
    // row. Match that geometry so a click chooses the swatch under the mouse
    // rather than applying whichever item was selected by the keyboard.
    let row = ly - 3;
    if row < 0 || row as usize >= os.accent_list.len() {
        return false;
    }
    if lx < 0 || lx >= hit.geo.width {
        return false;
    }
    os.accent_picker_selected = row as usize;
    os.apply_selected_accent();
    true
}

/// Close an overlay by kind, resetting the corresponding state field.
pub fn close_overlay(os: &mut Os, kind: &str) {
    match kind {
        "palette" => os.close_palette(),
        "switcher" => os.close_switcher(),
        "help" => os.help_open = false,
        "settings" => os.settings_open = false,
        "themepicker" => os.close_theme_picker(),
        "accent" => os.close_accent_picker(),
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
    // End drag if dragging this kind.
    if os.overlay_drag.kind == kind {
        os.overlay_drag.end();
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
    use crate::ui::overlay::{Geometry, Rect};

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
                title_bar: Rect::default(),
                tabs: vec![],
                tab_prev: Rect::default(),
                tab_next: Rect::default(),
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
        // Click on title bar (ly < 2) starts a drag.
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
    fn accent_click_selects_clicked_row() {
        let mut os = make_os();
        os.accent_picker_open = true;
        let hit = make_hit("accent", 10, 10, 40, 20, 100);
        assert!(accent_picker_press(&mut os, &hit, 5, 5));
        assert_eq!(os.config.appearance.border_focused_color.as_deref(), Some("green"));
        assert!(!os.accent_picker_open);
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
    fn wheel_delta_returns_correct_direction() {
        assert_eq!(wheel_delta(true), -1);
        assert_eq!(wheel_delta(false), 1);
    }

    #[test]
    fn overlay_row_hover_updates_palette_selection() {
        let mut os = make_os();
        os.palette_open = true;
        overlay_row_hover(&mut os, "palette", 3);
        assert_eq!(os.palette_selected, 3);
    }

    #[test]
    fn overlay_row_hover_updates_switcher_selection() {
        let mut os = make_os();
        os.switcher_open = true;
        overlay_row_hover(&mut os, "switcher", 2);
        assert_eq!(os.switcher_selected, 2);
    }

    #[test]
    fn close_overlay_ends_drag_for_same_kind() {
        let mut os = make_os();
        os.overlay_drag.active = true;
        os.overlay_drag.kind = "palette".into();
        os.palette_open = true;
        close_overlay(&mut os, "palette");
        assert!(!os.overlay_drag.active);
    }
}
