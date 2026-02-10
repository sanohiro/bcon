//! Glyph atlas
//!
//! Loads fonts and dynamically rasterizes glyphs,
//! packing them into a single texture.
//! ASCII is preloaded at initialization, CJK etc. added on-demand.

use anyhow::{anyhow, Result};
use fontdue::{Font, FontSettings};
use glow::HasContext;
use log::{debug, info, warn};
use std::collections::HashMap;

/// Glyph ID based lookup key (for shaping results)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlyphKey {
    /// Font index: 0=main, 1=CJK
    pub font_idx: u8,
    /// Glyph ID in font
    pub glyph_id: u16,
}

/// Metrics and texture coordinates for one glyph
#[derive(Debug, Clone, Copy)]
pub struct GlyphInfo {
    /// Top-left U coordinate on texture (0.0-1.0)
    pub uv_x: f32,
    /// Top-left V coordinate on texture (0.0-1.0)
    pub uv_y: f32,
    /// Width on texture (0.0-1.0)
    pub uv_w: f32,
    /// Height on texture (0.0-1.0)
    pub uv_h: f32,
    /// Glyph bitmap width (pixels)
    pub width: u32,
    /// Glyph bitmap height (pixels)
    pub height: u32,
    /// Horizontal offset from baseline
    pub x_offset: f32,
    /// Vertical offset from baseline
    pub y_offset: f32,
    /// Horizontal advance to next character
    pub advance: f32,
}

/// Supersampling scale (2.0 = 2x high quality)
const RENDER_SCALE: f32 = 2.0;

/// Glyph atlas: texture containing font glyphs
pub struct GlyphAtlas {
    texture: glow::Texture,
    /// Character -> glyph info map
    glyphs: HashMap<char, GlyphInfo>,
    /// Glyph ID -> glyph info map (for shaping results)
    glyph_id_map: HashMap<GlyphKey, GlyphInfo>,
    /// Texture width
    pub atlas_width: u32,
    /// Texture height
    pub atlas_height: u32,
    /// Cell width (based on monospace font)
    pub cell_width: f32,
    /// Cell height (line height)
    pub cell_height: f32,
    /// Baseline Y position (distance from cell top)
    pub ascent: f32,

    /// UV coordinates for opaque pixel (for rectangle drawing)
    solid_uv: (f32, f32),

    // Fields for dynamic rasterization
    /// Main font
    font_main: Font,
    /// CJK fallback font
    font_cjk: Option<Font>,
    /// Font size (logical size)
    font_size: f32,
    /// Rasterize size (font_size * RENDER_SCALE)
    render_size: f32,
    /// Shelf packing: current X position
    cursor_x: u32,
    /// Shelf packing: current Y position (top of row)
    cursor_y: u32,
    /// Shelf packing: max height of current row
    row_height: u32,
    /// CPU-side texture data
    atlas_data: Vec<u8>,
    /// GPU re-upload flag
    dirty: bool,
}

