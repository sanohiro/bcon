//! LCD subpixel rendering glyph atlas
//!
//! Uses FreeType for LCD subpixel rendering.
//! Used separately from the normal grayscale atlas.

use anyhow::{anyhow, Result};
use glow::HasContext;
use log::{debug, info, warn};
use std::collections::HashMap;

use super::freetype::{FtFont, FtGlyph, HintingMode, LcdFilterMode, LcdMode, SubpixelPhase};

/// Glyph ID based lookup key
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(dead_code)]
pub struct GlyphKey {
    pub font_idx: u8,
    pub glyph_id: u16,
}

/// Glyph key with subpixel phase
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct PhasedGlyphKey {
    ch: char,
    phase: u8, // 0, 1, 2
}

/// Glyph information
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct GlyphInfo {
    pub uv_x: f32,
    pub uv_y: f32,
    pub uv_w: f32,
    pub uv_h: f32,
    pub width: u32,
    pub height: u32,
    pub bearing_x: f32,
    pub bearing_y: f32,
    pub advance: f32,
}

/// LCD subpixel rendering glyph atlas
#[allow(dead_code)]
pub struct LcdGlyphAtlas {
    texture: glow::Texture,
    /// Glyphs without subpixel phase (backward compatible)
    glyphs: HashMap<char, GlyphInfo>,
    /// Glyphs with subpixel phase (1/3px precision)
    phased_glyphs: HashMap<PhasedGlyphKey, GlyphInfo>,
    glyph_id_map: HashMap<GlyphKey, GlyphInfo>,
    pub atlas_width: u32,
    pub atlas_height: u32,
    pub cell_width: f32,
    pub cell_height: f32,
    pub ascent: f32,
    solid_uv: (f32, f32),

    // FreeType font
    font_main: FtFont,
    font_cjk: Option<FtFont>,
    font_size: u32,

    // Shelf packing
    cursor_x: u32,
    cursor_y: u32,
    row_height: u32,

    // CPU-side texture data (RGB8)
    atlas_data: Vec<u8>,
    dirty: bool,

    /// Enable subpixel phase rendering
    subpixel_positioning: bool,
}

