//! Image placement tracking — where each placed image lives on screen so it
//! can be re-placed when panes move, resize, scroll, or change workspace.
//!
//! Ported from TUIOS `internal/app/kitty_passthrough_placement.go`:
//! `Placement` is a (window_id, image_id, virtual_position, z_index) record;
//! `PlacementStore` is the per-window map of placements. The store does not
//! hold image bytes — those live on the host terminal — only the geometry
//! needed to emit a re-placement (`a=p`) command.
//!
//! `PassthroughPlacement` is the richer record used by the forwarding path,
//! keyed by host image id (not guest id). Guest id 0 is kitty's "auto-assign"
//! sentinel — each transmit with id 0 is a distinct image, so it always gets
//! a fresh host id.

use std::collections::HashMap;

/// A single image placement on a pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    /// The host-side image id (after id remapping).
    pub host_image_id: u32,
    /// The guest-side image id (from the PTY's APC stream).
    pub guest_image_id: u32,
    /// Cell column (0-based) within the pane's inner rect.
    pub x: u32,
    /// Cell row (0-based) within the pane's inner rect.
    pub y: u32,
    /// Z-index (negative = below text, 0 = inline, positive = above).
    pub z: i32,
    /// The source rectangle within the image (in pixels). `(0,0,0,0)` means
    /// the whole image.
    pub source: (u32, u32, u32, u32),
    /// The placement's width/height in cells (0 = auto from image).
    pub cells: (u32, u32),
}

impl Placement {
    pub fn new(host_image_id: u32, guest_image_id: u32, x: u32, y: u32) -> Self {
        Self {
            host_image_id,
            guest_image_id,
            x,
            y,
            z: 0,
            source: (0, 0, 0, 0),
            cells: (0, 0),
        }
    }
}

/// Window position info for placement refresh. Ported from Go's
/// `WindowPositionInfo`. Carries the geometry and scroll state a placement
/// needs to compute its absolute host position and visibility.
#[derive(Debug, Clone, Default)]
pub struct WindowPositionInfo {
    pub window_x: i32,
    pub window_y: i32,
    pub content_offset_x: i32,
    pub content_offset_y: i32,
    pub width: i32,
    pub height: i32,
    pub visible: bool,
    /// Total scrollback lines.
    pub scrollback_len: i32,
    /// Current scroll offset (0 = at bottom).
    pub scroll_offset: i32,
    pub is_being_manipulated: bool,
    pub screen_width: i32,
    pub screen_height: i32,
    pub window_z: i32,
    pub is_alt_screen: bool,
}

/// A rich placement record for kitty passthrough, keyed by host image id.
/// Ported from Go's `PassthroughPlacement`. Unlike the simple `Placement`,
/// this tracks absolute scrollback line, host coordinates, clipping state,
/// visibility, and native pixel dimensions for source-rect cropping.
#[derive(Debug, Clone)]
pub struct PassthroughPlacement {
    pub guest_image_id: u32,
    pub host_image_id: u32,
    pub placement_id: u32,
    pub window_id: u32,
    /// Column position in the guest terminal at placement time.
    pub guest_x: i32,
    /// Absolute scrollback line (scrollback_len + cursor_y at placement).
    pub absolute_line: i32,
    /// Host terminal absolute cell coordinates.
    pub host_x: i32,
    pub host_y: i32,
    /// Image dimensions in cells (original, before clipping).
    pub cols: i32,
    pub rows: i32,
    /// Capped display rows for initial display.
    pub display_rows: i32,
    /// Source rectangle within the image (pixels).
    pub source_x: i32,
    pub source_y: i32,
    pub source_width: i32,
    pub source_height: i32,
    /// Cell offsets within the placement cell.
    pub x_offset: i32,
    pub y_offset: i32,
    pub z_index: i32,
    pub virtual_placement: bool,
    /// Whether the placement is currently hidden (offscreen/occluded).
    pub hidden: bool,
    /// Whether this placement is still receiving chunked data.
    pub streaming: bool,
    /// True if placed while alternate screen was active.
    pub placed_on_alt_screen: bool,
    /// Native pixel dimensions from the s/v transmit params.
    pub image_pixel_width: i32,
    pub image_pixel_height: i32,
    /// Current clipping state (rows/cols clipped from each edge).
    pub clip_top: i32,
    pub clip_bottom: i32,
    pub clip_left: i32,
    pub clip_right: i32,
    /// Max rows/cols showable in current viewport.
    pub max_showable: i32,
    pub max_showable_cols: i32,
}

