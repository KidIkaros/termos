//! Binary space partition (BSP) tree layout — a faithful port of TUIOS
//! `internal/layout/bsp.go`.
//!
//! The tree owns no windows or PTYs; a leaf names a window ID and the tree
//! answers layout questions about where that window's rectangle should be.
//! All geometry is in terminal cells.

use std::collections::HashMap;

use super::{DEFAULT_WINDOW_HEIGHT, DEFAULT_WINDOW_WIDTH};

/// A rectangle in terminal cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// How an internal node divides its space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SplitType {
    /// Leaf node (contains a window).
    #[default]
    None,
    /// Left/Right children (vertical divider).
    Vertical,
    /// Top/Bottom children (horizontal divider).
    Horizontal,
    /// Stacked children (only active child visible, others show as title bars).
    Stacked,
}

impl SplitType {
    pub fn name(self) -> &'static str {
        match self {
            SplitType::None => "none",
            SplitType::Vertical => "vertical",
            SplitType::Horizontal => "horizontal",
            SplitType::Stacked => "stacked",
        }
    }
}

/// How new windows are automatically inserted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AutoScheme {
    /// Split along the longest dimension of the target area.
    LongestSide,
    /// Alternate between vertical and horizontal splits.
    Alternate,
    /// Create a spiral pattern (like bspwm's default).
    #[default]
    Spiral,
    /// Choose split direction based on focused window aspect ratio.
    SmartSplit,
}

impl AutoScheme {
    pub fn parse(s: &str) -> Self {
        match s {
            "alternate" => AutoScheme::Alternate,
            "spiral" => AutoScheme::Spiral,
            "smart_split" => AutoScheme::SmartSplit,
            _ => AutoScheme::LongestSide,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            AutoScheme::LongestSide => "longest_side",
            AutoScheme::Alternate => "alternate",
            AutoScheme::Spiral => "spiral",
            AutoScheme::SmartSplit => "smart_split",
        }
    }
}

/// Direction for preselection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreselectionDir {
    None,
    Left,
    Right,
    Up,
    Down,
}

/// A node in the binary space partition tree.
///
/// Internal nodes have `left` and `right` children and define a split. Leaf
/// nodes have a `window_id` and represent an actual window.
#[derive(Debug)]
pub struct TileNode {
    /// Unique identifier for the node.
    pub id: u64,
    /// Parent node (None for root).
    pub parent: Option<usize>,
    /// Left/Top child (None for leaf nodes) — stored by node index.
    pub left: Option<usize>,
    /// Right/Bottom child (None for leaf nodes) — stored by node index.
    pub right: Option<usize>,
    /// Window ID (-1 for internal nodes).
    pub window_id: i32,
    /// How this node splits its space.
    pub split_type: SplitType,
    /// Position of split (0.0-1.0), 0.5 = middle.
    pub split_ratio: f64,
    /// For Stacked: true = left child is active (gets content area).
    pub stacked_active_left: bool,
}

impl TileNode {
    pub fn is_leaf(&self) -> bool {
        self.split_type == SplitType::None
    }

    pub fn is_root(&self) -> bool {
        self.parent.is_none()
    }
}

/// The BSP tree, arena-allocated so nodes can be indexed rather than held as
/// raw pointers (Rust-safe equivalent of the Go pointer graph).
#[derive(Debug, Default)]
pub struct BSPTree {
    /// Arena of all nodes, indexed by their slot.
    nodes: Vec<TileNode>,
    /// Root node index (None when empty).
    root: Option<usize>,
    /// Quick lookup: window ID -> node index.
    window_to_node: HashMap<i32, usize>,
    /// How to auto-insert new windows.
    auto_scheme: AutoScheme,
    /// Default split ratio for new splits.
    default_ratio: f64,
    /// Next node ID.
    next_id: u64,
}