#[allow(dead_code)]
impl LcdGlyphAtlas {
    /// Create LCD atlas
    /// subpixel_positioning: Enable 1/3 pixel phase rendering
    pub fn new(
        gl: &glow::Context,
        font_data: &[u8],
        font_size: u32,
        cjk_font_data: Option<&[u8]>,
        lcd_mode: LcdMode,
        lcd_filter: LcdFilterMode,
        lcd_weights: Option<[u8; 5]>,
        subpixel_positioning: bool,
        hinting_mode: HintingMode,
    ) -> Result<Self> {
        let font_main = FtFont::from_bytes(font_data, font_size, lcd_mode, lcd_filter, lcd_weights, hinting_mode)?;

        let font_cjk = if let Some(cjk_data) = cjk_font_data {
            match FtFont::from_bytes(cjk_data, font_size, lcd_mode, lcd_filter, lcd_weights, hinting_mode) {
                Ok(f) => {
                    info!("CJK font loaded (FreeType)");
                    Some(f)
                }
                Err(e) => {
                    warn!("Failed to load CJK font: {:?}", e);
                    None
                }
            }
        } else {
            None
        };

        // Line metrics
        // descender is negative, so ascent - descender is actual height
        let (ascent, descender, line_height) = font_main.line_metrics();
        let min_height = ascent - descender; // Minimum height to fit characters
        // +1.0 safety margin to prevent bottom clipping
        let cell_height = line_height.max(min_height).ceil() + 1.0;

        // Determine cell width from 'M' (round to integer to prevent pixel misalignment)
        let cell_width = font_main
            .rasterize('M')
            .map(|g| g.advance.round())
            .unwrap_or((font_size as f32 * 0.6).round());

        info!(
            "LCD font metrics: ascent={:.1}, cell={}x{:.0}",
            ascent, cell_width, cell_height
        );

        // Atlas size (RGB8 uses 3x memory)
        let atlas_width = 2048u32;
        let atlas_height = 2048u32;

        // RGB8 format (3 bytes per pixel)
        let mut atlas_data = vec![0u8; (atlas_width * atlas_height * 3) as usize];

        // White pixel for rectangle drawing (2x2)
        let aw = atlas_width as usize;
        for y in 0..2 {
            for x in 0..2 {
                let idx = (y * aw + x) * 3;
                atlas_data[idx] = 255; // R
                atlas_data[idx + 1] = 255; // G
                atlas_data[idx + 2] = 255; // B
            }
        }
        let solid_uv = (0.5 / atlas_width as f32, 0.5 / atlas_height as f32);

        let mut glyphs = HashMap::new();

        // Shelf packing initial position
        let pad = 4u32;
        let mut cursor_x = 4u32;
        let mut cursor_y = 0u32;
        let mut row_height = 0u32;

        // ASCII preload
        for ch in (0x20u8..=0x7Eu8).map(|c| c as char) {
            if let Some(ft_glyph) = font_main.rasterize(ch) {
                if ft_glyph.width == 0 || ft_glyph.height == 0 {
                    continue;
                }

                let bw = ft_glyph.width;
                let bh = ft_glyph.height;

                // Move to next row if doesn't fit
                if cursor_x + bw + pad > atlas_width {
                    cursor_y += row_height + pad;
                    cursor_x = 0;
                    row_height = 0;
                }

                if cursor_y + bh > atlas_height {
                    warn!("LCD atlas overflow: '{}'", ch);
                    continue;
                }

                let x0 = cursor_x;
                let y0 = cursor_y;

                // Copy RGB bitmap
                for y in 0..bh {
                    for x in 0..bw {
                        let src_idx = ((y * bw + x) * 3) as usize;
                        let dst_idx = (((y0 + y) * atlas_width + (x0 + x)) * 3) as usize;
                        if src_idx + 2 < ft_glyph.bitmap.len() && dst_idx + 2 < atlas_data.len() {
                            atlas_data[dst_idx] = ft_glyph.bitmap[src_idx];
                            atlas_data[dst_idx + 1] = ft_glyph.bitmap[src_idx + 1];
                            atlas_data[dst_idx + 2] = ft_glyph.bitmap[src_idx + 2];
                        }
                    }
                }

                let aw_f = atlas_width as f32;
                let ah_f = atlas_height as f32;

                glyphs.insert(
                    ch,
                    GlyphInfo {
                        uv_x: x0 as f32 / aw_f,
                        uv_y: y0 as f32 / ah_f,
                        uv_w: bw as f32 / aw_f,
                        uv_h: bh as f32 / ah_f,
                        width: bw,
                        height: bh,
                        bearing_x: ft_glyph.bearing_x as f32,
                        bearing_y: ft_glyph.bearing_y as f32,
                        advance: ft_glyph.advance,
                    },
                );

                cursor_x += bw + pad;
                row_height = row_height.max(bh);
            }
        }

        // Create OpenGL texture (RGB8)
        let texture = unsafe {
            let tex = gl
                .create_texture()
                .map_err(|e| anyhow!("Failed to create texture: {}", e))?;

            gl.bind_texture(glow::TEXTURE_2D, Some(tex));

            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGB8 as i32,
                atlas_width as i32,
                atlas_height as i32,
                0,
                glow::RGB,
                glow::UNSIGNED_BYTE,
                Some(&atlas_data),
            );

            // LCD subpixel uses NEAREST for accurate sampling
            // LINEAR causes RGB to mix and blur
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::NEAREST as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::NEAREST as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_S,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_T,
                glow::CLAMP_TO_EDGE as i32,
            );

