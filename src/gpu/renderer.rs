//! Text drawing renderer
//!
//! Combine glyph atlas and shader
//! to render text on GPU

use anyhow::{anyhow, Result};
use glow::HasContext;
use log::info;

use crate::font::atlas::{GlyphAtlas, GlyphKey};
use crate::gpu::shader::{self, TextShader};

/// Per-vertex data: position(2) + UV(2) + color(4) = 8 floats
const VERTEX_FLOATS: usize = 8;
/// 1 character = 4 vertices
const VERTICES_PER_GLYPH: usize = 4;
/// 1 character = 6 indices (2 triangles)
const INDICES_PER_GLYPH: usize = 6;
/// Maximum characters per batch
const MAX_GLYPHS: usize = 4096;

/// Text renderer
pub struct TextRenderer {
    shader: TextShader,
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    ebo: glow::Buffer,
    /// Vertex buffer (CPU side)
    vertices: Vec<f32>,
    /// Current number of characters in buffer
    glyph_count: usize,
}

impl TextRenderer {
    /// Create text renderer
    pub fn new(gl: &glow::Context) -> Result<Self> {
        let shader = TextShader::new(gl)?;

        unsafe {
            // VAO
            let vao = gl
                .create_vertex_array()
                .map_err(|e| anyhow!("Failed to create VAO: {}", e))?;
            gl.bind_vertex_array(Some(vao));

            // VBO
            let vbo = gl
                .create_buffer()
                .map_err(|e| anyhow!("Failed to create VBO: {}", e))?;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            let vbo_size = MAX_GLYPHS * VERTICES_PER_GLYPH * VERTEX_FLOATS * 4;
            gl.buffer_data_size(glow::ARRAY_BUFFER, vbo_size as i32, glow::DYNAMIC_DRAW);

            // EBO (index buffer)
            let ebo = gl
                .create_buffer()
                .map_err(|e| anyhow!("Failed to create EBO: {}", e))?;
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(ebo));

            // Pre-generate index data
            let mut indices: Vec<u16> = Vec::with_capacity(MAX_GLYPHS * INDICES_PER_GLYPH);
            for i in 0..MAX_GLYPHS as u16 {
                let base = i * 4;
                // top-left -> top-right -> bottom-right, top-left -> bottom-right -> bottom-left
                indices.push(base);
                indices.push(base + 1);
                indices.push(base + 2);
                indices.push(base);
                indices.push(base + 2);
                indices.push(base + 3);
            }
            let index_bytes: &[u8] = bytemuck_cast_slice(&indices);
            gl.buffer_data_u8_slice(glow::ELEMENT_ARRAY_BUFFER, index_bytes, glow::STATIC_DRAW);

            // Set vertex attributes
            let stride = (VERTEX_FLOATS * 4) as i32;

            // a_pos: location=0, vec2
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, stride, 0);

            // a_uv: location=1, vec2
            gl.enable_vertex_attrib_array(1);
            gl.vertex_attrib_pointer_f32(1, 2, glow::FLOAT, false, stride, 8);

            // a_color: location=2, vec4
            gl.enable_vertex_attrib_array(2);
            gl.vertex_attrib_pointer_f32(2, 4, glow::FLOAT, false, stride, 16);

            gl.bind_vertex_array(None);

            info!("Text renderer initialized");