impl BSPTree {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            root: None,
            window_to_node: HashMap::new(),
            auto_scheme: AutoScheme::Spiral,
            default_ratio: 0.5,
            next_id: 1,
        }
    }

    fn alloc(&mut self, mut node: TileNode) -> usize {
        node.id = self.next_id;
        self.next_id += 1;
        self.nodes.push(node);
        self.nodes.len() - 1
    }

    pub fn root(&self) -> Option<usize> {
        self.root
    }

    pub fn node_ref(&self, index: usize) -> Option<&TileNode> {
        self.nodes.get(index)
    }

    pub fn auto_scheme(&self) -> AutoScheme {
        self.auto_scheme
    }

    pub fn set_auto_scheme(&mut self, scheme: AutoScheme) {
        self.auto_scheme = scheme;
    }

    pub fn default_ratio(&self) -> f64 {
        self.default_ratio
    }

    pub fn set_default_ratio(&mut self, ratio: f64) {
        self.default_ratio = ratio;
    }

    pub fn is_empty(&self) -> bool {
        self.root.is_none()
    }

    pub fn window_count(&self) -> usize {
        self.window_to_node.len()
    }

    pub fn has_window(&self, window_id: i32) -> bool {
        self.window_to_node.contains_key(&window_id)
    }

    /// Leaf node index for the given window ID.
    pub fn find_node(&self, window_id: i32) -> Option<usize> {
        self.window_to_node.get(&window_id).copied()
    }

    /// Insert a new window by splitting the focused window. If direction is
    /// None, uses the auto scheme to determine split direction. The new window
    /// is inserted as the right/bottom child.
    pub fn insert_window(
        &mut self,
        window_id: i32,
        focused_window_id: i32,
        direction: SplitType,
        ratio: f64,
        bounds: Rect,
        gap: i32,
    ) {
        if self.has_window(window_id) {
            return;
        }

        let new_leaf = self.alloc(TileNode {
            id: 0,
            parent: None,
            left: None,
            right: None,
            window_id,
            split_type: SplitType::None,
            split_ratio: 0.5,
            stacked_active_left: true,
        });

        // First window — just make it the root.
        if self.root.is_none() {
            self.root = Some(new_leaf);
            self.window_to_node.insert(window_id, new_leaf);
            return;
        }

        // Find the node to split.
        let target = self.window_to_node.get(&focused_window_id).copied();
        let target = match target {
            Some(t) => t,
            None => {
                // Fallback: find any leaf.
                match self.find_any_leaf() {
                    Some(t) => t,
                    None => {
                        self.root = Some(new_leaf);
                        self.window_to_node.insert(window_id, new_leaf);
                        return;
                    }
                }
            }
        };

        // Determine split direction if not specified.
        let direction = if direction == SplitType::None {
            self.determine_auto_split(target, bounds, gap)
        } else {
            direction
        };

        // Use default ratio if not specified.
        let ratio = if ratio <= 0.0 || ratio >= 1.0 {
            self.default_ratio
        } else {
            ratio
        };

        // Create new internal node that replaces the target.
        let old_leaf = self.alloc(TileNode {
            id: 0,
            parent: None,
            left: None,
            right: None,
            window_id: self.nodes[target].window_id,
            split_type: SplitType::None,
            split_ratio: 0.5,
            stacked_active_left: true,
        });
        let internal = self.alloc(TileNode {
            id: 0,
            parent: None,
            left: Some(old_leaf),
            right: Some(new_leaf),
            window_id: -1,
            split_type: direction,
            split_ratio: ratio,
            stacked_active_left: true,
        });
        self.nodes[old_leaf].parent = Some(internal);
        self.nodes[new_leaf].parent = Some(internal);

        // Replace target in tree.
        let target_parent = self.nodes[target].parent;
        if let Some(parent) = target_parent {
            self.nodes[internal].parent = Some(parent);
            let is_left = self.nodes[parent].left == Some(target);
            if is_left {
                self.nodes[parent].left = Some(internal);
            } else {
                self.nodes[parent].right = Some(internal);
            }
        } else {
            self.root = Some(internal);
            self.nodes[internal].parent = None;
        }

        // Update window-to-node mapping.
        let old_window = self.nodes[target].window_id;
        self.window_to_node.insert(old_window, old_leaf);
        self.window_to_node.insert(window_id, new_leaf);
    }

    /// Insert a new window using a preselection direction.
    pub fn insert_window_with_preselection(
        &mut self,
        window_id: i32,
        focused_window_id: i32,
        preselect: PreselectionDir,
        bounds: Rect,
        gap: i32,
    ) {
        let (direction, new_window_is_left) = match preselect {
            PreselectionDir::Left => (SplitType::Vertical, true),
            PreselectionDir::Right => (SplitType::Vertical, false),
            PreselectionDir::Up => (SplitType::Horizontal, true),
            PreselectionDir::Down => (SplitType::Horizontal, false),
            PreselectionDir::None => {
                self.insert_window(
                    window_id,
                    focused_window_id,
                    SplitType::None,
                    self.default_ratio,
                    bounds,
                    gap,
                );
                return;
            }
        };

        if self.has_window(window_id) {
            return;
        }

        let new_leaf = self.alloc(TileNode {
            id: 0,
            parent: None,
            left: None,
            right: None,
            window_id,
            split_type: SplitType::None,
            split_ratio: 0.5,
            stacked_active_left: true,
        });

        if self.root.is_none() {
            self.root = Some(new_leaf);
            self.window_to_node.insert(window_id, new_leaf);
            return;
        }

        let target = self
            .window_to_node
            .get(&focused_window_id)
            .copied()
            .or_else(|| self.find_any_leaf());
        let Some(target) = target else {
            self.root = Some(new_leaf);
            self.window_to_node.insert(window_id, new_leaf);
            return;
        };

        let old_leaf = self.alloc(TileNode {
            id: 0,
            parent: None,
            left: None,
            right: None,
            window_id: self.nodes[target].window_id,
            split_type: SplitType::None,
            split_ratio: 0.5,
            stacked_active_left: true,
        });
        let internal = if new_window_is_left {
            self.alloc(TileNode {
                id: 0,
                parent: None,
                left: Some(new_leaf),
                right: Some(old_leaf),
                window_id: -1,
                split_type: direction,
                split_ratio: self.default_ratio,
                stacked_active_left: true,
            })
        } else {
            self.alloc(TileNode {
                id: 0,
                parent: None,
                left: Some(old_leaf),
                right: Some(new_leaf),
                window_id: -1,
                split_type: direction,
                split_ratio: self.default_ratio,
                stacked_active_left: true,
            })
        };
        self.nodes[old_leaf].parent = Some(internal);
        self.nodes[new_leaf].parent = Some(internal);

        let target_parent = self.nodes[target].parent;
        if let Some(parent) = target_parent {
            self.nodes[internal].parent = Some(parent);
            let is_left = self.nodes[parent].left == Some(target);
            if is_left {
                self.nodes[parent].left = Some(internal);
            } else {
                self.nodes[parent].right = Some(internal);
            }
        } else {
            self.root = Some(internal);
            self.nodes[internal].parent = None;
        }

        let old_window = self.nodes[target].window_id;
        self.window_to_node.insert(old_window, old_leaf);
        self.window_to_node.insert(window_id, new_leaf);
    }

    /// Remove a window and collapse the tree. When a window is removed, its
    /// sibling takes over the parent's space.
    pub fn remove_window(&mut self, window_id: i32) {
        let Some(node) = self.window_to_node.remove(&window_id) else {
            return;
        };

        // If this is the only window, tree becomes empty.
        if self.nodes[node].parent.is_none() {
            self.root = None;
            return;
        }

        let parent = self.nodes[node].parent.unwrap();
        let sibling = self.sibling(node);
        let grandparent = self.nodes[parent].parent;

        if let Some(sibling) = sibling {
            if let Some(gp) = grandparent {
                self.nodes[sibling].parent = Some(gp);
                let parent_is_left = self.nodes[gp].left == Some(parent);
                if parent_is_left {
                    self.nodes[gp].left = Some(sibling);
                } else {
                    self.nodes[gp].right = Some(sibling);
                }
            } else {
                // Parent was root, sibling becomes new root.
                self.root = Some(sibling);
                self.nodes[sibling].parent = None;
            }
        }
    }

    fn sibling(&self, node: usize) -> Option<usize> {
        let parent = self.nodes[node].parent?;
        if self.nodes[parent].left == Some(node) {
            self.nodes[parent].right
        } else {
            self.nodes[parent].left
        }
    }

    // -----------------------------------------------------------------------
    // Stacked pane operations
    // -----------------------------------------------------------------------

    /// Wrap two windows in a new `SplitType::Stacked` internal node.
    /// The active window gets the content area; the other becomes a 1-cell
    /// title bar.  Both windows must be leaves and must not already be in
    /// a stacked group.
    ///
    /// `active_id` keeps its content; `inactive_id` becomes the title bar.
    pub fn push_to_stack(&mut self, active_id: i32, inactive_id: i32) {
        if active_id == inactive_id {
            return;
        }
        let Some(&a_node) = self.window_to_node.get(&active_id) else {
            return;
        };
        let Some(&i_node) = self.window_to_node.get(&inactive_id) else {
            return;
        };
        // Don't double-stack.
        if self.find_stack_root(active_id).is_some() || self.find_stack_root(inactive_id).is_some() {
            return;
        }
        // Save old parents before we modify anything.
        let a_parent = self.nodes[a_node].parent;
        let i_parent = self.nodes[i_node].parent;
        // If both windows are siblings under the same parent, simply
        // convert that parent's split type to Stacked.
        if let (Some(ap), Some(ip)) = (a_parent, i_parent) {
            if ap == ip {
                // Ensure active is on the left (content) side.
                if self.nodes[ap].right == Some(a_node) {
                    let tmp = self.nodes[ap].left;
                    self.nodes[ap].left = self.nodes[ap].right;
                    self.nodes[ap].right = tmp;
                }
                self.nodes[ap].split_type = SplitType::Stacked;
                self.nodes[ap].stacked_active_left = true;
                return;
            }
        }
        // Non-sibling case: create a new stacked internal node that
        // replaces the active window's parent slot.
        let stacked = self.alloc(TileNode {
            id: 0,
            parent: a_parent,
            left: Some(a_node),
            right: Some(i_node),
            window_id: -1,
            split_type: SplitType::Stacked,
            split_ratio: 0.5,
            stacked_active_left: true,
        });
        self.nodes[a_node].parent = Some(stacked);
        self.nodes[i_node].parent = Some(stacked);
        // Re-parent under the active node's old parent.
        if let Some(gp) = a_parent {
            if self.nodes[gp].left == Some(a_node) {
                self.nodes[gp].left = Some(stacked);
            } else {
                self.nodes[gp].right = Some(stacked);
            }
        } else {
            self.root = Some(stacked);
        }
        // Splice the inactive node's old parent out of the tree.
        if let Some(ip) = i_parent {
            let sibling = self.sibling_of_node_in_parent(i_node, ip);
            if let Some(igp) = self.nodes[ip].parent {
                if let Some(sib) = sibling {
                    self.nodes[sib].parent = Some(igp);
                    if self.nodes[igp].left == Some(ip) {
                        self.nodes[igp].left = Some(sib);
                    } else {
                        self.nodes[igp].right = Some(sib);
                    }
                } else {
                    // Parent had only the inactive node — collapse.
                    if self.nodes[igp].left == Some(ip) {
                        self.nodes[igp].left = None;
                    } else {
                        self.nodes[igp].right = None;
                    }
                }
            } else {
                // ip was the root and had no grandparent.
                if let Some(sib) = sibling {
                    self.root = Some(sib);
                    self.nodes[sib].parent = None;
                } else {
                    self.root = Some(stacked);
                }
            }
        }
    }

    /// Like `sibling` but given the parent explicitly.
    fn sibling_of_node_in_parent(&self, node: usize, parent: usize) -> Option<usize> {
        if self.nodes[parent].left == Some(node) {
            self.nodes[parent].right
        } else {
            self.nodes[parent].left
        }
    }

    /// Remove a window from its stack, restoring it as a standalone leaf
    /// in the stack's layout slot.  Returns `true` if the window was in a
    /// stack and has been removed.
    pub fn pop_from_stack(&mut self, window_id: i32) -> bool {
        let Some(stack_root) = self.find_stack_root(window_id) else {
            return false;
        };
        // Simple case: just convert the stacked node back to a normal
        // Horizontal split.  Both children stay in place and the tree
        // structure is unchanged.
        self.nodes[stack_root].split_type = SplitType::Horizontal;
        self.nodes[stack_root].stacked_active_left = true;
        true
    }

    /// Find the stacked parent of a window, if it is in a stack.
    pub fn find_stack_root(&self, window_id: i32) -> Option<usize> {
        let &wnode = self.window_to_node.get(&window_id)?;
        let mut cur = wnode;
        while let Some(p) = self.nodes[cur].parent {
            if self.nodes[p].split_type == SplitType::Stacked {
                return Some(p);
            }
            cur = p;
        }
        None
    }

    /// Count how many windows share the same stack as the given window.
    /// Returns 1 for non-stacked windows.
    pub fn stack_count(&self, window_id: i32) -> usize {
        let Some(stack_root) = self.find_stack_root(window_id) else {
            return 1;
        };
        self.count_leaves(stack_root)
    }

    /// Return all window IDs in the same stack as `window_id` (leaf order).
    pub fn stack_windows(&self, window_id: i32) -> Vec<i32> {
        let Some(stack_root) = self.find_stack_root(window_id) else {
            return vec![window_id];
        };
        let mut ids = Vec::new();
        self.collect_window_ids(stack_root, &mut ids);
        ids
    }

    /// Which position (0-indexed, active = 0) the window occupies in its stack.
    pub fn stack_depth(&self, window_id: i32) -> usize {
        let windows = self.stack_windows(window_id);
        windows.iter().position(|&id| id == window_id).unwrap_or(0)
    }

    /// Cycle which pane is active (content area) in a stack.
    /// `forward = true` moves to the next pane; `false` to the previous.
    /// Returns the newly-active window ID, or the input if not stacked.
    pub fn cycle_stack_focus(&mut self, window_id: i32, forward: bool) -> i32 {
        let Some(stack_root) = self.find_stack_root(window_id) else {
            return window_id;
        };
        let mut windows = Vec::new();
        self.collect_window_ids(stack_root, &mut windows);
        let pos = windows.iter().position(|&id| id == window_id).unwrap_or(0);
        let next = if forward {
            (pos + 1) % windows.len()
        } else {
            (pos + windows.len() - 1) % windows.len()
        };
        let new_active = windows[next];
        // Swap: new_active goes left (content), old goes right (title).
        let left = self.nodes[stack_root].left;
        let right = self.nodes[stack_root].right;
        let left_win = left.map(|l| self.nodes[l].window_id);
        if left_win == Some(new_active) {
            // Already on the left — nothing to swap.
            return new_active;
        }
        self.nodes[stack_root].left = right;
        self.nodes[stack_root].right = left;
        self.nodes[stack_root].stacked_active_left = true;
        new_active
    }

    fn count_leaves(&self, node: usize) -> usize {
        let n = &self.nodes[node];
        if n.is_leaf() {
            return 1;
        }
        let mut count = 0;
        if let Some(l) = n.left {
            count += self.count_leaves(l);
        }
        if let Some(r) = n.right {
            count += self.count_leaves(r);
        }
        count
    }

    /// Calculate positions for all windows in the tree.
    pub fn apply_layout(&self, bounds: Rect, gap: i32) -> HashMap<i32, Rect> {
        let mut result = HashMap::new();
        self.apply_layout_into(bounds, &mut result, gap);
        result
    }

    /// ApplyLayout writing into a caller-owned map (avoids per-event allocs).
    pub fn apply_layout_into(&self, bounds: Rect, result: &mut HashMap<i32, Rect>, gap: i32) {
        result.clear();
        let Some(root) = self.root else {
            return;
        };
        self.apply_layout_recursive(root, bounds, result, gap);
        // Safety net: keep every laid-out rectangle inside the root bounds.
        for r in result.values_mut() {
            r.x = bounds
                .x
                .max(bounds.x.min(r.x).max(bounds.x + bounds.w - r.w));
            r.y = bounds
                .y
                .max(bounds.y.min(r.y).max(bounds.y + bounds.h - r.h));
        }
    }

    fn apply_layout_recursive(
        &self,
        node: usize,
        bounds: Rect,
        result: &mut HashMap<i32, Rect>,
        gap: i32,
    ) {
        let n = &self.nodes[node];
        if n.is_leaf() {
            // A leaf occupies exactly the rectangle the tree partitioned for it.
            // Floor at one cell so a degenerate split cannot yield zero/negative.
            result.insert(
                n.window_id,
                Rect {
                    x: bounds.x,
                    y: bounds.y,
                    w: bounds.w.max(1),
                    h: bounds.h.max(1),
                },
            );
            return;
        }

        let (left_bounds, right_bounds) = child_bounds(n, bounds, gap);
        if let Some(l) = n.left {
            self.apply_layout_recursive(l, left_bounds, result, gap);
        }
        if let Some(r) = n.right {
            self.apply_layout_recursive(r, right_bounds, result, gap);
        }
    }

    /// Move the divider that owns one edge of a window to `pos`, given in the
    /// same coordinate space as bounds. Reports whether a divider was found.
    pub fn resize_split(
        &mut self,
        window_id: i32,
        edge: ResizeEdge,
        pos: i32,
        bounds: Rect,
        gap: i32,
    ) -> bool {
        let Some(leaf) = self.window_to_node.get(&window_id).copied() else {
            return false;
        };

        // Walk up to the nearest ancestor that splits on this axis with the
        // window's subtree on the near side of the divider.
        let mut node: Option<usize> = None;
        let mut cur = leaf;
        while let Some(p) = self.nodes[cur].parent {
            let p_split = self.nodes[p].split_type;
            if p_split == SplitType::Stacked {
                cur = p;
                continue;
            }
            let vertical = p_split == SplitType::Vertical;
            if vertical != edge.vertical() {
                cur = p;
                continue;
            }
            let cur_is_left = self.nodes[p].left == Some(cur);
            if edge.far() == cur_is_left {
                node = Some(p);
                break;
            }
            cur = p;
        }
        let Some(node) = node else {
            return false;
        };

        let Some(rect) = self.node_bounds(node, bounds, gap) else {
            return false;
        };

        // The near child's far edge is the divider line itself; the far
        // child's near edge sits one separator cell past it.
        let mut line = pos;
        if !edge.far() {
            line -= gap;
        }

        let (origin, extent) = if edge.vertical() {
            (rect.x, rect.w)
        } else {
            (rect.y, rect.h)
        };
        if extent <= 0 {
            return false;
        }

        let lo = origin + self.min_extent(self.nodes[node].left, edge.vertical(), gap);
        let hi =
            origin + extent - gap - self.min_extent(self.nodes[node].right, edge.vertical(), gap);
        if lo > hi {
            return false;
        }
        line = lo.max(line.min(hi));

        // Aim at the middle of the target cell.
        self.nodes[node].split_ratio = ((line - origin) as f64 + 0.5) / extent as f64;
        true
    }

    /// The rectangle apply_layout would hand to `target`.
    fn node_bounds(&self, target: usize, bounds: Rect, gap: i32) -> Option<Rect> {
        let root = self.root?;

        // Path from the root down to target, collected by walking parent links up.
        let mut path = Vec::new();
        let mut cur = Some(target);
        while let Some(c) = cur {
            path.push(c);
            cur = self.nodes[c].parent;
        }
        if path.last() != Some(&root) {
            return None;
        }

        let mut rect = bounds;
        for i in (0..path.len() - 1).rev() {
            let node = path[i];
            let n = &self.nodes[node];
            if n.is_leaf() {
                return None;
            }
            let (left_bounds, right_bounds) = child_bounds(n, rect, gap);
            if path[i - 1] == n.left.unwrap() {
                rect = left_bounds;
            } else {
                rect = right_bounds;
            }
        }
        Some(rect)
    }

    /// Rotate split direction at the parent of the given window.
    pub fn rotate_split(&mut self, window_id: i32) {
        let Some(node) = self.window_to_node.get(&window_id).copied() else {
            return;
        };
        let Some(parent) = self.nodes[node].parent else {
            return;
        };
        let p = &mut self.nodes[parent];
        p.split_type = if p.split_type == SplitType::Vertical {
            SplitType::Horizontal
        } else {
            SplitType::Vertical
        };
    }

    /// Swap the positions of two windows in the tree.
    pub fn swap_windows(&mut self, window_id1: i32, window_id2: i32) {
        let Some(node1) = self.window_to_node.get(&window_id1).copied() else {
            return;
        };
        let Some(node2) = self.window_to_node.get(&window_id2).copied() else {
            return;
        };

        let w1 = self.nodes[node1].window_id;
        let w2 = self.nodes[node2].window_id;
        self.nodes[node1].window_id = w2;
        self.nodes[node2].window_id = w1;
        self.window_to_node.insert(window_id1, node2);
        self.window_to_node.insert(window_id2, node1);
    }

    /// Set all split ratios to 0.5.
    pub fn equalize_ratios(&mut self) {
        let Some(root) = self.root else {
            return;
        };
        self.equalize_ratios_recursive(root);
    }

    fn equalize_ratios_recursive(&mut self, node: usize) {
        let n = &self.nodes[node];
        if n.is_leaf() {
            return;
        }
        let (l, r) = (n.left, n.right);
        self.nodes[node].split_ratio = 0.5;
        if let Some(l) = l {
            self.equalize_ratios_recursive(l);
        }
        if let Some(r) = r {
            self.equalize_ratios_recursive(r);
        }
    }

    /// All window IDs in the tree (in-order traversal).
    pub fn get_all_window_ids(&self) -> Vec<i32> {
        let mut ids = Vec::new();
        if let Some(root) = self.root {
            self.collect_window_ids(root, &mut ids);
        }
        ids
    }

    fn collect_window_ids(&self, node: usize, ids: &mut Vec<i32>) {
        let n = &self.nodes[node];
        if n.is_leaf() {
            ids.push(n.window_id);
            return;
        }
        if let Some(l) = n.left {
            self.collect_window_ids(l, ids);
        }
        if let Some(r) = n.right {
            self.collect_window_ids(r, ids);
        }
    }

    /// Direction of the next auto-split ("V" or "H") for the dock indicator.
    pub fn get_next_split_direction(&self) -> &'static str {
        if self.auto_scheme == AutoScheme::Spiral {
            if self.deepest_leaf_depth() % 2 == 0 {
                return "V";
            }
            return "H";
        }
        if self.count_internal_nodes() % 2 == 0 {
            "V"
        } else {
            "H"
        }
    }

    fn deepest_leaf_depth(&self) -> i32 {
        let Some(root) = self.root else {
            return 0;
        };
        self.max_leaf_depth(root, 0)
    }

    fn max_leaf_depth(&self, node: usize, depth: i32) -> i32 {
        let n = &self.nodes[node];
        if n.is_leaf() {
            return depth;
        }
        let l = n
            .left
            .map(|l| self.max_leaf_depth(l, depth + 1))
            .unwrap_or(0);
        let r = n
            .right
            .map(|r| self.max_leaf_depth(r, depth + 1))
            .unwrap_or(0);
        l.max(r)
    }

    fn count_internal_nodes(&self) -> i32 {
        let Some(root) = self.root else {
            return 0;
        };
        self.count_internal_nodes_recursive(root)
    }

    fn count_internal_nodes_recursive(&self, node: usize) -> i32 {
        let n = &self.nodes[node];
        if n.is_leaf() {
            return 0;
        }
        1 + n
            .left
            .map(|l| self.count_internal_nodes_recursive(l))
            .unwrap_or(0)
            + n.right
                .map(|r| self.count_internal_nodes_recursive(r))
                .unwrap_or(0)
    }

    fn find_any_leaf(&self) -> Option<usize> {
        let root = self.root?;
        self.find_leaf_in_subtree(root)
    }

    fn find_leaf_in_subtree(&self, node: usize) -> Option<usize> {
        let n = &self.nodes[node];
        if n.is_leaf() {
            return Some(node);
        }
        if let Some(l) = n.left {
            if let Some(leaf) = self.find_leaf_in_subtree(l) {
                return Some(leaf);
            }
        }
        if let Some(r) = n.right {
            return self.find_leaf_in_subtree(r);
        }
        None
    }

    /// Determine split direction based on the auto scheme.
    fn determine_auto_split(&self, target: usize, bounds: Rect, gap: i32) -> SplitType {
        // cellAspect: how many times taller a character cell is than it is wide.
        const CELL_ASPECT: i32 = 2;
        match self.auto_scheme {
            AutoScheme::LongestSide => {
                if bounds.w >= bounds.h * CELL_ASPECT {
                    SplitType::Vertical
                } else {
                    SplitType::Horizontal
                }
            }
            AutoScheme::Alternate => {
                let split_count = self.count_internal_nodes();
                if split_count % 2 == 0 {
                    SplitType::Vertical
                } else {
                    SplitType::Horizontal
                }
            }
            AutoScheme::Spiral => {
                let mut vertical = self.node_depth(target) % 2 == 0;
                if bounds.w < bounds.h * CELL_ASPECT {
                    vertical = !vertical;
                }
                if vertical {
                    SplitType::Vertical
                } else {
                    SplitType::Horizontal
                }
            }
            AutoScheme::SmartSplit => {
                let r = self.node_bounds(target, bounds, gap).unwrap_or(bounds);
                let w = r.w;
                let h = r.h * CELL_ASPECT;
                if w > h * 2 {
                    return SplitType::Vertical;
                }
                if h > w {
                    return SplitType::Horizontal;
                }
                let depth = self.node_depth(target);
                if depth % 2 == 0 {
                    SplitType::Vertical
                } else {
                    SplitType::Horizontal
                }
            }
        }
    }

    fn node_depth(&self, node: usize) -> i32 {
        let mut depth = 0;
        let mut cur = Some(node);
        while let Some(c) = cur {
            cur = self.nodes[c].parent;
            if cur.is_some() {
                depth += 1;
            }
        }
        depth
    }

    /// Collect all separator line positions for shared border rendering.
    pub fn collect_splits(&self, bounds: Rect, gap: i32) -> Vec<SplitLine> {
        let mut splits = Vec::new();
        if self.root.is_none() || gap <= 0 {
            return splits;
        }
        let root = self.root.unwrap();
        self.collect_splits_recursive(root, bounds, &mut splits, gap);
        splits
    }

    fn collect_splits_recursive(
        &self,
        node: usize,
        bounds: Rect,
        splits: &mut Vec<SplitLine>,
        gap: i32,
    ) {
        let n = &self.nodes[node];
        if n.is_leaf() {
            return;
        }

        let (left_bounds, right_bounds) = child_bounds(n, bounds, gap);

        if n.split_type != SplitType::Stacked {
            if n.split_type == SplitType::Vertical {
                splits.push(SplitLine {
                    vertical: true,
                    pos: bounds.x + left_bounds.w,
                    from: bounds.y,
                    to: bounds.y + bounds.h - 1,
                });
            } else {
                splits.push(SplitLine {
                    vertical: false,
                    pos: bounds.y + left_bounds.h,
                    from: bounds.x,
                    to: bounds.x + bounds.w - 1,
                });
            }
        }

        if let Some(l) = n.left {
            self.collect_splits_recursive(l, left_bounds, splits, gap);
        }
        if let Some(r) = n.right {
            self.collect_splits_recursive(r, right_bounds, splits, gap);
        }
    }

    /// Re-derive split ratios from actual window geometry (after mouse resize).
    pub fn sync_ratios_from_geometry(
        &mut self,
        windows: &HashMap<i32, Rect>,
        bounds: Rect,
        gap: i32,
    ) {
        let Some(root) = self.root else {
            return;
        };
        self.sync_ratios_recursive(root, bounds, windows, gap);
    }

    fn sync_ratios_recursive(
        &mut self,
        node: usize,
        bounds: Rect,
        windows: &HashMap<i32, Rect>,
        gap: i32,
    ) {
        let (split_type, stacked_active_left, left, right) = {
            let n = &self.nodes[node];
            (n.split_type, n.stacked_active_left, n.left, n.right)
        };
        if split_type == SplitType::None {
            return;
        }

        if split_type == SplitType::Stacked {
            const TITLE_BAR_HEIGHT: i32 = 1;
            let mut content = Rect {
                x: bounds.x,
                y: bounds.y,
                w: bounds.w,
                h: bounds.h - TITLE_BAR_HEIGHT,
            };
            let mut title = Rect {
                x: bounds.x,
                y: bounds.y + bounds.h - TITLE_BAR_HEIGHT,
                w: bounds.w,
                h: TITLE_BAR_HEIGHT,
            };
            if stacked_active_left {
                if let Some(l) = left {
                    self.sync_ratios_recursive(l, content, windows, gap);
                }
                if let Some(r) = right {
                    self.sync_ratios_recursive(r, title, windows, gap);
                }
            } else {
                title.y = bounds.y;
                content.y = bounds.y + TITLE_BAR_HEIGHT;
                if let Some(l) = left {
                    self.sync_ratios_recursive(l, title, windows, gap);
                }
                if let Some(r) = right {
                    self.sync_ratios_recursive(r, content, windows, gap);
                }
            }
            return;
        }

        let (expected_left, expected_right) = child_bounds(&self.nodes[node], bounds, gap);

        if split_type == SplitType::Vertical {
            let mut split_x = -1;
            let mut ok = false;
            if let Some(id) = self.find_any_window_in_subtree(right) {
                if let Some(r) = windows.get(&id) {
                    split_x = r.x - gap;
                    ok = true;
                }
            }
            if !ok {
                if let Some(id) = self.find_any_window_in_subtree(left) {
                    if let Some(r) = windows.get(&id) {
                        split_x = r.x + r.w;
                        ok = true;
                    }
                }
            }
            if !ok {
                return;
            }
            let (mut left_bounds, mut right_bounds) = (expected_left, expected_right);
            if split_x != expected_left.x + expected_left.w {
                if bounds.w > 0 {
                    self.nodes[node].split_ratio = (split_x - bounds.x) as f64 / bounds.w as f64;
                }
                left_bounds = Rect {
                    x: bounds.x,
                    y: bounds.y,
                    w: split_x - bounds.x,
                    h: bounds.h,
                };
                right_bounds = Rect {
                    x: split_x + gap,
                    y: bounds.y,
                    w: bounds.x + bounds.w - split_x - gap,
                    h: bounds.h,
                };
            }
            if let Some(l) = left {
                self.sync_ratios_recursive(l, left_bounds, windows, gap);
            }
            if let Some(r) = right {
                self.sync_ratios_recursive(r, right_bounds, windows, gap);
            }
        } else {
            let mut split_y = -1;
            let mut ok = false;
            if let Some(id) = self.find_any_window_in_subtree(right) {
                if let Some(r) = windows.get(&id) {
                    split_y = r.y - gap;
                    ok = true;
                }
            }
            if !ok {
                if let Some(id) = self.find_any_window_in_subtree(left) {
                    if let Some(r) = windows.get(&id) {
                        split_y = r.y + r.h;
                        ok = true;
                    }
                }
            }
            if !ok {
                return;
            }
            let (mut left_bounds, mut right_bounds) = (expected_left, expected_right);
            if split_y != expected_left.y + expected_left.h {
                if bounds.h > 0 {
                    self.nodes[node].split_ratio = (split_y - bounds.y) as f64 / bounds.h as f64;
                }
                left_bounds = Rect {
                    x: bounds.x,
                    y: bounds.y,
                    w: bounds.w,
                    h: split_y - bounds.y,
                };
                right_bounds = Rect {
                    x: bounds.x,
                    y: split_y + gap,
                    w: bounds.w,
                    h: bounds.y + bounds.h - split_y - gap,
                };
            }
            if let Some(l) = left {
                self.sync_ratios_recursive(l, left_bounds, windows, gap);
            }
            if let Some(r) = right {
                self.sync_ratios_recursive(r, right_bounds, windows, gap);
            }
        }
    }

    fn find_any_window_in_subtree(&self, node: Option<usize>) -> Option<i32> {
        let node = node?;
        let n = &self.nodes[node];
        if n.is_leaf() {
            return Some(n.window_id);
        }
        if let Some(id) = self.find_any_window_in_subtree(n.left) {
            return Some(id);
        }
        self.find_any_window_in_subtree(n.right)
    }

    /// Deep clone of the tree.
    pub fn clone_tree(&self) -> BSPTree {
        let mut new_tree = BSPTree::new();
        new_tree.auto_scheme = self.auto_scheme;
        new_tree.default_ratio = self.default_ratio;
        if let Some(root) = self.root {
            let new_root = new_tree.clone_node(root, None);
            new_tree.root = Some(new_root);
        }
        new_tree
    }

    fn clone_node(&mut self, node: usize, parent: Option<usize>) -> usize {
        let (window_id, split_type, split_ratio, stacked_active_left, left, right) = {
            let n = &self.nodes[node];
            (
                n.window_id,
                n.split_type,
                n.split_ratio,
                n.stacked_active_left,
                n.left,
                n.right,
            )
        };
        let new_index = self.alloc(TileNode {
            id: 0,
            parent,
            left: None,
            right: None,
            window_id,
            split_type,
            split_ratio,
            stacked_active_left,
        });
        if split_type == SplitType::None {
            self.window_to_node.insert(window_id, new_index);
        } else {
            if let Some(l) = left {
                let l = self.clone_node(l, Some(new_index));
                self.nodes[new_index].left = Some(l);
            }
            if let Some(r) = right {
                let r = self.clone_node(r, Some(new_index));
                self.nodes[new_index].right = Some(r);
            }
        }
        new_index
    }
}

