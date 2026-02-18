//! Geometric drawing utilities for special characters
//!
//! This module provides pixel-perfect rendering of:
//! - Box drawing characters (U+2500-U+257F)
//! - Powerline glyphs (U+E0B0-U+E0D4)
//! - Block elements (U+2580-U+259F)
//!
//! These characters are rendered procedurally rather than from fonts
//! to ensure exact pixel alignment and seamless transitions.

pub mod geometry;

// Re-export commonly used functions
pub use geometry::{aa_alpha_from_distance, distance_to_segment, ellipse_sdf, smoothstep};
