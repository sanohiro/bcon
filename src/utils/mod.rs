//! Utility functions shared across bcon
//!
//! Common helpers that don't fit in specialized modules.

pub mod color;

pub use color::{parse_hex_color, parse_hex_color_to_f32, parse_osc_color};