impl PassthroughPlacement {
    /// Create a new passthrough placement with the given host/guest ids and
    /// initial geometry. All clipping/visibility fields start at zero/false.
    pub fn new(guest_image_id: u32, host_image_id: u32, window_id: u32) -> Self {
        Self {
            guest_image_id,
            host_image_id,
            placement_id: 0,
            window_id,
            guest_x: 0,
            absolute_line: 0,
            host_x: 0,
            host_y: 0,
            cols: 0,
            rows: 0,
            display_rows: 0,
            source_x: 0,
            source_y: 0,
            source_width: 0,
            source_height: 0,
            x_offset: 0,
            y_offset: 0,
            z_index: 0,
            virtual_placement: false,
            hidden: true,
            streaming: false,
            placed_on_alt_screen: false,
            image_pixel_width: 0,
            image_pixel_height: 0,
            clip_top: 0,
            clip_bottom: 0,
            clip_left: 0,
            clip_right: 0,
            max_showable: 0,
            max_showable_cols: 0,
        }
    }
}

/// Per-window placement store. Keyed by guest image id so a re-transmit of
/// the same image replaces rather than duplicates. Also holds a separate
/// host-id-keyed map of `PassthroughPlacement` records for the forwarding
/// path, and the guest→host id remap.
#[derive(Debug, Default)]
pub struct PlacementStore {
    /// window_id -> (guest_image_id -> Placement)
    windows: HashMap<u32, HashMap<u32, Placement>>,
    /// guest_image_id -> host_image_id (the id remap; one per window).
    id_map: HashMap<u32, HashMap<u32, u32>>,
    /// The next host-side image id to allocate.
    next_host_id: u32,
    /// window_id -> (host_image_id -> PassthroughPlacement) for the forwarding
    /// path. Keyed by host id so delete-by-id and refresh can find placements
    /// without a guest→host lookup.
    passthrough: HashMap<u32, HashMap<u32, PassthroughPlacement>>,
}

impl PlacementStore {
    pub fn new() -> Self {
        Self {
            windows: HashMap::new(),
            id_map: HashMap::new(),
            next_host_id: 1,
            passthrough: HashMap::new(),
        }
    }

    /// Allocate a fresh host image id.
    pub fn allocate_host_id(&mut self) -> u32 {
        let id = self.next_host_id;
        self.next_host_id += 1;
        if self.next_host_id == 0 {
            self.next_host_id = 1;
        }
        id
    }

    /// Map a guest image id to a host image id for a window, allocating one
    /// on first sight. Guest id 0 is kitty's auto-assign sentinel — each
    /// call with guest_id 0 allocates a fresh host id (distinct image).
    pub fn map_id(&mut self, window_id: u32, guest_id: u32) -> u32 {
        if guest_id == 0 {
            return self.allocate_host_id();
        }
        if let Some(&id) = self.id_map.get(&window_id).and_then(|m| m.get(&guest_id)) {
            return id;
        }
        let id = self.next_host_id;
        self.next_host_id += 1;
        if self.next_host_id == 0 {
            self.next_host_id = 1;
        }
        self.id_map
            .entry(window_id)
            .or_default()
            .insert(guest_id, id);
        id
    }

