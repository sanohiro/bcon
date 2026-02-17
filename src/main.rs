//! bcon - GPU-accelerated terminal for Linux console
//!
//! # Architecture
//!
//! ```text
//! ┌──────────────────────────────────────────┐
//! │              Event Loop                  │
//! ├──────────────────────────────────────────┤
//! │  Input (evdev)  →  Terminal (VT/PTY)    │
//! │                          ↓               │
//! │              GPU Renderer (OpenGL ES)    │
//! │                          ↓               │
//! │              DRM/KMS Output              │
//! └──────────────────────────────────────────┘
//! ```

mod config;
mod drm;
mod font;
mod gpu;
mod input;
mod session;
mod terminal;

use anyhow::{anyhow, Context, Result};
use glow::HasContext;
use log::{debug, info, trace, warn};
use std::time::Duration;

#[cfg(all(target_os = "linux", feature = "seatd"))]
use std::cell::RefCell;
#[cfg(all(target_os = "linux", feature = "seatd"))]
use std::rc::Rc;

// ============================================================================
// Constants
// ============================================================================

/// Box drawing line thickness scale relative to cell height
const LINE_THICKNESS_SCALE: f32 = 0.12;

/// Anti-aliasing width for solid powerline shapes (triangles, semicircles)
const AA_WIDTH_SOLID: f32 = 1.5;

/// Anti-aliasing width for outline shapes
const AA_WIDTH_OUTLINE: f32 = 0.7;

/// Half-width of outline strokes
const OUTLINE_STROKE_HALF: f32 = 0.35;

/// Alpha threshold for rendering pixels (below this = skip)
const ALPHA_THRESHOLD: f32 = 0.01;

/// Alpha threshold for outline rendering
const ALPHA_THRESHOLD_OUTLINE: f32 = 0.02;

/// Minimum font size (pixels)
const MIN_FONT_SIZE: f32 = 8.0;

/// Maximum font size (pixels)
const MAX_FONT_SIZE: f32 = 72.0;

/// Minimum display scale factor
const MIN_DISPLAY_SCALE: f32 = 0.5;

/// Maximum display scale factor
const MAX_DISPLAY_SCALE: f32 = 4.0;

// ============================================================================
// Graphics Helper Functions
// ============================================================================

/// Smoothstep interpolation for anti-aliasing.
/// Returns smooth transition from 0 to 1 as t goes from 0 to 1.
/// Formula: 3t² - 2t³ (Hermite interpolation)
#[inline]
fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// Compute anti-aliased alpha from signed distance.
/// d > 0: inside shape (alpha = 1.0)
/// d < 0: outside shape, with smooth falloff
#[inline]
fn aa_alpha_from_distance(d: f32, aa_width: f32) -> f32 {
    if d >= 0.0 {
        1.0
    } else {
        let t = (d / aa_width + 1.0).clamp(0.0, 1.0);
        smoothstep(t)
    }
}

/// Compute SDF (signed distance field) for an ellipse.
/// Positive inside, negative outside.
/// Uses approximation for arbitrary ellipse (not exact but visually correct).
///
/// # Arguments
/// * `nx`, `ny` - normalized coordinates (point / radius)
/// * `rx`, `ry` - ellipse radii
/// * `len` - length of normalized vector (precomputed)
#[inline]
fn ellipse_sdf(nx: f32, ny: f32, rx: f32, ry: f32, len: f32) -> f32 {
    if len <= 0.001 {
        return -rx.min(ry);
    }
    // Approximate gradient-based SDF
    let k = (rx * ry) / (rx * ny.abs() + ry * nx.abs()).max(0.001);
    (len - 1.0) * k.min(rx.min(ry))
}

/// Calculate distance from point to line segment.
/// Used for outline rendering of triangular shapes.
///
/// # Arguments
/// * `px`, `py` - point coordinates
/// * `ax`, `ay` - segment start
/// * `bx`, `by` - segment end
#[inline]
fn distance_to_segment(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let vx = bx - ax;
    let vy = by - ay;
    let wx = px - ax;
    let wy = py - ay;

    // Project P onto line AB
    let c1 = vx * wx + vy * wy;  // dot(v, w)
    if c1 <= 0.0 {
        // Before segment start
        return (wx * wx + wy * wy).sqrt();
    }

    let c2 = vx * vx + vy * vy;  // dot(v, v) = |v|²
    if c2 <= c1 {
        // After segment end
        let dx = px - bx;
        let dy = py - by;
        return (dx * dx + dy * dy).sqrt();
    }

    // Projection falls within segment
    let t = c1 / c2;
    let proj_x = ax + t * vx;
    let proj_y = ay + t * vy;
    let dx = px - proj_x;
    let dy = py - proj_y;
    (dx * dx + dy * dy).sqrt()
}

/// sRGB to linear conversion (same calculation as shader)
#[allow(dead_code)]
fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Linear to sRGB conversion (same calculation as shader)
#[allow(dead_code)]
fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// Check if character is a Powerline, block, or rounded corner transition glyph
/// These glyphs create visual transitions between background colors
#[inline]
fn is_transition_char(ch: char) -> bool {
    let cp = ch as u32;
    matches!(cp,
        // Powerline symbols: U+E0B0-U+E0D4 (arrows, rounded, etc.)
        0xE0B0..=0xE0D4 |
        // Block elements: U+2580-U+259F
        // Full block, half blocks, eighth blocks, quarter blocks
        0x2580..=0x259F
    )
}

/// Draw box-drawing characters programmatically for pixel-perfect alignment.
/// Uses procedural rendering to ensure exact pixel alignment regardless of font.
fn draw_box_drawing(
    ch: char,
    x: f32,
    y: f32,
    cell_w: f32,
    cell_h: f32,
    fg: [f32; 4],
    atlas: &crate::font::lcd_atlas::LcdGlyphAtlas,
    tr: &mut crate::gpu::LcdTextRendererInstanced,
) -> bool {
    // Line thickness: proportional to cell height, clamped for visibility
    let t = (cell_h * LINE_THICKNESS_SCALE).clamp(1.0, 2.0).round();
    // Line centers - where the middle of the line stroke sits
    let line_cx = (x + (cell_w - t) * 0.5).round() + t * 0.5;
    let line_cy = (y + (cell_h - t) * 0.5).round() + t * 0.5;
    // Rectangle positions for lines
    let rect_x = (x + (cell_w - t) * 0.5).round();
    let rect_y = (y + (cell_h - t) * 0.5).round();

    // First try rounded corners with line center info
    if draw_rounded_corner(ch, x, y, cell_w, cell_h, t, line_cx, line_cy, fg, atlas, tr) {
        return true;
    }

    match ch {
        '─' => {
            tr.push_rect(x, rect_y, cell_w, t, fg, atlas);
            true
        }
        '│' => {
            tr.push_rect(rect_x, y, t, cell_h, fg, atlas);
            true
        }
        '┌' => {
            tr.push_rect(line_cx, rect_y, x + cell_w - line_cx, t, fg, atlas);
            tr.push_rect(rect_x, line_cy, t, y + cell_h - line_cy, fg, atlas);
            true
        }
        '┐' => {
            tr.push_rect(x, rect_y, line_cx - x, t, fg, atlas);
            tr.push_rect(rect_x, line_cy, t, y + cell_h - line_cy, fg, atlas);
            true
        }
        '└' => {
            tr.push_rect(line_cx, rect_y, x + cell_w - line_cx, t, fg, atlas);
            tr.push_rect(rect_x, y, t, line_cy - y, fg, atlas);
            true
        }
        '┘' => {
            tr.push_rect(x, rect_y, line_cx - x, t, fg, atlas);
            tr.push_rect(rect_x, y, t, line_cy - y, fg, atlas);
            true
        }
        '├' => {
            tr.push_rect(line_cx, rect_y, x + cell_w - line_cx, t, fg, atlas);
            tr.push_rect(rect_x, y, t, cell_h, fg, atlas);
            true
        }
        '┤' => {
            tr.push_rect(x, rect_y, line_cx - x, t, fg, atlas);
            tr.push_rect(rect_x, y, t, cell_h, fg, atlas);
            true
        }
        '┬' => {
            tr.push_rect(x, rect_y, cell_w, t, fg, atlas);
            tr.push_rect(rect_x, line_cy, t, y + cell_h - line_cy, fg, atlas);
            true
        }
        '┴' => {
            tr.push_rect(x, rect_y, cell_w, t, fg, atlas);
            tr.push_rect(rect_x, y, t, line_cy - y, fg, atlas);
            true
        }
        '┼' => {
            tr.push_rect(x, rect_y, cell_w, t, fg, atlas);
            tr.push_rect(rect_x, y, t, cell_h, fg, atlas);
            true
        }
        _ => false,
    }
}

/// Draw powerline triangle (E0B0/E0B2 solid, with anti-aliasing).
/// Used for status line separators in shells like fish, zsh with powerline themes.
///
/// # Arguments
/// * `right` - true for right-pointing (▶), false for left-pointing (◀)
/// * `width_scale` - 1.0 = full width, 0.5 = thin variant
fn draw_powerline_triangle(
    x: f32,
    y: f32,
    cell_w: f32,
    cell_h: f32,
    fg: [f32; 4],
    bg: [f32; 3],
    atlas: &crate::font::lcd_atlas::LcdGlyphAtlas,
    tr: &mut crate::gpu::LcdTextRendererInstanced,
    right: bool,
    width_scale: f32,
) {
    let steps = cell_h as i32;
    let half = cell_h / 2.0;
    let max_w_px = (cell_w * width_scale - 1.0).max(1.0);
    let draw_w = max_w_px.floor() + 1.0;
    let start_x = if right && width_scale < 0.999 {
        x + (cell_w - draw_w)
    } else {
        x
    };
    let max_w_i = max_w_px.floor() as i32;
    let inv_half = if half > 0.0 { 1.0 / half } else { 0.0 };

    for i in 0..steps {
        let row_y = y + i as f32;
        // Calculate edge position based on vertical position (triangle shape)
        let dist = ((i as f32 + 0.5) - half).abs();
        let edge = (max_w_px * (1.0 - dist * inv_half) + 0.5).clamp(0.5, max_w_px + 0.5);

        for px in 0..=max_w_i {
            let px_center = px as f32 + 0.5;
            // Signed distance: positive inside triangle, negative outside
            let d = if right {
                edge - px_center
            } else {
                px_center - ((max_w_px + 0.5) - edge)
            };
            let alpha = aa_alpha_from_distance(d, AA_WIDTH_SOLID);
            if alpha > ALPHA_THRESHOLD {
                let aa_fg = [fg[0], fg[1], fg[2], fg[3] * alpha];
                tr.push_rect_with_bg(start_x + px as f32, row_y, 1.0, 1.0, aa_fg, bg, atlas);
            }
        }
    }
}

/// Draw powerline triangle outline (E0B1/E0B3).
/// Renders only the diagonal edges of the triangle shape.
fn draw_powerline_triangle_outline(
    x: f32,
    y: f32,
    cell_w: f32,
    cell_h: f32,
    fg: [f32; 4],
    bg: [f32; 3],
    atlas: &crate::font::lcd_atlas::LcdGlyphAtlas,
    tr: &mut crate::gpu::LcdTextRendererInstanced,
    right: bool,
) {
    let w_px = (cell_w - 1.0).max(1.0);
    let h_px = (cell_h - 1.0).max(1.0);
    let half = h_px * 0.5;

    // Define triangle vertices: base on one side, tip on the other
    let (x0, y0, x1, y1, x2, y2) = if right {
        // Right-pointing: tip at (w, half)
        (0.5, 0.5, w_px + 0.5, half + 0.5, 0.5, h_px + 0.5)
    } else {
        // Left-pointing: tip at (0, half)
        (w_px + 0.5, 0.5, 0.5, half + 0.5, w_px + 0.5, h_px + 0.5)
    };

    let ww = w_px as i32 + 1;
    let hh = h_px as i32 + 1;

    for py in 0..hh {
        for px in 0..ww {
            let px_f = px as f32 + 0.5;
            let py_f = py as f32 + 0.5;

            // Distance to either diagonal edge
            let d1 = distance_to_segment(px_f, py_f, x0, y0, x1, y1);
            let d2 = distance_to_segment(px_f, py_f, x2, y2, x1, y1);
            let d = d1.min(d2);

            // Skip pixels far from the outline
            if d > OUTLINE_STROKE_HALF + AA_WIDTH_OUTLINE {
                continue;
            }

            // Linear falloff from stroke edge
            let alpha = if d <= OUTLINE_STROKE_HALF {
                1.0
            } else {
                1.0 - (d - OUTLINE_STROKE_HALF) / AA_WIDTH_OUTLINE
            };

            if alpha > ALPHA_THRESHOLD_OUTLINE {
                let aa_fg = [fg[0], fg[1], fg[2], fg[3] * alpha];
                tr.push_rect_with_bg(x + px as f32, y + py as f32, 1.0, 1.0, aa_fg, bg, atlas);
            }
        }
    }
}

/// Draw powerline semicircle (E0B4/E0B6 solid).
/// Renders a filled semicircle for rounded powerline separators.
///
/// # Arguments
/// * `right` - true for right-pointing, false for left-pointing
/// * `width_scale` - 1.0 = full width, 0.5 = thin variant
fn draw_powerline_semicircle(
    x: f32,
    y: f32,
    cell_w: f32,
    cell_h: f32,
    fg: [f32; 4],
    bg: [f32; 3],
    atlas: &crate::font::lcd_atlas::LcdGlyphAtlas,
    tr: &mut crate::gpu::LcdTextRendererInstanced,
    right: bool,
    width_scale: f32,
) {
    let steps = cell_h as i32;
    let h_px = (cell_h - 1.0).max(1.0);
    let cy = h_px * 0.5 + 0.5;
    let ry = (h_px * 0.5).max(1.0);
    let max_w_px = (cell_w * width_scale - 1.0).max(1.0);
    let rx = max_w_px + 0.5;
    let draw_w = max_w_px.floor() + 1.0;
    let start_x = if right && width_scale < 0.999 {
        x + (cell_w - draw_w)
    } else {
        x
    };
    let rx_i = max_w_px.floor() as i32;

    for i in 0..steps {
        let row_y = y + i as f32;
        let py = (i as f32 + 0.5) - cy;

        for px in 0..=rx_i {
            let px_rel = px as f32 + 0.5;
            let px_f = if right { px_rel } else { (max_w_px + 0.5) - px_rel };

            // Normalized ellipse coordinates
            let nx = px_f / rx;
            let ny = py / ry;
            let len = (nx * nx + ny * ny).sqrt();

            // SDF: negative inside ellipse, positive outside
            let sdf = ellipse_sdf(nx, ny, rx, ry, len);
            let d = -sdf;  // Flip: positive inside for aa_alpha_from_distance

            let alpha = aa_alpha_from_distance(d, AA_WIDTH_SOLID);
            if alpha > ALPHA_THRESHOLD {
                let aa_fg = [fg[0], fg[1], fg[2], fg[3] * alpha];
                tr.push_rect_with_bg(start_x + px as f32, row_y, 1.0, 1.0, aa_fg, bg, atlas);
            }
        }
    }
}

/// Draw powerline semicircle outline (E0B5/E0B7).
/// Renders only the curved edge of the semicircle.
fn draw_powerline_semicircle_outline(
    x: f32,
    y: f32,
    cell_w: f32,
    cell_h: f32,
    fg: [f32; 4],
    bg: [f32; 3],
    atlas: &crate::font::lcd_atlas::LcdGlyphAtlas,
    tr: &mut crate::gpu::LcdTextRendererInstanced,
    right: bool,
) {
    let steps = cell_h as i32;
    let ry = (cell_h - 1.0).max(1.0) * 0.5;
    let cy = ry + 0.5;
    let max_w_px = (cell_w - 1.0).max(1.0);
    let rx = max_w_px + 0.5;
    let rx_i = max_w_px.floor() as i32;

    for i in 0..steps {
        let row_y = y + i as f32;
        let py = (i as f32 + 0.5) - cy;

        for px in 0..=rx_i {
            let px_rel = px as f32 + 0.5;
            let px_f = if right { px_rel } else { (max_w_px + 0.5) - px_rel };

            // Normalized ellipse coordinates
            let nx = px_f / rx;
            let ny = py / ry;
            let len = (nx * nx + ny * ny).sqrt();

            let sdf = ellipse_sdf(nx, ny, rx, ry, len);
            let d = sdf.abs();  // Distance to ellipse edge

            // Skip pixels far from the outline
            if d > OUTLINE_STROKE_HALF + AA_WIDTH_OUTLINE {
                continue;
            }

            // Linear falloff from stroke edge
            let alpha = if d <= OUTLINE_STROKE_HALF {
                1.0
            } else {
                1.0 - (d - OUTLINE_STROKE_HALF) / AA_WIDTH_OUTLINE
            };

            if alpha > ALPHA_THRESHOLD_OUTLINE {
                let aa_fg = [fg[0], fg[1], fg[2], fg[3] * alpha];
                tr.push_rect_with_bg(x + px as f32, row_y, 1.0, 1.0, aa_fg, bg, atlas);
            }
        }
    }
}

