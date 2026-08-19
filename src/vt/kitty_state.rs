//! Kitty graphics state management — ported from Go TUIOS
//! `internal/vt/kitty_state.go`.
//!
//! Tracks images and placements for the Kitty graphics protocol.
//! Thread-safe via `Mutex` for cross-thread access from PTY reader and
//! render threads.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

/// Maximum total accumulated chunk data for a single kitty transmission.
const MAX_KITTY_TRANSMIT_BYTES: usize = 64 * 1024 * 1024;

/// Pixel format of a kitty image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KittyFormat {
    #[default]
    Rgba,
    Rgb,
    Png,
}

/// Compression mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KittyCompression {
    #[default]
    None,
    Zlib,
}

/// A stored kitty image.
#[derive(Debug, Clone)]
pub struct KittyImage {
    pub id: u32,
    pub number: u32,
    pub width: i32,
    pub height: i32,
    pub format: KittyFormat,
    pub compression: KittyCompression,
    pub data: Vec<u8>,
    pub transmit_time: Instant,
    /// Animation group this image belongs to (0 = not animated).
    pub animation_group: u32,
}

/// An animation group: a sequence of images played as frames.
#[derive(Debug, Clone)]
pub struct AnimationGroup {
    pub group_id: u32,
    /// Image IDs in frame order.
    pub frames: Vec<u32>,
    /// Current frame index.
    pub current_frame: usize,
    /// Whether the animation is playing.
    pub playing: bool,
    /// Frame delay in milliseconds (0 = default).
    pub delay_ms: u32,
    /// Total duration in milliseconds (0 = loop forever).
    pub duration_ms: u32,
    /// Whether the animation loops.
    pub looping: bool,
}

/// A placement of an image on the screen.
#[derive(Debug, Clone)]
pub struct KittyPlacement {
    pub image_id: u32,
    pub placement_id: u32,
    pub screen_x: i32,
    pub screen_y: i32,
    pub absolute_line: i32,
    pub x_offset: i32,
    pub y_offset: i32,
    pub source_x: i32,
    pub source_y: i32,
    pub source_width: i32,
    pub source_height: i32,
    pub columns: i32,
    pub rows: i32,
    pub z_index: i32,
    pub cursor_move: i32,
    pub virtual_placement: bool,
}

/// In-progress chunked transmission.
#[derive(Debug)]
pub struct KittyPendingChunk {
    pub image_id: u32,
    pub image_number: u32,
    pub format: KittyFormat,
    pub compression: KittyCompression,
    pub width: i32,
    pub height: i32,
    pub data_buffer: Vec<u8>,
}

/// Thread-safe kitty graphics state.
pub struct KittyState {
    inner: Mutex<KittyStateInner>,
}

impl std::fmt::Debug for KittyState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.inner.lock().unwrap();
        f.debug_struct("KittyState")
            .field("images", &inner.images.len())
            .field("placements", &inner.placements.len())
            .field("next_id", &inner.next_id)
            .field("has_pending", &inner.pending.is_some())
            .finish()
    }
}

struct KittyStateInner {
    images: HashMap<u32, KittyImage>,
    images_by_num: HashMap<u32, u32>,
    placements: Vec<KittyPlacement>,
    next_id: u32,
    pending: Option<KittyPendingChunk>,
    /// Animation groups: group_id → group state.
    animation_groups: HashMap<u32, AnimationGroup>,
    next_group_id: u32,
}

