//! Damage tracking for incremental cell composition.

use crate::layout::Rect;

/// A reason a compositor region needs to be redrawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageReason {
    Output,
    Geometry,
    Overlay,
    Dock,
    Theme,
    Resize,
    Full,
}

/// A rectangle plus the event that invalidated it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DamageRect {
    pub rect: Rect,
    pub reason: DamageReason,
}

impl DamageRect {
    pub fn new(rect: Rect, reason: DamageReason) -> Self {
        Self { rect, reason }
    }
}

/// A bounded set of non-empty damage rectangles.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DamageSet {
    bounds: Rect,
    full: bool,
    rects: Vec<DamageRect>,
}

impl DamageSet {
    pub fn new(bounds: Rect) -> Self {
        Self {
            bounds,
            full: false,
            rects: Vec::new(),
        }
    }

    pub fn bounds(&self) -> Rect {
        self.bounds
    }

    pub fn is_full(&self) -> bool {
        self.full
    }

    pub fn is_empty(&self) -> bool {
        !self.full && self.rects.is_empty()
    }

    pub fn clear(&mut self) {
        self.full = false;
        self.rects.clear();
    }

    pub fn mark_full(&mut self, reason: DamageReason) {
        self.full = true;
        self.rects.clear();
        self.rects.push(DamageRect::new(self.bounds, reason));
    }

    pub fn mark(&mut self, rect: Rect, reason: DamageReason) {
        let Some(rect) = intersect(rect, self.bounds) else {
            return;
        };
        if rect == self.bounds {
            self.mark_full(reason);
            return;
        }
        if self.full {
            return;
        }

        let mut merged = rect;
        let mut remaining: Vec<DamageRect> = Vec::with_capacity(self.rects.len());
        for existing in self.rects.drain(..) {
            if touches_or_overlaps(merged, existing.rect) {
                merged = union(merged, existing.rect);
            } else {
                remaining.push(existing);
            }
        }
        self.rects = remaining;
        self.rects.push(DamageRect::new(merged, reason));
    }

    pub fn iter(&self) -> impl Iterator<Item = &DamageRect> {
        self.rects.iter()
    }

    pub fn take(&mut self) -> Vec<DamageRect> {
        if self.full {
            let reason = self
                .rects
                .first()
                .map(|r| r.reason)
                .unwrap_or(DamageReason::Full);
            self.clear();
            vec![DamageRect::new(self.bounds, reason)]
        } else {
            std::mem::take(&mut self.rects)
        }
    }
}

fn intersect(a: Rect, b: Rect) -> Option<Rect> {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = (a.x + a.w).min(b.x + b.w);
    let y1 = (a.y + a.h).min(b.y + b.h);
    if x1 <= x0 || y1 <= y0 {
        None
    } else {
        Some(Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 })
    }
}

fn union(a: Rect, b: Rect) -> Rect {
    let x0 = a.x.min(b.x);
    let y0 = a.y.min(b.y);
    let x1 = (a.x + a.w).max(b.x + b.w);
    let y1 = (a.y + a.h).max(b.y + b.h);
    Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
}

fn touches_or_overlaps(a: Rect, b: Rect) -> bool {
    a.x <= b.x + b.w
        && b.x <= a.x + a.w
        && a.y <= b.y + b.h
        && b.y <= a.y + a.h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds() -> Rect {
        Rect { x: 0, y: 0, w: 100, h: 40 }
    }

    #[test]
    fn merges_overlapping_regions() {
        let mut damage = DamageSet::new(bounds());
        damage.mark(Rect { x: 2, y: 3, w: 10, h: 5 }, DamageReason::Output);
        damage.mark(Rect { x: 8, y: 6, w: 10, h: 5 }, DamageReason::Geometry);
        assert_eq!(damage.iter().count(), 1);
        assert_eq!(damage.iter().next().unwrap().rect, Rect { x: 2, y: 3, w: 16, h: 8 });
    }

    #[test]
    fn merges_adjacent_regions() {
        let mut damage = DamageSet::new(bounds());
        damage.mark(Rect { x: 2, y: 2, w: 4, h: 4 }, DamageReason::Output);
        damage.mark(Rect { x: 6, y: 2, w: 4, h: 4 }, DamageReason::Output);
        assert_eq!(damage.iter().count(), 1);
    }

    #[test]
    fn clips_regions_to_bounds() {
        let mut damage = DamageSet::new(bounds());
        damage.mark(Rect { x: -5, y: -3, w: 10, h: 10 }, DamageReason::Resize);
        assert_eq!(damage.iter().next().unwrap().rect, Rect { x: 0, y: 0, w: 5, h: 7 });
    }

    #[test]
    fn ignores_outside_regions() {
        let mut damage = DamageSet::new(bounds());
        damage.mark(Rect { x: 110, y: 0, w: 5, h: 5 }, DamageReason::Output);
        assert!(damage.is_empty());
    }

    #[test]
    fn full_damage_replaces_partial_regions() {
        let mut damage = DamageSet::new(bounds());
        damage.mark(Rect { x: 2, y: 2, w: 4, h: 4 }, DamageReason::Output);
        damage.mark_full(DamageReason::Theme);
        assert!(damage.is_full());
        assert_eq!(damage.iter().count(), 1);
        assert_eq!(damage.iter().next().unwrap().rect, bounds());
    }

    #[test]
    fn take_clears_pending_damage() {
        let mut damage = DamageSet::new(bounds());
        damage.mark(Rect { x: 2, y: 2, w: 4, h: 4 }, DamageReason::Output);
        assert_eq!(damage.take().len(), 1);
        assert!(damage.is_empty());
    }

    #[test]
    fn take_preserves_full_reason() {
        let mut damage = DamageSet::new(bounds());
        damage.mark_full(DamageReason::Theme);
        let taken = damage.take();
        assert_eq!(taken.len(), 1);
        assert_eq!(taken[0].reason, DamageReason::Theme);
        assert!(damage.is_empty());
    }

    #[test]
    fn bridging_region_merges_two_separate_rects() {
        let mut damage = DamageSet::new(bounds());
        damage.mark(Rect { x: 0, y: 10, w: 4, h: 4 }, DamageReason::Output);
        damage.mark(Rect { x: 20, y: 10, w: 4, h: 4 }, DamageReason::Output);
        assert_eq!(damage.iter().count(), 2);
        // A wide bridge across both spans merges them into one region.
        damage.mark(Rect { x: 2, y: 10, w: 20, h: 4 }, DamageReason::Overlay);
        assert_eq!(damage.iter().count(), 1);
        assert_eq!(damage.iter().next().unwrap().rect, Rect { x: 0, y: 10, w: 24, h: 4 });
    }

    #[test]
    fn non_adjacent_regions_stay_separate() {
        let mut damage = DamageSet::new(bounds());
        damage.mark(Rect { x: 0, y: 0, w: 4, h: 4 }, DamageReason::Output);
        damage.mark(Rect { x: 20, y: 20, w: 4, h: 4 }, DamageReason::Geometry);
        assert_eq!(damage.iter().count(), 2);
    }
}