/// Which edge of a pane a resize gesture drags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeEdge {
    Right,
    Left,
    Bottom,
    Top,
}

impl ResizeEdge {
    pub fn vertical(self) -> bool {
        matches!(self, ResizeEdge::Right | ResizeEdge::Left)
    }

    /// The edge is on the high side of the pane (where the divider sits when
    /// the pane's subtree is the near child).
    fn far(self) -> bool {
        matches!(self, ResizeEdge::Right | ResizeEdge::Bottom)
    }
}

/// A separator line between two panes in shared border mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SplitLine {
    /// True = vertical line (│), false = horizontal line (─).
    pub vertical: bool,
    /// X coordinate for vertical, Y coordinate for horizontal.
    pub pos: i32,
    /// Start Y for vertical, start X for horizontal.
    pub from: i32,
    /// End Y for vertical, end X for horizontal.
    pub to: i32,
}

/// Divide an internal node's rectangle between its two children. This is the
/// single definition of the split model: every other part of the layout that
/// needs to know where a divider sits derives it from here.
fn child_bounds(node: &TileNode, bounds: Rect, gap: i32) -> (Rect, Rect) {
    if node.split_type == SplitType::Stacked {
        const TITLE_BAR_HEIGHT: i32 = 1;
        if node.stacked_active_left {
            let left = Rect {
                x: bounds.x,
                y: bounds.y,
                w: bounds.w,
                h: bounds.h - TITLE_BAR_HEIGHT,
            };
            let right = Rect {
                x: bounds.x,
                y: bounds.y + bounds.h - TITLE_BAR_HEIGHT,
                w: bounds.w,
                h: TITLE_BAR_HEIGHT,
            };
            return (left, right);
        }
        let left = Rect {
            x: bounds.x,
            y: bounds.y,
            w: bounds.w,
            h: TITLE_BAR_HEIGHT,
        };
        let right = Rect {
            x: bounds.x,
            y: bounds.y + TITLE_BAR_HEIGHT,
            w: bounds.w,
            h: bounds.h - TITLE_BAR_HEIGHT,
        };
        return (left, right);
    }

    if node.split_type == SplitType::Vertical {
        let mut split_x = bounds.x + (bounds.w as f64 * node.split_ratio) as i32;
        if gap > 0 {
            split_x = (bounds.x + 1).max(split_x.min(bounds.x + bounds.w - 2));
        }
        let left = Rect {
            x: bounds.x,
            y: bounds.y,
            w: split_x - bounds.x,
            h: bounds.h,
        };
        let right = Rect {
            x: split_x + gap,
            y: bounds.y,
            w: bounds.x + bounds.w - split_x - gap,
            h: bounds.h,
        };
        return (left, right);
    }

    let mut split_y = bounds.y + (bounds.h as f64 * node.split_ratio) as i32;
    if gap > 0 {
        split_y = (bounds.y + 1).max(split_y.min(bounds.y + bounds.h - 2));
    }
    let left = Rect {
        x: bounds.x,
        y: bounds.y,
        w: bounds.w,
        h: split_y - bounds.y,
    };
    let right = Rect {
        x: bounds.x,
        y: split_y + gap,
        w: bounds.w,
        h: bounds.y + bounds.h - split_y - gap,
    };
    (left, right)
}