impl KittyState {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(KittyStateInner {
                images: HashMap::new(),
                images_by_num: HashMap::new(),
                placements: Vec::new(),
                next_id: 1,
                pending: None,
                animation_groups: HashMap::new(),
                next_group_id: 1,
            }),
        }
    }

    pub fn allocate_id(&self) -> u32 {
        let mut inner = self.inner.lock().unwrap();
        let id = inner.next_id;
        inner.next_id += 1;
        if inner.next_id == 0 {
            inner.next_id = 1;
        }
        id
    }

    pub fn add_image(&self, img: KittyImage) {
        let mut inner = self.inner.lock().unwrap();
        if img.number > 0 {
            inner.images_by_num.insert(img.number, img.id);
        }
        inner.images.insert(img.id, img);
    }

    pub fn get_image(&self, id: u32) -> Option<KittyImage> {
        self.inner.lock().unwrap().images.get(&id).cloned()
    }

    pub fn get_image_by_number(&self, num: u32) -> Option<KittyImage> {
        let inner = self.inner.lock().unwrap();
        inner
            .images_by_num
            .get(&num)
            .and_then(|id| inner.images.get(id).cloned())
    }

    pub fn delete_image(&self, id: u32) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(img) = inner.images.remove(&id) {
            if img.number > 0 {
                inner.images_by_num.remove(&img.number);
            }
        }
        inner.placements.retain(|p| p.image_id != id);
    }

    pub fn delete_image_by_number(&self, num: u32) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(&id) = inner.images_by_num.get(&num) {
            inner.images.remove(&id);
            inner.placements.retain(|p| p.image_id != id);
        }
        inner.images_by_num.remove(&num);
    }

    pub fn add_placement(&self, p: KittyPlacement) {
        let mut inner = self.inner.lock().unwrap();
        if p.placement_id > 0 {
            if let Some(existing) = inner
                .placements
                .iter_mut()
                .find(|e| e.image_id == p.image_id && e.placement_id == p.placement_id)
            {
                *existing = p;
                return;
            }
        }
        inner.placements.push(p);
    }

    pub fn delete_placement(&self, image_id: u32, placement_id: u32) {
        let mut inner = self.inner.lock().unwrap();
        inner
            .placements
            .retain(|p| p.image_id != image_id || p.placement_id != placement_id);
    }

    pub fn delete_placements_at_cursor(&self, x: i32, y: i32) {
        let mut inner = self.inner.lock().unwrap();
        inner.placements.retain(|p| p.screen_x != x || p.screen_y != y);
    }

    pub fn delete_placements_in_column(&self, x: i32) {
        let mut inner = self.inner.lock().unwrap();
        inner.placements.retain(|p| p.screen_x != x);
    }

    pub fn delete_placements_in_row(&self, y: i32) {
        let mut inner = self.inner.lock().unwrap();
        inner.placements.retain(|p| p.screen_y != y);
    }

    pub fn delete_placements_by_z_index(&self, z: i32) {
        let mut inner = self.inner.lock().unwrap();
        inner.placements.retain(|p| p.z_index != z);
    }

    pub fn clear(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.images.clear();
        inner.images_by_num.clear();
        inner.placements.clear();
        inner.pending = None;
    }

    pub fn clear_placements(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.placements.clear();
    }

    pub fn get_images(&self) -> Vec<KittyImage> {
        self.inner.lock().unwrap().images.values().cloned().collect()
    }

    pub fn get_placements(&self) -> Vec<KittyPlacement> {
        self.inner.lock().unwrap().placements.clone()
    }

    pub fn set_pending(&self, chunk: KittyPendingChunk) {
        let mut inner = self.inner.lock().unwrap();
        inner.pending = Some(chunk);
    }

    pub fn has_pending(&self) -> bool {
        self.inner.lock().unwrap().pending.is_some()
    }

    pub fn append_to_pending(&self, data: &[u8]) -> bool {
        let mut inner = self.inner.lock().unwrap();
        let Some(pending) = inner.pending.as_mut() else {
            return false;
        };
        if pending.data_buffer.len() + data.len() > MAX_KITTY_TRANSMIT_BYTES {
            inner.pending = None;
            return false;
        }
        pending.data_buffer.extend_from_slice(data);
        true
    }

    pub fn finalize_pending(&self) -> Option<KittyImage> {
        let mut inner = self.inner.lock().unwrap();
        let pending = inner.pending.take()?;
        Some(KittyImage {
            id: pending.image_id,
            number: pending.image_number,
            width: pending.width,
            height: pending.height,
            format: pending.format,
            compression: KittyCompression::None,
            data: pending.data_buffer,
            transmit_time: Instant::now(),
            animation_group: 0,
        })
    }

    pub fn clear_pending(&self) {
        self.inner.lock().unwrap().pending = None;
    }

    // --- Animation group management ---

    /// Allocate a new animation group ID.
    pub fn allocate_group_id(&self) -> u32 {
        let mut inner = self.inner.lock().unwrap();
        let id = inner.next_group_id;
        inner.next_group_id += 1;
        if inner.next_group_id == 0 {
            inner.next_group_id = 1;
        }
        id
    }

    /// Get or create an animation group.
    pub fn get_or_create_group(&self, group_id: u32) -> AnimationGroup {
        let mut inner = self.inner.lock().unwrap();
        inner
            .animation_groups
            .entry(group_id)
            .or_insert_with(|| AnimationGroup {
                group_id,
                frames: Vec::new(),
                current_frame: 0,
                playing: false,
                delay_ms: 0,
                duration_ms: 0,
                looping: true,
            })
            .clone()
    }

    /// Add a frame to an animation group.
    pub fn add_frame_to_group(&self, group_id: u32, image_id: u32) {
        let mut inner = self.inner.lock().unwrap();
        let group = inner
            .animation_groups
            .entry(group_id)
            .or_insert_with(|| AnimationGroup {
                group_id,
                frames: Vec::new(),
                current_frame: 0,
                playing: false,
                delay_ms: 0,
                duration_ms: 0,
                looping: true,
            });
        group.frames.push(image_id);
    }

    /// Set animation group playing state.
    pub fn set_group_playing(&self, group_id: u32, playing: bool) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(group) = inner.animation_groups.get_mut(&group_id) {
            group.playing = playing;
        }
    }

    /// Set animation group delay.
    pub fn set_group_delay(&self, group_id: u32, delay_ms: u32) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(group) = inner.animation_groups.get_mut(&group_id) {
            group.delay_ms = delay_ms;
        }
    }

    /// Delete an animation group and its images.
    pub fn delete_group(&self, group_id: u32) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(group) = inner.animation_groups.remove(&group_id) {
            for img_id in &group.frames {
                inner.images.remove(img_id);
                inner.images_by_num.retain(|_, id| id != img_id);
            }
            inner
                .placements
                .retain(|p| !group.frames.contains(&p.image_id));
        }
    }

    /// Delete all animation groups.
    pub fn clear_groups(&self) {
        let mut inner = self.inner.lock().unwrap();
        let frame_ids: Vec<u32> = inner
            .animation_groups
            .drain()
            .flat_map(|(_, g)| g.frames)
            .collect();
        for img_id in &frame_ids {
            inner.images.remove(img_id);
            inner.images_by_num.retain(|_, id| id != img_id);
        }
        inner.placements.retain(|p| !frame_ids.contains(&p.image_id));
    }

    /// Collect all image IDs belonging to any animation group.
    pub fn group_image_ids(&self) -> Vec<u32> {
        let inner = self.inner.lock().unwrap();
        inner
            .animation_groups
            .values()
            .flat_map(|g| g.frames.iter().copied())
            .collect()
    }
}

