//! Sixel graphics decoder
//!
//! Parses Sixel image data sent via DCS sequences and converts to RGBA images.
//!
//! ## Sixel Format Overview
//!
//! Sixel is a bitmap graphics format designed for serial terminals.
//! Each character encodes 6 vertical pixels (hence "six-pixel" → "sixel").
//!
//! ### Sequence Structure
//! ```text
//! DCS Pa ; Pb ; Ph q [sixel-data] ST
//! ```
//! - `Pa`: Pixel aspect ratio (usually 0 or 1)
//! - `Pb`: Background mode (0=transparent, 1=use color 0)
//! - `Ph`: Horizontal grid size (ignored by most implementations)
//!
//! ### Data Characters
//! - `0x3F-0x7E` ('?' to '~'): 6-bit pixel pattern, value = char - 0x3F
//! - `#Pc;Pu;Px;Py;Pz`: Define color Pc (Pu=2: RGB as 0-100% each)
//! - `#Pc`: Select color Pc for subsequent pixels
//! - `!n<char>`: RLE - repeat character n times
//! - `$`: Carriage return (X = 0, stay on same row)
//! - `-`: Line feed (X = 0, Y += 6)
//!
//! ## References
//! - VT340 Graphics Programming: <https://vt100.net/docs/vt3xx-gp/chapter14.html>
//! - libsixel: <https://github.com/saitoha/libsixel>
//! - Sixel format: <https://en.wikipedia.org/wiki/Sixel>

use log::{trace, warn};

/// Maximum image dimension (16384 pixels - supports 8K and beyond)
const MAX_IMAGE_DIMENSION: u32 = 16384;

/// Maximum pixel buffer size (256MB - same as Kitty)
const MAX_PIXEL_BUFFER_SIZE: usize = 256 * 1024 * 1024;

/// Sixel image data (after decoding)
#[derive(Debug, Clone)]
pub struct SixelImage {
    /// Unique image ID
    pub id: u32,
    /// Image width (pixels)
    pub width: u32,
    /// Image height (pixels)
    pub height: u32,
    /// RGBA pixel data (row-major, 4 bytes/pixel)
    pub data: Vec<u8>,
}

/// Decoder internal state
#[derive(Debug, Clone, Copy, PartialEq)]
enum State {
    /// Normal (waiting for sixel data)
    Normal,
    /// # command (color definition/selection) parsing
    Color,
    /// ! RLE count parsing
    Rle,
    /// " raster attributes parsing (Pan;Pad;Ph;Pv)
    RasterAttr,
}

/// Sixel decoder
///
/// Receives data in streaming fashion, generates SixelImage via finish() at the end.
pub struct SixelDecoder {
    /// Color palette (256 colors, RGB)
    palette: [(u8, u8, u8); 256],
    /// Currently selected color index
    current_color: u8,
    /// Pixel buffer (palette indices)
    /// Unpainted pixels are 255 (treated as transparent)
    pixels: Vec<u8>,
    /// Current image width
    width: u32,
    /// Current image height
    height: u32,
    /// Current drawing X coordinate
    x: u32,
    /// Current drawing Y coordinate (row in 6-pixel units)
    y: u32,
    /// RLE count (None for single)
    rle_count: Option<u32>,
    /// Parameter accumulation buffer
    param_buf: String,
    /// Internal state
    state: State,
    /// Raster attributes: aspect ratio (Pan, Pad) and image size (Ph, Pv)
    raster_attr: Option<(u32, u32, u32, u32)>,
}

impl SixelDecoder {
    /// Create new decoder
    pub fn new() -> Self {
        // Default palette (VT340 compatible 16 colors + rest filled with black)
        let mut palette = [(0u8, 0u8, 0u8); 256];

        // VT340 default 16 colors
        palette[0] = (0, 0, 0); // black
        palette[1] = (51, 51, 204); // blue
        palette[2] = (204, 33, 33); // red
        palette[3] = (51, 204, 51); // green
        palette[4] = (204, 51, 204); // magenta
        palette[5] = (51, 204, 204); // cyan
        palette[6] = (204, 204, 51); // yellow
        palette[7] = (135, 135, 135); // gray 50%
        palette[8] = (68, 68, 68); // gray 25%
        palette[9] = (84, 84, 255); // light blue
        palette[10] = (255, 84, 84); // light red
        palette[11] = (84, 255, 84); // light green
        palette[12] = (255, 84, 255); // light magenta
        palette[13] = (84, 255, 255); // light cyan
        palette[14] = (255, 255, 84); // light yellow
        palette[15] = (204, 204, 204); // gray 75%

        Self {
            palette,
            current_color: 0,
            pixels: Vec::new(),
            width: 0,
            height: 0,
            x: 0,
            y: 0,
            rle_count: None,
            param_buf: String::new(),
            state: State::Normal,
            raster_attr: None,
        }
    }