impl GlyphAtlas {
    /// Generate glyph atlas from font file
    ///
    /// Preloads ASCII (0x20-0x7E) glyphs,
    /// CJK etc. added dynamically via ensure_glyph()
    pub fn new(
        gl: &glow::Context,
        font_data: &[u8],
        font_size: f32,
        cjk_font_data: Option<&[u8]>,
    ) -> Result<Self> {
        // Supersampling: rasterize at 2x size
        let render_size = font_size * RENDER_SCALE;

        // Load main font
        let font_main = Font::from_bytes(font_data, FontSettings::default())
            .map_err(|e| anyhow!("Failed to load font: {}", e))?;

        info!(
            "Main font loaded ({}x supersampling)",
            RENDER_SCALE
        );

        // Load CJK fallback font
        let font_cjk = if let Some(cjk_data) = cjk_font_data {
            match Font::from_bytes(cjk_data, FontSettings::default()) {
                Ok(f) => {
                    info!("CJK font loaded");
                    Some(f)
                }
                Err(e) => {
                    warn!("Failed to load CJK font (continuing): {}", e);
                    None
                }
            }
        } else {
            None
        };

        // Get line metrics (calculated at logical size)
        let metrics = font_main
            .horizontal_line_metrics(font_size)
            .ok_or_else(|| anyhow!("Cannot get line metrics"))?;

        let ascent = metrics.ascent;
        let descent = metrics.descent;
        let line_height = ascent - descent;

        // Rasterize ASCII characters (high resolution)
        let chars: Vec<char> = (0x20u8..=0x7Eu8).map(|c| c as char).collect();
        let mut rasterized: Vec<(char, fontdue::Metrics, Vec<u8>)> = Vec::new();

        for &ch in &chars {
            let (m, bitmap) = font_main.rasterize(ch, render_size);
            rasterized.push((ch, m, bitmap));
        }

        // Determine cell width from 'M' advance (convert to logical size)
        let cell_width = rasterized
            .iter()
            .find(|(c, _, _)| *c == 'M')
            .or_else(|| rasterized.first())
            .map(|(_, m, _)| m.advance_width / RENDER_SCALE)
            .unwrap_or(font_size * 0.6);

        info!(
            "Font metrics: ascent={:.1}, descent={:.1}, cell={}x{:.0}",
            ascent, descent, cell_width, line_height
        );

        // Allocate 2048x2048 atlas to accommodate CJK
        let atlas_width = 2048u32;
        let atlas_height = 2048u32;

        debug!(
            "Atlas size: {}x{} (preloading {} ASCII glyphs)",
            atlas_width,
            atlas_height,
            rasterized.len()
        );

        // Create atlas pixel data (R8)
        let mut atlas_data = vec![0u8; (atlas_width * atlas_height) as usize];

        // Place white pixel (2x2) at top-left for rectangle drawing
        // push_rect uses this UV to draw opaque rectangles
        let aw = atlas_width as usize;
        atlas_data[0] = 255; // (0, 0)
        atlas_data[1] = 255; // (1, 0)
        atlas_data[aw] = 255; // (0, 1)
        atlas_data[aw + 1] = 255; // (1, 1)
        let solid_uv = (0.5 / atlas_width as f32, 0.5 / atlas_height as f32);

        let mut glyphs = HashMap::new();

        // Initial position for shelf packing (start after white pixel area)
        // 6px padding to reliably prevent bleeding between glyphs (high quality)
        let pad = 6u32;
        let mut cursor_x = 4u32;
        let mut cursor_y = 0u32;
        let mut row_height = 0u32;

        for (ch, metrics, bitmap) in &rasterized {
            let bw = metrics.width as u32;
            let bh = metrics.height as u32;

            // Move to next row if doesn't fit in current row
            if cursor_x + bw + pad > atlas_width {
                cursor_y += row_height + pad;
                cursor_x = 0;
                row_height = 0;
            }

            // Warn and skip if doesn't fit in texture
            if cursor_y + bh > atlas_height {
                warn!("Atlas overflow: '{}'", ch);
                continue;
            }

            let x0 = cursor_x;
            let y0 = cursor_y;

            // Copy bitmap to atlas
            for y in 0..bh {
                for x in 0..bw {
                    let src_idx = (y * bw + x) as usize;
                    let dst_idx = ((y0 + y) * atlas_width + (x0 + x)) as usize;
                    if src_idx < bitmap.len() && dst_idx < atlas_data.len() {
                        atlas_data[dst_idx] = bitmap[src_idx];
                    }
                }
            }

            // Record UV coordinates (convert metrics to logical size)
            let aw = atlas_width as f32;
            let ah = atlas_height as f32;

            glyphs.insert(
                *ch,
                GlyphInfo {
                    uv_x: x0 as f32 / aw,
                    uv_y: y0 as f32 / ah,
                    uv_w: bw as f32 / aw,
                    uv_h: bh as f32 / ah,
                    // Convert metrics to logical size (drawing size)
                    width: ((bw as f32) / RENDER_SCALE).round() as u32,
                    height: ((bh as f32) / RENDER_SCALE).round() as u32,
                    x_offset: metrics.xmin as f32 / RENDER_SCALE,
                    y_offset: metrics.ymin as f32 / RENDER_SCALE,
                    advance: metrics.advance_width / RENDER_SCALE,
                },
            );

            cursor_x += bw + pad;
            row_height = row_height.max(bh);
        }

        // Update cursor for next glyph addition
        // (Preserve position at end of ASCII)

        // Create OpenGL texture
        let texture = unsafe {
            let tex = gl
                .create_texture()
                .map_err(|e| anyhow!("Failed to create texture: {}", e))?;

            gl.bind_texture(glow::TEXTURE_2D, Some(tex));

            // Upload in R8 format
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::R8 as i32,
                atlas_width as i32,
                atlas_height as i32,
                0,
                glow::RED,
                glow::UNSIGNED_BYTE,
                Some(&atlas_data),
            );

            // Filtering settings
            // Direct LINEAR minification (no mipmaps for sharper rendering)
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::LINEAR as i32,
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

            // Swizzle: R -> R,R,R,R (treat as grayscale)
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_SWIZZLE_A, glow::RED as i32);