            Ok(Self {
                shader,
                vao,
                vbo,
                ebo,
                vertices: Vec::with_capacity(MAX_GLYPHS * VERTICES_PER_GLYPH * VERTEX_FLOATS),
                glyph_count: 0,
            })
        }
    }

    /// Clear draw buffer
    pub fn begin(&mut self) {
        self.vertices.clear();
        self.glyph_count = 0;
    }

    /// Add text string to draw buffer
    ///
    /// # Arguments
    /// * `text` - Text string to draw
    /// * `x` - Starting X coordinate (pixels)
    /// * `y` - Starting Y coordinate (pixels, baseline)
    /// * `color` - Text color [r, g, b, a] (0.0-1.0)
    /// * `atlas` - Glyph atlas
    pub fn push_text(&mut self, text: &str, x: f32, y: f32, color: [f32; 4], atlas: &GlyphAtlas) {
        let mut cursor_x = x;

        for ch in text.chars() {
            if self.glyph_count >= MAX_GLYPHS {
                break;
            }

            let glyph = match atlas.get_glyph(ch) {
                Some(g) => g,
                None => {
                    // Unknown characters advance by space width
                    cursor_x += atlas.cell_width;
                    continue;
                }
            };

            // Skip if glyph has zero width/height (e.g., space)
            if glyph.width == 0 || glyph.height == 0 {
                cursor_x += glyph.advance;
                continue;
            }

            // Calculate glyph draw position
            // fontdue's ymin is distance from bottom (positive = upward)
            // Round to integer pixels to prevent blur from texel interpolation
            let gx = (cursor_x + glyph.x_offset).round();
            let gy = (y - glyph.y_offset - glyph.height as f32).round();
            let gw = glyph.width as f32;
            let gh = glyph.height as f32;

            // UV coordinates
            let u0 = glyph.uv_x;
            let v0 = glyph.uv_y;
            let u1 = glyph.uv_x + glyph.uv_w;
            let v1 = glyph.uv_y + glyph.uv_h;

            let [r, g, b, a] = color;

            // 4 vertices: top-left, top-right, bottom-right, bottom-left
            // Top-left
            self.vertices
                .extend_from_slice(&[gx, gy, u0, v0, r, g, b, a]);
            // Top-right
            self.vertices
                .extend_from_slice(&[gx + gw, gy, u1, v0, r, g, b, a]);
            // Bottom-right
            self.vertices
                .extend_from_slice(&[gx + gw, gy + gh, u1, v1, r, g, b, a]);
            // Bottom-left
            self.vertices
                .extend_from_slice(&[gx, gy + gh, u0, v1, r, g, b, a]);

            self.glyph_count += 1;
            cursor_x += glyph.advance;
        }
    }

    /// Add a single character at specified position to draw buffer (for per-cell drawing)
    ///
    /// # Arguments
    /// * `ch` - Character to draw
    /// * `x` - X coordinate (pixels)
    /// * `y` - Y coordinate (pixels, baseline)
    /// * `color` - Text color [r, g, b, a] (0.0-1.0)
    /// * `atlas` - Glyph atlas
    pub fn push_char(&mut self, ch: char, x: f32, y: f32, color: [f32; 4], atlas: &GlyphAtlas) {
        if self.glyph_count >= MAX_GLYPHS {
            return;
        }

        let glyph = match atlas.get_glyph(ch) {
            Some(g) => g,
            None => return,
        };

        if glyph.width == 0 || glyph.height == 0 {
            return;
        }

        // Round to integer pixels to prevent blur from texel interpolation
        let gx = (x + glyph.x_offset).round();
        let gy = (y - glyph.y_offset - glyph.height as f32).round();
        let gw = glyph.width as f32;
        let gh = glyph.height as f32;

        let u0 = glyph.uv_x;
        let v0 = glyph.uv_y;
        let u1 = glyph.uv_x + glyph.uv_w;
        let v1 = glyph.uv_y + glyph.uv_h;

        let [r, g, b, a] = color;

        // 4 vertices: top-left, top-right, bottom-right, bottom-left
        self.vertices
            .extend_from_slice(&[gx, gy, u0, v0, r, g, b, a]);
        self.vertices
            .extend_from_slice(&[gx + gw, gy, u1, v0, r, g, b, a]);
        self.vertices
            .extend_from_slice(&[gx + gw, gy + gh, u1, v1, r, g, b, a]);
        self.vertices
            .extend_from_slice(&[gx, gy + gh, u0, v1, r, g, b, a]);

        self.glyph_count += 1;
    }

    /// Add pre-shaped glyph at specified position to draw buffer
    ///
    /// Similar to `push_char` but looks up atlas by `GlyphKey` instead of character.
    /// When drawing glyphs spanning multiple cells (e.g., ligatures),
    /// center-align within `cell_span` cells width.
    ///
    /// # Arguments
    /// * `key` - Glyph ID based lookup key
    /// * `x` - X coordinate (pixels)
    /// * `y` - Y coordinate (pixels, baseline)
    /// * `color` - Text color [r, g, b, a] (0.0-1.0)
    /// * `cell_span` - Number of cells the glyph occupies
    /// * `cell_width` - Width of one cell (pixels)
    /// * `atlas` - Glyph atlas
    pub fn push_glyph(
        &mut self,
        key: &GlyphKey,
        x: f32,
        y: f32,
        color: [f32; 4],
        cell_span: u8,
        cell_width: f32,
        atlas: &GlyphAtlas,
    ) {
        if self.glyph_count >= MAX_GLYPHS {
            return;
        }

        let glyph = match atlas.get_glyph_by_id(key) {
            Some(g) => g,
            None => return,
        };

        if glyph.width == 0 || glyph.height == 0 {
            return;
        }

        // For ligatures, center glyph within occupied cell width
        let total_width = cell_span as f32 * cell_width;
        let glyph_draw_width = glyph.width as f32;
        let offset_x = (total_width - glyph_draw_width) / 2.0;
        // Round to integer pixels to prevent blur from texel interpolation
        let gx = (x + offset_x.max(glyph.x_offset)).round();
        let gy = (y - glyph.y_offset - glyph.height as f32).round();
        let gw = glyph.width as f32;
        let gh = glyph.height as f32;

        let u0 = glyph.uv_x;
        let v0 = glyph.uv_y;
        let u1 = glyph.uv_x + glyph.uv_w;
        let v1 = glyph.uv_y + glyph.uv_h;

        let [r, g, b, a] = color;

        // 4 vertices: top-left, top-right, bottom-right, bottom-left
        self.vertices
            .extend_from_slice(&[gx, gy, u0, v0, r, g, b, a]);
        self.vertices
            .extend_from_slice(&[gx + gw, gy, u1, v0, r, g, b, a]);
        self.vertices
            .extend_from_slice(&[gx + gw, gy + gh, u1, v1, r, g, b, a]);
        self.vertices
            .extend_from_slice(&[gx, gy + gh, u0, v1, r, g, b, a]);

        self.glyph_count += 1;
    }

    /// Add background rectangle to draw buffer (for cell background colors)
    ///
    /// Fill rectangle using UV coordinates of opaque white pixel (alpha=1.0)
    /// reserved at top-left of atlas.
    pub fn push_rect(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: [f32; 4],
        atlas: &GlyphAtlas,
    ) {
        if self.glyph_count >= MAX_GLYPHS {
            return;
        }

        let (u, v) = atlas.solid_uv();
        let [r, g, b, a] = color;

        self.vertices.extend_from_slice(&[x, y, u, v, r, g, b, a]);
        self.vertices
            .extend_from_slice(&[x + w, y, u, v, r, g, b, a]);
        self.vertices
            .extend_from_slice(&[x + w, y + h, u, v, r, g, b, a]);
        self.vertices
            .extend_from_slice(&[x, y + h, u, v, r, g, b, a]);

        self.glyph_count += 1;
    }

    /// Upload buffer contents to GPU and draw
    pub fn flush(&self, gl: &glow::Context, atlas: &GlyphAtlas, width: u32, height: u32) {
        if self.glyph_count == 0 {
            return;
        }

        unsafe {
            // Enable blending (for premultiplied alpha)
            gl.enable(glow::BLEND);
            gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);

            // Bind shader
            self.shader.bind(gl);

            // Set orthographic projection matrix
            let projection = shader::ortho_projection(width as f32, height as f32);
            self.shader.set_projection(gl, &projection);

            // Set gamma correction value (1.0 = no correction, lower = thicker)
            self.shader.set_gamma(gl, 1.0);

            // Bind texture
            atlas.bind(gl, 0);
            self.shader.set_atlas_unit(gl, 0);

            // Bind VAO and upload vertex data
            gl.bind_vertex_array(Some(self.vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));

            let vertex_bytes: &[u8] = bytemuck_cast_slice(&self.vertices);
            gl.buffer_sub_data_u8_slice(glow::ARRAY_BUFFER, 0, vertex_bytes);

            // Draw
            gl.draw_elements(
                glow::TRIANGLES,
                (self.glyph_count * INDICES_PER_GLYPH) as i32,
                glow::UNSIGNED_SHORT,
                0,
            );

            gl.bind_vertex_array(None);
            gl.disable(glow::BLEND);
        }
    }

    /// Release resources
    pub fn destroy(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_vertex_array(self.vao);
            gl.delete_buffer(self.vbo);
            gl.delete_buffer(self.ebo);
        }
        self.shader.destroy(gl);
    }
}

