//! Overlay hit testing, z-ordering, drag state, and mouse routing — ported
//! from Go TUIOS `internal/app/overlay_hit.go` and `overlay_mouse.go`.
//!
//! The overlay system records the on-screen geometry of each floating panel
//! each frame so mouse events can be routed without re-deriving layout. This
//! module provides the hit-testing infrastructure, z-order management, drag
//! state, and the mouse routing entry points.
//!
//! The rendering primitives (Rect, Geometry, Palette, Dialog, Panel) live in
//! `crate::ui::overlay`. This module is the app-level glue that connects them
//! to the `Os` state model.

use crate::ui::overlay::{Geometry, Rect};

// ---------------------------------------------------------------------------
// Overlay row hit — one interactive body row
// ---------------------------------------------------------------------------

/// A single interactive body row of an overlay panel, in panel-relative
/// coordinates. Dec/Inc mark the left/right control hot-zones (cycler arrows
/// or a toggle) when the row has an adjustable value.
///
/// Ported from Go `overlayRowHit`.
#[derive(Debug, Clone, Default)]
pub struct OverlayRowHit {
    /// The row's panel-relative rectangle.
    pub rect: Rect,
    /// The row index in the panel's item list.
    pub idx: usize,
    /// The decrement hot-zone (left arrow / toggle off).
    pub dec: Rect,
    /// The increment hot-zone (right arrow / toggle on).
    pub inc: Rect,
}

impl OverlayRowHit {
    /// Create a row hit with just a rect and index.
    pub fn new(rect: Rect, idx: usize) -> Self {
        Self {
            rect,
            idx,
            dec: Rect::default(),
            inc: Rect::default(),
        }
    }

    /// Create a row hit with control hot-zones.
    pub fn with_controls(rect: Rect, idx: usize, dec: Rect, inc: Rect) -> Self {
        Self { rect, idx, dec, inc }
    }
}

// ---------------------------------------------------------------------------
// Overlay panel hit — recorded geometry of one panel
// ---------------------------------------------------------------------------

/// The on-screen geometry of one overlay panel, recorded each frame so mouse
/// events can be routed without re-deriving layout.
///
/// Ported from Go `overlayPanelHit`.
#[derive(Debug, Clone, Default)]
pub struct OverlayPanelHit {
    /// The panel kind: "settings", "help", "palette", "themepicker", etc.
    pub kind: String,
    /// The screen-space X origin of the panel.
    pub origin_x: i32,
    /// The screen-space Y origin of the panel.
    pub origin_y: i32,
    /// The z-index (higher = front).
    pub z: i32,
    /// The panel's interactive geometry (panel-relative).
    pub geo: Geometry,
    /// The panel's interactive body rows.
    pub rows: Vec<OverlayRowHit>,
}

impl OverlayPanelHit {
    /// Whether a screen-space point falls within this panel.
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.origin_x
            && x < self.origin_x + self.geo.width
            && y >= self.origin_y
            && y < self.origin_y + self.geo.height
    }

    /// Convert screen coordinates to panel-relative coordinates.
    pub fn to_local(&self, x: i32, y: i32) -> (i32, i32) {
        (x - self.origin_x, y - self.origin_y)
    }
}

// ---------------------------------------------------------------------------
// Z-order management
// ---------------------------------------------------------------------------

/// The deterministic order newly-opened overlays are added to the stack
/// (used only to break ties when several open in the same frame).
///
/// Ported from Go `overlayKindOrder`.
pub const OVERLAY_KIND_ORDER: &[&str] = &[
    "help",
    "palette",
    "session",
    "workspace",
    "layout",
    "aggregate",
    "settings",
    "themepicker",
    "accent",
    "quit",
    "sessionclose",
    "tapemanager",
    "scrollback",
];

/// The base z-index for overlay panels.
pub const Z_INDEX_OVERLAY_BASE: i32 = 100;

/// Reconcile the z-order stack: drop closed overlays and append newly-opened
/// ones on top, preserving the order of ones already open.
///
/// Ported from Go `reconcileOverlayZOrder`. Takes the current z-order and a
/// set of open kinds, returns the updated z-order.
pub fn reconcile_z_order(current: &[String], open: &[String]) -> Vec<String> {
    let mut open_set: std::collections::HashSet<&str> = open.iter().map(|s| s.as_str()).collect();
    let mut kept: Vec<String> = Vec::new();
    for k in current {
        if open_set.contains(k.as_str()) {
            kept.push(k.clone());
            open_set.remove(k.as_str());
        }
    }
    for &k in OVERLAY_KIND_ORDER {
        if open_set.contains(k) {
            kept.push(k.to_string());
        }
    }
    kept
}