            gl.bind_texture(glow::TEXTURE_2D, None);

            tex
        };

        info!(
            "Glyph atlas generated: {}x{}, {} glyphs (ASCII preloaded)",
            atlas_width,
            atlas_height,
            glyphs.len()
        );

        Ok(Self {
            texture,
            glyphs,
            glyph_id_map: HashMap::new(),
            atlas_width,
            atlas_height,
            cell_width,
            cell_height: line_height,
            ascent,
            solid_uv,
            font_main,
            font_cjk,
            font_size,
            render_size,
            cursor_x,
            cursor_y,
            row_height,
            atlas_data,
            dirty: false,
        })
    }

    /// Pack bitmap into atlas and return GlyphInfo
    ///
    /// Returns None if atlas has no space
    fn pack_bitmap(
        &mut self,
        metrics: &fontdue::Metrics,
        bitmap: &[u8],
        label: &str,
    ) -> Option<GlyphInfo> {
        let bw = metrics.width as u32;
        let bh = metrics.height as u32;
        // 6px padding to reliably prevent bleeding between glyphs (high quality)
        let pad = 6u32;

        // Shelf packing: move to next row if doesn't fit
        if self.cursor_x + bw + pad > self.atlas_width {
            self.cursor_y += self.row_height + pad;
            self.cursor_x = 0;
            self.row_height = 0;
        }

        // Warn and skip if doesn't fit in texture
        if self.cursor_y + bh > self.atlas_height {
            warn!("Atlas full: {} ({}x{})", label, bw, bh);
            return None;
        }

        let x0 = self.cursor_x;
        let y0 = self.cursor_y;

        // Copy bitmap to CPU-side atlas
        for y in 0..bh {
            for x in 0..bw {
                let src_idx = (y * bw + x) as usize;
                let dst_idx = ((y0 + y) * self.atlas_width + (x0 + x)) as usize;
                if src_idx < bitmap.len() && dst_idx < self.atlas_data.len() {
                    self.atlas_data[dst_idx] = bitmap[src_idx];
                }
            }
        }

        let aw = self.atlas_width as f32;
        let ah = self.atlas_height as f32;

        // Convert metrics to logical size (drawing size)
        let info = GlyphInfo {
            uv_x: x0 as f32 / aw,
            uv_y: y0 as f32 / ah,
            uv_w: bw as f32 / aw,
            uv_h: bh as f32 / ah,
            width: ((bw as f32) / RENDER_SCALE).round() as u32,
            height: ((bh as f32) / RENDER_SCALE).round() as u32,
            x_offset: metrics.xmin as f32 / RENDER_SCALE,
            y_offset: metrics.ymin as f32 / RENDER_SCALE,
            advance: metrics.advance_width / RENDER_SCALE,
        };

        self.cursor_x += bw + pad;
        self.row_height = self.row_height.max(bh);
        self.dirty = true;

        Some(info)
    }

    /// Ensure character glyph exists in atlas
    ///
    /// If not registered, rasterize and add from main font -> CJK font
    pub fn ensure_glyph(&mut self, ch: char) {
        if self.glyphs.contains_key(&ch) {
            return;
        }

        // Skip space and control characters
        if ch <= ' ' {
            return;
        }

        // Select source font for rasterization (rasterize at high resolution)
        let (metrics, bitmap) = if self.font_main.lookup_glyph_index(ch) != 0 {
            self.font_main.rasterize(ch, self.render_size)
        } else if let Some(ref cjk_font) = self.font_cjk {
            if cjk_font.lookup_glyph_index(ch) != 0 {
                cjk_font.rasterize(ch, self.render_size)
            } else {
                debug!("Glyph not found: U+{:04X} '{}'", ch as u32, ch);
                return;
            }
        } else {
            debug!(
                "Glyph not found (no CJK font): U+{:04X} '{}'",
                ch as u32, ch
            );
            return;
        };

        let label = format!("U+{:04X} '{}'", ch as u32, ch);
        if let Some(info) = self.pack_bitmap(&metrics, &bitmap, &label) {
            self.glyphs.insert(ch, info);
        }
    }

    /// Add glyph to atlas by glyph ID (for shaping results)
    ///
    /// Uses `fontdue::Font::rasterize_indexed` to rasterize directly from glyph ID
    pub fn ensure_glyph_id(&mut self, key: GlyphKey) {
        if self.glyph_id_map.contains_key(&key) {
            return;
        }

        let font = match key.font_idx {
            0 => &self.font_main,
            1 => {
                if let Some(ref f) = self.font_cjk {
                    f
                } else {
                    return;
                }
            }
            _ => return,
        };

        let (metrics, bitmap) = font.rasterize_indexed(key.glyph_id, self.render_size);

        let label = format!("glyph_id={} font={}", key.glyph_id, key.font_idx);
        if let Some(info) = self.pack_bitmap(&metrics, &bitmap, &label) {
            self.glyph_id_map.insert(key, info);
        }
    }

    /// Get glyph info from glyph ID
    pub fn get_glyph_by_id(&self, key: &GlyphKey) -> Option<&GlyphInfo> {
        self.glyph_id_map.get(key)
    }

    /// Return reference to main font (used by shaper to check glyph existence)
    pub fn font_main_ref(&self) -> &Font {
        &self.font_main
    }

    /// Re-upload texture to GPU if dirty
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
                glow::R8 as i32,
                self.atlas_width as i32,
                self.atlas_height as i32,
                0,
                glow::RED,
                glow::UNSIGNED_BYTE,
                Some(&self.atlas_data),
            );
            gl.bind_texture(glow::TEXTURE_2D, None);
        }

        debug!(
            "Atlas texture re-uploaded: {} glyphs",
            self.glyphs.len()
        );
    }

    /// Return UV coordinates of opaque pixel for rectangle drawing
    pub fn solid_uv(&self) -> (f32, f32) {
        self.solid_uv
    }

    /// Get glyph info for character
    pub fn get_glyph(&self, ch: char) -> Option<&GlyphInfo> {
        self.glyphs.get(&ch)
    }

    /// Bind texture
    pub fn bind(&self, gl: &glow::Context, unit: u32) {
        unsafe {
            gl.active_texture(glow::TEXTURE0 + unit);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.texture));
        }
    }

    /// Release resources
    pub fn destroy(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_texture(self.texture);
        }
    }

    /// Get current font size
    #[allow(dead_code)]
    pub fn font_size(&self) -> f32 {
        self.font_size
    }

    /// Change font size and rebuild atlas
    ///
    /// Returns new cell size (cell_width, cell_height)
    pub fn resize(&mut self, new_font_size: f32) -> (f32, f32) {
        info!(
            "Font size change: {:.1} -> {:.1}",
            self.font_size, new_font_size
        );

        self.font_size = new_font_size;
        self.render_size = new_font_size * RENDER_SCALE;

        // Recalculate line metrics
        if let Some(metrics) = self.font_main.horizontal_line_metrics(new_font_size) {
            self.ascent = metrics.ascent;
            self.cell_height = metrics.ascent - metrics.descent;
        }

        // Update cell width from ASCII re-rasterization
        let (m, _) = self.font_main.rasterize('M', self.render_size);
        self.cell_width = m.advance_width / RENDER_SCALE;

        // Clear cache
        self.glyphs.clear();
        self.glyph_id_map.clear();

        // Clear atlas data
        self.atlas_data.fill(0);

        // Replace white pixel
        let aw = self.atlas_width as usize;
        self.atlas_data[0] = 255;
        self.atlas_data[1] = 255;
        self.atlas_data[aw] = 255;
        self.atlas_data[aw + 1] = 255;

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
            "Font resize complete: cell={}x{:.0}, ascent={:.1}",
            self.cell_width, self.cell_height, self.ascent
        );

        (self.cell_width, self.cell_height)
    }
}

