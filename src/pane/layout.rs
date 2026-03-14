//! Layout calculation: convert split tree into pixel rectangles
//!
//! Recursively walks the binary tree, dividing the available rectangle
//! according to split direction and ratio.

use std::collections::HashMap;

use super::split_tree::{Node, NodeId, SplitTree};
use super::{Direction, PaneId, PaneRect};

/// Padding on each side of the divider line (between text and line)
pub const BORDER_PADDING: f32 = 4.0;

/// Width of the divider line itself
pub const DIVIDER_LINE_WIDTH: f32 = 1.0;

/// Total gap between panes (padding + line + padding)
pub const BORDER_WIDTH: f32 = BORDER_PADDING * 2.0 + DIVIDER_LINE_WIDTH;

/// A divider line between two panes
#[derive(Debug, Clone)]
pub struct Divider {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Calculate pixel rectangles for all panes in the tree
pub fn calculate_layout(tree: &SplitTree, available: PaneRect) -> HashMap<PaneId, PaneRect> {
    let mut result = HashMap::new();
    if let Some(root) = tree.root {
        calculate_node(tree, root, available, &mut result);
    }
    result
}

/// Calculate divider lines between panes
pub fn calculate_dividers(tree: &SplitTree, available: PaneRect) -> Vec<Divider> {
    let mut result = Vec::new();
    if let Some(root) = tree.root {
        collect_dividers(tree, root, available, &mut result);
    }
    result
}

fn calculate_node(
    tree: &SplitTree,
    node_id: NodeId,
    rect: PaneRect,
    out: &mut HashMap<PaneId, PaneRect>,
) {
    match tree.get(node_id) {
        Some(Node::Leaf { pane_id }) => {
            out.insert(*pane_id, rect);
        }
        Some(Node::Split {
            direction,
            ratio,
            first,
            second,
        }) => {
            let dir = *direction;
            let r = *ratio;
            let f = *first;
            let s = *second;
            let (first_rect, second_rect) = split_rect(rect, dir, r);
            calculate_node(tree, f, first_rect, out);
            calculate_node(tree, s, second_rect, out);
        }
        None => {}
    }
}

fn collect_dividers(
    tree: &SplitTree,
    node_id: NodeId,
    rect: PaneRect,
    out: &mut Vec<Divider>,
) {
    match tree.get(node_id) {
        Some(Node::Leaf { .. }) => {}
        Some(Node::Split {
            direction,
            ratio,
            first,
            second,
        }) => {
            let dir = *direction;
            let r = *ratio;
            let f = *first;
            let s = *second;
            let (first_rect, second_rect) = split_rect(rect, dir, r);

            // Divider line at the center of the gap between panes
            match dir {
                Direction::Horizontal => {
                    let line_x = first_rect.x + first_rect.width + BORDER_PADDING;
                    out.push(Divider {
                        x: line_x,
                        y: rect.y,
                        width: DIVIDER_LINE_WIDTH,
                        height: rect.height,
                    });
                }
                Direction::Vertical => {
                    let line_y = first_rect.y + first_rect.height + BORDER_PADDING;
                    out.push(Divider {
                        x: rect.x,
                        y: line_y,
                        width: rect.width,
                        height: DIVIDER_LINE_WIDTH,
                    });
                }
            }

            collect_dividers(tree, f, first_rect, out);
            collect_dividers(tree, s, second_rect, out);
        }
        None => {}
    }
}

/// Split a rectangle into two parts based on direction and ratio
fn split_rect(rect: PaneRect, direction: Direction, ratio: f32) -> (PaneRect, PaneRect) {
    match direction {
        Direction::Horizontal => {
            let total = rect.width - BORDER_WIDTH;
            let first_w = (total * ratio).round();
            let second_w = total - first_w;
            (
                PaneRect::new(rect.x, rect.y, first_w, rect.height),
                PaneRect::new(
                    rect.x + first_w + BORDER_WIDTH,
                    rect.y,
                    second_w,
                    rect.height,
                ),
            )
        }
        Direction::Vertical => {
            let total = rect.height - BORDER_WIDTH;
            let first_h = (total * ratio).round();
            let second_h = total - first_h;
            (
                PaneRect::new(rect.x, rect.y, rect.width, first_h),
                PaneRect::new(
                    rect.x,
                    rect.y + first_h + BORDER_WIDTH,
                    rect.width,
                    second_h,
                ),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pane::split_tree::SplitTree;

    #[test]
    fn test_single_pane_layout() {
        let tree = SplitTree::new(PaneId(0));
        let available = PaneRect::new(0.0, 0.0, 800.0, 600.0);
        let layout = calculate_layout(&tree, available);

        assert_eq!(layout.len(), 1);
        let r = layout.get(&PaneId(0)).unwrap();
        assert_eq!(r.x, 0.0);
        assert_eq!(r.y, 0.0);
        assert_eq!(r.width, 800.0);
        assert_eq!(r.height, 600.0);
    }

    #[test]
    fn test_horizontal_split_layout() {
        let mut tree = SplitTree::new(PaneId(0));
        tree.split(PaneId(0), PaneId(1), Direction::Horizontal, 0.5);

        // Total = 809 - 9 (BORDER_WIDTH) = 800, half = 400
        let available = PaneRect::new(0.0, 0.0, 809.0, 600.0);
        let layout = calculate_layout(&tree, available);

        assert_eq!(layout.len(), 2);
        let left = layout.get(&PaneId(0)).unwrap();
        let right = layout.get(&PaneId(1)).unwrap();

        assert_eq!(left.width, 400.0);
        assert_eq!(right.width, 400.0);
        assert_eq!(left.x, 0.0);
        assert_eq!(right.x, 409.0); // 400 + 9 (BORDER_WIDTH)
    }

    #[test]
    fn test_vertical_split_layout() {
        let mut tree = SplitTree::new(PaneId(0));
        tree.split(PaneId(0), PaneId(1), Direction::Vertical, 0.5);

        // Total = 609 - 9 = 600, half = 300
        let available = PaneRect::new(0.0, 0.0, 800.0, 609.0);
        let layout = calculate_layout(&tree, available);

        let top = layout.get(&PaneId(0)).unwrap();
        let bottom = layout.get(&PaneId(1)).unwrap();

        assert_eq!(top.height, 300.0);
        assert_eq!(bottom.height, 300.0);
        assert_eq!(top.y, 0.0);
        assert_eq!(bottom.y, 309.0);
    }

    #[test]
    fn test_nested_layout() {
        let mut tree = SplitTree::new(PaneId(0));
        tree.split(PaneId(0), PaneId(1), Direction::Horizontal, 0.5);
        tree.split(PaneId(1), PaneId(2), Direction::Vertical, 0.5);

        let available = PaneRect::new(0.0, 0.0, 809.0, 609.0);
        let layout = calculate_layout(&tree, available);

        assert_eq!(layout.len(), 3);
        let left = layout.get(&PaneId(0)).unwrap();
        let top_right = layout.get(&PaneId(1)).unwrap();
        let bottom_right = layout.get(&PaneId(2)).unwrap();

        // Left pane: full height, half width
        assert_eq!(left.width, 400.0);
        assert_eq!(left.height, 609.0);

        // Top-right: half of right area
        assert_eq!(top_right.x, 409.0);
        assert!(top_right.height > 0.0);

        // Bottom-right: below top-right
        assert_eq!(bottom_right.x, 409.0);
        assert!(bottom_right.y > top_right.y);
    }

    #[test]
    fn test_dividers_single_pane() {
        let tree = SplitTree::new(PaneId(0));
        let available = PaneRect::new(0.0, 0.0, 800.0, 600.0);
        let dividers = calculate_dividers(&tree, available);
        assert!(dividers.is_empty());
    }

    #[test]
    fn test_dividers_horizontal_split() {
        let mut tree = SplitTree::new(PaneId(0));
        tree.split(PaneId(0), PaneId(1), Direction::Horizontal, 0.5);

        let available = PaneRect::new(0.0, 0.0, 809.0, 600.0);
        let dividers = calculate_dividers(&tree, available);

        assert_eq!(dividers.len(), 1);
        let div = &dividers[0];
        // Divider at center of gap: left_width(400) + padding(4) = 404
        assert_eq!(div.x, 404.0);
        assert_eq!(div.y, 0.0);
        assert_eq!(div.width, DIVIDER_LINE_WIDTH);
        assert_eq!(div.height, 600.0);
    }

    #[test]
    fn test_dividers_nested() {
        let mut tree = SplitTree::new(PaneId(0));
        tree.split(PaneId(0), PaneId(1), Direction::Horizontal, 0.5);
        tree.split(PaneId(1), PaneId(2), Direction::Vertical, 0.5);

        let available = PaneRect::new(0.0, 0.0, 809.0, 609.0);
        let dividers = calculate_dividers(&tree, available);

        // 2 dividers: vertical (between left and right) + horizontal (between top-right and bottom-right)
        assert_eq!(dividers.len(), 2);
    }
}