/// The z-index for an overlay kind from its position in the stacking order.
///
/// Ported from Go `overlayZ`.
pub fn overlay_z(z_order: &[String], kind: &str) -> i32 {
    for (i, k) in z_order.iter().enumerate() {
        if k == kind {
            return Z_INDEX_OVERLAY_BASE + i as i32;
        }
    }
    Z_INDEX_OVERLAY_BASE
}

/// Move a kind to the top of the stacking order. Returns the updated order.
///
/// Ported from Go `raiseOverlay`.
pub fn raise_overlay(z_order: &[String], kind: &str) -> Vec<String> {
    let mut order: Vec<String> = z_order.to_vec();
    if let Some(idx) = order.iter().position(|k| k == kind) {
        if idx == order.len() - 1 {
            return order; // already on top
        }
        let k = order.remove(idx);
        order.push(k);
    }
    order
}

/// The kind of the frontmost open overlay, or empty string.
///
/// Ported from Go `topmostOverlayKind`.
pub fn topmost_overlay_kind(z_order: &[String]) -> &str {
    z_order.last().map(|s| s.as_str()).unwrap_or("")
}

// ---------------------------------------------------------------------------
// Overlay drag state
// ---------------------------------------------------------------------------

/// Tracks an in-progress overlay move.
///
/// Ported from Go `overlayDragState`.
#[derive(Debug, Clone, Default)]
pub struct OverlayDragState {
    /// Whether a drag is in progress.
    pub active: bool,
    /// Which overlay panel is being dragged.
    pub kind: String,
    /// Cursor X offset within the panel at grab time.
    pub offset_x: i32,
    /// Cursor Y offset within the panel at grab time.
    pub offset_y: i32,
}

impl OverlayDragState {
    /// Begin dragging the given panel, remembering the grab point.
    ///
    /// Ported from Go `startOverlayDrag`.
    pub fn start(&mut self, kind: &str, lx: i32, ly: i32) {
        self.active = true;
        self.kind = kind.to_string();
        self.offset_x = lx;
        self.offset_y = ly;
    }

    /// End any in-progress drag.
    ///
    /// Ported from Go `OverlayMouseRelease`.
    pub fn end(&mut self) {
        self.active = false;
        self.kind.clear();
        self.offset_x = 0;
        self.offset_y = 0;
    }
}

// ---------------------------------------------------------------------------
// Overlay offset storage
// ---------------------------------------------------------------------------

/// A map of overlay kind → drag displacement (x, y), stored as a Vec for
/// simple iteration. Zero displacement means centered.
pub type OverlayOffsets = Vec<(String, (i32, i32))>;

/// Get the drag displacement for an overlay kind (zero when unset).
///
/// Ported from Go `overlayOffset`.
pub fn overlay_offset(offsets: &OverlayOffsets, kind: &str) -> (i32, i32) {
    offsets
        .iter()
        .find(|(k, _)| k == kind)
        .map(|(_, v)| *v)
        .unwrap_or((0, 0))
}

/// Store the drag displacement for an overlay kind.
///
/// Ported from Go `setOverlayOffset`.
pub fn set_overlay_offset(offsets: &mut OverlayOffsets, kind: &str, x: i32, y: i32) {
    if let Some(entry) = offsets.iter_mut().find(|(k, _)| k == kind) {
        entry.1 = (x, y);
    } else {
        offsets.push((kind.to_string(), (x, y)));
    }
}

// ---------------------------------------------------------------------------
// Overlay origin computation
// ---------------------------------------------------------------------------

/// Compute the top-left screen cell that centers a w×h block on the screen.
///
/// Ported from Go `centerOrigin`.
pub fn center_origin(screen_w: i32, screen_h: i32, w: i32, h: i32) -> (i32, i32) {
    let x = ((screen_w - w) / 2).max(0);
    let y = ((screen_h - h) / 2).max(0);
    (x, y)
}