    /// Process one byte in streaming fashion
    pub fn push(&mut self, byte: u8) {
        match self.state {
            State::Normal => self.handle_normal(byte),
            State::Color => self.handle_color(byte),
            State::Rle => self.handle_rle(byte),
            State::RasterAttr => self.handle_raster_attr(byte),
        }
    }

    /// Handle normal state
    fn handle_normal(&mut self, byte: u8) {
        match byte {
            b'#' => {
                // Color command start
                self.param_buf.clear();
                self.state = State::Color;
            }
            b'!' => {
                // RLE start
                self.param_buf.clear();
                self.state = State::Rle;
            }
            b'$' => {
                // Graphics return (X=0)
                self.x = 0;
            }
            b'-' => {
                // Graphics newline (Y+=6, X=0)
                self.y += 1;
                self.x = 0;
            }
            b'"' => {
                // Raster attributes start
                self.param_buf.clear();
                self.state = State::RasterAttr;
            }
            0x3F..=0x7E => {
                // Sixel data character (6-bit pattern)
                let pattern = byte - 0x3F;
                let count = self.rle_count.take().unwrap_or(1);
                self.draw_sixel(pattern, count);
            }
            _ => {
                // Ignore
            }
        }
    }

    /// Handle color command (#Pc or #Pc;Pu;Px;Py;Pz)
    fn handle_color(&mut self, byte: u8) {
        match byte {
            b'0'..=b'9' | b';' => {
                self.param_buf.push(byte as char);
            }
            _ => {
                // Command end, parse and apply
                self.parse_color_command();
                self.state = State::Normal;
                // Re-process if terminator is Sixel data
                if (0x3F..=0x7E).contains(&byte) {
                    self.handle_normal(byte);
                } else if byte == b'#' {
                    self.param_buf.clear();
                    self.state = State::Color;
                } else if byte == b'!' {
                    self.param_buf.clear();
                    self.state = State::Rle;
                } else if byte == b'$' || byte == b'-' {
                    self.handle_normal(byte);
                }
            }
        }
    }

    /// Handle RLE count (!n)
    fn handle_rle(&mut self, byte: u8) {
        match byte {
            b'0'..=b'9' => {
                self.param_buf.push(byte as char);
            }
            _ => {
                // Count finalized
                let count: u32 = self.param_buf.parse().unwrap_or(1);
                self.rle_count = Some(count.max(1));
                self.state = State::Normal;
                // Draw immediately if terminator is Sixel data
                if (0x3F..=0x7E).contains(&byte) {
                    self.handle_normal(byte);
                }
            }
        }
    }

    /// Handle raster attributes ("Pan;Pad;Ph;Pv)
    fn handle_raster_attr(&mut self, byte: u8) {
        match byte {
            b'0'..=b'9' | b';' => {
                self.param_buf.push(byte as char);
            }
            _ => {
                self.parse_raster_attr();
                self.state = State::Normal;
                // Re-process terminator
                if (0x3F..=0x7E).contains(&byte)
                    || byte == b'#'
                    || byte == b'!'
                    || byte == b'$'
                    || byte == b'-'
                {
                    self.handle_normal(byte);
                }
            }
        }
    }