    /// Get or allocate a host id for a (window, guest_id) pair. Returns
    /// `(host_id, is_new)` where `is_new` indicates a fresh allocation.
    /// Guest id 0 always allocates fresh.
    pub fn get_or_allocate(&mut self, window_id: u32, guest_id: u32) -> (u32, bool) {
        if guest_id == 0 {
            return (self.allocate_host_id(), true);
        }
        if let Some(&id) = self.id_map.get(&window_id).and_then(|m| m.get(&guest_id)) {
            return (id, false);
        }
        let id = self.allocate_host_id();
        self.id_map
            .entry(window_id)
            .or_default()
            .insert(guest_id, id);
        (id, true)
    }

    /// Look up the host id for a (window, guest_id) pair without allocating.
    pub fn lookup_host_id(&self, window_id: u32, guest_id: u32) -> Option<u32> {
        self.id_map
            .get(&window_id)
            .and_then(|m| m.get(&guest_id))
            .copied()
    }

    /// Record or replace a placement for a (window, guest_image_id) pair.
    pub fn place(&mut self, window_id: u32, placement: Placement) {
        self.windows
            .entry(window_id)
            .or_default()
            .insert(placement.guest_image_id, placement);
    }

    /// Record or replace a passthrough placement keyed by host image id.
    pub fn place_passthrough(&mut self, window_id: u32, placement: PassthroughPlacement) {
        self.passthrough
            .entry(window_id)
            .or_default()
            .insert(placement.host_image_id, placement);
    }

    /// Get a passthrough placement by host image id.
    pub fn passthrough_get(
        &self,
        window_id: u32,
        host_image_id: u32,
    ) -> Option<&PassthroughPlacement> {
        self.passthrough
            .get(&window_id)
            .and_then(|m| m.get(&host_image_id))
    }

    /// Get a mutable passthrough placement by host image id.
    pub fn passthrough_get_mut(
        &mut self,
        window_id: u32,
        host_image_id: u32,
    ) -> Option<&mut PassthroughPlacement> {
        self.passthrough
            .get_mut(&window_id)
            .and_then(|m| m.get_mut(&host_image_id))
    }

    /// Remove a passthrough placement by host image id.
    pub fn passthrough_remove(
        &mut self,
        window_id: u32,
        host_image_id: u32,
    ) -> Option<PassthroughPlacement> {
        self.passthrough
            .get_mut(&window_id)
            .and_then(|m| m.remove(&host_image_id))
    }

    /// All passthrough placements for a window.
    pub fn passthrough_for(&self, window_id: u32) -> Vec<&PassthroughPlacement> {
        self.passthrough
            .get(&window_id)
            .map(|m| m.values().collect())
            .unwrap_or_default()
    }

    /// All passthrough placements for a window (mutable).
    pub fn passthrough_for_mut(
        &mut self,
        window_id: u32,
    ) -> Vec<&mut PassthroughPlacement> {
        self.passthrough
            .get_mut(&window_id)
            .map(|m| m.values_mut().collect())
            .unwrap_or_default()
    }

    /// All window ids that have passthrough placements.
    pub fn passthrough_window_ids(&self) -> Vec<u32> {
        self.passthrough.keys().copied().collect()
    }

    /// Remove a guest→host id mapping for a window.
    pub fn remove_id_mapping(&mut self, window_id: u32, guest_id: u32) {
        if let Some(m) = self.id_map.get_mut(&window_id) {
            m.remove(&guest_id);
        }
    }

    /// All placements for a window, in insertion order.
    pub fn placements_for(&self, window_id: u32) -> Vec<&Placement> {
        self.windows
            .get(&window_id)
            .map(|m| m.values().collect())
            .unwrap_or_default()
    }

    /// Remove all placements for a window (e.g. on close).
    pub fn clear_window(&mut self, window_id: u32) {
        self.windows.remove(&window_id);
        self.id_map.remove(&window_id);
        self.passthrough.remove(&window_id);
    }

    /// Remove a single placement by guest image id.
    pub fn remove(&mut self, window_id: u32, guest_image_id: u32) -> Option<Placement> {
        self.windows
            .get_mut(&window_id)
            .and_then(|m| m.remove(&guest_image_id))
    }