/// Compute the top-left screen cell for an overlay panel: centered, shifted
/// by the kind's drag offset, and clamped so the panel stays on screen.
///
/// Ported from Go `overlayOrigin`.
pub fn overlay_origin(
    screen_w: i32,
    screen_h: i32,
    geo: &Geometry,
    offsets: &OverlayOffsets,
    kind: &str,
) -> (i32, i32) {
    let (off_x, off_y) = overlay_offset(offsets, kind);
    let x = (screen_w - geo.width) / 2 + off_x;
    let y = (screen_h - geo.height) / 2 + off_y;
    let x = x.clamp(0, (screen_w - geo.width).max(0));
    let y = y.clamp(0, (screen_h - geo.height).max(0));
    (x, y)
}

// ---------------------------------------------------------------------------
// Hit testing
// ---------------------------------------------------------------------------

/// Find the highest-z overlay panel containing screen (x, y).
///
/// Ported from Go `overlayHitAt`.
pub fn overlay_hit_at(hits: &[OverlayPanelHit], x: i32, y: i32) -> Option<&OverlayPanelHit> {
    let mut best: Option<&OverlayPanelHit> = None;
    for h in hits {
        if h.contains(x, y) && (best.is_none() || h.z > best.unwrap().z) {
            best = Some(h);
        }
    }
    best
}

/// Find the recorded hit geometry for a specific kind.
///
/// Ported from Go `overlayHitByKind`.
pub fn overlay_hit_by_kind<'a>(
    hits: &'a [OverlayPanelHit],
    kind: &str,
) -> Option<&'a OverlayPanelHit> {
    hits.iter().find(|h| h.kind == kind)
}

// ---------------------------------------------------------------------------
// Panel fitting
// ---------------------------------------------------------------------------

/// The fewest body rows a scrolling panel is squeezed to.
pub const MIN_PANEL_ROWS: i32 = 3;

/// Rows an overlay panel always spends on chrome: top pad, title, blank, bottom pad.
pub const PANEL_CHROME_ROWS: i32 = 4;

/// Compute the inner content width for a panel, given the preferred width and
/// the screen width.
///
/// Ported from Go `panelWidth`.
pub fn panel_width(preferred: i32, screen_w: i32) -> i32 {
    crate::ui::overlay::fit_width(preferred, screen_w)
}

/// Compute how many scrolling body rows a panel can show.
///
/// Ported from Go `panelBodyRows`.
#[allow(clippy::too_many_arguments)]
pub fn panel_body_rows(
    preferred: i32,
    extra: i32,
    width: i32,
    has_tabs: bool,
    tab_count: i32,
    hint_rows: i32,
    screen_h: i32,
) -> i32 {
    if screen_h <= 0 {
        return preferred;
    }
    let chrome = panel_chrome(extra, width, has_tabs, tab_count, hint_rows);
    preferred.min(screen_h - chrome).max(MIN_PANEL_ROWS)
}

/// Every row of a panel that is not a body row.
///
/// Ported from Go `panelChrome`.
pub fn panel_chrome(
    extra: i32,
    _width: i32,
    has_tabs: bool,
    tab_count: i32,
    hint_rows: i32,
) -> i32 {
    let mut chrome = PANEL_CHROME_ROWS + extra;
    if has_tabs {
        chrome += tab_count + 2; // rule + blank
    }
    if hint_rows > 0 {
        chrome += 2 + hint_rows; // blank + rule + hints
    }
    chrome
}

/// Fit both the row count and the footer to the screen height. On a screen
/// too short to hold the minimum body and the footer both, the footer goes.
///
/// Ported from Go `panelBody`.
#[allow(clippy::too_many_arguments)]
pub fn panel_body(
    preferred: i32,
    extra: i32,
    width: i32,
    has_tabs: bool,
    tab_count: i32,
    hint_rows: i32,
    screen_h: i32,
) -> (i32, i32) {
    let rows = panel_body_rows(
        preferred,
        extra,
        width,
        has_tabs,
        tab_count,
        hint_rows,
        screen_h,
    );
    if screen_h <= 0 || hint_rows == 0 {
        return (rows, hint_rows);
    }
    let chrome = panel_chrome(extra, width, has_tabs, tab_count, hint_rows);
    if rows + chrome <= screen_h {
        (rows, hint_rows)
    } else {
        (
            panel_body_rows(preferred, extra, width, has_tabs, tab_count, 0, screen_h),
            0,
        )
    }
}