    /// Parse color command
    fn parse_color_command(&mut self) {
        let parts: Vec<&str> = self.param_buf.split(';').collect();
        if parts.is_empty() {
            return;
        }

        let color_idx: u8 = parts[0].parse().unwrap_or(0);

        if parts.len() == 1 {
            // Color selection only: #Pc
            self.current_color = color_idx;
            trace!("Sixel: color selection #{}", color_idx);
        } else if parts.len() >= 5 {
            // Color definition: #Pc;Pu;Px;Py;Pz
            let pu: u32 = parts[1].parse().unwrap_or(0);
            let px: u32 = parts[2].parse().unwrap_or(0);
            let py: u32 = parts[3].parse().unwrap_or(0);
            let pz: u32 = parts[4].parse().unwrap_or(0);

            let (r, g, b) = match pu {
                1 => {
                    // HLS color space (0-360, 0-100, 0-100)
                    hls_to_rgb(px, py, pz)
                }
                2 => {
                    // RGB color space (0-100%)
                    let r = (px * 255 / 100).min(255) as u8;
                    let g = (py * 255 / 100).min(255) as u8;
                    let b = (pz * 255 / 100).min(255) as u8;
                    (r, g, b)
                }
                _ => {
                    // Default: interpret as RGB
                    let r = (px * 255 / 100).min(255) as u8;
                    let g = (py * 255 / 100).min(255) as u8;
                    let b = (pz * 255 / 100).min(255) as u8;
                    (r, g, b)
                }
            };

            self.palette[color_idx as usize] = (r, g, b);
            self.current_color = color_idx;
            trace!(
                "Sixel: color definition #{} = ({}, {}, {})",
                color_idx,
                r,
                g,
                b
            );
        }
    }

    /// Parse raster attributes
    fn parse_raster_attr(&mut self) {
        let parts: Vec<&str> = self.param_buf.split(';').collect();
        if parts.len() >= 4 {
            let pan: u32 = parts[0].parse().unwrap_or(1);
            let pad: u32 = parts[1].parse().unwrap_or(1);
            let ph: u32 = parts[2].parse().unwrap_or(0);
            let pv: u32 = parts[3].parse().unwrap_or(0);
            self.raster_attr = Some((pan, pad, ph, pv));

            // Pre-allocate if image size is specified
            if ph > 0 && pv > 0 {
                self.ensure_size(ph, pv);
            }
            trace!("Sixel: raster attributes {}:{} {}x{}", pan, pad, ph, pv);
        }
    }

    /// Draw Sixel pattern
    fn draw_sixel(&mut self, pattern: u8, count: u32) {
        for _ in 0..count {
            // Draw 6-bit pattern as 6 vertical pixels
            let px = self.x;
            let py_base = self.y * 6;

            for bit in 0..6 {
                if (pattern >> bit) & 1 != 0 {
                    let py = py_base + bit;
                    self.set_pixel(px, py, self.current_color);
                }
            }

            self.x += 1;
        }
    }

    /// Set pixel (expand buffer as needed)
    fn set_pixel(&mut self, x: u32, y: u32, color: u8) {
        // Expand image size
        let new_w = x + 1;
        let new_h = y + 1;
        self.ensure_size(new_w, new_h);

        let idx = (y * self.width + x) as usize;
        if idx < self.pixels.len() {
            self.pixels[idx] = color;
        }
    }

    /// Ensure image size
    fn ensure_size(&mut self, new_w: u32, new_h: u32) {
        let need_resize = new_w > self.width || new_h > self.height;
        if !need_resize {
            return;
        }

        let target_w = new_w.max(self.width).min(MAX_IMAGE_DIMENSION);
        let target_h = new_h.max(self.height).min(MAX_IMAGE_DIMENSION);

        // Check total buffer size (use checked_mul to prevent overflow)
        match (target_w as usize).checked_mul(target_h as usize) {
            Some(size) if size <= MAX_PIXEL_BUFFER_SIZE => {}
            _ => {
                warn!(
                    "Sixel: image size {}x{} exceeds buffer limit, ignoring",
                    target_w, target_h
                );
                return;
            }
        }

        if self.width == 0 {
            // Initialize
            self.width = target_w;
            self.height = target_h;
            self.pixels = vec![255; (target_w * target_h) as usize];
        } else {
            // Resize while preserving existing data
            let mut new_pixels = vec![255u8; (target_w * target_h) as usize];
            for row in 0..self.height {
                let src_start = (row * self.width) as usize;
                let src_end = src_start + self.width as usize;
                let dst_start = (row * target_w) as usize;
                let dst_end = dst_start + self.width as usize;
                if src_end <= self.pixels.len() && dst_end <= new_pixels.len() {
                    new_pixels[dst_start..dst_end]
                        .copy_from_slice(&self.pixels[src_start..src_end]);
                }
            }
            self.pixels = new_pixels;
            self.width = target_w;
            self.height = target_h;
        }
    }