/// Draw rounded corner with anti-aliased arc aligned to line centers
fn draw_rounded_corner(
    ch: char,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    t: f32,
    line_cx: f32,
    line_cy: f32,
    fg: [f32; 4],
    atlas: &crate::font::lcd_atlas::LcdGlyphAtlas,
    tr: &mut crate::gpu::LcdTextRendererInstanced,
) -> bool {
    let half_t = t * 0.5;
    let aa = 0.75_f32.max(half_t);

    // Cell boundaries
    let left = x;
    let right = x + w;
    let top = y;
    let bottom = y + h;
    let rect_x = (x + (w - t) * 0.5).round();
    let rect_y = (y + (h - t) * 0.5).round();

    // Arc center at correct corner, radius = distance from line center to corner
    // ╭: top-left corner, lines go right & down
    // ╮: top-right corner, lines go left & down
    // ╯: bottom-right corner, lines go left & up
    // ╰: bottom-left corner, lines go right & up
    let (arc_cx, arc_cy, r, qx, qy) = match ch {
        '╭' => {
            let r = (line_cx - left).min(line_cy - top) - half_t;
            (left + half_t, top + half_t, r.max(1.0), 1.0f32, 1.0f32)
        }
        '╮' => {
            let r = (right - line_cx).min(line_cy - top) - half_t;
            (right - half_t, top + half_t, r.max(1.0), -1.0f32, 1.0f32)
        }
        '╯' => {
            let r = (right - line_cx).min(bottom - line_cy) - half_t;
            (
                right - half_t,
                bottom - half_t,
                r.max(1.0),
                -1.0f32,
                -1.0f32,
            )
        }
        '╰' => {
            let r = (line_cx - left).min(bottom - line_cy) - half_t;
            (left + half_t, bottom - half_t, r.max(1.0), 1.0f32, -1.0f32)
        }
        _ => return false,
    };

    let ww = w.ceil() as i32;
    let hh = h.ceil() as i32;

    // Draw anti-aliased arc
    for py in 0..hh {
        for px in 0..ww {
            let px_f = x + px as f32 + 0.5;
            let py_f = y + py as f32 + 0.5;

            let dx = px_f - arc_cx;
            let dy = py_f - arc_cy;

            // Quadrant constraint: only draw in the correct quadrant from arc center
            if dx * qx < 0.0 || dy * qy < 0.0 {
                continue;
            }
            // Clip to interior side of the line centers to avoid outside artifacts
            let inside = match ch {
                '╭' => px_f >= line_cx && py_f >= line_cy,
                '╮' => px_f <= line_cx && py_f >= line_cy,
                '╯' => px_f <= line_cx && py_f <= line_cy,
                '╰' => px_f >= line_cx && py_f <= line_cy,
                _ => true,
            };
            if !inside {
                continue;
            }
            let dist = (dx * dx + dy * dy).sqrt();
            let d = (dist - r).abs();
            if d > half_t + aa {
                continue;
            }
            let alpha = if d <= half_t {
                1.0
            } else {
                1.0 - (d - half_t) / aa
            }
            .powf(1.5);
            if alpha > 0.02 {
                tr.push_rect(
                    x + px as f32,
                    y + py as f32,
                    1.0,
                    1.0,
                    [fg[0], fg[1], fg[2], fg[3] * alpha],
                    atlas,
                );
            }
        }
    }

    // Connection stubs to ensure no gaps with adjacent cells
    match ch {
        '╭' => {
            // Horizontal stub: from line_cx to right edge
            tr.push_rect(line_cx, rect_y, right - line_cx, t, fg, atlas);
            // Vertical stub: from line_cy to bottom edge
            tr.push_rect(rect_x, line_cy, t, bottom - line_cy, fg, atlas);
        }
        '╮' => {
            // Horizontal stub: from left edge to line_cx
            tr.push_rect(left, rect_y, line_cx - left, t, fg, atlas);
            // Vertical stub: from line_cy to bottom edge
            tr.push_rect(rect_x, line_cy, t, bottom - line_cy, fg, atlas);
        }
        '╯' => {
            // Horizontal stub: from left edge to line_cx
            tr.push_rect(left, rect_y, line_cx - left, t, fg, atlas);
            // Vertical stub: from top edge to line_cy
            tr.push_rect(rect_x, top, t, line_cy - top, fg, atlas);
        }
        '╰' => {
            // Horizontal stub: from line_cx to right edge
            tr.push_rect(line_cx, rect_y, right - line_cx, t, fg, atlas);
            // Vertical stub: from top edge to line_cy
            tr.push_rect(rect_x, top, t, line_cy - top, fg, atlas);
        }
        _ => {}
    }

    true
}

/// Auto-detect DRM device
fn find_drm_device() -> Result<String> {
    for i in 0..8 {
        let path = format!("/dev/dri/card{}", i);
        if std::path::Path::new(&path).exists() {
            return Ok(path);
        }
    }
    Err(anyhow!("/dev/dri/card* not found"))
}

/// Load font for testing: BCON_FONT env var -> ligature font -> system font
fn load_test_font() -> Result<Vec<u8>> {
    // Use environment variable if specified
    if let Ok(path) = std::env::var("BCON_FONT") {
        let data = std::fs::read(&path)
            .with_context(|| format!("Cannot read font specified by BCON_FONT: {}", path))?;
        eprintln!("Font: {} (BCON_FONT)", path);
        return Ok(data);
    }

    // Search for ligature-capable fonts first
    let ligature_fonts = [
        "/usr/share/fonts/truetype/firacode/FiraCode-Regular.ttf",
        "/usr/share/fonts/truetype/firacode/FiraCode-Retina.ttf",
        "/usr/share/fonts/opentype/firacode/FiraCode-Regular.otf",
        "/Library/Fonts/FiraCode-Regular.ttf",
        "/usr/share/fonts/JetBrainsMono-Regular.ttf",
        "/usr/share/fonts/truetype/jetbrains-mono/JetBrainsMono-Regular.ttf",
    ];

    for path in &ligature_fonts {
        if let Ok(data) = std::fs::read(path) {
            eprintln!("Font: {} (ligature-capable)", path);
            return Ok(data);
        }
    }

    // Fallback: system font
    eprintln!("Ligature font not found - falling back to system font");
    font::atlas::load_system_font()
}

/// Print help message
fn print_help() {
    println!(
        r#"bcon {} - GPU-accelerated terminal emulator for Linux console

USAGE:
    bcon [OPTIONS]

OPTIONS:
    -h, --help              Print this help message
    -V, --version           Print version information
    -t, --test              Test mode (verify build without DRM)
    --init-config[=PRESET]  Generate config file with optional preset
    -f, --force             Overwrite config file without confirmation
    --test-shaper           Test font shaping (debug mode)

PRESETS (for --init-config):
    default    Standard keybinds (Ctrl+Shift+C/V, etc.)
    vim        Vim-like scroll (Ctrl+Shift+U/D)
    emacs      Emacs-like scroll (Alt+Shift+V/N)
    japanese   CJK fonts + IME auto-disable (alias: jp)

    Combine presets with comma: --init-config=vim,jp

EXAMPLES:
    bcon                              Run bcon (requires TTY, not X11/Wayland)
    bcon --init-config                Generate default config
    bcon --init-config=vim,jp         Generate config with vim and japanese presets
    bcon --init-config=vim,jp --force Overwrite existing config
    sudo bcon                         Run with root privileges (required for DRM)

CONFIG FILE:
    ~/.config/bcon/config.toml

For more information, see: https://github.com/sanohiro/bcon
"#,
        env!("CARGO_PKG_VERSION")
    );
}

/// Shaper test mode: verify text shaping without GPU
fn test_shaper_mode() -> Result<()> {
    eprintln!("=== Text Shaper Test ===\n");

    // Use BCON_FONT env var or prioritize ligature fonts
    let font_data: &'static [u8] = Box::leak(
        load_test_font()
            .context("Failed to load font")?
            .into_boxed_slice(),
    );
    let cjk_font_data: Option<&'static [u8]> =
        font::atlas::load_cjk_font().map(|d| -> &'static [u8] { Box::leak(d.into_boxed_slice()) });

    // fontdue Font (for glyph existence check)
    let font_main = fontdue::Font::from_bytes(font_data, fontdue::FontSettings::default())
        .map_err(|e| anyhow!("Failed to load font: {}", e))?;

    eprintln!("Font loaded");

    // Create shaper
    let mut shaper = match font::shaper::TextShaper::new(font_data, cjk_font_data) {
        Some(s) => s,
        None => {
            eprintln!("Failed to create shaper");
            return Ok(());
        }
    };
    eprintln!("Shaper initialized\n");

    // Test strings
    let test_strings = [
        "val == 0",
        "a != b",
        "fn -> bool",
        "x <= y",
        "a => b",
        "<=>",
        "hello world",
        "abc 日本語 xyz",
        "// comment",
        "www",
        "|> >>= <|>",
    ];

    let cols = 80;
    let rows = 1;
    let mut grid = terminal::grid::Grid::new(cols, rows);

    for test in &test_strings {
        // Clear grid
        grid.erase_in_display(2);
        grid.move_cursor_to(1, 1);

        // Write characters to grid
        for ch in test.chars() {
            grid.put_char(ch);
        }

        // Execute shaping
        let shaped = shaper.shape_line(&grid, 0, &font_main);

        eprintln!("Input: \"{}\"", test);
        eprintln!("  Shaping result ({} glyphs):", shaped.len());

        let mut has_ligature = false;
        let mut has_calt = false;
        for (col, sg) in &shaped {
            // Compare with default glyph ID (without shaping)
            let default_gid = font_main.lookup_glyph_index(sg.ch);
            let is_substituted =
                sg.key.glyph_id != 0 && default_gid != 0 && sg.key.glyph_id != default_gid;
            let is_merged = sg.cell_span > 1;

            let marker = if is_merged {
                has_ligature = true;
                " <- ligature (merged)"
            } else if is_substituted {
                has_calt = true;
                " <- calt substitution!"
            } else {
                ""
            };
            eprintln!(
                "    col={:<3} glyph_id={:<5} (default={:<5}) span={} ch='{}'{}",
                col, sg.key.glyph_id, default_gid, sg.cell_span, sg.ch, marker
            );
        }
        if has_ligature || has_calt {
            let kind = if has_ligature && has_calt {
                "merged ligature + calt substitution"
            } else if has_ligature {
                "merged ligature"
            } else {
                "calt context substitution"
            };
            eprintln!("  => Shaping effect detected ({})", kind);
        } else {
            eprintln!("  (No shaping effect)");
        }
        eprintln!();
    }

    eprintln!("=== Test complete ===");
    Ok(())
}

/// Update text selection range with Shift+Arrow keys
fn handle_selection_key(term: &mut terminal::Terminal, keycode: u32, cols: usize) {
    let cur_row = term.grid.cursor_row;
    let cur_col = term.grid.cursor_col;
    let rows = term.grid.rows();

    // Start selection with cursor position as anchor if no selection exists
    let sel = term.selection.get_or_insert(terminal::Selection {
        anchor_row: cur_row,
        anchor_col: cur_col,
        end_row: cur_row,
        end_col: cur_col,
    });

    // evdev keycodes
    const KEY_LEFT: u32 = 105;
    const KEY_RIGHT: u32 = 106;
    const KEY_UP: u32 = 103;
    const KEY_DOWN: u32 = 108;
    const KEY_HOME: u32 = 102;
    const KEY_END: u32 = 107;

    match keycode {
        KEY_LEFT => {
            if sel.end_col > 0 {
                sel.end_col -= 1;
            } else if sel.end_row > 0 {
                sel.end_row -= 1;
                sel.end_col = cols.saturating_sub(1);
            }
        }
        KEY_RIGHT => {
            if sel.end_col < cols - 1 {
                sel.end_col += 1;
            } else if sel.end_row < rows - 1 {
                sel.end_row += 1;
                sel.end_col = 0;
            }
        }
        KEY_UP => {
            sel.end_row = sel.end_row.saturating_sub(1);
        }
        KEY_DOWN => {
            sel.end_row = (sel.end_row + 1).min(rows - 1);
        }
        KEY_HOME => {
            sel.end_col = 0;
        }
        KEY_END => {
            sel.end_col = cols.saturating_sub(1);
        }
        _ => {}
    }
}

/// Expand ~ in path to user's home directory
/// Expand ~ to user's home directory.
/// Uses provided user_home if available, falls back to dirs::home_dir().
fn expand_path(path: &str, user_home: Option<&str>) -> String {
    if !path.starts_with('~') {
        return path.to_string();
    }

    // Get home directory: prefer provided value, fallback to dirs
    let home = user_home
        .map(|h| h.to_string())
        .or_else(|| dirs::home_dir().map(|p| p.to_string_lossy().to_string()));

    match home {
        Some(home) if path == "~" => home,
        Some(home) => format!("{}{}", home, &path[1..]),
        None => path.to_string(),
    }
}

