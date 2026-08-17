//! Image placement tracking — where each placed image lives on screen so it
//! can be re-placed when panes move, resize, scroll, or change workspace.
//!
//! Ported from TUIOS `internal/app/kitty_passthrough_placement.go`:
//! `Placement` is a (window_id, image_id, virtual_position, z_index) record;
//! `PlacementStore` is the per-window map of placements. The store does not
//! hold image bytes — those live on the host terminal — only the geometry
//! needed to emit a re-placement (`a=p`) command.

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

/// Per-window placement store. Keyed by guest image id so a re-transmit of
/// the same image replaces rather than duplicates.
#[derive(Debug, Default)]
pub struct PlacementStore {
    /// window_id -> (guest_image_id -> Placement)
    windows: HashMap<u32, HashMap<u32, Placement>>,
    /// guest_image_id -> host_image_id (the id remap; one per window).
    id_map: HashMap<u32, HashMap<u32, u32>>,
    /// The next host-side image id to allocate.
    next_host_id: u32,
}

impl PlacementStore {
    pub fn new() -> Self {
        Self {
            windows: HashMap::new(),
            id_map: HashMap::new(),
            next_host_id: 1,
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
    /// on first sight.
    pub fn map_id(&mut self, window_id: u32, guest_id: u32) -> u32 {
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

    /// Record or replace a placement for a (window, guest_image_id) pair.
    pub fn place(&mut self, window_id: u32, placement: Placement) {
        self.windows
            .entry(window_id)
            .or_default()
            .insert(placement.guest_image_id, placement);
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
    }

    /// Remove a single placement by guest image id.
    pub fn remove(&mut self, window_id: u32, guest_image_id: u32) -> Option<Placement> {
        self.windows
            .get_mut(&window_id)
            .and_then(|m| m.remove(&guest_image_id))
    }

    /// True if the window has any placements.
    pub fn has_placements(&self, window_id: u32) -> bool {
        self.windows
            .get(&window_id)
            .map(|m| !m.is_empty())
            .unwrap_or(false)
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
    }
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
}