    /// Decode complete, generate RGBA image
    pub fn finish(self, id: u32) -> Option<SixelImage> {
        if self.width == 0 || self.height == 0 {
            return None;
        }

        // Check RGBA buffer size (4 bytes per pixel)
        let rgba_size = (self.width as usize)
            .checked_mul(self.height as usize)
            .and_then(|wh| wh.checked_mul(4));
        let rgba_size = match rgba_size {
            Some(size) if size <= MAX_PIXEL_BUFFER_SIZE => size,
            _ => {
                warn!(
                    "Sixel: RGBA buffer too large ({}x{}x4), skipping",
                    self.width, self.height
                );
                return None;
            }
        };

        // Convert palette indices to RGBA
        let mut data = Vec::with_capacity(rgba_size);
        for &idx in &self.pixels {
            if idx == 255 {
                // Transparent pixel
                data.extend_from_slice(&[0, 0, 0, 0]);
            } else {
                let (r, g, b) = self.palette[idx as usize];
                data.extend_from_slice(&[r, g, b, 255]);
            }
        }

        trace!(
            "Sixel: decode complete {}x{} ({} bytes)",
            self.width,
            self.height,
            data.len()
        );

        Some(SixelImage {
            id,
            width: self.width,
            height: self.height,
            data,
        })
    }
}

impl Default for SixelDecoder {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert HLS (0-360, 0-100, 0-100) to RGB (0-255)
fn hls_to_rgb(h: u32, l: u32, s: u32) -> (u8, u8, u8) {
    if s == 0 {
        // Achromatic
        let v = (l * 255 / 100).min(255) as u8;
        return (v, v, v);
    }

    let h = h as f32;
    let l = l as f32 / 100.0;
    let s = s as f32 / 100.0;

    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;

    let (r1, g1, b1) = match h as u32 {
        0..=59 => (c, x, 0.0),
        60..=119 => (x, c, 0.0),
        120..=179 => (0.0, c, x),
        180..=239 => (0.0, x, c),
        240..=299 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };

    let r = ((r1 + m) * 255.0).round().min(255.0).max(0.0) as u8;
    let g = ((g1 + m) * 255.0).round().min(255.0).max(0.0) as u8;
    let b = ((b1 + m) * 255.0).round().min(255.0).max(0.0) as u8;

    (r, g, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_sixel() {
        let mut decoder = SixelDecoder::new();
        // 1x6 red vertical line
        // #0;2;100;0;0 sets color 0 to red
        // ? (0x3F) is all bits 0
        // ~ (0x7E) is all bits 1 (paints 6 vertical pixels)
        for b in b"#0;2;100;0;0~" {
            decoder.push(*b);
        }
        let img = decoder.finish(1).unwrap();
        assert_eq!(img.width, 1);
        assert_eq!(img.height, 6);
        // First pixel is red
        assert_eq!(&img.data[0..4], &[255, 0, 0, 255]);
    }

    #[test]
    fn test_rle() {
        let mut decoder = SixelDecoder::new();
        // Repeat 3 times
        for b in b"#0;2;0;100;0!3~" {
            decoder.push(*b);
        }
        let img = decoder.finish(1).unwrap();
        assert_eq!(img.width, 3);
        assert_eq!(img.height, 6);
    }

    #[test]
    fn test_color_selection() {
        let mut decoder = SixelDecoder::new();
        // Set color 0 to red, color 1 to blue and draw
        for b in b"#0;2;100;0;0~#1;2;0;0;100~" {
            decoder.push(*b);
        }
        let img = decoder.finish(1).unwrap();
        assert_eq!(img.width, 2);
        // First column is red
        assert_eq!(&img.data[0..4], &[255, 0, 0, 255]);
        // Second column is blue
        assert_eq!(&img.data[4..8], &[0, 0, 255, 255]);
    }
}