impl Default for KittyState {
    fn default() -> Self {
        Self::new()
    }
}

/// Build a kitty graphics protocol response (`\x1b_G<i=ID>;OK\x1b\\`).
pub fn build_kitty_response(ok: bool, image_id: u32, message: &str) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(b"\x1b_G");
    if image_id > 0 {
        buf.extend_from_slice(format!("i={image_id};").as_bytes());
    }
    if ok {
        buf.extend_from_slice(b"OK");
    } else if !message.is_empty() {
        buf.extend_from_slice(message.as_bytes());
    } else {
        buf.extend_from_slice(b"ENOENT:file not found");
    }
    buf.extend_from_slice(b"\x1b\\");
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kitty_state_lifecycle() {
        let state = KittyState::new();
        let id = state.allocate_id();
        assert_eq!(id, 1);
        let id2 = state.allocate_id();
        assert_eq!(id2, 2);

        let img = KittyImage {
            id,
            number: 10,
            width: 100,
            height: 50,
            format: KittyFormat::Rgba,
            compression: KittyCompression::None,
            data: vec![0; 100],
            transmit_time: Instant::now(),
            animation_group: 0,
        };
        state.add_image(img);
        assert!(state.get_image(id).is_some());
        assert!(state.get_image_by_number(10).is_some());

        state.add_placement(KittyPlacement {
            image_id: id,
            placement_id: 1,
            screen_x: 0,
            screen_y: 0,
            absolute_line: 0,
            x_offset: 0,
            y_offset: 0,
            source_x: 0,
            source_y: 0,
            source_width: 0,
            source_height: 0,
            columns: 10,
            rows: 5,
            z_index: 0,
            cursor_move: 0,
            virtual_placement: false,
        });
        assert_eq!(state.get_placements().len(), 1);

        state.delete_image(id);
        assert!(state.get_image(id).is_none());
        assert_eq!(state.get_placements().len(), 0);
    }

    #[test]
    fn kitty_chunked_transmit() {
        let state = KittyState::new();
        let id = state.allocate_id();
        state.set_pending(KittyPendingChunk {
            image_id: id,
            image_number: 0,
            format: KittyFormat::Rgba,
            compression: KittyCompression::None,
            width: 10,
            height: 10,
            data_buffer: vec![0; 50],
        });
        assert!(state.has_pending());
        assert!(state.append_to_pending(&[1; 50]));
        let img = state.finalize_pending().unwrap();
        assert_eq!(img.data.len(), 100);
        assert!(!state.has_pending());
    }

    #[test]
    fn kitty_response_format() {
        let resp = build_kitty_response(true, 5, "");
        let s = String::from_utf8_lossy(&resp);
        assert!(s.contains("\x1b_Gi=5;OK\x1b\\"));
    }

    #[test]
    fn animation_group_lifecycle() {
        let state = KittyState::new();
        let gid = state.allocate_group_id();
        assert_eq!(gid, 1);

        // Add frames to the group.
        let id1 = state.allocate_id();
        let id2 = state.allocate_id();
        state.add_frame_to_group(gid, id1);
        state.add_frame_to_group(gid, id2);

        let group = state.get_or_create_group(gid);
        assert_eq!(group.frames, vec![id1, id2]);
        assert!(!group.playing);

        // Start playing.
        state.set_group_playing(gid, true);
        let group = state.get_or_create_group(gid);
        assert!(group.playing);

        // Set delay.
        state.set_group_delay(gid, 100);
        let group = state.get_or_create_group(gid);
        assert_eq!(group.delay_ms, 100);

        // Delete the group.
        state.delete_group(gid);
        let group = state.get_or_create_group(gid);
        assert!(group.frames.is_empty());
    }

    #[test]
    fn clear_groups_removes_images() {
        let state = KittyState::new();
        let gid = state.allocate_group_id();
        let id1 = state.allocate_id();
        let id2 = state.allocate_id();

        state.add_image(KittyImage {
            id: id1,
            number: 0,
            width: 10,
            height: 10,
            format: KittyFormat::Rgba,
            compression: KittyCompression::None,
            data: vec![0; 10],
            transmit_time: Instant::now(),
            animation_group: gid,
        });
        state.add_image(KittyImage {
            id: id2,
            number: 0,
            width: 10,
            height: 10,
            format: KittyFormat::Rgba,
            compression: KittyCompression::None,
            data: vec![0; 10],
            transmit_time: Instant::now(),
            animation_group: gid,
        });
        state.add_frame_to_group(gid, id1);
        state.add_frame_to_group(gid, id2);

        assert!(state.get_image(id1).is_some());
        assert!(state.get_image(id2).is_some());

        state.clear_groups();

        assert!(state.get_image(id1).is_none());
        assert!(state.get_image(id2).is_none());
    }
}