/// Save screenshot
fn save_screenshot(
    gl: &glow::Context,
    width: u32,
    height: u32,
    screenshot_dir: &str,
    user_home: Option<&str>,
) -> Result<()> {
    use glow::HasContext;

    // Expand ~ to user's home directory
    let screenshot_dir = expand_path(screenshot_dir, user_home);

    // Read pixel data from framebuffer
    let mut pixels = vec![0u8; (width * height * 4) as usize];
    unsafe {
        gl.read_pixels(
            0,
            0,
            width as i32,
            height as i32,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelPackData::Slice(&mut pixels),
        );
    }

    // Flip vertically since OpenGL origin is bottom-left
    let row_size = (width * 4) as usize;
    let mut flipped = vec![0u8; pixels.len()];
    for y in 0..height as usize {
        let src_row = (height as usize - 1 - y) * row_size;
        let dst_row = y * row_size;
        flipped[dst_row..dst_row + row_size].copy_from_slice(&pixels[src_row..src_row + row_size]);
    }

    // Generate filename
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let path = format!("{}/bcon_screenshot_{}.png", screenshot_dir, timestamp);

    // Save as PNG
    let file = std::fs::File::create(&path)?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&flipped)?;

    info!("Screenshot saved: {}", path);
    Ok(())
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    // Check command line arguments
    let args: Vec<String> = std::env::args().collect();

    // --help
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return Ok(());
    }

    // --version
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("bcon {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    info!("bcon starting...");

    let test_mode = args.iter().any(|a| a == "--test" || a == "-t");

    if test_mode {
        info!("Test mode: skipping DRM initialization");
        eprintln!("[OK] bcon build verification complete");
        return Ok(());
    }

    // Config file generation mode
    // --init-config or --init-config=PRESET (default, emacs-like, vim-like)
    let init_config_arg = args.iter().find(|a| a.starts_with("--init-config"));
    if let Some(arg) = init_config_arg {
        let preset = if arg.contains('=') {
            arg.split('=').nth(1).unwrap_or("default")
        } else {
            "default"
        };

        let force = args.iter().any(|a| a == "--force" || a == "-f");

        // Check if config file already exists
        if let Ok(config_path) = config::Config::get_config_path_for_preset(preset) {
            if config_path.exists() && !force {
                println!("Config file already exists: {}", config_path.display());
                print!("Overwrite? [y/N]: ");
                std::io::Write::flush(&mut std::io::stdout())?;

                let mut input = String::new();
                std::io::stdin().read_line(&mut input)?;
                let input = input.trim().to_lowercase();

                if input != "y" && input != "yes" {
                    println!("Aborted.");
                    return Ok(());
                }
            }
        }

        // Check if Nerd Font is installed (for yazi, lsd icons)
        let nerd_font_found = config::detect_nerd_font_path().is_some();

        if !nerd_font_found {
            println!("Tip: For icon display (yazi, lsd, etc.), install Nerd Font first:");
            println!();
            println!("  sudo apt install fontconfig curl  # if not installed");
            println!("  mkdir -p ~/.local/share/fonts");
            println!("  cd ~/.local/share/fonts");
            println!("  curl -OL https://github.com/ryanoasis/nerd-fonts/releases/latest/download/Hack.tar.xz");
            println!("  tar xf Hack.tar.xz && rm Hack.tar.xz");
            println!("  fc-cache -fv");
            println!();
            println!("Then re-run --init-config to auto-detect the font.");
            println!();
        }

        match config::Config::write_config_with_preset(preset) {
            Ok(path) => {
                println!("Config file generated:");
                println!("  Preset: {}", preset);
                println!("  Path:   {}", path.display());
                if nerd_font_found {
                    println!("  Nerd Font: detected (symbols configured)");
                }
                println!();
                println!("Available presets (combine with comma):");
                println!("  default  - Standard keybinds (Ctrl+Shift+C/V, etc.)");
                println!("  vim      - Vim-like scroll (Ctrl+Shift+U/D)");
                println!("  emacs    - Emacs-like scroll (Alt+Shift+V/N)");
                println!("  japanese - CJK fonts + IME auto-disable (alias: jp)");
                println!();
                println!("Examples:");
                println!("  bcon --init-config=vim,jp");
                println!("  bcon --init-config=emacs,japanese");
                return Ok(());
            }
            Err(e) => {
                eprintln!("Failed to generate config: {}", e);
                return Err(e);
            }
        }
    }

    // Load config file
    let mut cfg = config::Config::load();

    // Parse keybinds (multiple keys per action supported)
    let mut kb_copy = config::ParsedKeybinds::parse(&cfg.keybinds.copy);
    let mut kb_paste = config::ParsedKeybinds::parse(&cfg.keybinds.paste);
    let mut kb_screenshot = config::ParsedKeybinds::parse(&cfg.keybinds.screenshot);
    let mut kb_search = config::ParsedKeybinds::parse(&cfg.keybinds.search);
    let mut kb_copy_mode = config::ParsedKeybinds::parse(&cfg.keybinds.copy_mode);
    let mut kb_font_increase = config::ParsedKeybinds::parse(&cfg.keybinds.font_increase);
    let mut kb_font_decrease = config::ParsedKeybinds::parse(&cfg.keybinds.font_decrease);
    let mut kb_font_reset = config::ParsedKeybinds::parse(&cfg.keybinds.font_reset);
    let mut kb_scroll_up = config::ParsedKeybinds::parse(&cfg.keybinds.scroll_up);
    let mut kb_scroll_down = config::ParsedKeybinds::parse(&cfg.keybinds.scroll_down);
    let mut kb_reset_terminal = config::ParsedKeybinds::parse(&cfg.keybinds.reset_terminal);

    // Config file change watcher (Linux only)
    // Watch the actual loaded config path, not just the default path
    #[cfg(target_os = "linux")]
    let config_watcher = config::Config::config_path().and_then(|path| {
        config::ConfigWatcher::new(&path).ok()
    });
    #[cfg(target_os = "linux")]
    if config_watcher.is_some() {
        info!("Config hot-reload enabled");
    }

    // Shaper test mode: log font shaping results
    let test_shaper = args.iter().any(|a| a == "--test-shaper");
    if test_shaper {
        return test_shaper_mode();
    }

    // Set up SIGTERM handler for graceful shutdown (systemd stop)
    drm::setup_sigterm_handler();

    // Phase 0: Session management
    // - With seatd feature: Use libseat for rootless operation
    // - Without seatd: Use VtSwitcher (requires root)

    // Determine target VT from stdin (systemd's TTYPath sets this)
    let target_vt = drm::get_target_vt();
    if let Some(vt) = target_vt {
        info!("Target VT from stdin: tty{}", vt);
    } else {
        info!("Could not determine target VT from stdin");
    }

    #[cfg(all(target_os = "linux", feature = "seatd"))]
    let seat_session = {
        // Wait for target VT to become active before opening libseat session
        // This prevents bcon from "stealing" the display when started on an inactive VT
        if let Some(vt) = target_vt {
            if !drm::is_vt_active(vt) {
                info!("VT{} is not active, waiting...", vt);
                if let Err(e) = drm::wait_for_vt(vt) {
                    warn!("Failed to wait for VT{}: {}", vt, e);
                }
            }
        }

        info!("Opening libseat session...");
        Rc::new(RefCell::new(
            session::SeatSession::open().context("Failed to open libseat session")?,
        ))
    };

    #[cfg(not(all(target_os = "linux", feature = "seatd")))]
    let mut vt_switcher = {
        // VtSwitcher sets up process-controlled VT switching via VT_SETMODE
        // The kernel sends SIGUSR1/SIGUSR2 signals for VT switch requests
        drm::VtSwitcher::new().context("Failed to initialize VT switcher")?
    };

    // Phase 1: DRM/KMS initialization
    let drm_path = find_drm_device()?;
    info!("DRM device: {}", drm_path);

    #[cfg(all(target_os = "linux", feature = "seatd"))]
    let drm_device = {
        let seat_drm_device = seat_session
            .borrow_mut()
            .open_device(&drm_path)
            .context("Cannot open DRM device via libseat")?;
        drm::Device::from_fd(seat_drm_device.as_raw_fd())
            .context("Cannot create DRM device from libseat fd")?
    };

    #[cfg(not(all(target_os = "linux", feature = "seatd")))]
    let drm_device = drm::Device::open(&drm_path)
        .context("Cannot open DRM device. Root privileges may be required.")?;

    // Acquire DRM master only if VT is active
    // This prevents bcon from "stealing" the display when started on an inactive VT
    #[cfg(not(all(target_os = "linux", feature = "seatd")))]
    let initial_drm_master = if vt_switcher.is_focused() {
        match drm_device.set_master() {
            Ok(()) => {
                info!("VT is active, DRM master acquired");
                true
            }
            Err(e) => {
                warn!("VT is active but failed to acquire DRM master: {}", e);
                false
            }
        }
    } else {
        info!("VT is not active, deferring DRM master acquisition");
        false
    };

    #[cfg(all(target_os = "linux", feature = "seatd"))]
    let initial_drm_master = {
        // libseat handles DRM master, but check if we're on the target VT
        if let Some(vt) = target_vt {
            drm::is_vt_active(vt)
        } else {
            true // No target VT info, assume active
        }
    };

    // Detect display configuration (prefer external monitors if configured)
    let mut display_config =
        drm::DisplayConfig::detect_with_preference(&drm_device, cfg.display.prefer_external)
            .context("Failed to detect display configuration")?;

    info!(
        "Display: {}x{} (external: {})",
        display_config.width,
        display_config.height,
        display_config.is_external(&drm_device)
    );

    // Initialize DRM hotplug monitor (Linux only)
    #[cfg(target_os = "linux")]
    let mut hotplug_monitor = match drm::HotplugMonitor::new() {
        Ok(m) => Some(m),
        Err(e) => {
            info!("Hotplug monitor unavailable: {}", e);
            None
        }
    };
    #[cfg(target_os = "linux")]
    let mut last_connector_snapshot =
        drm::hotplug::snapshot_connectors(&drm_device).unwrap_or_default();

    // Create GBM device (shares fd with DRM)
    let gbm_file = drm_device.dup_fd()?;
    let gbm_device = gpu::GbmDevice::new(gbm_file)?;

    // Create GBM surface
    let gbm_surface = gpu::GbmSurface::new(
        gbm_device.device(),
        display_config.width,
        display_config.height,
    )?;

    // Create EGL context
    let egl_context = gpu::EglContext::new(gbm_device.device(), gbm_surface.surface())?;

    // Create OpenGL ES renderer
    let renderer = gpu::GlRenderer::new(&egl_context)?;
    renderer.set_viewport(
        0,
        0,
        display_config.width as i32,
        display_config.height as i32,
    );

    // Save original CRTC settings (restore on exit)
    // Note: This may fail if we don't have DRM master, but that's OK
    let saved_crtc = drm::SavedCrtc::save(&drm_device, &display_config).ok();

    // Render first frame and set mode (only if we have DRM master)
    #[cfg(not(all(target_os = "linux", feature = "seatd")))]
    let mut needs_initial_mode_set = !initial_drm_master;

    #[cfg(all(target_os = "linux", feature = "seatd"))]
    let _needs_initial_mode_set = false;

    let (initial_bo, initial_fb) = if initial_drm_master {
        let init_bg = cfg.appearance.background_rgb();
        renderer.clear(init_bg.0, init_bg.1, init_bg.2, 1.0);
        egl_context.swap_buffers()?;

        let bo = gbm_surface.lock_front_buffer()?;
        let fb = drm::DrmFramebuffer::from_bo(&drm_device, &bo)?;
        drm::set_crtc(&drm_device, &display_config, &fb)?;
        info!("Phase 1 initialization complete");
        (Some(bo), Some(fb))
    } else {
        info!("Phase 1 initialization complete (VT not active, mode set deferred)");
        (None, None)
    };

    // Phase 2: Text rendering initialization (FreeType + LCD subpixel)
    let gl = renderer.gl();

    // Load font
    let font_data: &'static [u8] = if !cfg.font.main.is_empty() {
        Box::leak(
            std::fs::read(&cfg.font.main)
                .with_context(|| format!("Failed to read font: {}", cfg.font.main))?
                .into_boxed_slice(),
        )
    } else {
        Box::leak(
            font::atlas::load_system_font()
                .context("Failed to load font")?
                .into_boxed_slice(),
        )
    };

    // Apply display scale factor to font size
    let scale_factor = cfg.appearance.scale.clamp(MIN_DISPLAY_SCALE, MAX_DISPLAY_SCALE);
    let font_size = (cfg.font.size * scale_factor) as u32;
    if (scale_factor - 1.0).abs() > 0.01 {
        info!(
            "Display scale: {}x (font size: {}pt → {}px)",
            scale_factor, cfg.font.size, font_size
        );
    }

    // Load CJK font (continue on failure)
    let cjk_font_data: Option<&[u8]> = if !cfg.font.cjk.is_empty() {
        std::fs::read(&cfg.font.cjk)
            .ok()
            .map(|d| -> &'static [u8] { Box::leak(d.into_boxed_slice()) })
    } else {
        font::atlas::load_cjk_font().map(|d| -> &'static [u8] { Box::leak(d.into_boxed_slice()) })
    };

    // Load symbols/Nerd Font (for yazi, etc. - continue on failure)
    let symbols_font_data: Option<&[u8]> = if !cfg.font.symbols.is_empty() {
        std::fs::read(&cfg.font.symbols)
            .ok()
            .map(|d| -> &'static [u8] { Box::leak(d.into_boxed_slice()) })
    } else {
        // Auto-detect Nerd Font via fontconfig
        font::fontconfig::load_nerd_font_fc()
            .map(|d| -> &'static [u8] { Box::leak(d.into_boxed_slice()) })
    };

    // LCD filter settings (from config)
    let lcd_filter = font::freetype::LcdFilterMode::from_str(&cfg.font.lcd_filter);
    let lcd_subpixel = font::freetype::LcdSubpixel::from_str(&cfg.font.lcd_subpixel);
    let lcd_mode = lcd_subpixel.to_lcd_mode();
    let subpixel_bgr = lcd_subpixel.is_bgr();
    let hinting_mode = font::freetype::HintingMode::from_str(&cfg.font.lcd_hinting);

    // Create FreeType + LCD subpixel rendering atlas
    let mut glyph_atlas = font::lcd_atlas::LcdGlyphAtlas::new(
        gl,
        font_data,
        font_size,
        symbols_font_data,
        cjk_font_data,
        lcd_mode,
        lcd_filter,
        cfg.font.lcd_weights,
        cfg.font.lcd_subpixel_positioning,
        hinting_mode,
    )
    .context("Failed to create LCD glyph atlas")?;

    info!(
        "FreeType LCD atlas initialized: cell={}x{:.0}, subpixel_pos={}, hinting={:?}",
        glyph_atlas.cell_width,
        glyph_atlas.cell_height,
        glyph_atlas.is_subpixel_positioning_enabled(),
        hinting_mode
    );

    // Create emoji atlas
    let emoji_font_path = if !cfg.font.emoji.is_empty() {
        Some(cfg.font.emoji.as_str())
    } else {
        Some("/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf")
    };
    let mut emoji_atlas =
        font::emoji::EmojiAtlas::new(emoji_font_path, glyph_atlas.cell_height as u32);
    if emoji_atlas.is_available() {
        info!("Emoji font loaded: {:?}", emoji_font_path);
    } else {
        info!("No emoji font (monochrome fallback)");
    }

    // Create LCD text renderer (FreeType + linear color space compositing)
    // Uses GPU instancing: 1 draw call per flush, 66% less data transfer
    let mut text_renderer =
        gpu::LcdTextRendererInstanced::new(gl).context("Failed to initialize LCD text renderer")?;
    text_renderer.set_subpixel_bgr(subpixel_bgr);
    text_renderer.set_gamma(cfg.font.lcd_gamma);
    text_renderer.set_stem_darkening(cfg.font.lcd_stem_darkening);
    text_renderer.set_contrast(cfg.font.lcd_contrast);
    text_renderer.set_fringe_reduction(cfg.font.lcd_fringe_reduction);

    info!(
        "LCD subpixel settings: {:?} (BGR={}, gamma={:.2}, contrast={:.2}, fringe={:.2})",
        lcd_subpixel,
        subpixel_bgr,
        cfg.font.lcd_gamma,
        cfg.font.lcd_contrast,
        cfg.font.lcd_fringe_reduction
    );

    // Create UI renderer (rounded rectangles for candidate window, etc.)
    let mut ui_renderer = gpu::UiRenderer::new(gl).context("Failed to initialize UI renderer")?;

    // Create image renderer (Sixel image rendering, etc.)
    let mut image_renderer =
        gpu::ImageRenderer::new(gl).context("Failed to initialize image renderer")?;

    // Create emoji renderer (color emoji rendering)
    let mut emoji_renderer =
        gpu::EmojiRenderer::new(gl).context("Failed to initialize emoji renderer")?;

    // Create curly underline renderer (SDF + smoothstep anti-aliasing)
    let mut curly_renderer =
        gpu::CurlyRenderer::new(gl).context("Failed to initialize curly renderer")?;

    // Create FBO for cached rendering (enables partial updates)
    let fbo = gpu::Fbo::new(gl, display_config.width, display_config.height)
        .context("Failed to initialize FBO")?;

    info!("Phase 2 initialization complete");

    // Phase 3: Terminal initialization
    let screen_w = display_config.width;
    let screen_h = display_config.height;
    // Cell dimensions from font metrics (always positive, but guard against edge cases)
    let mut cell_w = glyph_atlas.cell_width.max(1.0);
    let mut cell_h = glyph_atlas.cell_height.max(1.0);

    // Terminal margin (padding from screen edges)
    let margin_x = 8.0_f32;
    let margin_y = 8.0_f32;

    // Calculate grid size (accounting for margins)
    // Ensure at least 1x1 grid to prevent underflow in Grid::with_scrollback
    let mut grid_cols = ((screen_w as f32 - margin_x * 2.0) / cell_w).max(1.0) as usize;
    let mut grid_rows = ((screen_h as f32 - margin_y * 2.0) / cell_h).max(1.0) as usize;
    info!(
        "Grid size: {}x{} (cell: {:.0}x{:.0}px)",
        grid_cols, grid_rows, cell_w, cell_h
    );

    // Base font size (for reset)
    let base_font_size = font_size;

    // Japanese IME support: ensure D-Bus session is available
    let extra_env: Vec<(String, String)> = if !cfg.terminal.ime_disabled_apps.is_empty() {
        // First check if D-Bus session is already available (e.g., from systemd user session)
        if std::env::var("DBUS_SESSION_BUS_ADDRESS").is_ok() {
            info!("D-Bus session already available from environment");
            Vec::new()
        } else {
            // Start a D-Bus session daemon (console-friendly, unlike dbus-launch which is X11)
            match std::process::Command::new("dbus-daemon")
                .args(["--session", "--fork", "--print-address=1"])
                .output()
            {
                Ok(output) => {
                    let addr = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if !addr.is_empty() {
                        info!("Started D-Bus session: {}", addr);
                        vec![("DBUS_SESSION_BUS_ADDRESS".to_string(), addr)]
                    } else {
                        debug!("dbus-daemon returned empty address");
                        Vec::new()
                    }
                }
                Err(e) => {
                    // D-Bus is optional for IME - only log at debug level
                    debug!("D-Bus session not available (IME may not work): {}", e);
                    Vec::new()
                }
            }
        }
    } else {
        Vec::new()
    };

    let extra_env_refs: Vec<(&str, &str)> = extra_env
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();

    let mut term = terminal::Terminal::with_scrollback_env(
        grid_cols,
        grid_rows,
        cfg.terminal.scrollback_lines,
        &cfg.terminal.term_env,
        &extra_env_refs,
    )
    .context("Failed to initialize terminal")?;

    // Set cell size for Sixel image placement
    term.set_cell_size(cell_w as u32, cell_h as u32);

    // Set clipboard path
    term.set_clipboard_path(&cfg.paths.clipboard_file);

    // Set custom ANSI 16 colors palette from config
    term.grid.set_ansi_palette(cfg.colors.to_palette());

    // Display /etc/issue (like getty does) if running as root
    // Note: When using libseat (seatd feature), we don't run as root
    #[cfg(not(all(target_os = "linux", feature = "seatd")))]
    if unsafe { libc::getuid() } == 0 {
        let tty_name = format!("tty{}", vt_switcher.target_vt());
        if let Some(issue) = terminal::pty::read_issue(&tty_name) {
            // Process issue text through terminal to display it
            term.process_output(issue.as_bytes());
            // Ensure cursor is at column 0 for login prompt
            term.process_output(b"\r\n");
            info!("Displayed /etc/issue for {}", tty_name);
        }
    }

    // Phase 4: Keyboard input initialization
    // TTY keyboard (may fail when running from GDM without a TTY)
    #[cfg(all(target_os = "linux", feature = "seatd"))]
    let keyboard: Option<input::Keyboard> = match input::Keyboard::new() {
        Ok(kb) => {
            info!("TTY keyboard initialized");
            Some(kb)
        }
        Err(e) => {
            info!("TTY keyboard unavailable (libseat mode): {}", e);
            None
        }
    };

    #[cfg(not(all(target_os = "linux", feature = "seatd")))]
    let keyboard: Option<input::Keyboard> =
        Some(input::Keyboard::new().context("Failed to initialize keyboard")?);

    // evdev input (keyboard + mouse, continue with SSH stdin if unavailable)
    #[cfg(all(target_os = "linux", feature = "seatd"))]
    let mut evdev_keyboard = match input::EvdevKeyboard::new_with_seat(
        display_config.width,
        display_config.height,
        seat_session.clone(),
        &cfg.keyboard,
    ) {
        Ok(kb) => {
            info!("evdev input initialized via libseat (keyboard + mouse)");
            Some(kb)
        }
        Err(e) => {
            info!("evdev input unavailable (continuing with SSH stdin): {}", e);
            None
        }
    };

    #[cfg(not(all(target_os = "linux", feature = "seatd")))]
    let mut evdev_keyboard =
        match input::EvdevKeyboard::new(display_config.width, display_config.height, &cfg.keyboard)
        {
            Ok(kb) => {
                info!("evdev input initialized (keyboard + mouse)");
                Some(kb)
            }
            Err(e) => {
                info!("evdev input unavailable (continuing with SSH stdin): {}", e);
                None
            }
        };

    // Require at least one input method
    if keyboard.is_none() && evdev_keyboard.is_none() {
        anyhow::bail!("No input method available (both TTY keyboard and evdev failed)");
    }

    info!("Phase 4 initialization complete");

    // Phase 5d: fcitx5 IME initialization (optional)
    let ime_client = match input::ime::ImeClient::try_new() {
        Ok(c) => {
            info!("fcitx5 IME connected");
            Some(c)
        }
        Err(e) => {
            info!(
                "fcitx5 IME unavailable (continuing with direct input): {}",
                e
            );
            None
        }
    };
    let mut preedit = input::ime::PreeditState::new();
    let mut candidate_state: Option<input::ime::CandidateState> = None;

    info!("Terminal loop started");

    // Notify systemd that we're ready
    let _ = sd_notify::notify(true, &[sd_notify::NotifyState::Ready]);

    // Keep previous frame's BO/FB
    let mut prev_bo = initial_bo;
    let mut prev_fb = initial_fb;
    let mut key_buf = [0u8; 256];
    let mut needs_redraw = true;

    // Mouse state
    let mut mouse_selecting = false;
    let mut mouse_x: f64 = (screen_w / 2) as f64;
    let mut mouse_y: f64 = (screen_h / 2) as f64;
    let mut scroll_accum: f64 = 0.0;
    let mut mouse_button_held: Option<u8> = None;

    // Double/triple click detection
    let mut last_click_time = std::time::Instant::now();
    let mut click_count = 0u8;
    let mut last_click_col = 0usize;
    let mut last_click_row = 0usize;
    const DOUBLE_CLICK_MS: u128 = 300;

    // Bell flash
    let mut bell_flash_until: Option<std::time::Instant> = None;
    const BELL_FLASH_DURATION_MS: u64 = 100;

    // Cursor blink
    let mut cursor_blink_visible = true;
    let mut last_blink_toggle = std::time::Instant::now();
    const CURSOR_BLINK_INTERVAL_MS: u64 = 530; // ~530ms blink interval

    // Font size change request (currently log output only)
    let mut font_size_delta: i32 = 0;

    // Ctrl key state tracking (for URL click)
    let mut ctrl_pressed = false;

    // Search mode
    let mut search_mode = false;

    // Screenshot flag
    let mut take_screenshot = false;

    // IME is always enabled when ime_client is connected
    // User controls input method switching via fcitx5's Ctrl+Space

    // DRM master state (for VT switching)
    let mut drm_master_held = initial_drm_master;

    // Appearance colors (from config, may be overridden by OSC 10/11)
    let config_bg = cfg.appearance.background_rgb();
    let config_fg = cfg.appearance.foreground_rgb();
    let config_cursor = cfg.appearance.cursor_rgb();
    let config_selection = cfg.appearance.selection_rgb();
    let config_cursor_opacity = cfg.appearance.cursor_opacity;

    loop {
        // Poll for session events (VT switching)
        #[cfg(all(target_os = "linux", feature = "seatd"))]
        {
            // Dispatch libseat events
            if let Ok(true) = seat_session.borrow_mut().dispatch() {
                // Events dispatched, check for session state changes
            }

            // Process session events
            while let Some(event) = seat_session.borrow().try_recv_event() {
                match event {
                    session::SessionEvent::Disable => {
                        // Session disabled (VT switched away)
                        info!("libseat: session disabled");

                        // Send focus out event to terminal applications
                        if let Err(e) = term.send_focus_event(false) {
                            log::debug!("Failed to send FocusOut event: {}", e);
                        }

                        // Suspend input devices
                        if let Some(ref mut evdev) = evdev_keyboard {
                            evdev.suspend();
                        }

                        drm_master_held = false;
                        // Note: libseat handles DRM master automatically
                    }
                    session::SessionEvent::Enable => {
                        // Session enabled (VT acquired, or resume from suspend)
                        info!("libseat: session enabled");

                        // Resume input devices
                        if let Some(ref mut evdev) = evdev_keyboard {
                            evdev.resume();
                        }

                        drm_master_held = true;
                        needs_redraw = true;

                        // Invalidate GPU textures (may have been lost during suspend)
                        glyph_atlas.invalidate();
                        emoji_atlas.invalidate();
                        image_renderer.invalidate_all(gl);
                        info!("GPU textures invalidated for re-upload");

                        // Send focus in event to terminal applications
                        if let Err(e) = term.send_focus_event(true) {
                            log::debug!("Failed to send FocusIn event: {}", e);
                        }
                    }
                }
            }
        }

        #[cfg(not(all(target_os = "linux", feature = "seatd")))]
        {
            // Poll for VT switch signals (SIGUSR1/SIGUSR2 via signalfd)
            while let Some(event) = vt_switcher.poll() {
                match event {
                    drm::VtEvent::Release => {
                        // Kernel requests us to release the VT
                        info!("VT release requested");

                        // Send focus out event to terminal applications
                        if let Err(e) = term.send_focus_event(false) {
                            log::debug!("Failed to send FocusOut event: {}", e);
                        }

                        // Suspend input devices before releasing VT
                        if let Some(ref mut evdev) = evdev_keyboard {
                            evdev.suspend();
                        }

                        if drm_master_held {
                            if let Err(e) = drm_device.drop_master() {
                                log::warn!("Failed to drop DRM master: {}", e);
                            } else {
                                drm_master_held = false;
                            }
                        }
                        // Acknowledge release - this allows the VT switch to proceed
                        if let Err(e) = vt_switcher.ack_release() {
                            log::warn!("Failed to acknowledge VT release: {}", e);
                        }
                    }
                    drm::VtEvent::Acquire => {
                        // Kernel grants us the VT (or resume from suspend)
                        info!("VT acquire");

                        // Resume input devices
                        if let Some(ref mut evdev) = evdev_keyboard {
                            evdev.resume();
                        }

                        if !drm_master_held {
                            if let Err(e) = drm_device.set_master() {
                                log::warn!("Failed to acquire DRM master: {}", e);
                            } else {
                                drm_master_held = true;
                                needs_redraw = true;
                            }
                        }
                        // Acknowledge acquire
                        if let Err(e) = vt_switcher.ack_acquire() {
                            log::warn!("Failed to acknowledge VT acquire: {}", e);
                        }

                        // If mode setting was deferred (VT wasn't active at startup),
                        // do it now that we have DRM master
                        if needs_initial_mode_set && drm_master_held {
                            info!("Performing deferred mode setting");
                            let init_bg = cfg.appearance.background_rgb();
                            renderer.clear(init_bg.0, init_bg.1, init_bg.2, 1.0);
                            if let Err(e) = egl_context.swap_buffers() {
                                log::warn!("Failed to swap buffers: {}", e);
                            }
                            if let Ok(bo) = gbm_surface.lock_front_buffer() {
                                if let Ok(fb) = drm::DrmFramebuffer::from_bo(&drm_device, &bo) {
                                    if let Err(e) = drm::set_crtc(&drm_device, &display_config, &fb) {
                                        log::warn!("Failed to set CRTC: {}", e);
                                    } else {
                                        needs_initial_mode_set = false;
                                        info!("Deferred mode setting complete");
                                    }
                                }
                            }
                        }

                        // Invalidate GPU textures (may have been lost during suspend)
                        glyph_atlas.invalidate();
                        emoji_atlas.invalidate();
                        image_renderer.invalidate_all(gl);
                        info!("GPU textures invalidated for re-upload");

                        // Send focus in event to terminal applications
                        if let Err(e) = term.send_focus_event(true) {
                            log::debug!("Failed to send FocusIn event: {}", e);
                        }
                    }
                }
            }
        }

        // Check for SIGTERM (graceful shutdown from systemd)
        if drm::sigterm_received() {
            info!("Received SIGTERM, shutting down gracefully...");
            let _ = sd_notify::notify(true, &[sd_notify::NotifyState::Stopping]);
            break;
        }

        // Skip most processing if we don't have DRM master (VT switched away)
        if !drm_master_held {
            std::thread::sleep(Duration::from_millis(100));
            continue;
        }
        // Cursor blink update
        let now = std::time::Instant::now();
        if now.duration_since(last_blink_toggle).as_millis() >= CURSOR_BLINK_INTERVAL_MS as u128 {
            cursor_blink_visible = !cursor_blink_visible;
            last_blink_toggle = now;
            // Trigger redraw in blink mode
            if term.grid.cursor.blink {
                needs_redraw = true;
            }
        }

        // Check child process alive
        if !term.is_alive() {
            info!("Child process terminated");
            let _ = sd_notify::notify(true, &[sd_notify::NotifyState::Stopping]);
            break;
        }

        // Config hot-reload (Linux only)
        #[cfg(target_os = "linux")]
        if let Some(ref watcher) = config_watcher {
            if watcher.check_reload() {
                info!("Config file change detected, reloading...");
                let new_cfg = config::Config::load();

                // Re-parse keybinds
                kb_copy = config::ParsedKeybinds::parse(&new_cfg.keybinds.copy);
                kb_paste = config::ParsedKeybinds::parse(&new_cfg.keybinds.paste);
                kb_screenshot = config::ParsedKeybinds::parse(&new_cfg.keybinds.screenshot);
                kb_search = config::ParsedKeybinds::parse(&new_cfg.keybinds.search);
                kb_copy_mode = config::ParsedKeybinds::parse(&new_cfg.keybinds.copy_mode);
                kb_font_increase = config::ParsedKeybinds::parse(&new_cfg.keybinds.font_increase);
                kb_font_decrease = config::ParsedKeybinds::parse(&new_cfg.keybinds.font_decrease);
                kb_font_reset = config::ParsedKeybinds::parse(&new_cfg.keybinds.font_reset);
                kb_scroll_up = config::ParsedKeybinds::parse(&new_cfg.keybinds.scroll_up);
                kb_scroll_down = config::ParsedKeybinds::parse(&new_cfg.keybinds.scroll_down);
                kb_reset_terminal = config::ParsedKeybinds::parse(&new_cfg.keybinds.reset_terminal);

                // Update IME disable app list
                cfg = new_cfg;

                info!("Config reload complete");
                needs_redraw = true;
            }
        }

        // DRM hotplug detection (Linux only)
        #[cfg(target_os = "linux")]
        if let Some(ref mut monitor) = hotplug_monitor {
            if monitor.poll().is_some() {
                // Hotplug event detected - re-enumerate connectors
                if let Ok(new_snapshot) = drm::hotplug::snapshot_connectors(&drm_device) {
                    let changes =
                        drm::hotplug::detect_changes(&last_connector_snapshot, &new_snapshot);

                    if changes.has_changes() {
                        changes.log();

                        // Check if we should switch displays
                        let should_switch = cfg.display.auto_switch
                            && (
                                // External monitor connected
                                (cfg.display.prefer_external && changes.external_connected()) ||
                            // Current display disconnected
                            changes.disconnected.iter().any(|s| s.handle == display_config.connector_handle)
                            );

                        if should_switch {
                            // Try to switch to preferred display
                            match drm::DisplayConfig::detect_with_preference(
                                &drm_device,
                                cfg.display.prefer_external,
                            ) {
                                Ok(new_config) => {
                                    if new_config.connector_handle
                                        != display_config.connector_handle
                                    {
                                        // Check if resolution changed
                                        if new_config.width == display_config.width
                                            && new_config.height == display_config.height
                                        {
                                            // Same resolution - update config, next frame will apply
                                            info!(
                                                "Switching display to {:?} (same resolution)",
                                                new_config.connector_handle
                                            );
                                            display_config = new_config;
                                            info!("Display switch scheduled (will apply on next frame)");
                                        } else {
                                            // Resolution changed - need restart
                                            log::warn!(
                                                "Display resolution changed ({}x{} -> {}x{}). Restart bcon to apply.",
                                                display_config.width, display_config.height,
                                                new_config.width, new_config.height
                                            );
                                        }
                                    }
                                }
                                Err(e) => {
                                    log::warn!("Failed to detect new display: {}", e);
                                }
                            }
                        }

                        last_connector_snapshot = new_snapshot;
                        needs_redraw = true;
                    }
                }
            }
        }

        // Read TTY stdin keyboard input and forward to PTY
        // Skip if evdev keyboard is available (prevent double input)
        if evdev_keyboard.is_none() {
            if let Some(ref kb) = keyboard {
                match kb.read(&mut key_buf) {
                    Ok(0) => {}
                    Ok(n) => {
                        let _ = term.write_to_pty(&key_buf[..n]);
                    }
                    Err(e) => {
                        log::warn!("Keyboard read error: {}", e);
                    }
                }
            }
        }

        // Process evdev input (keyboard + mouse)
        if let Some(ref mut evdev_kb) = evdev_keyboard {
            let (key_events, mouse_events) = evdev_kb.process_raw_events();

            // Keyboard event processing
            for raw in &key_events {
                // Check for VT switch key combination (Ctrl+Alt+Fn)
                // In KD_GRAPHICS mode, kernel doesn't see keypresses, so we must handle this
                #[cfg(not(all(target_os = "linux", feature = "seatd")))]
                if let Some(target) = input::evdev::check_vt_switch(raw) {
                    if target != vt_switcher.target_vt() {
                        if let Err(e) = vt_switcher.switch_to(target) {
                            warn!("Failed to switch to VT{}: {}", target, e);
                        }
                    }
                    continue;
                }

                #[cfg(all(target_os = "linux", feature = "seatd"))]
                if let Some(target) = input::evdev::check_vt_switch(raw) {
                    if let Err(e) = seat_session.borrow_mut().switch_session(target as i32) {
                        warn!("Failed to switch to VT{}: {}", target, e);
                    }
                    continue;
                }

                // Update Ctrl state (for URL click detection)
                ctrl_pressed = raw.mods_ctrl;

                if !raw.is_press {
                    // Send release events to IME if connected
                    if let Some(ref ime) = ime_client {
                        ime.send_key(input::ime::ImeKeyEvent {
                            keysym: raw.keysym,
                            keycode: raw.keycode,
                            state: raw.xkb_state,
                            is_release: true,
                        });
                    }
                    continue;
                }

                let shift = raw.mods_shift;

                let ctrl = raw.mods_ctrl;
                let alt = raw.mods_alt;
                let keysym = raw.keysym;

                // Scroll up (configurable)
                if kb_scroll_up.matches(ctrl, shift, alt, raw.keycode, keysym) {
                    term.scroll_back(grid_rows / 2);
                    needs_redraw = true;
                    continue;
                }
                // Scroll down (configurable)
                if kb_scroll_down.matches(ctrl, shift, alt, raw.keycode, keysym) {
                    term.scroll_forward(grid_rows / 2);
                    needs_redraw = true;
                    continue;
                }

                // Copy (configurable)
                if kb_copy.matches(ctrl, shift, alt, raw.keycode, keysym) {
                    term.copy_selection();
                    needs_redraw = true;
                    continue;
                }

                // Paste (configurable)
                if kb_paste.matches(ctrl, shift, alt, raw.keycode, keysym) {
                    let _ = term.paste_clipboard();
                    needs_redraw = true;
                    continue;
                }

                // Screenshot (configurable)
                if kb_screenshot.matches(ctrl, shift, alt, raw.keycode, keysym) {
                    // Set screenshot flag (execute after rendering)
                    take_screenshot = true;
                    needs_redraw = true;
                    continue;
                }

                // Reset terminal modes (configurable)
                if kb_reset_terminal.matches(ctrl, shift, alt, raw.keycode, keysym) {
                    term.reset_enhanced_modes();
                    info!("Terminal modes reset by user");
                    needs_redraw = true;
                    continue;
                }

                // Copy mode start (configurable)
                if kb_copy_mode.matches(ctrl, shift, alt, raw.keycode, keysym) {
                    if term.copy_mode.is_none() {
                        term.enter_copy_mode();
                    }
                    needs_redraw = true;
                    continue;
                }

                // Key input processing in copy mode
                if term.copy_mode.is_some() {
                    match raw.keysym {
                        // Escape: Exit copy mode
                        xkbcommon::xkb::keysyms::KEY_Escape => {
                            term.exit_copy_mode();
                        }
                        // h / Left arrow: Move left
                        xkbcommon::xkb::keysyms::KEY_h | xkbcommon::xkb::keysyms::KEY_Left => {
                            term.copy_mode_move(0, -1);
                        }
                        // j / Down arrow: Move down
                        xkbcommon::xkb::keysyms::KEY_j | xkbcommon::xkb::keysyms::KEY_Down => {
                            term.copy_mode_move(1, 0);
                        }
                        // k / Up arrow: Move up
                        xkbcommon::xkb::keysyms::KEY_k | xkbcommon::xkb::keysyms::KEY_Up => {
                            term.copy_mode_move(-1, 0);
                        }
                        // l / Right arrow: Move right
                        xkbcommon::xkb::keysyms::KEY_l | xkbcommon::xkb::keysyms::KEY_Right => {
                            term.copy_mode_move(0, 1);
                        }
                        // v: Toggle selection
                        xkbcommon::xkb::keysyms::KEY_v => {
                            term.copy_mode_toggle_selection();
                        }
                        // y: Yank (copy and exit)
                        xkbcommon::xkb::keysyms::KEY_y => {
                            term.copy_mode_yank();
                        }
                        // g: Go to top
                        xkbcommon::xkb::keysyms::KEY_g => {
                            term.copy_mode_goto_top();
                        }
                        // G: Go to bottom
                        xkbcommon::xkb::keysyms::KEY_G => {
                            term.copy_mode_goto_bottom();
                        }
                        // 0 / Home: Line start
                        xkbcommon::xkb::keysyms::KEY_0 | xkbcommon::xkb::keysyms::KEY_Home => {
                            term.copy_mode_goto_line_start();
                        }
                        // $ / End: Line end
                        xkbcommon::xkb::keysyms::KEY_dollar | xkbcommon::xkb::keysyms::KEY_End => {
                            term.copy_mode_goto_line_end();
                        }
                        // w: Word forward
                        xkbcommon::xkb::keysyms::KEY_w => {
                            term.copy_mode_word_forward();
                        }
                        // b: Word backward
                        xkbcommon::xkb::keysyms::KEY_b => {
                            term.copy_mode_word_backward();
                        }
                        // Ctrl+u: Half page up
                        xkbcommon::xkb::keysyms::KEY_u if ctrl => {
                            term.copy_mode_page_up();
                        }
                        // Ctrl+d: Half page down
                        xkbcommon::xkb::keysyms::KEY_d if ctrl => {
                            term.copy_mode_page_down();
                        }
                        // /: Enter search mode
                        xkbcommon::xkb::keysyms::KEY_slash => {
                            search_mode = true;
                            term.start_search();
                        }
                        _ => {}
                    }
                    needs_redraw = true;
                    continue;
                }

                // Search mode start (configurable)
                if kb_search.matches(ctrl, shift, alt, raw.keycode, keysym) {
                    if !search_mode {
                        search_mode = true;
                        term.start_search();
                        info!("Search mode started");
                    }
                    needs_redraw = true;
                    continue;
                }

                // Key input processing in search mode
                if search_mode {
                    if let Some(ref mut search) = term.search {
                        let has_matches = !search.matches.is_empty();

                        match raw.keysym {
                            // Escape: End search
                            xkbcommon::xkb::keysyms::KEY_Escape => {
                                search_mode = false;
                                term.end_search();
                                info!("Search mode ended");
                            }
                            // Enter: Execute search
                            xkbcommon::xkb::keysyms::KEY_Return
                            | xkbcommon::xkb::keysyms::KEY_KP_Enter => {
                                term.execute_search();
                                term.scroll_to_current_match();
                                // If in copy mode, end search and move cursor to match position
                                if term.copy_mode.is_some() {
                                    if let Some(ref s) = term.search {
                                        if !s.matches.is_empty() {
                                            let (abs_row, start_col, _) =
                                                s.matches[s.current_match];
                                            let scrollback_len = term.grid.scrollback_len();
                                            // Convert to display coordinates
                                            if abs_row >= scrollback_len {
                                                let display_row = abs_row - scrollback_len;
                                                if let Some(ref mut cm) = term.copy_mode {
                                                    cm.cursor_row = display_row;
                                                    cm.cursor_col = start_col;
                                                }
                                            }
                                        }
                                    }
                                    search_mode = false;
                                    term.end_search();
                                }
                            }
                            // Backspace: Delete one character
                            xkbcommon::xkb::keysyms::KEY_BackSpace => {
                                search.query.pop();
                                // Clear matches when query changes
                                search.matches.clear();
                            }
                            // n / N: Move between matches (only after search executed)
                            xkbcommon::xkb::keysyms::KEY_n | xkbcommon::xkb::keysyms::KEY_N
                                if has_matches =>
                            {
                                if shift {
                                    search.prev_match();
                                } else {
                                    search.next_match();
                                }
                                term.scroll_to_current_match();
                            }
                            // Normal character: Add to query
                            _ => {
                                if !raw.utf8.is_empty() && !ctrl {
                                    search.query.push_str(&raw.utf8);
                                    // Clear matches when query changes
                                    search.matches.clear();
                                }
                            }
                        }
                    }
                    needs_redraw = true;
                    continue;
                }

                // Font size increase (configurable)
                if kb_font_increase.matches(ctrl, shift, alt, raw.keycode, keysym) {
                    font_size_delta += 2;
                    let new_size = (base_font_size as f32 + font_size_delta as f32)
                        .clamp(MIN_FONT_SIZE, MAX_FONT_SIZE);
                    let (new_cell_w, new_cell_h) = glyph_atlas.resize(new_size);
                    cell_w = new_cell_w.max(1.0);
                    cell_h = new_cell_h.max(1.0);
                    grid_cols = ((screen_w as f32 - margin_x * 2.0) / cell_w).max(1.0) as usize;
                    grid_rows = ((screen_h as f32 - margin_y * 2.0) / cell_h).max(1.0) as usize;
                    term.resize(grid_cols, grid_rows);
                    term.set_cell_size(cell_w as u32, cell_h as u32);
                    emoji_atlas.resize(cell_h as u32);
                    needs_redraw = true;
                    continue;
                }

                // Font size decrease (configurable)
                if kb_font_decrease.matches(ctrl, shift, alt, raw.keycode, keysym) {
                    font_size_delta -= 2;
                    let new_size = (base_font_size as f32 + font_size_delta as f32)
                        .clamp(MIN_FONT_SIZE, MAX_FONT_SIZE);
                    let (new_cell_w, new_cell_h) = glyph_atlas.resize(new_size);
                    cell_w = new_cell_w.max(1.0);
                    cell_h = new_cell_h.max(1.0);
                    grid_cols = ((screen_w as f32 - margin_x * 2.0) / cell_w).max(1.0) as usize;
                    grid_rows = ((screen_h as f32 - margin_y * 2.0) / cell_h).max(1.0) as usize;
                    term.resize(grid_cols, grid_rows);
                    term.set_cell_size(cell_w as u32, cell_h as u32);
                    emoji_atlas.resize(cell_h as u32);
                    needs_redraw = true;
                    continue;
                }

                // Font size reset (configurable)
                if kb_font_reset.matches(ctrl, shift, alt, raw.keycode, keysym) {
                    font_size_delta = 0;
                    let (new_cell_w, new_cell_h) = glyph_atlas.resize(base_font_size as f32);
                    cell_w = new_cell_w.max(1.0);
                    cell_h = new_cell_h.max(1.0);
                    grid_cols = ((screen_w as f32 - margin_x * 2.0) / cell_w).max(1.0) as usize;
                    grid_rows = ((screen_h as f32 - margin_y * 2.0) / cell_h).max(1.0) as usize;
                    term.resize(grid_cols, grid_rows);
                    term.set_cell_size(cell_w as u32, cell_h as u32);
                    emoji_atlas.resize(cell_h as u32);
                    needs_redraw = true;
                    continue;
                }

                // Shift+Arrow: Text selection
                if shift && matches!(raw.keycode, 105 | 106 | 103 | 108 | 102 | 107) {
                    handle_selection_key(&mut term, raw.keycode, grid_cols);
                    needs_redraw = true;
                    continue;
                }

                // Clear selection on non-modified key input
                if term.selection.is_some() && !shift {
                    term.selection = None;
                    needs_redraw = true;
                }

                // Process keys: send to fcitx5 if connected, otherwise directly to PTY
                if let Some(ref ime) = ime_client {
                    ime.send_key(input::ime::ImeKeyEvent {
                        keysym: raw.keysym,
                        keycode: raw.keycode,
                        state: raw.xkb_state,
                        is_release: false,
                    });
                } else {
                    // Send directly to PTY when IME client is unavailable
                    let sym = xkbcommon::xkb::Keysym::new(raw.keysym);
                    let kb_config = input::KeyboardConfig {
                        application_cursor_keys: term.grid.modes.application_cursor_keys,
                        modify_other_keys: term.grid.keyboard.modify_other_keys,
                        kitty_flags: term.grid.keyboard.kitty_flags,
                        key_action: raw.action,
                    };
                    let bytes = input::keysym_to_bytes_with_mods(
                        sym,
                        &raw.utf8,
                        raw.mods_ctrl,
                        raw.mods_alt,
                        raw.mods_shift,
                        &kb_config,
                    );
                    if !bytes.is_empty() {
                        let _ = term.write_to_pty(&bytes);
                    }
                }
            }

            // Mouse event processing
            for mouse in &mouse_events {
                match mouse {
                    input::MouseEvent::ButtonPress { button, x, y } => {
                        // Account for terminal margin when converting to cell coordinates
                        let col = ((*x - margin_x as f64).max(0.0) / cell_w as f64) as usize;
                        let row = ((*y - margin_y as f64).max(0.0) / cell_h as f64) as usize;

                        // Ctrl+Left click: Copy URL to clipboard
                        if ctrl_pressed && *button == input::BTN_LEFT {
                            let clamped_row = row.min(grid_rows - 1);
                            let clamped_col = col.min(grid_cols - 1);
                            if let Some(url) = term.detect_url_at(clamped_row, clamped_col) {
                                term.copy_url_to_clipboard(&url);
                                // Visual feedback with bell flash
                                bell_flash_until = Some(
                                    std::time::Instant::now()
                                        + Duration::from_millis(BELL_FLASH_DURATION_MS),
                                );
                            }
                            needs_redraw = true;
                            continue;
                        }

                        // Send to PTY if mouse mode is enabled
                        if term.mouse_mode_enabled() {
                            let btn = match *button {
                                input::BTN_LEFT => 0,
                                input::BTN_MIDDLE => 1,
                                input::BTN_RIGHT => 2,
                                _ => 0,
                            };
                            let _ = term.send_mouse_press(
                                btn,
                                col.min(grid_cols - 1),
                                row.min(grid_rows - 1),
                            );
                            mouse_button_held = Some(btn);
                        } else if *button == input::BTN_LEFT {
                            let now = std::time::Instant::now();
                            let elapsed = now.duration_since(last_click_time).as_millis();
                            let same_pos = col == last_click_col && row == last_click_row;

                            // Double/triple click detection
                            if elapsed < DOUBLE_CLICK_MS && same_pos {
                                click_count = (click_count + 1).min(3);
                            } else {
                                click_count = 1;
                            }
                            last_click_time = now;
                            last_click_col = col;
                            last_click_row = row;

                            let clamped_row = row.min(grid_rows - 1);
                            let clamped_col = col.min(grid_cols - 1);

                            match click_count {
                                2 => {
                                    // Double click: Word selection
                                    term.select_word(clamped_row, clamped_col);
                                    mouse_selecting = false;
                                }
                                3 => {
                                    // Triple click: Line selection
                                    term.select_line(clamped_row);
                                    mouse_selecting = false;
                                }
                                _ => {
                                    // Single click: Start normal selection
                                    term.selection = Some(terminal::Selection {
                                        anchor_row: clamped_row,
                                        anchor_col: clamped_col,
                                        end_row: clamped_row,
                                        end_col: clamped_col,
                                    });
                                    mouse_selecting = true;
                                }
                            }
                        }

                        if *button == input::BTN_MIDDLE && !term.mouse_mode_enabled() {
                            // Middle button: Paste (only when mouse mode is disabled)
                            let _ = term.paste_clipboard();
                        }
                        needs_redraw = true;
                    }
                    input::MouseEvent::Move { x, y } => {
                        // Account for terminal margin when converting to cell coordinates
                        let col = ((*x - margin_x as f64).max(0.0) / cell_w as f64) as usize;
                        let row = ((*y - margin_y as f64).max(0.0) / cell_h as f64) as usize;

                        // Send move event if mouse mode is enabled
                        if term.mouse_mode_enabled() {
                            let _ = term.send_mouse_move(
                                col.min(grid_cols - 1),
                                row.min(grid_rows - 1),
                                mouse_button_held,
                            );
                        } else if mouse_selecting {
                            // Dragging: Update selection range
                            if let Some(ref mut sel) = term.selection {
                                sel.end_row = row.min(grid_rows - 1);
                                sel.end_col = col.min(grid_cols - 1);
                            }
                            needs_redraw = true;
                        }
                        mouse_x = *x;
                        mouse_y = *y;
                    }
                    input::MouseEvent::ButtonRelease { button, x, y } => {
                        // Account for terminal margin when converting to cell coordinates
                        let col = ((*x - margin_x as f64).max(0.0) / cell_w as f64) as usize;
                        let row = ((*y - margin_y as f64).max(0.0) / cell_h as f64) as usize;

                        // Send to PTY if mouse mode is enabled
                        if term.mouse_mode_enabled() {
                            let btn = match *button {
                                input::BTN_LEFT => 0,
                                input::BTN_MIDDLE => 1,
                                input::BTN_RIGHT => 2,
                                _ => 0,
                            };
                            let _ = term.send_mouse_release(
                                btn,
                                col.min(grid_cols - 1),
                                row.min(grid_rows - 1),
                            );
                        } else if *button == input::BTN_LEFT {
                            // Left button release: Confirm selection + auto copy
                            mouse_selecting = false;
                            if term.selection.is_some() {
                                term.copy_selection();
                            }
                        }
                        // Always reset button state
                        mouse_button_held = None;
                    }
                    input::MouseEvent::Scroll { delta, x, y } => {
                        // Account for terminal margin when converting to cell coordinates
                        let col = ((*x - margin_x as f64).max(0.0) / cell_w as f64) as usize;
                        let row = ((*y - margin_y as f64).max(0.0) / cell_h as f64) as usize;

                        // Send to PTY if ButtonEvent/AnyEvent + SGR is enabled
                        // Don't send wheel events in X10 mode
                        let use_mouse_wheel = term.mouse_mode_enabled()
                            && term.grid.modes.mouse_mode != terminal::grid::MouseMode::X10;

                        if use_mouse_wheel {
                            let d = if *delta < 0.0 { -1i8 } else { 1i8 };
                            let _ = term.send_mouse_wheel(
                                d,
                                col.min(grid_cols - 1),
                                row.min(grid_rows - 1),
                            );
                        } else {
                            // Scroll accumulation (negative=up/history, positive=down/live)
                            scroll_accum += delta;
                            // Execute scroll when accumulated to 1 line
                            while scroll_accum >= 1.0 {
                                term.scroll_forward(1);
                                scroll_accum -= 1.0;
                                needs_redraw = true;
                            }
                            while scroll_accum <= -1.0 {
                                term.scroll_back(1);
                                scroll_accum += 1.0;
                                needs_redraw = true;
                            }
                        }
                    }
                }
            }

            if !key_events.is_empty() || !mouse_events.is_empty() {
                needs_redraw = true;
            }
        }

        // Process IME events
        if let Some(ref ime) = ime_client {
            for event in ime.poll_events() {
                match event {
                    input::ime::ImeEvent::Commit(text) => {
                        info!("IME Commit: {:?}", text);
                        // Mark cursor row dirty before clearing preedit (for FBO cache)
                        term.grid.mark_dirty(term.grid.cursor_row);
                        let _ = term.write_to_pty(text.as_bytes());
                        preedit.clear();
                        candidate_state = None;
                        needs_redraw = true;
                    }
                    input::ime::ImeEvent::Preedit { segments, cursor } => {
                        let pe_text: String = segments.iter().map(|s| s.text.as_str()).collect();
                        let formats: Vec<i32> = segments.iter().map(|s| s.format).collect();
                        trace!(
                            "IME Preedit: {:?} formats={:?} cursor={}",
                            pe_text,
                            formats,
                            cursor
                        );
                        preedit.segments = segments;
                        preedit.cursor = cursor;
                        needs_redraw = true;
                    }
                    input::ime::ImeEvent::PreeditClear => {
                        trace!("IME PreeditClear");
                        // Mark cursor row dirty before clearing preedit (for FBO cache)
                        term.grid.mark_dirty(term.grid.cursor_row);
                        preedit.clear();
                        needs_redraw = true;
                    }
                    input::ime::ImeEvent::ForwardKey {
                        keysym,
                        state,
                        is_release,
                    } => {
                        if !is_release {
                            // Extract modifiers from xkb state
                            let mods_ctrl = (state & 0x4) != 0; // Control
                            let mods_alt = (state & 0x8) != 0; // Mod1 (Alt)
                            let mods_shift = (state & 0x1) != 0; // Shift

                            let sym = xkbcommon::xkb::Keysym::new(keysym);
                            // keysym_to_utf8 may contain NUL terminator, filter it out
                            let utf8_raw = xkbcommon::xkb::keysym_to_utf8(sym);
                            let utf8: String = utf8_raw.chars().filter(|&c| c != '\0').collect();
                            let kb_config = input::KeyboardConfig {
                                application_cursor_keys: term.grid.modes.application_cursor_keys,
                                modify_other_keys: term.grid.keyboard.modify_other_keys,
                                kitty_flags: term.grid.keyboard.kitty_flags,
                                key_action: input::KeyAction::Press, // IME passthrough is always press
                            };
                            let bytes = input::keysym_to_bytes_with_mods(
                                sym, &utf8, mods_ctrl, mods_alt, mods_shift, &kb_config,
                            );
                            // Filter NUL bytes from result as well
                            let bytes: Vec<u8> = bytes.into_iter().filter(|&b| b != 0).collect();
                            if !bytes.is_empty() {
                                let _ = term.write_to_pty(&bytes);
                                needs_redraw = true;
                            }
                        }
                    }
                    input::ime::ImeEvent::UpdateCandidates(state) => {
                        trace!(
                            "IME Candidates: {} items, selected={}",
                            state.candidates.len(),
                            state.selected_index
                        );
                        candidate_state = Some(state);
                        needs_redraw = true;
                    }
                    input::ime::ImeEvent::ClearCandidates => {
                        trace!("IME ClearCandidates");
                        // Mark all rows dirty to clear candidate window from FBO cache
                        term.grid.mark_all_dirty();
                        candidate_state = None;
                        needs_redraw = true;
                    }
                }
            }
        }

        // Process PTY output
        let mut total_read = 0;
        loop {
            match term.process_pty_output() {
                Ok(0) => break,
                Ok(n) => total_read += n,
                Err(e) => {
                    log::warn!("PTY read error: {}", e);
                    break;
                }
            }
        }

        if total_read > 0 {
            needs_redraw = true;
            // Return to live position on new output
            term.scroll_to_bottom();
            // Clear selection on new output
            term.selection = None;

            // Start flash if bell notification
            if term.grid.bell_triggered {
                term.grid.bell_triggered = false;
                bell_flash_until =
                    Some(std::time::Instant::now() + Duration::from_millis(BELL_FLASH_DURATION_MS));
            }
        }

        // Continue redraw if bell flash is active
        if let Some(until) = bell_flash_until {
            if std::time::Instant::now() < until {
                needs_redraw = true;
            } else {
                bell_flash_until = None;
                needs_redraw = true; // Redraw once more to clear the flash
            }
        }

        // Focus events are sent when VT switch signals are processed above
        // (VtSwitcher handles SIGUSR1/SIGUSR2 for acquire/release)

        // IME auto-switch removed - user controls via fcitx5's Ctrl+Space

        // Sleep briefly if no changes (reduce CPU load)
        if !needs_redraw {
            std::thread::sleep(Duration::from_millis(8));
            continue;
        }
        needs_redraw = false;

        // Render screen to FBO
        // Use OSC 11 dynamic background if set, otherwise config background
        let bg_color = if let Some((r, g, b)) = term.grid.colors.bg {
            (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0)
        } else {
            config_bg
        };

        // Cursor is drawn outside FBO (after blit), so no need to mark cursor rows dirty

        // Bind FBO for rendering
        fbo.bind(gl);

        // Overlays that require full FBO clear (dynamic content that doesn't use dirty tracking)
        let has_overlays = term.selection.is_some()
            || search_mode
            || term.copy_mode.is_some()
            || !preedit.is_empty()  // IME preedit changes without dirty tracking
            || candidate_state.is_some(); // IME candidate window

        // Clear FBO: full clear when overlays active or all dirty, otherwise only dirty rows
        if term.grid.is_all_dirty() || has_overlays {
            fbo.clear(gl, bg_color.0, bg_color.1, bg_color.2, 1.0);
        } else if term.has_dirty_rows() {
            for row in 0..term.grid.rows() {
                if term.grid.is_row_dirty(row) {
                    fbo.clear_rows(
                        gl, row, row, cell_h, margin_y, bg_color.0, bg_color.1, bg_color.2,
                    );
                }
            }
        }

        text_renderer.begin();
        text_renderer.set_bg_color(bg_color.0, bg_color.1, bg_color.2);
        emoji_renderer.begin();
        curly_renderer.begin();

        // Effective foreground color: OSC 10 > config > hardcoded white
        let default_fg = if let Some((r, g, b)) = term.grid.colors.fg {
            [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0]
        } else {
            [config_fg.0, config_fg.1, config_fg.2, 1.0]
        };

        // Effective background color: OSC 11 > config > hardcoded black
        let default_bg = [bg_color.0, bg_color.1, bg_color.2, 1.0];

        let grid = &term.grid;

        // Helper to get foreground color, using default_fg for Color::Default
        // Uses grid.color_to_rgba for custom ANSI palette support
        let effective_fg = |color: &terminal::grid::Color| -> [f32; 4] {
            if matches!(color, terminal::grid::Color::Default) {
                default_fg
            } else {
                grid.color_to_rgba(color, true)
            }
        };

        // Helper to get background color, using default_bg for Color::Default
        // Uses grid.color_to_rgba for custom ANSI palette support
        let effective_bg = |color: &terminal::grid::Color| -> [f32; 4] {
            if matches!(color, terminal::grid::Color::Default) {
                default_bg
            } else {
                grid.color_to_rgba(color, false)
            }
        };
        let ascent = glyph_atlas.ascent;

        // Determine if partial rendering is possible (no overlays active)
        // When overlays are active, we must render all rows for correctness
        let partial_render = !has_overlays && !grid.is_all_dirty();

        // === Pass 1: Background color (run-length encoded) ===
        // Selection is blended into background here (not in separate pass)
        // This ensures LCD compositing uses exact same color as rendered background
        let selection_color = [config_selection.0, config_selection.1, config_selection.2];
        let selection_alpha = 0.35_f32;

        // Combine consecutive cells with same background into single rectangles
        for row in 0..grid.rows() {
            // Skip non-dirty rows when partial rendering is enabled
            if partial_render && !grid.is_row_dirty(row) {
                continue;
            }
            let y = margin_y + row as f32 * cell_h;
            let mut run_start: Option<(usize, [f32; 4])> = None;

            // Get selection range for this row
            let row_selection = term
                .selection
                .as_ref()
                .and_then(|sel| sel.cols_for_row(row, grid.cols()));

            for col in 0..grid.cols() {
                let cell = term.display_cell(row, col);

                // Skip continuation cells (width==0)
                if cell.width == 0 {
                    continue;
                }

                // Check if this cell contains a transition character (Powerline, rounded corners, etc.)
                // These characters have transparent/curved regions where rectangular backgrounds
                // would show through, causing visual artifacts (vertical lines)
                let first_ch = cell.grapheme.chars().next().unwrap_or(' ');
                let is_transition = is_transition_char(first_ch);

                // For transition characters: flush current run and skip this cell
                // The background will be handled specially in Pass 2
                // Use floor() for x2 to avoid overlap with transition cell
                if is_transition {
                    if let Some((start, run_color)) = run_start.take() {
                        if run_color[3] > 0.0 {
                            let x1 = (margin_x + start as f32 * cell_w).floor();
                            let x2 = (margin_x + col as f32 * cell_w).floor();
                            if x2 > x1 {
                                text_renderer.push_rect(
                                    x1,
                                    y,
                                    x2 - x1,
                                    cell_h,
                                    run_color,
                                    &glyph_atlas,
                                );
                            }
                        }
                    }
                    continue;
                }

                // Handle INVERSE attribute (SGR 7): swap fg and bg
                let mut bg = if cell.attrs.contains(terminal::grid::CellAttrs::INVERSE) {
                    effective_fg(&cell.fg)
                } else {
                    effective_bg(&cell.bg)
                };

                // Blend selection color into background if cell is selected
                if let Some((sel_start, sel_end)) = row_selection {
                    if col >= sel_start && col < sel_end {
                        let a = selection_alpha;
                        bg[0] = selection_color[0] * a + bg[0] * (1.0 - a);
                        bg[1] = selection_color[1] * a + bg[1] * (1.0 - a);
                        bg[2] = selection_color[2] * a + bg[2] * (1.0 - a);
                        bg[3] = 1.0; // Ensure opaque after blending
                    }
                }

                if let Some((start, run_color)) = run_start {
                    if bg == run_color {
                        // Continue the run
                        continue;
                    } else {
                        // Flush previous run
                        if run_color[3] > 0.0 {
                            // Use floor/ceil to ensure pixel-aligned rectangles without gaps
                            let x1 = (margin_x + start as f32 * cell_w).floor();
                            let x2 = (margin_x + col as f32 * cell_w).ceil();
                            text_renderer.push_rect(
                                x1,
                                y,
                                x2 - x1,
                                cell_h,
                                run_color,
                                &glyph_atlas,
                            );
                        }
                        // Start new run
                        run_start = Some((col, bg));
                    }
                } else {
                    // Start first run
                    run_start = Some((col, bg));
                }
            }

            // Flush final run
            if let Some((start, run_color)) = run_start {
                if run_color[3] > 0.0 {
                    let x1 = (margin_x + start as f32 * cell_w).floor();
                    let x2 = (margin_x + grid.cols() as f32 * cell_w).ceil();
                    text_renderer.push_rect(x1, y, x2 - x1, cell_h, run_color, &glyph_atlas);
                }
            }
        }

        // === Pass 1.6: Search match highlight (one rect per match) ===
        if search_mode {
            let current_match = term.current_search_match();
            for row in 0..grid.rows() {
                let matches = term.get_search_matches_for_display_row(row);
                for &(start_col, end_col, match_idx) in matches {
                    let clamped_end = end_col.min(grid.cols());
                    if start_col >= clamped_end {
                        continue;
                    }
                    let x = margin_x + start_col as f32 * cell_w;
                    let y = margin_y + row as f32 * cell_h;
                    let w = (clamped_end - start_col) as f32 * cell_w;
                    let is_current = current_match == Some(match_idx);
                    let color = if is_current {
                        [1.0, 0.6, 0.0, 0.5] // Current match: orange
                    } else {
                        [1.0, 1.0, 0.0, 0.3] // Others: yellow
                    };
                    text_renderer.push_rect(x, y, w, cell_h, color, &glyph_atlas);
                }
            }
        }

        // === Pass 2: Text rendering (FreeType LCD mode) ===
        let max_cols = grid.cols();

        for row in 0..grid.rows() {
            // Skip non-dirty rows when partial rendering is enabled
            if partial_render && !grid.is_row_dirty(row) {
                continue;
            }

            // Pre-compute selection range for this row (avoids per-cell normalized() call)
            let row_selection = term
                .selection
                .as_ref()
                .and_then(|s| s.cols_for_row(row, max_cols));

            // Pre-compute search matches for this row (avoids per-cell function call)
            // Returns slice reference - no allocation needed
            let row_search_matches = if search_mode {
                term.get_search_matches_for_display_row(row)
            } else {
                &[][..]
            };
            let current_match_idx = term.current_search_match();

            // Underline rendering: batch consecutive cells with same style into runs
            let mut col = 0;
            while col < grid.cols() {
                let cell = if term.scroll_offset > 0 {
                    term.display_cell(row, col)
                } else {
                    grid.cell(row, col)
                };

                // Skip width=0 (continuation cells)
                if cell.width == 0 {
                    col += 1;
                    continue;
                }

                // Skip if no underline
                let has_underline = cell.underline_style != terminal::grid::UnderlineStyle::None
                    || cell.hyperlink.is_some();
                if !has_underline {
                    col += 1;
                    continue;
                }

                // Run start: group consecutive cells with same style and color
                let run_start = col;
                let run_style = cell.underline_style;
                let run_color = cell
                    .underline_color
                    .map(|c| grid.color_to_rgba(&c, true))
                    .unwrap_or_else(|| effective_fg(&cell.fg));
                let run_has_hyperlink = cell.hyperlink.is_some();

                // Find run end
                col += 1;
                while col < grid.cols() {
                    let next_cell = if term.scroll_offset > 0 {
                        term.display_cell(row, col)
                    } else {
                        grid.cell(row, col)
                    };
                    if next_cell.width == 0 {
                        col += 1;
                        continue;
                    }
                    let next_style = next_cell.underline_style;
                    let next_color = next_cell
                        .underline_color
                        .map(|c| grid.color_to_rgba(&c, true))
                        .unwrap_or_else(|| effective_fg(&next_cell.fg));
                    let next_has_hyperlink = next_cell.hyperlink.is_some();

                    // Continue run while style, color, and hyperlink match
                    if next_style == run_style
                        && next_color == run_color
                        && next_has_hyperlink == run_has_hyperlink
                    {
                        col += 1;
                    } else {
                        break;
                    }
                }
                let run_end = col;
                let run_len = run_end - run_start;

                // Run coordinates (including margins)
                let run_x = margin_x + run_start as f32 * cell_w;
                let run_w = run_len as f32 * cell_w;
                let run_y = margin_y + row as f32 * cell_h;
                let underline_y = run_y + cell_h - 2.0;

                // Draw according to underline style
                match run_style {
                    terminal::grid::UnderlineStyle::None => {
                        // Single line for hyperlinks
                        if run_has_hyperlink {
                            text_renderer.push_rect(
                                run_x,
                                underline_y,
                                run_w,
                                1.0,
                                run_color,
                                &glyph_atlas,
                            );
                        }
                    }
                    terminal::grid::UnderlineStyle::Single => {
                        text_renderer.push_rect(
                            run_x,
                            underline_y,
                            run_w,
                            1.0,
                            run_color,
                            &glyph_atlas,
                        );
                    }
                    terminal::grid::UnderlineStyle::Double => {
                        text_renderer.push_rect(
                            run_x,
                            underline_y - 2.0,
                            run_w,
                            1.0,
                            run_color,
                            &glyph_atlas,
                        );
                        text_renderer.push_rect(
                            run_x,
                            underline_y,
                            run_w,
                            1.0,
                            run_color,
                            &glyph_atlas,
                        );
                    }
                    terminal::grid::UnderlineStyle::Curly => {
                        // Curly line: draw with SDF shader (anti-aliased)
                        let baseline_y = (run_y + ascent).round();
                        let desc = (cell_h - ascent).max(2.0);

                        // Allowed line: below the upper half of descender
                        let allowed_top = baseline_y + desc * 0.5;

                        let mut amplitude = (desc * 0.40).clamp(1.2, 2.0);
                        let mut thickness = (desc * 0.12).clamp(0.6, 0.9);
                        if thickness > amplitude * 0.5 {
                            thickness = amplitude * 0.5;
                        }

                        let wavelength = (cell_w * 1.3).clamp(6.0, 9.0);

                        // Place base position near cell bottom
                        let mut base_y = run_y + cell_h - 1.0;
                        let mut wave_top = base_y - amplitude - thickness;

                        // If wave top is above allowed line, push down base_y
                        if wave_top < allowed_top {
                            let delta = allowed_top - wave_top;
                            base_y += delta;
                            wave_top += delta;
                        }

                        // Reduce amplitude if overflowing bottom
                        let max_base = run_y + cell_h - 1.0;
                        if base_y > max_base {
                            let overshoot = base_y - max_base;
                            amplitude = (amplitude - overshoot).max(1.0);
                            base_y = max_base;
                            wave_top = base_y - amplitude - thickness;
                        }

                        base_y = base_y.floor() + 0.5;

                        let wave_height = (amplitude + thickness) * 2.0 + 2.0;

                        curly_renderer.push_curly(
                            run_x,
                            wave_top,
                            run_w,
                            wave_height,
                            run_color,
                            amplitude,
                            wavelength,
                            thickness,
                            base_y,
                        );
                    }
                    terminal::grid::UnderlineStyle::Dotted => {
                        let dot_size = 1.0;
                        let gap = 2.0;
                        let mut dx = run_x;
                        while dx < run_x + run_w {
                            text_renderer.push_rect(
                                dx,
                                underline_y,
                                dot_size,
                                1.0,
                                run_color,
                                &glyph_atlas,
                            );
                            dx += dot_size + gap;
                        }
                    }
                    terminal::grid::UnderlineStyle::Dashed => {
                        let dash_len = 4.0_f32;
                        let gap = 2.0_f32;
                        let mut dx = run_x;
                        while dx < run_x + run_w {
                            text_renderer.push_rect(
                                dx,
                                underline_y,
                                dash_len.min(run_x + run_w - dx),
                                1.0,
                                run_color,
                                &glyph_atlas,
                            );
                            dx += dash_len + gap;
                        }
                    }
                }
            }

            // Character-based rendering (FreeType LCD mode) + overline/strikethrough
            // Combined into single loop to reduce grid traversals
            for col in 0..grid.cols() {
                let cell = term.display_cell(row, col);
                if cell.width == 0 {
                    continue;
                }

                let x = margin_x + col as f32 * cell_w;
                let y = margin_y + row as f32 * cell_h;

                // Width of cell in pixels (wide chars occupy 2 cells)
                let cell_pixel_w = cell.width as f32 * cell_w;

                // Check INVERSE for decorations
                let is_inverse = cell.attrs.contains(terminal::grid::CellAttrs::INVERSE);

                // Overline rendering (CSI 53 m)
                if cell.attrs.contains(terminal::grid::CellAttrs::OVERLINE) {
                    let fg = if is_inverse {
                        effective_bg(&cell.bg)
                    } else {
                        effective_fg(&cell.fg)
                    };
                    text_renderer.push_rect(x, y, cell_pixel_w, 1.0, fg, &glyph_atlas);
                }

                // Strikethrough rendering (CSI 9 m)
                if cell.attrs.contains(terminal::grid::CellAttrs::STRIKE) {
                    let fg = if is_inverse {
                        effective_bg(&cell.bg)
                    } else {
                        effective_fg(&cell.fg)
                    };
                    let strike_y = y + cell_h / 2.0;
                    text_renderer.push_rect(x, strike_y, cell_pixel_w, 1.0, fg, &glyph_atlas);
                }

                let grapheme = &cell.grapheme;
                let first_ch = grapheme.chars().next().unwrap_or(' ');

                // Box-drawing characters: draw programmatically for pixel-perfect alignment
                {
                    let fg = if is_inverse {
                        effective_bg(&cell.bg)
                    } else {
                        effective_fg(&cell.fg)
                    };
                    if draw_box_drawing(
                        first_ch,
                        x,
                        y,
                        cell_w,
                        cell_h,
                        fg,
                        &glyph_atlas,
                        &mut text_renderer,
                    ) {
                        continue;
                    }
                }
                if !grapheme.is_empty() && grapheme != " " {
                    // Handle INVERSE attribute (SGR 7): swap fg and bg (is_inverse defined above)
                    let fg = if is_inverse {
                        effective_bg(&cell.bg)
                    } else {
                        effective_fg(&cell.fg)
                    };

                    // Calculate final background color (needed for LCD subpixel compositing)
                    // Composite in order: cell BG -> selection highlight -> search highlight
                    // Blend in linear space to match shader
                    let mut bg_rgba = if is_inverse {
                        effective_fg(&cell.fg)
                    } else {
                        effective_bg(&cell.bg)
                    };

                    // Selection highlight - LCD compositing uses same blend as Pass 1
                    if let Some((sel_start, sel_end)) = row_selection {
                        if col >= sel_start && col < sel_end {
                            let a = selection_alpha;
                            bg_rgba[0] = selection_color[0] * a + bg_rgba[0] * (1.0 - a);
                            bg_rgba[1] = selection_color[1] * a + bg_rgba[1] * (1.0 - a);
                            bg_rgba[2] = selection_color[2] * a + bg_rgba[2] * (1.0 - a);
                        }
                    }

                    // Search highlight - blend in sRGB space to match GPU blending
                    if search_mode {
                        for &(start_col, end_col, match_idx) in row_search_matches {
                            if col >= start_col && col < end_col.min(grid.cols()) {
                                let is_current = current_match_idx == Some(match_idx);
                                let hl_color = if is_current {
                                    [1.0, 0.6, 0.0, 0.5] // Orange
                                } else {
                                    [1.0, 1.0, 0.0, 0.3] // Yellow
                                };
                                let a = hl_color[3];
                                // Blend in sRGB space (matches GPU's default blend mode)
                                bg_rgba[0] = hl_color[0] * a + bg_rgba[0] * (1.0 - a);
                                bg_rgba[1] = hl_color[1] * a + bg_rgba[1] * (1.0 - a);
                                bg_rgba[2] = hl_color[2] * a + bg_rgba[2] * (1.0 - a);
                                break;
                            }
                        }
                    }

                    let bg = [bg_rgba[0], bg_rgba[1], bg_rgba[2]];
                    let first_char = cell.ch();

                    // Half-block and Powerline characters: draw as pixel-perfect rectangles
                    // This prevents visible lines caused by font glyph antialiasing
                    let cp = first_char as u32;
                    if is_transition_char(first_char) {
                        let cell_pixel_w = cell.width as f32 * cell_w;
                        let is_powerline = (0xE0B0..=0xE0D4).contains(&cp);
                        let is_block_element = (0x2580..=0x259F).contains(&cp);

                        // Draw per-cell background for block elements (Pass1 skipped)
                        // Block elements need explicit background to fill the "empty" portion
                        if is_powerline || is_block_element {
                            text_renderer.push_rect(
                                x,
                                y,
                                cell_pixel_w,
                                cell_h,
                                [bg[0], bg[1], bg[2], 1.0],
                                &glyph_atlas,
                            );
                        }

                        // For transition characters: use programmatic draw or glyph fallback
                        // Block elements helper - fractional heights and widths
                        let eighth_h = cell_h / 8.0;
                        let eighth_w = cell_pixel_w / 8.0;

                        match cp {
                            // === Block elements: U+2580-U+259F ===

                            // Upper/Lower eighth blocks (vertical divisions)
                            0x2580 => {
                                // ▀ Upper half block (4/8 from top)
                                text_renderer.push_rect(
                                    x,
                                    y,
                                    cell_pixel_w,
                                    (4.0 * eighth_h).ceil(),
                                    fg,
                                    &glyph_atlas,
                                );
                            }
                            0x2581 => {
                                // ▁ Lower one eighth block
                                let h = eighth_h.ceil();
                                text_renderer.push_rect(
                                    x,
                                    y + cell_h - h,
                                    cell_pixel_w,
                                    h,
                                    fg,
                                    &glyph_atlas,
                                );
                            }
                            0x2582 => {
                                // ▂ Lower one quarter block (2/8)
                                let h = (2.0 * eighth_h).ceil();
                                text_renderer.push_rect(
                                    x,
                                    y + cell_h - h,
                                    cell_pixel_w,
                                    h,
                                    fg,
                                    &glyph_atlas,
                                );
                            }
                            0x2583 => {
                                // ▃ Lower three eighths block
                                let h = (3.0 * eighth_h).ceil();
                                text_renderer.push_rect(
                                    x,
                                    y + cell_h - h,
                                    cell_pixel_w,
                                    h,
                                    fg,
                                    &glyph_atlas,
                                );
                            }
                            0x2584 => {
                                // ▄ Lower half block (4/8)
                                let h = (4.0 * eighth_h).ceil();
                                text_renderer.push_rect(
                                    x,
                                    y + cell_h - h,
                                    cell_pixel_w,
                                    h,
                                    fg,
                                    &glyph_atlas,
                                );
                            }
                            0x2585 => {
                                // ▅ Lower five eighths block
                                let h = (5.0 * eighth_h).ceil();
                                text_renderer.push_rect(
                                    x,
                                    y + cell_h - h,
                                    cell_pixel_w,
                                    h,
                                    fg,
                                    &glyph_atlas,
                                );
                            }
                            0x2586 => {
                                // ▆ Lower three quarters block (6/8)
                                let h = (6.0 * eighth_h).ceil();
                                text_renderer.push_rect(
                                    x,
                                    y + cell_h - h,
                                    cell_pixel_w,
                                    h,
                                    fg,
                                    &glyph_atlas,
                                );
                            }
                            0x2587 => {
                                // ▇ Lower seven eighths block
                                let h = (7.0 * eighth_h).ceil();
                                text_renderer.push_rect(
                                    x,
                                    y + cell_h - h,
                                    cell_pixel_w,
                                    h,
                                    fg,
                                    &glyph_atlas,
                                );
                            }
                            0x2588 => {
                                // █ Full block (8/8)
                                text_renderer.push_rect(
                                    x,
                                    y,
                                    cell_pixel_w,
                                    cell_h,
                                    fg,
                                    &glyph_atlas,
                                );
                            }

                            // Left/Right eighth blocks (horizontal divisions)
                            0x2589 => {
                                // ▉ Left seven eighths block
                                let w = (7.0 * eighth_w).ceil();
                                text_renderer.push_rect(x, y, w, cell_h, fg, &glyph_atlas);
                            }
                            0x258A => {
                                // ▊ Left three quarters block (6/8)
                                let w = (6.0 * eighth_w).ceil();
                                text_renderer.push_rect(x, y, w, cell_h, fg, &glyph_atlas);
                            }
                            0x258B => {
                                // ▋ Left five eighths block
                                let w = (5.0 * eighth_w).ceil();
                                text_renderer.push_rect(x, y, w, cell_h, fg, &glyph_atlas);
                            }
                            0x258C => {
                                // ▌ Left half block (4/8)
                                let w = (4.0 * eighth_w).ceil();
                                text_renderer.push_rect(x, y, w, cell_h, fg, &glyph_atlas);
                            }
                            0x258D => {
                                // ▍ Left three eighths block
                                let w = (3.0 * eighth_w).ceil();
                                text_renderer.push_rect(x, y, w, cell_h, fg, &glyph_atlas);
                            }
                            0x258E => {
                                // ▎ Left one quarter block (2/8)
                                let w = (2.0 * eighth_w).ceil();
                                text_renderer.push_rect(x, y, w, cell_h, fg, &glyph_atlas);
                            }
                            0x258F => {
                                // ▏ Left one eighth block
                                let w = eighth_w.ceil();
                                text_renderer.push_rect(x, y, w, cell_h, fg, &glyph_atlas);
                            }
                            0x2590 => {
                                // ▐ Right half block (4/8 from right)
                                let w = (4.0 * eighth_w).floor();
                                text_renderer.push_rect(
                                    x + cell_pixel_w - w,
                                    y,
                                    w,
                                    cell_h,
                                    fg,
                                    &glyph_atlas,
                                );
                            }

                            // Shade blocks
                            0x2591 => {
                                // ░ Light shade (25%) - draw with reduced alpha
                                let shade_fg = [fg[0], fg[1], fg[2], 0.25];
                                text_renderer.push_rect(
                                    x,
                                    y,
                                    cell_pixel_w,
                                    cell_h,
                                    shade_fg,
                                    &glyph_atlas,
                                );
                            }
                            0x2592 => {
                                // ▒ Medium shade (50%)
                                let shade_fg = [fg[0], fg[1], fg[2], 0.50];
                                text_renderer.push_rect(
                                    x,
                                    y,
                                    cell_pixel_w,
                                    cell_h,
                                    shade_fg,
                                    &glyph_atlas,
                                );
                            }
                            0x2593 => {
                                // ▓ Dark shade (75%)
                                let shade_fg = [fg[0], fg[1], fg[2], 0.75];
                                text_renderer.push_rect(
                                    x,
                                    y,
                                    cell_pixel_w,
                                    cell_h,
                                    shade_fg,
                                    &glyph_atlas,
                                );
                            }

                            // Upper and right one eighth blocks
                            0x2594 => {
                                // ▔ Upper one eighth block
                                let h = eighth_h.ceil();
                                text_renderer.push_rect(x, y, cell_pixel_w, h, fg, &glyph_atlas);
                            }
                            0x2595 => {
                                // ▕ Right one eighth block
                                let w = eighth_w.ceil();
                                text_renderer.push_rect(
                                    x + cell_pixel_w - w,
                                    y,
                                    w,
                                    cell_h,
                                    fg,
                                    &glyph_atlas,
                                );
                            }

                            // Quarter blocks (quadrants)
                            0x2596 => {
                                // ▖ Quadrant lower left
                                let hw = (cell_pixel_w / 2.0).ceil();
                                let hh = (cell_h / 2.0).ceil();
                                text_renderer.push_rect(
                                    x,
                                    y + cell_h - hh,
                                    hw,
                                    hh,
                                    fg,
                                    &glyph_atlas,
                                );
                            }
                            0x2597 => {
                                // ▗ Quadrant lower right
                                let hw = (cell_pixel_w / 2.0).floor();
                                let hh = (cell_h / 2.0).ceil();
                                text_renderer.push_rect(
                                    x + cell_pixel_w - hw,
                                    y + cell_h - hh,
                                    hw,
                                    hh,
                                    fg,
                                    &glyph_atlas,
                                );
                            }
                            0x2598 => {
                                // ▘ Quadrant upper left
                                let hw = (cell_pixel_w / 2.0).ceil();
                                let hh = (cell_h / 2.0).ceil();
                                text_renderer.push_rect(x, y, hw, hh, fg, &glyph_atlas);
                            }
                            0x2599 => {
                                // ▙ Quadrant upper left and lower left and lower right
                                let hw = (cell_pixel_w / 2.0).ceil();
                                let hh = (cell_h / 2.0).ceil();
                                // Upper left
                                text_renderer.push_rect(x, y, hw, hh, fg, &glyph_atlas);
                                // Lower half
                                text_renderer.push_rect(
                                    x,
                                    y + cell_h - hh,
                                    cell_pixel_w,
                                    hh,
                                    fg,
                                    &glyph_atlas,
                                );
                            }
                            0x259A => {
                                // ▚ Quadrant upper left and lower right (diagonal)
                                let hw = (cell_pixel_w / 2.0).ceil();
                                let hh = (cell_h / 2.0).ceil();
                                // Upper left
                                text_renderer.push_rect(x, y, hw, hh, fg, &glyph_atlas);
                                // Lower right
                                text_renderer.push_rect(
                                    x + cell_pixel_w - (cell_pixel_w / 2.0).floor(),
                                    y + cell_h - hh,
                                    (cell_pixel_w / 2.0).floor(),
                                    hh,
                                    fg,
                                    &glyph_atlas,
                                );
                            }
                            0x259B => {
                                // ▛ Quadrant upper left and upper right and lower left
                                let hw = (cell_pixel_w / 2.0).ceil();
                                let hh = (cell_h / 2.0).ceil();
                                // Upper half
                                text_renderer.push_rect(x, y, cell_pixel_w, hh, fg, &glyph_atlas);
                                // Lower left
                                text_renderer.push_rect(
                                    x,
                                    y + cell_h - hh,
                                    hw,
                                    hh,
                                    fg,
                                    &glyph_atlas,
                                );
                            }
                            0x259C => {
                                // ▜ Quadrant upper left and upper right and lower right
                                let hw = (cell_pixel_w / 2.0).floor();
                                let hh = (cell_h / 2.0).ceil();
                                // Upper half
                                text_renderer.push_rect(x, y, cell_pixel_w, hh, fg, &glyph_atlas);
                                // Lower right
                                text_renderer.push_rect(
                                    x + cell_pixel_w - hw,
                                    y + cell_h - hh,
                                    hw,
                                    hh,
                                    fg,
                                    &glyph_atlas,
                                );
                            }
                            0x259D => {
                                // ▝ Quadrant upper right
                                let hw = (cell_pixel_w / 2.0).floor();
                                let hh = (cell_h / 2.0).ceil();
                                text_renderer.push_rect(
                                    x + cell_pixel_w - hw,
                                    y,
                                    hw,
                                    hh,
                                    fg,
                                    &glyph_atlas,
                                );
                            }
                            0x259E => {
                                // ▞ Quadrant upper right and lower left (diagonal)
                                let hw_ceil = (cell_pixel_w / 2.0).ceil();
                                let hw_floor = (cell_pixel_w / 2.0).floor();
                                let hh = (cell_h / 2.0).ceil();
                                // Upper right
                                text_renderer.push_rect(
                                    x + cell_pixel_w - hw_floor,
                                    y,
                                    hw_floor,
                                    hh,
                                    fg,
                                    &glyph_atlas,
                                );
                                // Lower left
                                text_renderer.push_rect(
                                    x,
                                    y + cell_h - hh,
                                    hw_ceil,
                                    hh,
                                    fg,
                                    &glyph_atlas,
                                );
                            }
                            0x259F => {
                                // ▟ Quadrant upper right and lower left and lower right
                                let hw = (cell_pixel_w / 2.0).floor();
                                let hh = (cell_h / 2.0).ceil();
                                // Upper right
                                text_renderer.push_rect(
                                    x + cell_pixel_w - hw,
                                    y,
                                    hw,
                                    hh,
                                    fg,
                                    &glyph_atlas,
                                );
                                // Lower half
                                text_renderer.push_rect(
                                    x,
                                    y + cell_h - hh,
                                    cell_pixel_w,
                                    hh,
                                    fg,
                                    &glyph_atlas,
                                );
                            }
                            // Powerline: draw programmatically with smoothstep AA
                            // E0B0/E0B2: solid separators
                            0xE0B0 => draw_powerline_triangle(
                                x,
                                y,
                                cell_w,
                                cell_h,
                                fg,
                                bg,
                                &glyph_atlas,
                                &mut text_renderer,
                                true,
                                1.0,
                            ),
                            0xE0B2 => draw_powerline_triangle(
                                x,
                                y,
                                cell_w,
                                cell_h,
                                fg,
                                bg,
                                &glyph_atlas,
                                &mut text_renderer,
                                false,
                                1.0,
                            ),
                            // E0B1/E0B3: thin separators (outline only)
                            0xE0B1 => draw_powerline_triangle_outline(
                                x,
                                y,
                                cell_w,
                                cell_h,
                                fg,
                                bg,
                                &glyph_atlas,
                                &mut text_renderer,
                                true,
                            ),
                            0xE0B3 => draw_powerline_triangle_outline(
                                x,
                                y,
                                cell_w,
                                cell_h,
                                fg,
                                bg,
                                &glyph_atlas,
                                &mut text_renderer,
                                false,
                            ),
                            // E0B4/E0B6: solid semicircles
                            0xE0B4 => draw_powerline_semicircle(
                                x,
                                y,
                                cell_w,
                                cell_h,
                                fg,
                                bg,
                                &glyph_atlas,
                                &mut text_renderer,
                                true,
                                1.0,
                            ),
                            0xE0B6 => draw_powerline_semicircle(
                                x,
                                y,
                                cell_w,
                                cell_h,
                                fg,
                                bg,
                                &glyph_atlas,
                                &mut text_renderer,
                                false,
                                1.0,
                            ),
                            // E0B5/E0B7: thin semicircles (outline only)
                            0xE0B5 => draw_powerline_semicircle_outline(
                                x,
                                y,
                                cell_w,
                                cell_h,
                                fg,
                                bg,
                                &glyph_atlas,
                                &mut text_renderer,
                                true,
                            ),
                            0xE0B7 => draw_powerline_semicircle_outline(
                                x,
                                y,
                                cell_w,
                                cell_h,
                                fg,
                                bg,
                                &glyph_atlas,
                                &mut text_renderer,
                                false,
                            ),
                            _ => {
                                // Fallback for unhandled transition chars (powerline variants, etc.)
                                // Apply LCD compositing with actual cell background
                                glyph_atlas.ensure_glyph(first_char);
                                let baseline_y = (y + ascent).round();

                                // Apply same lcd_mix logic as normal text
                                let srgb_to_linear = |c: f32| -> f32 {
                                    if c <= 0.04045 {
                                        c / 12.92
                                    } else {
                                        ((c + 0.055) / 1.055).powf(2.4)
                                    }
                                };
                                let bg_chroma =
                                    bg[0].max(bg[1]).max(bg[2]) - bg[0].min(bg[1]).min(bg[2]);
                                let mut lcd_mix = ((bg_chroma - 0.06) / 0.14).clamp(0.0, 1.0);
                                let fg_luma = 0.2126 * srgb_to_linear(fg[0])
                                    + 0.7152 * srgb_to_linear(fg[1])
                                    + 0.0722 * srgb_to_linear(fg[2]);
                                let bg_luma = 0.2126 * srgb_to_linear(bg[0])
                                    + 0.7152 * srgb_to_linear(bg[1])
                                    + 0.0722 * srgb_to_linear(bg[2]);
                                let contrast = (fg_luma - bg_luma).abs();
                                let contrast_factor =
                                    1.0 - ((contrast - 0.30) / 0.30).clamp(0.0, 1.0);
                                lcd_mix = (lcd_mix * contrast_factor).clamp(0.0, 1.0);
                                let bright = ((bg_luma - 0.70) / 0.20).clamp(0.0, 1.0);
                                lcd_mix = lcd_mix.max(bright * 0.75);
                                lcd_mix = lcd_mix.clamp(0.0, 0.85);

                                text_renderer.push_text_with_bg_lcd(
                                    grapheme,
                                    x,
                                    baseline_y,
                                    fg,
                                    bg,
                                    lcd_mix,
                                    &glyph_atlas,
                                );
                            }
                        }
                        continue;
                    }

                    // Emoji check (use cached IS_EMOJI flag from Cell)
                    let is_emoji_grapheme =
                        cell.attrs.contains(terminal::grid::CellAttrs::IS_EMOJI);
                    let mut emoji_drawn = false;
                    if is_emoji_grapheme && emoji_atlas.is_available() {
                        // Search by entire grapheme (supports flags and ZWJ sequences)
                        if let Some(info) = emoji_atlas.ensure_grapheme(gl, grapheme, cell_h as u32)
                        {
                            let emoji_size = cell_h;
                            let cell_span_w = cell_w * 2.0;
                            let emoji_x = x + (cell_span_w - emoji_size) / 2.0;
                            emoji_renderer.push_emoji(
                                emoji_x, y, emoji_size, emoji_size, info.uv_x, info.uv_y,
                                info.uv_w, info.uv_h,
                            );
                            emoji_drawn = true;
                        } else if let Some(first_emoji) =
                            grapheme.chars().find(|c| font::emoji::is_emoji(*c))
                        {
                            // Retry with first emoji if entire ZWJ sequence not found
                            if let Some(info) = emoji_atlas.ensure_glyph(first_emoji, cell_h as u32)
                            {
                                let emoji_size = cell_h;
                                let cell_span_w = cell_w * 2.0;
                                let emoji_x = x + (cell_span_w - emoji_size) / 2.0;
                                emoji_renderer.push_emoji(
                                    emoji_x, y, emoji_size, emoji_size, info.uv_x, info.uv_y,
                                    info.uv_w, info.uv_h,
                                );
                                emoji_drawn = true;
                            }
                        }
                    }
                    if !emoji_drawn {
                        // Ensure all glyphs in grapheme are in atlas
                        for ch in grapheme.chars() {
                            glyph_atlas.ensure_glyph(ch);
                        }
                        let baseline_y = (y + ascent).round();

                        // Calculate LCD mix factor for colored backgrounds
                        // Uses gradual mixing (0.0-0.7) instead of binary on/off
                        let bg_chroma = bg[0].max(bg[1]).max(bg[2]) - bg[0].min(bg[1]).min(bg[2]);

                        // chroma <= 0.06: full LCD, chroma >= 0.20: max reduction
                        let mut lcd_mix = ((bg_chroma - 0.06) / 0.14).clamp(0.0, 1.0);

                        // Preserve LCD when fg/bg contrast is high (text remains sharp)
                        let srgb_to_linear = |c: f32| -> f32 {
                            if c <= 0.04045 {
                                c / 12.92
                            } else {
                                ((c + 0.055) / 1.055).powf(2.4)
                            }
                        };
                        let fg_luma = 0.2126 * srgb_to_linear(fg[0])
                            + 0.7152 * srgb_to_linear(fg[1])
                            + 0.0722 * srgb_to_linear(fg[2]);
                        let bg_luma = 0.2126 * srgb_to_linear(bg[0])
                            + 0.7152 * srgb_to_linear(bg[1])
                            + 0.0722 * srgb_to_linear(bg[2]);
                        let contrast = (fg_luma - bg_luma).abs();
                        // High contrast (>0.60) -> reduce lcd_mix, low contrast -> keep lcd_mix
                        let contrast_factor = 1.0 - ((contrast - 0.30) / 0.30).clamp(0.0, 1.0);

                        lcd_mix = (lcd_mix * contrast_factor).clamp(0.0, 1.0);

                        // 明るい背景は必ずLCD抑制を効かせる (contrast_factorが0でも無効化されない)
                        let bright = ((bg_luma - 0.70) / 0.20).clamp(0.0, 1.0);
                        lcd_mix = lcd_mix.max(bright * 0.75);
                        lcd_mix = lcd_mix.clamp(0.0, 0.85);

                        // Render full grapheme (handles combining characters correctly)
                        text_renderer.push_text_with_bg_lcd(
                            grapheme,
                            x,
                            baseline_y,
                            fg,
                            bg,
                            lcd_mix,
                            &glyph_atlas,
                        );
                    }
                }
            }
        }

        // === Pass 3: Sixel image rendering ===
        image_renderer.begin();
        for placement in &term.grid.image_placements {
            if let Some(image) = term.images.get(placement.id) {
                // Upload texture if not already uploaded
                if !image_renderer.has_texture(placement.id) {
                    image_renderer.upload_image(gl, image);
                }
                // Drawing coordinates (accounting for scroll offset)
                let img_row = placement.row as isize - term.scroll_offset as isize;
                if img_row + placement.height_cells as isize <= 0 {
                    continue; // Off-screen (above)
                }
                if img_row >= grid_rows as isize {
                    continue; // Off-screen (below)
                }
                let x = margin_x + placement.col as f32 * cell_w;
                let y = margin_y + img_row as f32 * cell_h;
                image_renderer.draw(placement.id, x, y, image.width as f32, image.height as f32);
            }
        }
        image_renderer.flush(gl, screen_w, screen_h);

        // === Pass 3.5: Curly line rendering (SDF + smoothstep) ===
        curly_renderer.flush(gl, screen_w, screen_h);

        // === Preedit rendering (IME composition text) ===
        // Hide preedit and cursor during scrollback display
        let preedit_total_cols = if term.scroll_offset > 0 {
            0
        } else if !preedit.is_empty() {
            let pe_col = grid.cursor_col;
            let pe_row = grid.cursor_row;
            let pe_y = margin_y + pe_row as f32 * cell_h;

            // Calculate total width first (for background)
            // fcitx5 format flags: 8=Underline(composing), 16=Highlight(conversion target)
            let total_cols: usize = preedit
                .segments
                .iter()
                .flat_map(|seg| seg.text.chars())
                .map(|ch| unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0))
                .sum();

            // Pass A: Draw background for entire preedit area (covers grid content)
            let pe_x = margin_x + pe_col as f32 * cell_w;
            let pe_w = total_cols as f32 * cell_w;
            text_renderer.push_rect(
                pe_x,
                pe_y,
                pe_w,
                cell_h,
                [bg_color.0, bg_color.1, bg_color.2, 1.0], // Terminal background
                &glyph_atlas,
            );

            // Pass B: Draw highlight backgrounds for conversion target segments
            let mut offset_col = 0usize;
            for seg in &preedit.segments {
                let is_highlight = (seg.format & 16) != 0;
                let seg_cols: usize = seg
                    .text
                    .chars()
                    .map(|ch| unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0))
                    .sum();

                if is_highlight {
                    let seg_x = margin_x + (pe_col + offset_col) as f32 * cell_w;
                    let seg_w = seg_cols as f32 * cell_w;
                    text_renderer.push_rect(
                        seg_x,
                        pe_y,
                        seg_w,
                        cell_h,
                        [0.7, 0.7, 0.7, 1.0], // Highlight background
                        &glyph_atlas,
                    );
                }
                offset_col += seg_cols;
            }

            // Pass C: Draw text and underlines
            offset_col = 0;
            for seg in &preedit.segments {
                let is_highlight = (seg.format & 16) != 0;
                let fg = if is_highlight {
                    [0.0, 0.0, 0.0, 1.0] // Black (on highlight background)
                } else {
                    [1.0, 1.0, 1.0, 1.0] // White (on terminal background)
                };

                let seg_start = offset_col;
                for ch in seg.text.chars() {
                    let char_x = margin_x + (pe_col + offset_col) as f32 * cell_w;
                    let baseline_y = (pe_y + ascent).round();
                    let ch_w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);

                    // Skip zero-width characters for positioning but still render them
                    glyph_atlas.ensure_glyph(ch);
                    text_renderer.push_char(ch, char_x, baseline_y, fg, &glyph_atlas);
                    offset_col += ch_w;
                }

                // Underline for composing (non-highlight) segments
                if !is_highlight {
                    let ul_x = margin_x + (pe_col + seg_start) as f32 * cell_w;
                    let ul_w = (offset_col - seg_start) as f32 * cell_w;
                    let ul_y = pe_y + cell_h - 2.0;
                    text_renderer.push_rect(
                        ul_x,
                        ul_y,
                        ul_w,
                        2.0,
                        [1.0, 1.0, 1.0, 1.0],
                        &glyph_atlas,
                    );
                }
            }

            total_cols
        } else {
            0
        };

        // Text cursor and mouse cursor are drawn after FBO blit (outside FBO cache)
        // to avoid cursor trails in cached content

        // === Pass 1: Text rendering (grid + preedit + cursor) ===
        glyph_atlas.upload_if_dirty(gl);
        text_renderer.flush(gl, &glyph_atlas, screen_w, screen_h);

        // === Pass 1.5: Emoji rendering (after text, so emoji shows on top of selection) ===
        // text_renderer disables blending for LCD compositing, so emoji must be drawn after
        emoji_atlas.upload_if_dirty(gl);
        emoji_renderer.flush(gl, &emoji_atlas, screen_w, screen_h);

        // === Candidate window rendering (3 passes: UI background -> candidate text) ===
        if let Some(ref cands) = candidate_state {
            if !cands.candidates.is_empty() {
                // Layout constants
                let padding = 8.0_f32;
                let item_gap = 4.0_f32;
                let corner_radius = 6.0_f32;
                let highlight_radius = 4.0_f32;

                // Calculate candidate text width (use unwrap_or(0) for control chars)
                let mut max_label_w = 0.0_f32;
                let mut max_text_w = 0.0_f32;
                for (label, text) in &cands.candidates {
                    let label_cols: usize = label
                        .chars()
                        .map(|ch| unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0))
                        .sum();
                    let text_cols: usize = text
                        .chars()
                        .map(|ch| unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0))
                        .sum();
                    max_label_w = max_label_w.max(label_cols as f32 * cell_w);
                    max_text_w = max_text_w.max(text_cols as f32 * cell_w);
                }
                let label_text_gap = cell_w; // Gap between label and text
                let win_w = padding * 2.0 + max_label_w + label_text_gap + max_text_w;
                let item_h = cell_h + item_gap;
                let num_cands = cands.candidates.len() as f32;
                // Page indicator row height (if has_prev/has_next)
                let indicator_h = if cands.has_prev || cands.has_next {
                    cell_h
                } else {
                    0.0
                };
                let win_h = padding * 2.0 + num_cands * item_h - item_gap + indicator_h;

                // Window position: directly below preedit (include margin)
                let pe_col = grid.cursor_col;
                let pe_row = grid.cursor_row;
                let mut win_x = margin_x + pe_col as f32 * cell_w;
                let mut win_y = margin_y + (pe_row + 1) as f32 * cell_h;

                // Screen edge overflow correction
                if win_x + win_w > screen_w as f32 {
                    win_x = (screen_w as f32 - win_w).max(0.0);
                }
                if win_y + win_h > screen_h as f32 {
                    // Display above
                    win_y = (pe_row as f32 * cell_h - win_h).max(0.0);
                }

                // Pass 2: UI background rendering (rounded rectangle)
                ui_renderer.begin();

                // Drop shadow + window background
                ui_renderer.push_shadow_rounded_rect(
                    win_x,
                    win_y,
                    win_w,
                    win_h,
                    corner_radius,
                    [0.15, 0.15, 0.18, 0.95], // Window background
                    3.0,                      // Shadow offset
                    [0.0, 0.0, 0.0, 0.4],     // Shadow color
                );

                // Selection highlight
                if cands.selected_index >= 0
                    && (cands.selected_index as usize) < cands.candidates.len()
                {
                    let sel_y = win_y + padding + cands.selected_index as f32 * item_h;
                    ui_renderer.push_rounded_rect(
                        win_x + padding * 0.5,
                        sel_y,
                        win_w - padding,
                        cell_h,
                        highlight_radius,
                        [0.3, 0.45, 0.7, 0.9], // Selection highlight color
                    );
                }

                ui_renderer.flush(gl, screen_w, screen_h);

                // Pass 3: Candidate text rendering
                text_renderer.begin();

                // Background colors for LCD subpixel compositing
                let win_bg = [0.15, 0.15, 0.18]; // Window background (matches ui_renderer)
                let sel_bg = [0.3, 0.45, 0.7]; // Selection highlight background

                for (i, (label, text)) in cands.candidates.iter().enumerate() {
                    let is_selected = i as i32 == cands.selected_index;
                    let item_y = win_y + padding + i as f32 * item_h;
                    let baseline_y = (item_y + ascent).round();

                    // Background for LCD compositing
                    let item_bg = if is_selected { sel_bg } else { win_bg };

                    // Label color
                    let label_color = if is_selected {
                        [0.8, 0.85, 0.9, 1.0]
                    } else {
                        [0.5, 0.55, 0.6, 1.0]
                    };
                    // Text color
                    let text_color = if is_selected {
                        [1.0, 1.0, 1.0, 1.0]
                    } else {
                        [0.92, 0.92, 0.95, 1.0]
                    };

                    // Label rendering
                    let label_x = win_x + padding;
                    for ch in label.chars() {
                        glyph_atlas.ensure_glyph(ch);
                    }
                    text_renderer.push_text_with_bg(
                        label,
                        label_x,
                        baseline_y,
                        label_color,
                        item_bg,
                        &glyph_atlas,
                    );

                    // Text rendering
                    let text_x = win_x + padding + max_label_w + label_text_gap;
                    for ch in text.chars() {
                        glyph_atlas.ensure_glyph(ch);
                    }
                    text_renderer.push_text_with_bg(
                        text,
                        text_x,
                        baseline_y,
                        text_color,
                        item_bg,
                        &glyph_atlas,
                    );
                }

                // Page indicator
                if cands.has_prev || cands.has_next {
                    let ind_y = win_y + padding + num_cands * item_h;
                    let baseline_y = (ind_y + ascent).round();
                    let indicator = match (cands.has_prev, cands.has_next) {
                        (true, true) => "< >",
                        (true, false) => "<",
                        (false, true) => ">",
                        _ => "",
                    };
                    if !indicator.is_empty() {
                        for ch in indicator.chars() {
                            glyph_atlas.ensure_glyph(ch);
                        }
                        // Right-align
                        let ind_cols: usize = indicator
                            .chars()
                            .map(|ch| unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0))
                            .sum();
                        let ind_x = win_x + win_w - padding - ind_cols as f32 * cell_w;
                        text_renderer.push_text_with_bg(
                            indicator,
                            ind_x,
                            baseline_y,
                            [0.5, 0.55, 0.6, 1.0],
                            win_bg, // Use window background for LCD compositing
                            &glyph_atlas,
                        );
                    }
                }

                glyph_atlas.upload_if_dirty(gl);
                text_renderer.flush(gl, &glyph_atlas, screen_w, screen_h);
            }
        }

        // === Copy mode status (screen bottom) ===
        if term.copy_mode.is_some() && !search_mode {
            let bar_h = cell_h + 8.0;
            let bar_y = screen_h as f32 - bar_h;
            let padding = 8.0_f32;

            // Background
            ui_renderer.begin();
            ui_renderer.push_rounded_rect(
                0.0,
                bar_y,
                screen_w as f32,
                bar_h,
                0.0,
                [0.2, 0.15, 0.1, 0.95], // Warm color
            );
            ui_renderer.flush(gl, screen_w, screen_h);

            // Text
            text_renderer.begin();

            let status = if term
                .copy_mode
                .as_ref()
                .map(|cm| cm.selecting)
                .unwrap_or(false)
            {
                "[VISUAL] v:toggle  y:yank  hjkl:move  /:search  Esc:exit"
            } else {
                "[COPY] v:select  y:yank  hjkl:move  /:search  Esc:exit"
            };

            // Background color for LCD compositing (matches ui_renderer bar)
            let bar_bg = [0.2, 0.15, 0.1];

            for ch in status.chars() {
                glyph_atlas.ensure_glyph(ch);
            }
            text_renderer.push_text_with_bg(
                status,
                padding,
                bar_y + 4.0 + ascent,
                [1.0, 0.9, 0.7, 1.0],
                bar_bg,
                &glyph_atlas,
            );

            glyph_atlas.upload_if_dirty(gl);
            text_renderer.flush(gl, &glyph_atlas, screen_w, screen_h);
        }

        // === Search bar (screen bottom) ===
        if search_mode {
            if let Some(ref search) = term.search {
                let bar_h = cell_h + 8.0;
                let bar_y = screen_h as f32 - bar_h;
                let padding = 8.0_f32;

                // Background
                ui_renderer.begin();
                ui_renderer.push_rounded_rect(
                    0.0,
                    bar_y,
                    screen_w as f32,
                    bar_h,
                    0.0,
                    [0.15, 0.15, 0.2, 0.95],
                );
                ui_renderer.flush(gl, screen_w, screen_h);

                // Text
                text_renderer.begin();

                // Background color for LCD compositing (matches ui_renderer bar)
                let search_bar_bg = [0.15, 0.15, 0.2];

                // Prompt "/"
                let prompt = "/";
                for ch in prompt.chars() {
                    glyph_atlas.ensure_glyph(ch);
                }
                text_renderer.push_text_with_bg(
                    prompt,
                    padding,
                    bar_y + 4.0 + ascent,
                    [0.6, 0.6, 0.6, 1.0],
                    search_bar_bg,
                    &glyph_atlas,
                );

                // Search query
                let query_x = padding + cell_w;
                for ch in search.query.chars() {
                    glyph_atlas.ensure_glyph(ch);
                }
                text_renderer.push_text_with_bg(
                    &search.query,
                    query_x,
                    bar_y + 4.0 + ascent,
                    [1.0, 1.0, 1.0, 1.0],
                    search_bar_bg,
                    &glyph_atlas,
                );

                // Cursor
                let query_cols: usize = search
                    .query
                    .chars()
                    .map(|ch| unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0))
                    .sum();
                let cursor_x = query_x + query_cols as f32 * cell_w;
                text_renderer.push_rect(
                    cursor_x,
                    bar_y + 4.0,
                    2.0,
                    cell_h,
                    [1.0, 1.0, 1.0, 0.8],
                    &glyph_atlas,
                );

                // Match count display
                if !search.matches.is_empty() {
                    let match_info =
                        format!("{}/{}", search.current_match + 1, search.matches.len());
                    for ch in match_info.chars() {
                        glyph_atlas.ensure_glyph(ch);
                    }
                    let info_cols: usize = match_info
                        .chars()
                        .map(|ch| unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0))
                        .sum();
                    let info_x = screen_w as f32 - padding - info_cols as f32 * cell_w;
                    text_renderer.push_text_with_bg(
                        &match_info,
                        info_x,
                        bar_y + 4.0 + ascent,
                        [0.7, 0.7, 0.7, 1.0],
                        search_bar_bg,
                        &glyph_atlas,
                    );
                }

                glyph_atlas.upload_if_dirty(gl);
                text_renderer.flush(gl, &glyph_atlas, screen_w, screen_h);
            }
        }

        // Bell flash is drawn after FBO blit (outside FBO cache)

        // === Screenshot ===
        if take_screenshot {
            take_screenshot = false;
            let user_home = term.user_home_dir();
            if let Err(e) = save_screenshot(
                gl,
                screen_w,
                screen_h,
                &cfg.paths.screenshot_dir,
                user_home.as_deref(),
            ) {
                log::warn!("Screenshot save failed: {}", e);
            } else {
                // Flash on success
                bell_flash_until =
                    Some(std::time::Instant::now() + Duration::from_millis(BELL_FLASH_DURATION_MS));
            }
        }

        // Unbind FBO and blit to screen
        fbo.unbind(gl);
        fbo.blit_to_screen(gl, screen_w, screen_h);

        // Draw text cursor directly to screen (outside FBO cache)
        // This prevents cursor trails in cached content
        {
            text_renderer.begin();

            if let Some(ref cm) = term.copy_mode {
                // Copy mode cursor (yellow outline)
                let cursor_x = margin_x + cm.cursor_col as f32 * cell_w;
                let cursor_y = margin_y + cm.cursor_row as f32 * cell_h;
                let border = 2.0_f32;
                // Top edge
                text_renderer.push_rect(
                    cursor_x,
                    cursor_y,
                    cell_w,
                    border,
                    [1.0, 0.8, 0.0, 0.9],
                    &glyph_atlas,
                );
                // Bottom edge
                text_renderer.push_rect(
                    cursor_x,
                    cursor_y + cell_h - border,
                    cell_w,
                    border,
                    [1.0, 0.8, 0.0, 0.9],
                    &glyph_atlas,
                );
                // Left edge
                text_renderer.push_rect(
                    cursor_x,
                    cursor_y,
                    border,
                    cell_h,
                    [1.0, 0.8, 0.0, 0.9],
                    &glyph_atlas,
                );
                // Right edge
                text_renderer.push_rect(
                    cursor_x + cell_w - border,
                    cursor_y,
                    border,
                    cell_h,
                    [1.0, 0.8, 0.0, 0.9],
                    &glyph_atlas,
                );
            } else if term.scroll_offset == 0 && grid.modes.cursor_visible {
                // Normal cursor (hidden during scrollback display)
                // In blink mode, toggle visibility with cursor_blink_visible
                let should_draw = !grid.cursor.blink || cursor_blink_visible;

                if should_draw {
                    // Display at end of preedit when composing
                    let cursor_x =
                        margin_x + (grid.cursor_col + preedit_total_cols) as f32 * cell_w;
                    let cursor_y = margin_y + grid.cursor_row as f32 * cell_h;
                    let mut cursor_rgb = [config_cursor.0, config_cursor.1, config_cursor.2];
                    let cursor_row = grid.cursor_row;
                    let cursor_col = grid.cursor_col;

                    // Draw according to cursor style
                    // Compute actual background under cursor (selection/search/inverse)
                    let cell = grid.cell(cursor_row, cursor_col);
                    let is_inverse = cell.attrs.contains(terminal::grid::CellAttrs::INVERSE);
                    let mut bg_rgba = if is_inverse {
                        effective_fg(&cell.fg)
                    } else {
                        effective_bg(&cell.bg)
                    };

                    if let Some((sel_start, sel_end)) = term
                        .selection
                        .as_ref()
                        .and_then(|s| s.cols_for_row(cursor_row, grid.cols()))
                    {
                        if cursor_col >= sel_start && cursor_col < sel_end {
                            let a = selection_alpha;
                            bg_rgba[0] = selection_color[0] * a + bg_rgba[0] * (1.0 - a);
                            bg_rgba[1] = selection_color[1] * a + bg_rgba[1] * (1.0 - a);
                            bg_rgba[2] = selection_color[2] * a + bg_rgba[2] * (1.0 - a);
                        }
                    }

                    if search_mode {
                        let current_match_idx = term.current_search_match();
                        for &(start_col, end_col, match_idx) in
                            term.get_search_matches_for_display_row(cursor_row)
                        {
                            if cursor_col >= start_col && cursor_col < end_col.min(grid.cols()) {
                                let is_current = current_match_idx == Some(match_idx);
                                let hl_color = if is_current {
                                    [1.0, 0.6, 0.0, 0.5]
                                } else {
                                    [1.0, 1.0, 0.0, 0.3]
                                };
                                let a = hl_color[3];
                                bg_rgba[0] = hl_color[0] * a + bg_rgba[0] * (1.0 - a);
                                bg_rgba[1] = hl_color[1] * a + bg_rgba[1] * (1.0 - a);
                                bg_rgba[2] = hl_color[2] * a + bg_rgba[2] * (1.0 - a);
                                break;
                            }
                        }
                    }

                    let cell_bg = [bg_rgba[0], bg_rgba[1], bg_rgba[2]];

                    // Ensure cursor is visible against light/dark backgrounds
                    let bg_luma = 0.2126 * cell_bg[0] + 0.7152 * cell_bg[1] + 0.0722 * cell_bg[2];
                    let cur_luma =
                        0.2126 * cursor_rgb[0] + 0.7152 * cursor_rgb[1] + 0.0722 * cursor_rgb[2];
                    if (cur_luma - bg_luma).abs() < 0.25 {
                        cursor_rgb = if bg_luma > 0.5 {
                            [0.0, 0.0, 0.0]
                        } else {
                            [1.0, 1.0, 1.0]
                        };
                    }
                    let cursor_color = [
                        cursor_rgb[0],
                        cursor_rgb[1],
                        cursor_rgb[2],
                        config_cursor_opacity,
                    ];

                    match grid.cursor.style {
                        terminal::grid::CursorStyle::Block => {
                            // First, draw cursor rectangle
                            let cursor_bg = [
                                cursor_color[0] * cursor_color[3]
                                    + cell_bg[0] * (1.0 - cursor_color[3]),
                                cursor_color[1] * cursor_color[3]
                                    + cell_bg[1] * (1.0 - cursor_color[3]),
                                cursor_color[2] * cursor_color[3]
                                    + cell_bg[2] * (1.0 - cursor_color[3]),
                            ];
                            text_renderer.push_rect(
                                cursor_x,
                                cursor_y,
                                cell_w,
                                cell_h,
                                [cursor_bg[0], cursor_bg[1], cursor_bg[2], 1.0],
                                &glyph_atlas,
                            );
                            // Flush cursor rectangle first to ensure it's below the text
                            text_renderer.flush(gl, &glyph_atlas, screen_w, screen_h);
                            text_renderer.begin();

                            // Draw character under cursor in inverted color
                            // (only when not in preedit mode)
                            if preedit_total_cols == 0 {
                                let cell = grid.cell(cursor_row, cursor_col);
                                let ch = cell.grapheme.chars().next().unwrap_or(' ');
                                if ch != ' ' {
                                    // Choose text color based on cursor luminance (black for light cursor, white for dark)
                                    // Prefer original cell foreground; if contrast is too low, invert to cell_bg
                                    let cell_fg = if is_inverse {
                                        effective_bg(&cell.bg)
                                    } else {
                                        effective_fg(&cell.fg)
                                    };
                                    let fg_luma = 0.2126 * cell_fg[0]
                                        + 0.7152 * cell_fg[1]
                                        + 0.0722 * cell_fg[2];
                                    let bg_luma = 0.2126 * cursor_bg[0]
                                        + 0.7152 * cursor_bg[1]
                                        + 0.0722 * cursor_bg[2];
                                    let contrast = (fg_luma - bg_luma).abs();
                                    let text_color = if contrast < 0.35 {
                                        [cell_bg[0], cell_bg[1], cell_bg[2], 1.0]
                                    } else {
                                        [cell_fg[0], cell_fg[1], cell_fg[2], 1.0]
                                    };
                                    let baseline_y = (cursor_y + glyph_atlas.ascent).round();
                                    glyph_atlas.ensure_glyph(ch);
                                    // Use lcd_disable=1.0 to avoid LCD artifacts on cursor
                                    text_renderer.push_char_with_bg(
                                        ch,
                                        cursor_x,
                                        baseline_y,
                                        text_color,
                                        cursor_bg,
                                        1.0,
                                        &glyph_atlas,
                                    );

                                    // Clip glyph to cursor cell to avoid bleeding outside the block
                                    unsafe {
                                        gl.enable(glow::SCISSOR_TEST);
                                        let sx = cursor_x.floor() as i32;
                                        let sy = cursor_y.floor() as i32;
                                        let sw = cell_w.ceil() as i32;
                                        let sh = cell_h.ceil() as i32;
                                        let flipped_y = screen_h as i32 - sy - sh;
                                        gl.scissor(sx, flipped_y, sw, sh);
                                    }
                                    text_renderer.flush(gl, &glyph_atlas, screen_w, screen_h);
                                    unsafe {
                                        gl.disable(glow::SCISSOR_TEST);
                                    }
                                    text_renderer.begin();
                                }
                            }
                        }
                        terminal::grid::CursorStyle::Underline => {
                            let underline_h = 2.0;
                            text_renderer.push_rect(
                                cursor_x,
                                cursor_y + cell_h - underline_h,
                                cell_w,
                                underline_h,
                                cursor_color,
                                &glyph_atlas,
                            );
                        }
                        terminal::grid::CursorStyle::Bar => {
                            let bar_w = 2.0;
                            text_renderer.push_rect(
                                cursor_x,
                                cursor_y,
                                bar_w,
                                cell_h,
                                cursor_color,
                                &glyph_atlas,
                            );
                        }
                    }
                }
            }

            text_renderer.flush(gl, &glyph_atlas, screen_w, screen_h);
        }

        // Draw mouse cursor directly to screen (outside FBO cache)
        {
            let cursor_size = 8.0_f32;
            let mx = mouse_x as f32;
            let my = mouse_y as f32;
            text_renderer.begin();
            // Crosshair style with outline for visibility on any background
            let outline_color = [0.0, 0.0, 0.0, 0.9];
            let fill_color = [1.0, 1.0, 1.0, 0.9];
            // Black outline (1px larger on each side)
            text_renderer.push_rect(
                mx - cursor_size / 2.0 - 1.0,
                my - 2.0,
                cursor_size + 2.0,
                4.0,
                outline_color,
                &glyph_atlas,
            );
            text_renderer.push_rect(
                mx - 2.0,
                my - cursor_size / 2.0 - 1.0,
                4.0,
                cursor_size + 2.0,
                outline_color,
                &glyph_atlas,
            );
            // White fill
            text_renderer.push_rect(
                mx - cursor_size / 2.0,
                my - 1.0,
                cursor_size,
                2.0,
                fill_color,
                &glyph_atlas,
            );
            text_renderer.push_rect(
                mx - 1.0,
                my - cursor_size / 2.0,
                2.0,
                cursor_size,
                fill_color,
                &glyph_atlas,
            );
            text_renderer.flush(gl, &glyph_atlas, screen_w, screen_h);
        }

        // === Bell flash (visual bell) - outside FBO cache ===
        if bell_flash_until.is_some() {
            // Light up screen edges (border only)
            let border = 4.0;
            let color = [1.0, 0.6, 0.2, 0.8]; // Orange
            text_renderer.begin();
            // Top
            text_renderer.push_rect(0.0, 0.0, screen_w as f32, border, color, &glyph_atlas);
            // Bottom
            text_renderer.push_rect(
                0.0,
                screen_h as f32 - border,
                screen_w as f32,
                border,
                color,
                &glyph_atlas,
            );
            // Left
            text_renderer.push_rect(
                0.0,
                border,
                border,
                screen_h as f32 - border * 2.0,
                color,
                &glyph_atlas,
            );
            // Right
            text_renderer.push_rect(
                screen_w as f32 - border,
                border,
                border,
                screen_h as f32 - border * 2.0,
                color,
                &glyph_atlas,
            );
            text_renderer.flush(gl, &glyph_atlas, screen_w, screen_h);
        }

        // Buffer swap (skip during Synchronized Update mode or when VT switched away)
        // CSI ? 2026 h starts buffering, CSI ? 2026 l displays all at once
        if !term.is_synchronized_update() && drm_master_held {
            egl_context.swap_buffers()?;

            // Get front buffer and display to DRM
            let new_bo = gbm_surface.lock_front_buffer()?;
            let new_fb = drm::DrmFramebuffer::from_bo(&drm_device, &new_bo)?;
            drm::set_crtc(&drm_device, &display_config, &new_fb)?;

            // Release previous frame
            prev_bo = Some(new_bo);
            prev_fb = Some(new_fb);

            // Clear dirty flags after successful render
            term.clear_dirty();
        }
    }

    // Keyboard restoration (automatic on drop)
    drop(keyboard);

    // Resource cleanup
    fbo.destroy(gl);
    emoji_renderer.destroy(gl);
    image_renderer.destroy(gl);
    ui_renderer.destroy(gl);
    curly_renderer.destroy(gl);
    text_renderer.destroy(gl);
    glyph_atlas.destroy(gl);
    emoji_atlas.destroy(gl);

    // Restore previous mode
    drop(prev_fb);
    drop(prev_bo);
    if let Some(crtc) = saved_crtc {
        crtc.restore(&drm_device, display_config.crtc_handle);
    }

    // Delete clipboard file
    let _ = std::fs::remove_file(&cfg.paths.clipboard_file);

    info!("bcon terminated");
    Ok(())
}
