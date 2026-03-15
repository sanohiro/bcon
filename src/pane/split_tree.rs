//! Arena-based binary split tree for pane layout
//!
//! Each leaf holds a PaneId, each internal node is a split with direction and ratio.

use super::{Direction, NavDirection, PaneId, PaneRect};

/// Index into the arena
pub type NodeId = usize;

/// Tree node: either a leaf (pane) or a split (two children)
#[derive(Debug, Clone)]
pub enum Node {
    Leaf {
        pane_id: PaneId,
    },
    Split {
        direction: Direction,
        ratio: f32, // 0.0..1.0 — fraction allocated to `first`
        first: NodeId,
        second: NodeId,
    },
}

/// Arena-based binary tree for pane splits
#[derive(Debug)]
pub struct SplitTree {
    nodes: Vec<Option<Node>>,
    pub root: Option<NodeId>,
}

impl SplitTree {
    /// Create a tree with a single leaf
    pub fn new(pane_id: PaneId) -> Self {
        let mut tree = Self {
            nodes: Vec::new(),
            root: None,
        };
        let id = tree.alloc(Node::Leaf { pane_id });
        tree.root = Some(id);
        tree
    }

    /// Allocate a node in the arena, reusing free slots
    fn alloc(&mut self, node: Node) -> NodeId {
        for (i, slot) in self.nodes.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(node);
                return i;
            }
        }
        let id = self.nodes.len();
        self.nodes.push(Some(node));
        id
    }

    /// Free a node slot
    fn free(&mut self, id: NodeId) {
        if id < self.nodes.len() {
            self.nodes[id] = None;
        }
    }

    /// Get a reference to a node
    pub fn get(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id).and_then(|n| n.as_ref())
    }

    /// Split the leaf containing `pane_id` into two panes.
    /// The existing pane goes to `first`, the new pane to `second`.
    /// Returns the NodeId of the new split node.
    pub fn split(
        &mut self,
        target_pane: PaneId,
        new_pane: PaneId,
        direction: Direction,
        ratio: f32,
    ) -> Option<NodeId> {
        let leaf_id = self.find_leaf(target_pane)?;
        let new_leaf = self.alloc(Node::Leaf {
            pane_id: new_pane,
        });

        // Replace the old leaf with a split node
        // The old leaf stays in place as `first`, new leaf is `second`
        // NOTE: Must push directly instead of using alloc() here.
        // alloc() reuses free slots, so after take() it would reuse leaf_id,
        // creating a self-referencing node (first -> itself) -> infinite recursion.
        let old_node = self.nodes[leaf_id].take()?;
        let old_leaf_id = self.nodes.len();
        self.nodes.push(Some(old_node));
        self.nodes[leaf_id] = Some(Node::Split {
            direction,
            ratio,
            first: old_leaf_id,
            second: new_leaf,
        });

        // Fix root if needed (root stays the same since we replaced in-place)
        Some(leaf_id)
    }

    /// Remove a pane, collapsing its parent split.
    /// Returns the PaneId of the sibling that takes over (for focus).
    pub fn remove(&mut self, pane_id: PaneId) -> Option<PaneId> {
        let root = self.root?;

        // Special case: removing the only pane
        if let Some(Node::Leaf { pane_id: pid }) = self.get(root) {
            if *pid == pane_id {
                return None; // Cannot remove last pane
            }
        }

        // Find parent of the leaf
        let (parent_id, is_first) = self.find_parent(root, pane_id)?;
        let parent = self.nodes[parent_id].take()?;

        if let Node::Split {
            first, second, ..
        } = parent
        {
            let (remove_id, keep_id) = if is_first {
                (first, second)
            } else {
                (second, first)
            };

            // Move the kept subtree into the parent's slot
            let kept_node = self.nodes[keep_id].take()?;
            self.nodes[parent_id] = Some(kept_node);
            self.free(remove_id);
            self.free(keep_id);

            // Return the first leaf in the kept subtree for focus
            Some(self.first_leaf(parent_id)?)
        } else {
            None
        }
    }

    /// Find the NodeId of the leaf with the given PaneId
    fn find_leaf(&self, pane_id: PaneId) -> Option<NodeId> {
        self.find_leaf_recursive(self.root?, pane_id)
    }

    fn find_leaf_recursive(&self, node_id: NodeId, pane_id: PaneId) -> Option<NodeId> {
        match self.get(node_id)? {
            Node::Leaf { pane_id: pid } => {
                if *pid == pane_id {
                    Some(node_id)
                } else {
                    None
                }
            }
            Node::Split { first, second, .. } => {
                let f = *first;
                let s = *second;
                self.find_leaf_recursive(f, pane_id)
                    .or_else(|| self.find_leaf_recursive(s, pane_id))
            }
        }
    }

    /// Find the parent of a leaf. Returns (parent_id, is_first_child).
    fn find_parent(&self, node_id: NodeId, pane_id: PaneId) -> Option<(NodeId, bool)> {
        match self.get(node_id)? {
            Node::Leaf { .. } => None,
            Node::Split { first, second, .. } => {
                let f = *first;
                let s = *second;

                // Check if first child is the target
                if let Some(Node::Leaf { pane_id: pid }) = self.get(f) {
                    if *pid == pane_id {
                        return Some((node_id, true));
                    }
                }
                // Check if second child is the target
                if let Some(Node::Leaf { pane_id: pid }) = self.get(s) {
                    if *pid == pane_id {
                        return Some((node_id, false));
                    }
                }

                // Recurse
                self.find_parent(f, pane_id)
                    .or_else(|| self.find_parent(s, pane_id))
            }
        }
    }

    /// Get the first leaf PaneId in a subtree (leftmost/topmost)
    fn first_leaf(&self, node_id: NodeId) -> Option<PaneId> {
        match self.get(node_id)? {
            Node::Leaf { pane_id } => Some(*pane_id),
            Node::Split { first, .. } => self.first_leaf(*first),
        }
    }

    /// Collect all leaf PaneIds
    pub fn leaves(&self) -> Vec<PaneId> {
        let mut result = Vec::new();
        if let Some(root) = self.root {
            self.collect_leaves(root, &mut result);
        }
        result
    }

    fn collect_leaves(&self, node_id: NodeId, out: &mut Vec<PaneId>) {
        match self.get(node_id) {
            Some(Node::Leaf { pane_id }) => out.push(*pane_id),
            Some(Node::Split { first, second, .. }) => {
                let f = *first;
                let s = *second;
                self.collect_leaves(f, out);
                self.collect_leaves(s, out);
            }
            None => {}
        }
    }

    /// Navigate from `current_pane` in `direction`.
    /// Uses layout rects to find the nearest pane in the given direction.
    pub fn navigate(
        &self,
        current_pane: PaneId,
        direction: NavDirection,
        rects: &std::collections::HashMap<PaneId, PaneRect>,
    ) -> Option<PaneId> {
        let current_rect = rects.get(&current_pane)?;
        let cx = current_rect.x + current_rect.width / 2.0;
        let cy = current_rect.y + current_rect.height / 2.0;

        let leaves = self.leaves();
        let mut best: Option<(PaneId, f32)> = None;

        for &pid in &leaves {
            if pid == current_pane {
                continue;
            }
            let rect = match rects.get(&pid) {
                Some(r) => r,
                None => continue,
            };
            let px = rect.x + rect.width / 2.0;
            let py = rect.y + rect.height / 2.0;

            let valid = match direction {
                NavDirection::Left => px < cx,
                NavDirection::Right => px > cx,
                NavDirection::Up => py < cy,
                NavDirection::Down => py > cy,
            };
            if !valid {
                continue;
            }

            let dist = (px - cx).abs() + (py - cy).abs();
            if best.is_none() || dist < best.unwrap().1 {
                best = Some((pid, dist));
            }
        }

        best.map(|(pid, _)| pid)
    }

    /// Adjust the split ratio of the nearest ancestor with matching direction.
    /// `delta` is added to the ratio (positive = grow first child).
    /// `target_dir` specifies which split direction to look for:
    ///   - Horizontal for left/right resize
    ///   - Vertical for up/down resize
    pub fn resize_ratio(&mut self, pane_id: PaneId, delta: f32, target_dir: Direction) -> bool {
        let root = match self.root {
            Some(r) => r,
            None => return false,
        };
        let mut path = Vec::new();
        if !self.path_to_leaf(root, pane_id, &mut path) {
            return false;
        }
        // Walk path from leaf towards root, find nearest split with matching direction
        for &nid in path.iter().rev() {
            if let Some(Node::Split { direction, .. }) = self.get(nid) {
                if *direction == target_dir {
                    if let Some(Node::Split { ratio, .. }) = self.nodes[nid].as_mut() {
                        *ratio = (*ratio + delta).clamp(0.1, 0.9);
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Collect the path of node IDs from root to a leaf with the given PaneId.
    fn path_to_leaf(&self, node_id: NodeId, pane_id: PaneId, path: &mut Vec<NodeId>) -> bool {
        path.push(node_id);
        match self.get(node_id) {
            Some(Node::Leaf { pane_id: pid }) => {
                if *pid == pane_id {
                    return true;
                }
                path.pop();
                false
            }
            Some(Node::Split { first, second, .. }) => {
                let f = *first;
                let s = *second;
                if self.path_to_leaf(f, pane_id, path) {
                    return true;
                }
                if self.path_to_leaf(s, pane_id, path) {
                    return true;
                }
                path.pop();
                false
            }
            None => {
                path.pop();
                false
            }
        }
    }

    /// Count total leaves
    #[allow(dead_code)]
    pub fn leaf_count(&self) -> usize {
        self.leaves().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_single_leaf() {
        let tree = SplitTree::new(PaneId(0));
        assert_eq!(tree.leaves(), vec![PaneId(0)]);
        assert_eq!(tree.leaf_count(), 1);
    }

    #[test]
    fn test_split() {
        let mut tree = SplitTree::new(PaneId(0));
        tree.split(PaneId(0), PaneId(1), Direction::Horizontal, 0.5);
        let leaves = tree.leaves();
        assert_eq!(leaves.len(), 2);
        assert!(leaves.contains(&PaneId(0)));
        assert!(leaves.contains(&PaneId(1)));
    }

    #[test]
    fn test_nested_split() {
        let mut tree = SplitTree::new(PaneId(0));
        tree.split(PaneId(0), PaneId(1), Direction::Horizontal, 0.5);
        tree.split(PaneId(1), PaneId(2), Direction::Vertical, 0.5);
        let leaves = tree.leaves();
        assert_eq!(leaves.len(), 3);
        assert!(leaves.contains(&PaneId(0)));
        assert!(leaves.contains(&PaneId(1)));
        assert!(leaves.contains(&PaneId(2)));
    }

    #[test]
    fn test_remove() {
        let mut tree = SplitTree::new(PaneId(0));
        tree.split(PaneId(0), PaneId(1), Direction::Horizontal, 0.5);

        let focus = tree.remove(PaneId(1));
        assert_eq!(focus, Some(PaneId(0)));
        assert_eq!(tree.leaves(), vec![PaneId(0)]);
    }

    #[test]
    fn test_remove_first_child() {
        let mut tree = SplitTree::new(PaneId(0));
        tree.split(PaneId(0), PaneId(1), Direction::Horizontal, 0.5);

        let focus = tree.remove(PaneId(0));
        assert_eq!(focus, Some(PaneId(1)));
        assert_eq!(tree.leaves(), vec![PaneId(1)]);
    }

    #[test]
    fn test_cannot_remove_last_pane() {
        let mut tree = SplitTree::new(PaneId(0));
        assert_eq!(tree.remove(PaneId(0)), None);
    }

    #[test]
    fn test_navigate() {
        let tree = {
            let mut t = SplitTree::new(PaneId(0));
            t.split(PaneId(0), PaneId(1), Direction::Horizontal, 0.5);
            t
        };
        let mut rects = HashMap::new();
        rects.insert(PaneId(0), PaneRect::new(0.0, 0.0, 400.0, 600.0));
        rects.insert(PaneId(1), PaneRect::new(401.0, 0.0, 400.0, 600.0));

        assert_eq!(
            tree.navigate(PaneId(0), NavDirection::Right, &rects),
            Some(PaneId(1))
        );
        assert_eq!(
            tree.navigate(PaneId(1), NavDirection::Left, &rects),
            Some(PaneId(0))
        );
        assert_eq!(tree.navigate(PaneId(0), NavDirection::Left, &rects), None);
    }

    #[test]
    fn test_resize_ratio() {
        let mut tree = SplitTree::new(PaneId(0));
        tree.split(PaneId(0), PaneId(1), Direction::Horizontal, 0.5);

        assert!(tree.resize_ratio(PaneId(0), 0.1, Direction::Horizontal));
        // Check ratio changed
        if let Some(Node::Split { ratio, .. }) = tree.get(tree.root.unwrap()) {
            assert!((ratio - 0.6).abs() < 0.001);
        }

        // Clamp test
        assert!(tree.resize_ratio(PaneId(0), 1.0, Direction::Horizontal));
        if let Some(Node::Split { ratio, .. }) = tree.get(tree.root.unwrap()) {
            assert!((ratio - 0.9).abs() < 0.001);
        }

        // Wrong direction: should not find a matching split
        let mut tree2 = SplitTree::new(PaneId(0));
        tree2.split(PaneId(0), PaneId(1), Direction::Horizontal, 0.5);
        assert!(!tree2.resize_ratio(PaneId(0), 0.1, Direction::Vertical));
    }

    #[test]
    fn test_resize_nested_finds_ancestor() {
        // Left | Right, Right has Top / Bottom
        let mut tree = SplitTree::new(PaneId(0));
        tree.split(PaneId(0), PaneId(1), Direction::Horizontal, 0.5); // Left | Right
        tree.split(PaneId(1), PaneId(2), Direction::Vertical, 0.5);   // Right: Top / Bottom

        // From PaneId(1) (right-top), horizontal resize should adjust the outer H-split
        let root = tree.root.unwrap();
        let orig_ratio = if let Some(Node::Split { ratio, .. }) = tree.get(root) {
            *ratio
        } else {
            panic!("root should be split");
        };

        assert!(tree.resize_ratio(PaneId(1), 0.1, Direction::Horizontal));
        if let Some(Node::Split { ratio, .. }) = tree.get(root) {
            assert!((ratio - (orig_ratio + 0.1)).abs() < 0.001);
        }

        // From PaneId(2) (right-bottom), horizontal resize should also adjust outer H-split
        assert!(tree.resize_ratio(PaneId(2), -0.1, Direction::Horizontal));
        if let Some(Node::Split { ratio, .. }) = tree.get(root) {
            assert!((ratio - orig_ratio).abs() < 0.001);
        }
    }

    #[test]
    fn test_remove_from_triple_split() {
        let mut tree = SplitTree::new(PaneId(0));
        tree.split(PaneId(0), PaneId(1), Direction::Horizontal, 0.5);
        tree.split(PaneId(1), PaneId(2), Direction::Vertical, 0.5);
        assert_eq!(tree.leaf_count(), 3);

        // Remove middle pane
        let focus = tree.remove(PaneId(1));
        assert!(focus.is_some());
        assert_eq!(tree.leaf_count(), 2);
        let leaves = tree.leaves();
        assert!(leaves.contains(&PaneId(0)));
        assert!(leaves.contains(&PaneId(2)));
    }

    #[test]
    fn test_split_no_self_reference() {
        // Regression test: split() must not create self-referencing nodes
        // (which would cause infinite recursion in layout/traversal)
        let mut tree = SplitTree::new(PaneId(0));
        tree.split(PaneId(0), PaneId(1), Direction::Horizontal, 0.5);

        let root = tree.root.unwrap();
        if let Some(Node::Split { first, second, .. }) = tree.get(root) {
            assert_ne!(*first, root, "first child must not point to itself");
            assert_ne!(*second, root, "second child must not point to itself");
            assert_ne!(first, second, "children must be different nodes");
        } else {
            panic!("root should be a Split node after split()");
        }
    }
}
