//! Global constants for bcon
//!
//! Consolidates timing, rendering, and Unicode range constants
//! to eliminate magic numbers throughout the codebase.

#![allow(dead_code)]

// ============================================================================
// Timing Constants
// ============================================================================

/// Double-click detection threshold in milliseconds
pub const DOUBLE_CLICK_THRESHOLD_MS: u128 = 300;

/// Visual bell flash duration in milliseconds
pub const BELL_FLASH_DURATION_MS: u64 = 100;

/// Cursor blink interval in milliseconds (~530ms is standard)
pub const CURSOR_BLINK_INTERVAL_MS: u64 = 530;

// ============================================================================
// XKB Modifier Bits
// ============================================================================

/// Shift modifier bit
pub const XKB_MOD_SHIFT: u32 = 0x1;

/// Control modifier bit
pub const XKB_MOD_CONTROL: u32 = 0x4;

/// Alt (Mod1) modifier bit
pub const XKB_MOD_ALT: u32 = 0x8;

// ============================================================================
// Rendering Constants
// ============================================================================

/// Box drawing line thickness scale relative to cell height
pub const LINE_THICKNESS_SCALE: f32 = 0.12;

/// Anti-aliasing width for solid powerline shapes (triangles, semicircles)
pub const AA_WIDTH_SOLID: f32 = 1.5;

/// Anti-aliasing width for outline shapes
pub const AA_WIDTH_OUTLINE: f32 = 0.7;

/// Half-width of outline strokes
pub const OUTLINE_STROKE_HALF: f32 = 0.35;

/// Alpha threshold for rendering pixels (below this = skip)
pub const ALPHA_THRESHOLD: f32 = 0.01;

/// Alpha threshold for outline rendering
pub const ALPHA_THRESHOLD_OUTLINE: f32 = 0.02;

/// Minimum font size (pixels)
pub const MIN_FONT_SIZE: f32 = 8.0;

/// Maximum font size (pixels)
pub const MAX_FONT_SIZE: f32 = 72.0;

/// Minimum display scale factor
pub const MIN_DISPLAY_SCALE: f32 = 0.5;

/// Maximum display scale factor
pub const MAX_DISPLAY_SCALE: f32 = 4.0;

// ============================================================================
// Underline Style Constants
// ============================================================================

/// Dotted underline: dot size in pixels
pub const DOTTED_LINE_DOT_SIZE: f32 = 1.0;

/// Dotted underline: gap between dots in pixels
pub const DOTTED_LINE_GAP: f32 = 2.0;

/// Dashed underline: dash length in pixels
pub const DASHED_LINE_DASH_SIZE: f32 = 4.0;

/// Dashed underline: gap between dashes in pixels
pub const DASHED_LINE_GAP: f32 = 2.0;

// ============================================================================
// Unicode Ranges for Special Character Rendering
// ============================================================================

/// Powerline symbols range (U+E0B0 - U+E0D4)
/// Includes arrows, rounded separators, and other powerline glyphs
pub const POWERLINE_RANGE_START: u32 = 0xE0B0;
pub const POWERLINE_RANGE_END: u32 = 0xE0D4;

/// Block elements range (U+2580 - U+259F)
/// Full block, half blocks, eighth blocks, quarter blocks, shade characters
pub const BLOCK_ELEMENT_RANGE_START: u32 = 0x2580;
pub const BLOCK_ELEMENT_RANGE_END: u32 = 0x259F;

/// Box drawing characters range (U+2500 - U+257F)
/// Light and heavy lines, corners, T-junctions, crosses
pub const BOX_DRAWING_RANGE_START: u32 = 0x2500;
pub const BOX_DRAWING_RANGE_END: u32 = 0x257F;

/// Braille patterns range (U+2800 - U+28FF)
pub const BRAILLE_RANGE_START: u32 = 0x2800;
pub const BRAILLE_RANGE_END: u32 = 0x28FF;

// ============================================================================
// Helper Functions for Unicode Range Checks
// ============================================================================

/// Check if a code point is in the Powerline symbols range
#[inline]
pub const fn is_powerline(cp: u32) -> bool {
    cp >= POWERLINE_RANGE_START && cp <= POWERLINE_RANGE_END
}

/// Check if a code point is a block element
#[inline]
pub const fn is_block_element(cp: u32) -> bool {
    cp >= BLOCK_ELEMENT_RANGE_START && cp <= BLOCK_ELEMENT_RANGE_END
}

/// Check if a code point is a box drawing character
#[inline]
pub const fn is_box_drawing(cp: u32) -> bool {
    cp >= BOX_DRAWING_RANGE_START && cp <= BOX_DRAWING_RANGE_END
}

/// Check if a code point is a Braille pattern
#[inline]
pub const fn is_braille(cp: u32) -> bool {
    cp >= BRAILLE_RANGE_START && cp <= BRAILLE_RANGE_END
}

/// Check if character is a Powerline, block, or transition glyph
/// These glyphs create visual transitions between background colors
#[inline]
pub const fn is_transition_char(cp: u32) -> bool {
    is_powerline(cp) || is_block_element(cp)
}