/// &[T] -> &[u8] conversion (minimal implementation without bytemuck)
fn bytemuck_cast_slice<T>(slice: &[T]) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(
            slice.as_ptr() as *const u8,
            slice.len() * std::mem::size_of::<T>(),
        )
    }
}

// === Curly underline renderer ===

use crate::gpu::shader::CurlyShader;

/// Curly underline renderer (anti-aliasing with SDF + smoothstep)
///
/// Per-vertex: pos(2) + rect(4) + color(4) + params(4) = 14 floats
const CURLY_VERTEX_FLOATS: usize = 14;
const CURLY_MAX_RUNS: usize = 1024;

pub struct CurlyRenderer {
    shader: CurlyShader,
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    ebo: glow::Buffer,
    vertices: Vec<f32>,
    run_count: usize,
}

impl CurlyRenderer {
    pub fn new(gl: &glow::Context) -> Result<Self> {
        let shader = CurlyShader::new(gl)?;

        unsafe {
            let vao = gl
                .create_vertex_array()
                .map_err(|e| anyhow!("Failed to create VAO (Curly): {}", e))?;
            gl.bind_vertex_array(Some(vao));

            let vbo = gl
                .create_buffer()
                .map_err(|e| anyhow!("Failed to create VBO (Curly): {}", e))?;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            let vbo_size = CURLY_MAX_RUNS * 4 * CURLY_VERTEX_FLOATS * 4;
            gl.buffer_data_size(glow::ARRAY_BUFFER, vbo_size as i32, glow::DYNAMIC_DRAW);

            let ebo = gl
                .create_buffer()
                .map_err(|e| anyhow!("Failed to create EBO (Curly): {}", e))?;
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(ebo));

            let mut indices: Vec<u16> = Vec::with_capacity(CURLY_MAX_RUNS * 6);
            for i in 0..CURLY_MAX_RUNS as u16 {
                let base = i * 4;
                indices.push(base);
                indices.push(base + 1);
                indices.push(base + 2);
                indices.push(base);
                indices.push(base + 2);
                indices.push(base + 3);
            }
            let index_bytes: &[u8] = bytemuck_cast_slice(&indices);
            gl.buffer_data_u8_slice(glow::ELEMENT_ARRAY_BUFFER, index_bytes, glow::STATIC_DRAW);

            let stride = (CURLY_VERTEX_FLOATS * 4) as i32;

            // a_pos: location=0, vec2
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, stride, 0);