/// Clamp a scroll offset so a list of `count` items showing `visible` rows
/// keeps `selected` in view.
///
/// Ported from Go `scrollWindow`.
pub fn scroll_window(scroll: i32, selected: i32, count: i32, visible: i32) -> i32 {
    if count <= visible {
        return 0;
    }
    let max_scroll = count - visible;
    let mut s = scroll.clamp(0, max_scroll);
    if selected < s {
        s = selected;
    }
    if selected >= s + visible {
        s = selected - visible + 1;
    }
    s.clamp(0, max_scroll)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_hit(kind: &str, x: i32, y: i32, w: i32, h: i32, z: i32) -> OverlayPanelHit {
        OverlayPanelHit {
            kind: kind.to_string(),
            origin_x: x,
            origin_y: y,
            z,
            geo: Geometry {
                width: w,
                height: h,
                ..Default::default()
            },
            rows: Vec::new(),
        }
    }

    #[test]
    fn panel_hit_contains() {
        let h = make_hit("help", 10, 5, 30, 20, 100);
        assert!(h.contains(10, 5));
        assert!(h.contains(39, 24));
        assert!(!h.contains(40, 5));
        assert!(!h.contains(10, 25));
    }

    #[test]
    fn panel_hit_to_local() {
        let h = make_hit("help", 10, 5, 30, 20, 100);
        let (lx, ly) = h.to_local(15, 10);
        assert_eq!(lx, 5);
        assert_eq!(ly, 5);
    }

    #[test]
    fn overlay_hit_at_finds_topmost() {
        let hits = [
            make_hit("help", 0, 0, 80, 24, 100),
            make_hit("palette", 0, 0, 80, 24, 101),
        ];
        let found = overlay_hit_at(&hits, 10, 10).unwrap();
        assert_eq!(found.kind, "palette");
    }

    #[test]
    fn overlay_hit_at_none_outside() {
        let hits = [make_hit("help", 10, 10, 20, 10, 100)];
        assert!(overlay_hit_at(&hits, 0, 0).is_none());
    }

    #[test]
    fn hit_by_kind_finds_panel() {
        let hits = [
            make_hit("help", 0, 0, 80, 24, 100),
            make_hit("palette", 0, 0, 80, 24, 101),
        ];
        assert!(overlay_hit_by_kind(&hits, "palette").is_some());
        assert!(overlay_hit_by_kind(&hits, "missing").is_none());
    }

    #[test]
    fn reconcile_z_order_drops_closed() {
        let current = ["help".to_string(), "palette".to_string(), "quit".to_string()];
        let open = ["help".to_string(), "quit".to_string()];
        let result = reconcile_z_order(&current, &open);
        assert_eq!(result, vec!["help", "quit"]);
    }

    #[test]
    fn reconcile_z_order_appends_new() {
        let current = ["help".to_string()];
        let open = ["help".to_string(), "palette".to_string()];
        let result = reconcile_z_order(&current, &open);
        assert_eq!(result, vec!["help", "palette"]);
    }

    #[test]
    fn reconcile_z_order_preserves_existing() {
        let current = ["quit".to_string(), "help".to_string()];
        let open = ["help".to_string(), "quit".to_string()];
        let result = reconcile_z_order(&current, &open);
        // Existing order preserved.
        assert_eq!(result, vec!["quit", "help"]);
    }

    #[test]
    fn overlay_z_from_position() {
        let order = ["help".to_string(), "palette".to_string(), "quit".to_string()];
        assert_eq!(overlay_z(&order, "help"), Z_INDEX_OVERLAY_BASE);
        assert_eq!(overlay_z(&order, "palette"), Z_INDEX_OVERLAY_BASE + 1);
        assert_eq!(overlay_z(&order, "quit"), Z_INDEX_OVERLAY_BASE + 2);
        assert_eq!(overlay_z(&order, "missing"), Z_INDEX_OVERLAY_BASE);
    }

    #[test]
    fn raise_overlay_moves_to_top() {
        let order = ["help".to_string(), "palette".to_string(), "quit".to_string()];
        let raised = raise_overlay(&order, "help");
        assert_eq!(raised, vec!["palette", "quit", "help"]);
    }

    #[test]
    fn raise_overlay_already_on_top() {
        let order = ["help".to_string(), "palette".to_string()];
        let raised = raise_overlay(&order, "palette");
        assert_eq!(raised, vec!["help", "palette"]);
    }

    #[test]
    fn raise_overlay_not_found() {
        let order = ["help".to_string()];
        let raised = raise_overlay(&order, "missing");
        assert_eq!(raised, vec!["help"]);
    }

    #[test]
    fn topmost_overlay_kind_returns_last() {
        let order = ["help".to_string(), "palette".to_string()];
        assert_eq!(topmost_overlay_kind(&order), "palette");
    }

    #[test]
    fn topmost_overlay_kind_empty() {
        let order: Vec<String> = vec![];
        assert_eq!(topmost_overlay_kind(&order), "");
    }

    #[test]
    fn drag_state_start_and_end() {
        let mut drag = OverlayDragState::default();
        assert!(!drag.active);
        drag.start("help", 5, 3);
        assert!(drag.active);
        assert_eq!(drag.kind, "help");
        assert_eq!(drag.offset_x, 5);
        assert_eq!(drag.offset_y, 3);
        drag.end();
        assert!(!drag.active);
        assert!(drag.kind.is_empty());
    }

    #[test]
    fn overlay_offset_get_and_set() {
        let mut offsets = OverlayOffsets::new();
        assert_eq!(overlay_offset(&offsets, "help"), (0, 0));
        set_overlay_offset(&mut offsets, "help", 10, 20);
        assert_eq!(overlay_offset(&offsets, "help"), (10, 20));
        // Update existing.
        set_overlay_offset(&mut offsets, "help", 5, 5);
        assert_eq!(overlay_offset(&offsets, "help"), (5, 5));
    }

    #[test]
    fn center_origin_centers_block() {
        let (x, y) = center_origin(80, 24, 30, 10);
        assert_eq!(x, 25);
        assert_eq!(y, 7);
    }

    #[test]
    fn center_origin_clamps_to_zero() {
        let (x, y) = center_origin(10, 5, 30, 10);
        assert_eq!(x, 0);
        assert_eq!(y, 0);
    }

    #[test]
    fn overlay_origin_with_offset() {
        let geo = Geometry {
            width: 30,
            height: 10,
            ..Default::default()
        };
        let mut offsets = OverlayOffsets::new();
        set_overlay_offset(&mut offsets, "help", 5, -3);
        let (x, y) = overlay_origin(80, 24, &geo, &offsets, "help");
        // Center is (25, 7), offset (5, -3) → (30, 4), clamped.
        assert_eq!(x, 30);
        assert_eq!(y, 4);
    }

    #[test]
    fn overlay_origin_clamps_to_screen() {
        let geo = Geometry {
            width: 30,
            height: 10,
            ..Default::default()
        };
        let mut offsets = OverlayOffsets::new();
        set_overlay_offset(&mut offsets, "help", 100, 100);
        let (x, y) = overlay_origin(80, 24, &geo, &offsets, "help");
        // Clamped so panel stays on screen.
        assert_eq!(x, 50); // 80 - 30
        assert_eq!(y, 14); // 24 - 10
    }

    #[test]
    fn scroll_window_keeps_selected_in_view() {
        assert_eq!(scroll_window(0, 5, 20, 10), 0);
        assert_eq!(scroll_window(0, 15, 20, 10), 6);
        assert_eq!(scroll_window(10, 3, 20, 10), 3);
    }

    #[test]
    fn scroll_window_small_list() {
        assert_eq!(scroll_window(5, 2, 5, 10), 0);
    }

    #[test]
    fn panel_width_fits_to_screen() {
        assert_eq!(panel_width(40, 80), 40);
        assert_eq!(panel_width(80, 40), 36);
    }

    #[test]
    fn panel_body_rows_fits_to_screen() {
        // 24 rows screen, 4 chrome, no tabs/hints → 20 available.
        let rows = panel_body_rows(30, 0, 40, false, 0, 0, 24);
        assert_eq!(rows, 20);
    }

    #[test]
    fn panel_body_rows_min_clamp() {
        // Very small screen → clamped to MIN_PANEL_ROWS.
        let rows = panel_body_rows(30, 0, 40, false, 0, 0, 5);
        assert_eq!(rows, MIN_PANEL_ROWS);
    }

    #[test]
    fn panel_body_drops_hints_when_tight() {
        // Screen too small for both body and hints → hints dropped.
        let (rows, hints) = panel_body(30, 0, 40, false, 0, 3, 10);
        assert_eq!(hints, 0);
        assert!(rows >= MIN_PANEL_ROWS);
    }

    #[test]
    fn panel_body_keeps_hints_when_room() {
        let (rows, hints) = panel_body(10, 0, 40, false, 0, 2, 30);
        assert_eq!(hints, 2);
        assert!(rows >= MIN_PANEL_ROWS);
    }
}