    /// True if the window has any placements (simple or passthrough).
    pub fn has_placements(&self, window_id: u32) -> bool {
        let simple = self
            .windows
            .get(&window_id)
            .map(|m| !m.is_empty())
            .unwrap_or(false);
        let pass = self
            .passthrough
            .get(&window_id)
            .map(|m| !m.is_empty())
            .unwrap_or(false);
        simple || pass
    }

    /// True if any window has passthrough placements.
    pub fn has_any_passthrough(&self) -> bool {
        self.passthrough.values().any(|m| !m.is_empty())
    }

    /// The total number of placements across all windows.
    pub fn len(&self) -> usize {
        self.windows.values().map(|m| m.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Clear everything (e.g. on workspace switch or host clear).
    pub fn clear_all(&mut self) {
        self.windows.clear();
        self.id_map.clear();
        self.passthrough.clear();
    }
}

/// Check if two rectangles overlap. Ported from Go's `rectsOverlap`.
#[allow(clippy::too_many_arguments)]
pub fn rects_overlap(
    x1: i32,
    y1: i32,
    w1: i32,
    h1: i32,
    x2: i32,
    y2: i32,
    w2: i32,
    h2: i32,
) -> bool {
    x1 < x2 + w2 && x1 + w1 > x2 && y1 < y2 + h2 && y1 + h1 > y2
}

/// Check if an image region is fully occluded by a window with higher z-index.
/// Ported from Go's `isOccludedByHigherWindow`.
pub fn is_occluded_by_higher_window(
    screen_x: i32,
    screen_y: i32,
    width: i32,
    height: i32,
    window_z: i32,
    all_windows: &HashMap<u32, WindowPositionInfo>,
    exclude_window_id: u32,
) -> bool {
    for (id, info) in all_windows {
        if *id == exclude_window_id || !info.visible || info.window_z <= window_z {
            continue;
        }
        if rects_overlap(
            screen_x,
            screen_y,
            width,
            height,
            info.window_x,
            info.window_y,
            info.width,
            info.height,
        ) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_id_allocates_sequentially() {
        let mut store = PlacementStore::new();
        assert_eq!(store.map_id(1, 10), 1);
        assert_eq!(store.map_id(1, 20), 2);
        // Same guest id returns the same host id.
        assert_eq!(store.map_id(1, 10), 1);
        // Different window has its own map but shares the id counter.
        assert_eq!(store.map_id(2, 10), 3);
    }

    #[test]
    fn place_and_retrieve() {
        let mut store = PlacementStore::new();
        let p = Placement::new(1, 10, 5, 3);
        store.place(1, p.clone());
        let placements = store.placements_for(1);
        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0], &p);
        assert!(store.has_placements(1));
        assert!(!store.has_placements(2));
    }

    #[test]
    fn replace_on_re_place() {
        let mut store = PlacementStore::new();
        store.place(1, Placement::new(1, 10, 0, 0));
        store.place(1, Placement::new(1, 10, 5, 5));
        assert_eq!(store.placements_for(1).len(), 1);
        assert_eq!(store.placements_for(1)[0].x, 5);
    }

    #[test]
    fn clear_window_removes_everything() {
        let mut store = PlacementStore::new();
        store.place(1, Placement::new(1, 10, 0, 0));
        store.place(1, Placement::new(2, 20, 1, 1));
        store.place(2, Placement::new(3, 30, 2, 2));
        store.clear_window(1);
        assert!(!store.has_placements(1));
        assert!(store.has_placements(2));
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn remove_single_placement() {
        let mut store = PlacementStore::new();
        store.place(1, Placement::new(1, 10, 0, 0));
        assert!(store.remove(1, 10).is_some());
        assert!(!store.has_placements(1));
        assert!(store.remove(1, 10).is_none());
    }

    #[test]
    fn clear_all_empties() {
        let mut store = PlacementStore::new();
        store.place(1, Placement::new(1, 10, 0, 0));
        store.place(2, Placement::new(2, 20, 1, 1));
        store.clear_all();
        assert!(store.is_empty());
    }

    #[test]
    fn guest_id_zero_always_allocates() {
        let mut store = PlacementStore::new();
        let h1 = store.map_id(1, 0);
        let h2 = store.map_id(1, 0);
        assert_ne!(h1, h2, "guest id 0 must always get a fresh host id");
    }

    #[test]
    fn get_or_allocate_reuses_nonzero() {
        let mut store = PlacementStore::new();
        let (id1, new1) = store.get_or_allocate(1, 5);
        assert!(new1);
        let (id2, new2) = store.get_or_allocate(1, 5);
        assert!(!new2);
        assert_eq!(id1, id2);
    }

    #[test]
    fn get_or_allocate_zero_is_new() {
        let mut store = PlacementStore::new();
        let (_, new1) = store.get_or_allocate(1, 0);
        let (_, new2) = store.get_or_allocate(1, 0);
        assert!(new1);
        assert!(new2);
    }

    #[test]
    fn passthrough_place_and_get() {
        let mut store = PlacementStore::new();
        let p = PassthroughPlacement::new(10, 1, 1);
        store.place_passthrough(1, p.clone());
        assert!(store.has_placements(1));
        let got = store.passthrough_get(1, 1).unwrap();
        assert_eq!(got.guest_image_id, 10);
    }

    #[test]
    fn passthrough_remove() {
        let mut store = PlacementStore::new();
        store.place_passthrough(1, PassthroughPlacement::new(10, 1, 1));
        assert!(store.passthrough_remove(1, 1).is_some());
        assert!(!store.has_placements(1));
    }

    #[test]
    fn passthrough_clear_window() {
        let mut store = PlacementStore::new();
        store.place_passthrough(1, PassthroughPlacement::new(10, 1, 1));
        store.place_passthrough(1, PassthroughPlacement::new(20, 2, 1));
        store.clear_window(1);
        assert!(!store.has_placements(1));
    }

    #[test]
    fn passthrough_has_any() {
        let mut store = PlacementStore::new();
        assert!(!store.has_any_passthrough());
        store.place_passthrough(1, PassthroughPlacement::new(10, 1, 1));
        assert!(store.has_any_passthrough());
    }

    #[test]
    fn rects_overlap_basic() {
        assert!(rects_overlap(0, 0, 10, 10, 5, 5, 10, 10));
        assert!(!rects_overlap(0, 0, 10, 10, 20, 20, 10, 10));
        // Edge touching is not overlap.
        assert!(!rects_overlap(0, 0, 10, 10, 10, 0, 10, 10));
    }

    #[test]
    fn occlusion_detects_higher_z() {
        let mut windows = HashMap::new();
        windows.insert(
            2,
            WindowPositionInfo {
                window_x: 0,
                window_y: 0,
                width: 20,
                height: 20,
                visible: true,
                window_z: 10,
                ..Default::default()
            },
        );
        assert!(is_occluded_by_higher_window(
            0, 0, 10, 10, 5, &windows, 1
        ));
    }

    #[test]
    fn occlusion_ignores_lower_z() {
        let mut windows = HashMap::new();
        windows.insert(
            2,
            WindowPositionInfo {
                window_x: 0,
                window_y: 0,
                width: 20,
                height: 20,
                visible: true,
                window_z: 1,
                ..Default::default()
            },
        );
        assert!(!is_occluded_by_higher_window(
            0, 0, 10, 10, 5, &windows, 1
        ));
    }

    #[test]
    fn occlusion_ignores_invisible() {
        let mut windows = HashMap::new();
        windows.insert(
            2,
            WindowPositionInfo {
                window_x: 0,
                window_y: 0,
                width: 20,
                height: 20,
                visible: false,
                window_z: 10,
                ..Default::default()
            },
        );
        assert!(!is_occluded_by_higher_window(
            0, 0, 10, 10, 5, &windows, 1
        ));
    }
}