            // a_rect: location=1, vec4
            gl.enable_vertex_attrib_array(1);
            gl.vertex_attrib_pointer_f32(1, 4, glow::FLOAT, false, stride, 8);

            // a_color: location=2, vec4
            gl.enable_vertex_attrib_array(2);
            gl.vertex_attrib_pointer_f32(2, 4, glow::FLOAT, false, stride, 24);

            // a_params: location=3, vec4
            gl.enable_vertex_attrib_array(3);
            gl.vertex_attrib_pointer_f32(3, 4, glow::FLOAT, false, stride, 40);

            gl.bind_vertex_array(None);

            info!("Curly underline renderer initialized");

            Ok(Self {
                shader,
                vao,
                vbo,
                ebo,
                vertices: Vec::with_capacity(CURLY_MAX_RUNS * 4 * CURLY_VERTEX_FLOATS),
                run_count: 0,
            })
        }
    }

    pub fn begin(&mut self) {
        self.vertices.clear();
        self.run_count = 0;
    }

    /// Add curly underline
    ///
    /// # Arguments
    /// * `x` - Starting X coordinate
    /// * `y` - Starting Y coordinate (cell top)
    /// * `w` - Width
    /// * `h` - Height (cell height)
    /// * `color` - Color [r, g, b, a]
    /// * `amplitude` - Wave amplitude
    /// * `wavelength` - Wavelength
    /// * `thickness` - Line thickness
    /// * `base_y` - Base Y coordinate for wave
    pub fn push_curly(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: [f32; 4],
        amplitude: f32,
        wavelength: f32,
        thickness: f32,
        base_y: f32,
    ) {
        if self.run_count >= CURLY_MAX_RUNS {
            return;
        }

        let [r, g, b, a] = color;
        let rect = [x, y, w, h];
        let params = [amplitude, wavelength, thickness, base_y];

        // 4 vertices
        // Top-left
        self.vertices.extend_from_slice(&[x, y]);
        self.vertices.extend_from_slice(&rect);
        self.vertices.extend_from_slice(&[r, g, b, a]);
        self.vertices.extend_from_slice(&params);

        // Top-right
        self.vertices.extend_from_slice(&[x + w, y]);
        self.vertices.extend_from_slice(&rect);
        self.vertices.extend_from_slice(&[r, g, b, a]);
        self.vertices.extend_from_slice(&params);

        // Bottom-right
        self.vertices.extend_from_slice(&[x + w, y + h]);
        self.vertices.extend_from_slice(&rect);
        self.vertices.extend_from_slice(&[r, g, b, a]);
        self.vertices.extend_from_slice(&params);

        // Bottom-left
        self.vertices.extend_from_slice(&[x, y + h]);
        self.vertices.extend_from_slice(&rect);
        self.vertices.extend_from_slice(&[r, g, b, a]);
        self.vertices.extend_from_slice(&params);

        self.run_count += 1;
    }

    pub fn flush(&self, gl: &glow::Context, width: u32, height: u32) {
        if self.run_count == 0 {
            return;
        }

        unsafe {
            gl.enable(glow::BLEND);
            gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);

            self.shader.bind(gl);

            let projection = shader::ortho_projection(width as f32, height as f32);
            self.shader.set_projection(gl, &projection);

            gl.bind_vertex_array(Some(self.vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));

            let vertex_bytes: &[u8] = bytemuck_cast_slice(&self.vertices);
            gl.buffer_sub_data_u8_slice(glow::ARRAY_BUFFER, 0, vertex_bytes);

            gl.draw_elements(
                glow::TRIANGLES,
                (self.run_count * 6) as i32,
                glow::UNSIGNED_SHORT,
                0,
            );

            gl.bind_vertex_array(None);
            gl.disable(glow::BLEND);
        }
    }

    pub fn destroy(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_vertex_array(self.vao);
            gl.delete_buffer(self.vbo);
            gl.delete_buffer(self.ebo);
        }
        self.shader.destroy(gl);
    }
}
