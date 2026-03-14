//! Pane management: split tree, tabs, and layout
//!
//! Provides a Ghostty-style binary tree for pane splitting,
//! tab management, and layout calculation.

pub mod layout;
pub mod split_tree;
pub mod tab;

use crate::terminal::Terminal;

/// Unique identifier for a pane
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PaneId(pub u16);

/// Pixel rectangle for a pane's viewport
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaneRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl PaneRect {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Check if a pixel coordinate is within this rect
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.x + self.width && py >= self.y && py < self.y + self.height
    }
}

/// Split direction
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Horizontal, // left | right
    Vertical,   // top / bottom
}

/// Navigation direction for moving between panes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavDirection {
    Left,
    Right,
    Up,
    Down,
}

/// A single pane holding a terminal instance
pub struct Pane {
    #[allow(dead_code)]
    pub id: PaneId,
    pub terminal: Terminal,
    pub rect: PaneRect,
}

impl Pane {
    pub fn new(id: PaneId, terminal: Terminal, rect: PaneRect) -> Self {
        Self { id, terminal, rect }
    }
}