/// Search and load system font
///
/// Search order:
/// 1. BCON_FONT environment variable
/// 2. Known paths (hardcoded)
/// 3. fontconfig (fallback)
pub fn load_system_font() -> Result<Vec<u8>> {
    // Custom font can be specified via BCON_FONT environment variable
    if let Ok(path) = std::env::var("BCON_FONT") {
        let data = std::fs::read(&path)
            .map_err(|e| anyhow!("Failed to load BCON_FONT: {} ({})", path, e))?;
        info!("Font loaded: {} (BCON_FONT)", path);
        return Ok(data);
    }

    let candidates = [
        // Linux
        "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
        "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
        "/usr/share/fonts/dejavu-sans-mono-fonts/DejaVuSansMono.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
        "/usr/share/fonts/liberation-mono/LiberationMono-Regular.ttf",
        "/usr/share/fonts/truetype/noto/NotoSansMono-Regular.ttf",
        "/usr/share/fonts/noto/NotoSansMono-Regular.ttf",
        // macOS (development/testing)
        "/System/Library/Fonts/Menlo.ttc",
        "/System/Library/Fonts/Monaco.ttf",
        "/System/Library/Fonts/Courier.ttc",
        "/Library/Fonts/Courier New.ttf",
    ];

    for path in &candidates {
        if let Ok(data) = std::fs::read(path) {
            info!("Font loaded: {}", path);
            return Ok(data);
        }
    }

    // Fallback to fontconfig
    debug!("Not found in hardcoded paths, trying fontconfig");
    if let Ok(data) = super::fontconfig::load_system_font_fc() {
        return Ok(data);
    }

    Err(anyhow!(
        "System font not found. Please check the following paths:\n{}",
        candidates.join("\n")
    ))
}