impl BSPTree {
    /// The smallest width (or height) a subtree can be laid out in without
    /// pushing one of its leaves under the minimum window size.
    fn min_extent(&self, node: Option<usize>, vertical: bool, gap: i32) -> i32 {
        let Some(node) = node else {
            return 0;
        };
        let n = &self.nodes[node];
        if n.is_leaf() {
            return if vertical {
                DEFAULT_WINDOW_WIDTH
            } else {
                DEFAULT_WINDOW_HEIGHT
            };
        }
        let left = self.min_extent(n.left, vertical, gap);
        let right = self.min_extent(n.right, vertical, gap);
        if n.split_type == SplitType::Stacked {
            if vertical {
                return left.max(right);
            }
            return left.max(right) + 1;
        }
        if (n.split_type == SplitType::Vertical) != vertical {
            return left.max(right);
        }
        left + right + gap
    }
}

/// Serialized representation of a BSP tree node.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SerializedNode {
    pub window_id: i32,
    pub split_type: i32,
    pub split_ratio: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left: Option<Box<SerializedNode>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right: Option<Box<SerializedNode>>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct SerializedBSPTree {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root: Option<SerializedNode>,
    pub auto_scheme: i32,
    pub default_ratio: f64,
}

impl BSPTree {
    pub fn serialize(&self) -> SerializedBSPTree {
        SerializedBSPTree {
            root: self.root.map(|r| serialize_node(&self.nodes, r)),
            auto_scheme: auto_scheme_to_int(self.auto_scheme),
            default_ratio: self.default_ratio,
        }
    }