            gl.bind_texture(glow::TEXTURE_2D, None);
            tex
        };

        info!(
            "LCD glyph atlas generated: {}x{}, {} glyphs (subpixel_positioning={})",
            atlas_width,
            atlas_height,
            glyphs.len(),
            subpixel_positioning
        );

        Ok(Self {
            texture,
            glyphs,
            phased_glyphs: HashMap::new(),
            glyph_id_map: HashMap::new(),
            atlas_width,
            atlas_height,
            cell_width,
            cell_height,
            ascent,
            solid_uv,
            font_main,
            font_cjk,
            font_size,
            cursor_x,
            cursor_y,
            row_height,
            atlas_data,
            subpixel_positioning,
            dirty: false,
        })
    }

    /// Ensure glyph for character
    /// Caches all 3 phases if subpixel phase is enabled
    pub fn ensure_glyph(&mut self, ch: char) {
        if ch <= ' ' {
            return;
        }

        // Normal glyph (Phase0 or without phase)
        if !self.glyphs.contains_key(&ch) {
            let ft_glyph = if let Some(g) = self.font_main.rasterize(ch) {
                g
            } else if let Some(ref cjk) = self.font_cjk {
                if let Some(g) = cjk.rasterize(ch) {
                    g
                } else {
                    debug!("Glyph not found: U+{:04X}", ch as u32);
                    return;
                }
            } else {
                return;
            };

            if let Some(info) = self.pack_glyph(&ft_glyph, &format!("U+{:04X}", ch as u32)) {
                self.glyphs.insert(ch, info);
            }
        }

        // Cache all 3 phases if subpixel phase is enabled
        if self.subpixel_positioning {
            for phase in [SubpixelPhase::Phase0, SubpixelPhase::Phase1, SubpixelPhase::Phase2] {
                let key = PhasedGlyphKey {
                    ch,
                    phase: phase as u8,
                };
                if self.phased_glyphs.contains_key(&key) {
                    continue;
                }

                let ft_glyph = if let Some(g) = self.font_main.rasterize_with_phase(ch, phase) {
                    g
                } else if let Some(ref mut cjk) = self.font_cjk {
                    if let Some(g) = cjk.rasterize_with_phase(ch, phase) {
                        g
                    } else {
                        continue;
                    }
                } else {
                    continue;
                };

                if let Some(info) = self.pack_glyph(&ft_glyph, &format!("U+{:04X}@{}", ch as u32, phase as u8)) {
                    self.phased_glyphs.insert(key, info);
                }
            }
        }
    }

    fn pack_glyph(&mut self, glyph: &FtGlyph, label: &str) -> Option<GlyphInfo> {
        let bw = glyph.width;
        let bh = glyph.height;

        if bw == 0 || bh == 0 {
            return None;
        }

        let pad = 4u32;

        if self.cursor_x + bw + pad > self.atlas_width {
            self.cursor_y += self.row_height + pad;
            self.cursor_x = 0;
            self.row_height = 0;
        }

        if self.cursor_y + bh > self.atlas_height {
            warn!("LCD atlas full: {}", label);
            return None;
        }

        let x0 = self.cursor_x;
        let y0 = self.cursor_y;

        for y in 0..bh {
            for x in 0..bw {
                let src_idx = ((y * bw + x) * 3) as usize;
                let dst_idx = (((y0 + y) * self.atlas_width + (x0 + x)) * 3) as usize;
                if src_idx + 2 < glyph.bitmap.len() && dst_idx + 2 < self.atlas_data.len() {
                    self.atlas_data[dst_idx] = glyph.bitmap[src_idx];
                    self.atlas_data[dst_idx + 1] = glyph.bitmap[src_idx + 1];
                    self.atlas_data[dst_idx + 2] = glyph.bitmap[src_idx + 2];
                }
            }
        }

        let aw = self.atlas_width as f32;
        let ah = self.atlas_height as f32;

        self.cursor_x += bw + pad;
        self.row_height = self.row_height.max(bh);
        self.dirty = true;

        Some(GlyphInfo {
            uv_x: x0 as f32 / aw,
            uv_y: y0 as f32 / ah,
            uv_w: bw as f32 / aw,
            uv_h: bh as f32 / ah,
            width: bw,
            height: bh,
            bearing_x: glyph.bearing_x as f32,
            bearing_y: glyph.bearing_y as f32,
            advance: glyph.advance,
        })
    }

    /// Upload to GPU
    pub fn upload_if_dirty(&mut self, gl: &glow::Context) {
        if !self.dirty {
            return;
        }
        self.dirty = false;

        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(self.texture));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGB8 as i32,
                self.atlas_width as i32,
                self.atlas_height as i32,
                0,
                glow::RGB,
                glow::UNSIGNED_BYTE,
                Some(&self.atlas_data),
            );
            gl.bind_texture(glow::TEXTURE_2D, None);
        }

        debug!("LCD atlas re-uploaded: {} glyphs", self.glyphs.len());
    }

    pub fn solid_uv(&self) -> (f32, f32) {
        self.solid_uv
    }

    pub fn get_glyph(&self, ch: char) -> Option<&GlyphInfo> {
        self.glyphs.get(&ch)
    }

    /// Get glyph with subpixel phase
    /// x_frac: Fractional part of X coordinate (0.0..1.0)
    /// Returns normal glyph if phase rendering is disabled
    pub fn get_glyph_phased(&self, ch: char, x_frac: f32) -> Option<&GlyphInfo> {
        if !self.subpixel_positioning {
            return self.glyphs.get(&ch);
        }

        let phase = SubpixelPhase::from_frac(x_frac);
        let key = PhasedGlyphKey {
            ch,
            phase: phase as u8,
        };
        self.phased_glyphs.get(&key).or_else(|| self.glyphs.get(&ch))
    }

    /// Ensure glyph with subpixel phase
    pub fn ensure_glyph_phased(&mut self, ch: char, x_frac: f32) {
        if !self.subpixel_positioning {
            self.ensure_glyph(ch);
            return;
        }

        let phase = SubpixelPhase::from_frac(x_frac);
        let key = PhasedGlyphKey {
            ch,
            phase: phase as u8,
        };

        if self.phased_glyphs.contains_key(&key) || ch <= ' ' {
            return;
        }

        // Rasterize with subpixel phase
        let ft_glyph = if let Some(g) = self.font_main.rasterize_with_phase(ch, phase) {
            g
        } else if let Some(ref mut cjk) = self.font_cjk {
            if let Some(g) = cjk.rasterize_with_phase(ch, phase) {
                g
            } else {
                return;
            }
        } else {
            return;
        };

        if let Some(info) = self.pack_glyph(&ft_glyph, &format!("U+{:04X}@{}", ch as u32, phase as u8)) {
            self.phased_glyphs.insert(key, info);
        }
    }

    /// Get drawing position correction from subpixel phase
    pub fn phase_offset(&self, x_frac: f32) -> f32 {
        if !self.subpixel_positioning {
            return 0.0;
        }
        let phase = SubpixelPhase::from_frac(x_frac);
        // Subtract phase offset to place glyph at correct position
        -phase.offset()
    }

    /// Check if subpixel phase rendering is enabled
    pub fn is_subpixel_positioning_enabled(&self) -> bool {
        self.subpixel_positioning
    }

    pub fn bind(&self, gl: &glow::Context, unit: u32) {
        unsafe {
            gl.active_texture(glow::TEXTURE0 + unit);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.texture));
        }
    }

    pub fn destroy(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_texture(self.texture);
        }
    }

    /// Change font size and rebuild atlas
    pub fn resize(&mut self, new_size: f32) -> (f32, f32) {
        let new_size_u32 = new_size as u32;
        if new_size_u32 == self.font_size {
            return (self.cell_width, self.cell_height);
        }

        // Change FreeType font size
        if self.font_main.set_size(new_size_u32).is_err() {
            warn!("FreeType size change failed");
            return (self.cell_width, self.cell_height);
        }
        if let Some(ref mut cjk) = self.font_cjk {
            let _ = cjk.set_size(new_size_u32);
        }

        // Recalculate line metrics
        let (ascent, descender, line_height) = self.font_main.line_metrics();
        let min_height = ascent - descender;
        // +1.0 safety margin to prevent bottom clipping
        self.cell_height = line_height.max(min_height).ceil() + 1.0;
        self.ascent = ascent;

        // Redetermine cell width from 'M' (round to integer)
        self.cell_width = self.font_main
            .rasterize('M')
            .map(|g| g.advance.round())
            .unwrap_or((new_size * 0.6).round());

        self.font_size = new_size_u32;

        // Clear atlas
        self.glyphs.clear();
        self.phased_glyphs.clear();
        self.glyph_id_map.clear();
        self.atlas_data.fill(0);

        // Replace white pixel
        let aw = self.atlas_width as usize;
        for y in 0..2 {
            for x in 0..2 {
                let idx = (y * aw + x) * 3;
                self.atlas_data[idx] = 255;
                self.atlas_data[idx + 1] = 255;
                self.atlas_data[idx + 2] = 255;
            }
        }

        // Reset packing cursor
        self.cursor_x = 4;
        self.cursor_y = 0;
        self.row_height = 0;

        // Preload ASCII
        for ch in (0x20u8..=0x7Eu8).map(|c| c as char) {
            self.ensure_glyph(ch);
        }

        self.dirty = true;

        info!(
            "LCD font resize complete: cell={}x{:.0}, ascent={:.1}",
            self.cell_width, self.cell_height, self.ascent
        );

        (self.cell_width, self.cell_height)
    }
}