/// Search and load CJK font
///
/// Search order:
/// 1. Known paths (hardcoded)
/// 2. fontconfig (fallback)
pub fn load_cjk_font() -> Option<Vec<u8>> {
    let candidates = [
        // Noto Sans CJK (Debian/Ubuntu fonts-noto-cjk package)
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJKjp-Regular.otf",
        "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
        // IPA Gothic
        "/usr/share/fonts/truetype/fonts-japanese-gothic.ttf",
        "/usr/share/fonts/ipa-gothic/ipag.ttf",
        "/usr/share/fonts/opentype/ipafont-gothic/ipag.ttf",
        // VL Gothic
        "/usr/share/fonts/truetype/vlgothic/VL-Gothic-Regular.ttf",
        // Takao
        "/usr/share/fonts/truetype/takao-gothic/TakaoGothic.ttf",
        // macOS (development/testing)
        "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc",
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
        "/Library/Fonts/Arial Unicode.ttf",
    ];

    for path in &candidates {
        if let Ok(data) = std::fs::read(path) {
            info!("CJK font loaded: {}", path);
            return Some(data);
        }
    }

    // Fallback to fontconfig
    debug!("Not found in hardcoded paths, trying fontconfig");
    if let Some(data) = super::fontconfig::load_cjk_font_fc() {
        return Some(data);
    }

    warn!("CJK font not found. Japanese display will be unavailable.");
    None
}