    pub fn deserialize(s: &SerializedBSPTree) -> BSPTree {
        let mut tree = BSPTree::new();
        tree.auto_scheme = auto_scheme_from_int(s.auto_scheme);
        tree.default_ratio = s.default_ratio;
        if let Some(root) = &s.root {
            let new_root = tree.deserialize_node(root, None);
            tree.root = Some(new_root);
        }
        tree
    }

    fn deserialize_node(&mut self, s: &SerializedNode, parent: Option<usize>) -> usize {
        let index = self.alloc(TileNode {
            id: 0,
            parent,
            left: None,
            right: None,
            window_id: s.window_id,
            split_type: split_type_from_int(s.split_type),
            split_ratio: s.split_ratio,
            stacked_active_left: true,
        });
        if self.nodes[index].is_leaf() && s.window_id >= 0 {
            self.window_to_node.insert(s.window_id, index);
        }
        if let Some(l) = &s.left {
            let l = self.deserialize_node(l, Some(index));
            self.nodes[index].left = Some(l);
        }
        if let Some(r) = &s.right {
            let r = self.deserialize_node(r, Some(index));
            self.nodes[index].right = Some(r);
        }
        index
    }
}

fn serialize_node(nodes: &[TileNode], index: usize) -> SerializedNode {
    let n = &nodes[index];
    SerializedNode {
        window_id: n.window_id,
        split_type: split_type_to_int(n.split_type),
        split_ratio: n.split_ratio,
        left: n.left.map(|l| Box::new(serialize_node(nodes, l))),
        right: n.right.map(|r| Box::new(serialize_node(nodes, r))),
    }
}

fn split_type_to_int(t: SplitType) -> i32 {
    match t {
        SplitType::None => 0,
        SplitType::Vertical => 1,
        SplitType::Horizontal => 2,
        SplitType::Stacked => 3,
    }
}

fn split_type_from_int(i: i32) -> SplitType {
    match i {
        1 => SplitType::Vertical,
        2 => SplitType::Horizontal,
        3 => SplitType::Stacked,
        _ => SplitType::None,
    }
}

fn auto_scheme_to_int(s: AutoScheme) -> i32 {
    match s {
        AutoScheme::LongestSide => 0,
        AutoScheme::Alternate => 1,
        AutoScheme::Spiral => 2,
        AutoScheme::SmartSplit => 3,
    }
}

fn auto_scheme_from_int(i: i32) -> AutoScheme {
    match i {
        1 => AutoScheme::Alternate,
        2 => AutoScheme::Spiral,
        3 => AutoScheme::SmartSplit,
        _ => AutoScheme::LongestSide,
    }
}
