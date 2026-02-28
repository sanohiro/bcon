//! LCD subpixel rendering text renderer
//!
//! High-quality text rendering with FreeType + RGB atlas + LCD shader
//! Each vertex has FG/BG colors for accurate per-cell subpixel compositing
//!
//! Note: This module is kept for potential future use with subpixel rendering.

#![allow(dead_code)]

use anyhow::{anyhow, Result};
use glow::HasContext;
use log::info;

use crate::font::lcd_atlas::{GlyphInfo, LcdGlyphAtlas};
use crate::gpu::shader;

/// LCD subpixel + linear color space compositing shader (per-vertex FG/BG)
const LCD_VERTEX_SHADER: &str = r#"#version 300 es
precision highp float;

layout(location = 0) in vec2 a_pos;
layout(location = 1) in vec2 a_uv;
layout(location = 2) in vec4 a_fg_color;
layout(location = 3) in vec3 a_bg_color;

uniform mat4 u_projection;

out vec2 v_uv;
out vec4 v_fg_color;
out vec3 v_bg_color;

void main() {
    gl_Position = u_projection * vec4(a_pos, 0.0, 1.0);
    v_uv = a_uv;
    v_fg_color = a_fg_color;
    v_bg_color = a_bg_color;
}
"#;

const LCD_FRAGMENT_SHADER: &str = r#"#version 300 es
precision highp float;

in vec2 v_uv;
in vec4 v_fg_color;
in vec3 v_bg_color;

uniform sampler2D u_atlas;
uniform float u_gamma;
uniform float u_stem_darkening;  // 0.0 = disabled, 0.05-0.15 = enabled
uniform float u_contrast;        // 1.0 = normal, 1.1-1.3 = contrast boost
uniform float u_fringe_reduction; // 0.0 = disabled, 0.3-0.5 = color fringe reduction
uniform int u_subpixel_bgr;  // 0 = RGB, 1 = BGR

out vec4 frag_color;

// Precise sRGB to linear conversion
float srgb_to_linear_channel(float c) {
    return c <= 0.04045 ? c / 12.92 : pow((c + 0.055) / 1.055, 2.4);
}
vec3 srgb_to_linear(vec3 srgb) {
    return vec3(
        srgb_to_linear_channel(srgb.r),
        srgb_to_linear_channel(srgb.g),
        srgb_to_linear_channel(srgb.b)
    );
}

// Precise linear to sRGB conversion
float linear_to_srgb_channel(float c) {
    return c <= 0.0031308 ? c * 12.92 : 1.055 * pow(c, 1.0/2.4) - 0.055;
}
vec3 linear_to_srgb(vec3 linear) {
    return vec3(
        linear_to_srgb_channel(max(linear.r, 0.0)),
        linear_to_srgb_channel(max(linear.g, 0.0)),
        linear_to_srgb_channel(max(linear.b, 0.0))
    );
}

void main() {
    // Each RGB channel is the subpixel coverage value
    vec3 coverage = texture(u_atlas, v_uv).rgb;

    // Swap R and B for BGR panels
    if (u_subpixel_bgr == 1) {
        coverage = coverage.bgr;
    }

    // Color fringe reduction: bring RGB differences closer to average
    // Effective when rainbow colors appear at character edges
    if (u_fringe_reduction > 0.0) {
        float avg = (coverage.r + coverage.g + coverage.b) / 3.0;
        coverage = mix(coverage, vec3(avg), u_fringe_reduction);
    }

    // Gamma correction for weight adjustment (emphasize thin strokes)
    coverage = pow(coverage, vec3(u_gamma));

    // Contrast boost: emphasize midtones for sharper edges
    if (u_contrast != 1.0) {
        coverage = clamp((coverage - 0.5) * u_contrast + 0.5, 0.0, 1.0);
    }

    // Stem darkening: make thin lines appear slightly thicker (effective for small fonts)
    // u_stem_darkening = 0.0 disables, 0.05-0.15 enables
    if (u_stem_darkening > 0.0) {
        float avg_cov = (coverage.r + coverage.g + coverage.b) / 3.0;
        float darken = smoothstep(0.0, 0.5, avg_cov) * u_stem_darkening;
        coverage = min(coverage + darken, vec3(1.0));
    }

    // Convert to linear color space
    vec3 fg_linear = srgb_to_linear(v_fg_color.rgb);
    vec3 bg_linear = srgb_to_linear(v_bg_color);

    // Dynamic gamma adjustment based on background luminance
    // Slightly thinner on bright backgrounds, unchanged on dark
    float bg_luma = dot(bg_linear, vec3(0.2126, 0.7152, 0.0722));
    float dynamic_boost = mix(1.0, 0.95, bg_luma);
    coverage *= dynamic_boost;

    // Independent alpha blend per subpixel (linear space)
    vec3 blended_linear = mix(bg_linear, fg_linear, coverage * v_fg_color.a);

    // Convert back to sRGB
    vec3 blended_srgb = linear_to_srgb(blended_linear);

    // Overall alpha is maximum coverage
    float alpha = max(max(coverage.r, coverage.g), coverage.b) * v_fg_color.a;

    frag_color = vec4(blended_srgb, alpha);
}
"#;

/// LCD shader (subpixel rendering with per-vertex FG/BG)
pub struct LcdShader {
    program: glow::Program,
    pub u_projection: glow::UniformLocation,
    pub u_atlas: glow::UniformLocation,
    pub u_gamma: glow::UniformLocation,
    pub u_stem_darkening: glow::UniformLocation,
    pub u_contrast: glow::UniformLocation,
    pub u_fringe_reduction: glow::UniformLocation,
    pub u_subpixel_bgr: glow::UniformLocation,
}

impl LcdShader {
    pub fn new(gl: &glow::Context) -> Result<Self> {
        let program = compile_program(gl, LCD_VERTEX_SHADER, LCD_FRAGMENT_SHADER)?;

        let u_projection = unsafe {
            gl.get_uniform_location(program, "u_projection")
                .ok_or_else(|| anyhow!("u_projection uniform not found"))?
        };
        let u_atlas = unsafe {
            gl.get_uniform_location(program, "u_atlas")
                .ok_or_else(|| anyhow!("u_atlas uniform not found"))?
        };
        let u_gamma = unsafe {
            gl.get_uniform_location(program, "u_gamma")
                .ok_or_else(|| anyhow!("u_gamma uniform not found"))?
        };
        let u_stem_darkening = unsafe {
            gl.get_uniform_location(program, "u_stem_darkening")
                .ok_or_else(|| anyhow!("u_stem_darkening uniform not found"))?
        };
        let u_contrast = unsafe {
            gl.get_uniform_location(program, "u_contrast")
                .ok_or_else(|| anyhow!("u_contrast uniform not found"))?
        };
        let u_fringe_reduction = unsafe {
            gl.get_uniform_location(program, "u_fringe_reduction")
                .ok_or_else(|| anyhow!("u_fringe_reduction uniform not found"))?
        };
        let u_subpixel_bgr = unsafe {
            gl.get_uniform_location(program, "u_subpixel_bgr")
                .ok_or_else(|| anyhow!("u_subpixel_bgr uniform not found"))?
        };

        info!("LCD shader compiled (contrast boost + fringe reduction + dynamic gamma)");
        Ok(Self {
            program,
            u_projection,
            u_atlas,
            u_gamma,
            u_stem_darkening,
            u_contrast,
            u_fringe_reduction,
            u_subpixel_bgr,
        })
    }

    pub fn bind(&self, gl: &glow::Context) {
        unsafe {
            gl.use_program(Some(self.program));
        }
    }

    pub fn set_projection(&self, gl: &glow::Context, matrix: &[f32; 16]) {
        unsafe {
            gl.uniform_matrix_4_f32_slice(Some(&self.u_projection), false, matrix);
        }
    }

    pub fn set_atlas_unit(&self, gl: &glow::Context, unit: i32) {
        unsafe {
            gl.uniform_1_i32(Some(&self.u_atlas), unit);
        }
    }

    pub fn set_gamma(&self, gl: &glow::Context, gamma: f32) {
        unsafe {
            gl.uniform_1_f32(Some(&self.u_gamma), gamma);
        }
    }

    pub fn set_stem_darkening(&self, gl: &glow::Context, strength: f32) {
        unsafe {
            gl.uniform_1_f32(Some(&self.u_stem_darkening), strength);
        }
    }

    pub fn set_contrast(&self, gl: &glow::Context, contrast: f32) {
        unsafe {
            gl.uniform_1_f32(Some(&self.u_contrast), contrast);
        }
    }

    pub fn set_fringe_reduction(&self, gl: &glow::Context, reduction: f32) {
        unsafe {
            gl.uniform_1_f32(Some(&self.u_fringe_reduction), reduction);
        }
    }

    pub fn set_subpixel_bgr(&self, gl: &glow::Context, bgr: bool) {
        unsafe {
            gl.uniform_1_i32(Some(&self.u_subpixel_bgr), if bgr { 1 } else { 0 });
        }
    }

    pub fn destroy(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_program(self.program);
        }
    }
}

/// Per-vertex data: position(2) + UV(2) + FG color(4) + BG color(3) = 11 floats
const VERTEX_FLOATS: usize = 11;
/// 1 character = 4 vertices
const VERTICES_PER_GLYPH: usize = 4;
/// 1 character = 6 indices (2 triangles)
const INDICES_PER_GLYPH: usize = 6;
/// Maximum characters per batch
/// Maximum glyphs per batch (32K supports 4K displays: ~240x135 cells)
const MAX_GLYPHS: usize = 32768;

/// LCD text renderer
pub struct LcdTextRenderer {
    shader: LcdShader,
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    ebo: glow::Buffer,
    vertices: Vec<f32>,
    glyph_count: usize,
    /// Default background color (sRGB, 0.0-1.0)
    default_bg: [f32; 3],
    /// Whether to use BGR subpixel order
    subpixel_bgr: bool,
    /// Gamma correction value (1.0 = normal, 1.1-1.25 = thinner/tighter appearance)
    gamma: f32,
    /// Stem darkening strength (0.0 = disabled, 0.05-0.15 = enabled)
    stem_darkening: f32,
    /// Contrast boost (1.0 = normal, 1.1-1.3 = sharper)
    contrast: f32,
    /// Color fringe reduction (0.0 = disabled, 0.3-0.5 = enabled)
    fringe_reduction: f32,
}

impl LcdTextRenderer {
    pub fn new(gl: &glow::Context) -> Result<Self> {
        let shader = LcdShader::new(gl)?;

        unsafe {
            let vao = gl
                .create_vertex_array()
                .map_err(|e| anyhow!("Failed to create VAO: {}", e))?;
            gl.bind_vertex_array(Some(vao));

            let vbo = gl
                .create_buffer()
                .map_err(|e| anyhow!("Failed to create VBO: {}", e))?;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            let vbo_size = MAX_GLYPHS * VERTICES_PER_GLYPH * VERTEX_FLOATS * 4;
            gl.buffer_data_size(glow::ARRAY_BUFFER, vbo_size as i32, glow::DYNAMIC_DRAW);

            let ebo = gl
                .create_buffer()
                .map_err(|e| anyhow!("Failed to create EBO: {}", e))?;
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(ebo));

            let mut indices: Vec<u16> = Vec::with_capacity(MAX_GLYPHS * INDICES_PER_GLYPH);
            for i in 0..MAX_GLYPHS as u16 {
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

            let stride = (VERTEX_FLOATS * 4) as i32;

            // a_pos: location=0, vec2
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, stride, 0);

            // a_uv: location=1, vec2
            gl.enable_vertex_attrib_array(1);
            gl.vertex_attrib_pointer_f32(1, 2, glow::FLOAT, false, stride, 8);

            // a_fg_color: location=2, vec4
            gl.enable_vertex_attrib_array(2);
            gl.vertex_attrib_pointer_f32(2, 4, glow::FLOAT, false, stride, 16);

            // a_bg_color: location=3, vec3
            gl.enable_vertex_attrib_array(3);
            gl.vertex_attrib_pointer_f32(3, 3, glow::FLOAT, false, stride, 32);

            gl.bind_vertex_array(None);

            info!(
                "LCD text renderer initialized (contrast boost + fringe reduction + dynamic gamma)"
            );

            Ok(Self {
                shader,
                vao,
                vbo,
                ebo,
                vertices: Vec::with_capacity(MAX_GLYPHS * VERTICES_PER_GLYPH * VERTEX_FLOATS),
                glyph_count: 0,
                default_bg: [0.0, 0.0, 0.0],
                subpixel_bgr: false,
                gamma: 1.15,           // Default: thinner/tighter appearance
                stem_darkening: 0.0,   // Default: disabled (prioritize sharpness)
                contrast: 1.15,        // Default: contrast boost
                fringe_reduction: 0.1, // Default: light fringe reduction
            })
        }
    }

    /// Set default background color
    pub fn set_bg_color(&mut self, r: f32, g: f32, b: f32) {
        self.default_bg = [r, g, b];
    }

    /// Set BGR subpixel order
    pub fn set_subpixel_bgr(&mut self, bgr: bool) {
        self.subpixel_bgr = bgr;
    }

    /// Set gamma correction value (1.0 = normal, 1.1-1.25 = thinner/tighter appearance)
    pub fn set_gamma(&mut self, gamma: f32) {
        self.gamma = gamma;
    }

    /// Set stem darkening strength (0.0 = disabled, 0.05-0.15 = enabled)
    pub fn set_stem_darkening(&mut self, strength: f32) {
        self.stem_darkening = strength;
    }

    /// Set contrast boost (1.0 = normal, 1.1-1.3 = sharper)
    pub fn set_contrast(&mut self, contrast: f32) {
        self.contrast = contrast;
    }

    /// Set color fringe reduction (0.0 = disabled, 0.3-0.5 = enabled)
    pub fn set_fringe_reduction(&mut self, reduction: f32) {
        self.fringe_reduction = reduction;
    }

    pub fn begin(&mut self) {
        self.vertices.clear();
        self.glyph_count = 0;
    }

    /// Add a character to draw buffer (using default background color)
    pub fn push_char(&mut self, ch: char, x: f32, y: f32, fg: [f32; 4], atlas: &LcdGlyphAtlas) {
        self.push_char_with_bg(ch, x, y, fg, self.default_bg, atlas);
    }

    /// Add a character to draw buffer (with specified background color)
    pub fn push_char_with_bg(
        &mut self,
        ch: char,
        x: f32,
        y: f32,
        fg: [f32; 4],
        bg: [f32; 3],
        atlas: &LcdGlyphAtlas,
    ) {
        if self.glyph_count >= MAX_GLYPHS {
            return;
        }

        // Subpixel phase rendering: determine phase from X coordinate fractional part
        let x_frac = x.fract();
        let x_frac = if x_frac < 0.0 { x_frac + 1.0 } else { x_frac };

        let glyph = match atlas.get_glyph_phased(ch, x_frac) {
            Some(g) => g,
            None => return,
        };

        // Apply phase offset to draw position
        let phase_offset = atlas.phase_offset(x_frac);
        self.push_glyph_info(glyph, x + phase_offset, y, fg, bg);
    }

    /// Add text string to draw buffer (using default background color)
    pub fn push_text(&mut self, text: &str, x: f32, y: f32, fg: [f32; 4], atlas: &LcdGlyphAtlas) {
        self.push_text_with_bg(text, x, y, fg, self.default_bg, atlas);
    }

    /// Add text string to draw buffer (with specified background color)
    pub fn push_text_with_bg(
        &mut self,
        text: &str,
        x: f32,
        y: f32,
        fg: [f32; 4],
        bg: [f32; 3],
        atlas: &LcdGlyphAtlas,
    ) {
        let mut cursor_x = x;
        for ch in text.chars() {
            // Subpixel phase rendering
            let x_frac = cursor_x.fract();
            let x_frac = if x_frac < 0.0 { x_frac + 1.0 } else { x_frac };

            if let Some(glyph) = atlas.get_glyph_phased(ch, x_frac) {
                let phase_offset = atlas.phase_offset(x_frac);
                self.push_glyph_info(glyph, cursor_x + phase_offset, y, fg, bg);
                cursor_x += glyph.advance;
            } else {
                cursor_x += atlas.cell_width;
            }
        }
    }

    /// Add glyph info to draw buffer
    fn push_glyph_info(&mut self, glyph: &GlyphInfo, x: f32, y: f32, fg: [f32; 4], bg: [f32; 3]) {
        if glyph.width == 0 || glyph.height == 0 {
            return;
        }

        // LCD subpixel: snap X to 0.5px only (for horizontal subpixels)
        // Y snaps to integer pixels (vertical causes bleeding)
        let gx = (x + glyph.bearing_x).floor() + 0.5;
        let gy = (y - glyph.bearing_y).round();
        let gw = glyph.width as f32;
        let gh = glyph.height as f32;

        let u0 = glyph.uv_x;
        let v0 = glyph.uv_y;
        let u1 = glyph.uv_x + glyph.uv_w;
        let v1 = glyph.uv_y + glyph.uv_h;

        let [fr, fg_c, fb, fa] = fg;
        let [br, bg_c, bb] = bg;

        // 4 vertices in one extend (reduces function call overhead)
        // Each vertex: pos(2) + uv(2) + fg(4) + bg(3) = 11 floats
        #[rustfmt::skip]
        self.vertices.extend_from_slice(&[
            gx,      gy,      u0, v0, fr, fg_c, fb, fa, br, bg_c, bb,  // top-left
            gx + gw, gy,      u1, v0, fr, fg_c, fb, fa, br, bg_c, bb,  // top-right
            gx + gw, gy + gh, u1, v1, fr, fg_c, fb, fa, br, bg_c, bb,  // bottom-right
            gx,      gy + gh, u0, v1, fr, fg_c, fb, fa, br, bg_c, bb,  // bottom-left
        ]);

        self.glyph_count += 1;
    }

    /// Add background rectangle to draw buffer (using default background color)
    pub fn push_rect(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: [f32; 4],
        atlas: &LcdGlyphAtlas,
    ) {
        self.push_rect_with_bg(x, y, w, h, color, self.default_bg, atlas);
    }

    /// Add background rectangle to draw buffer (with specified background color)
    pub fn push_rect_with_bg(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        fg: [f32; 4],
        bg: [f32; 3],
        atlas: &LcdGlyphAtlas,
    ) {
        if self.glyph_count >= MAX_GLYPHS {
            return;
        }

        let (u, v) = atlas.solid_uv();
        let [fr, fg_c, fb, fa] = fg;
        let [br, bg_c, bb] = bg;

        // 4 vertices in one extend (reduces function call overhead)
        #[rustfmt::skip]
        self.vertices.extend_from_slice(&[
            x,     y,     u, v, fr, fg_c, fb, fa, br, bg_c, bb,  // top-left
            x + w, y,     u, v, fr, fg_c, fb, fa, br, bg_c, bb,  // top-right
            x + w, y + h, u, v, fr, fg_c, fb, fa, br, bg_c, bb,  // bottom-right
            x,     y + h, u, v, fr, fg_c, fb, fa, br, bg_c, bb,  // bottom-left
        ]);

        self.glyph_count += 1;
    }

    /// Upload buffer contents to GPU and draw
    pub fn flush(&self, gl: &glow::Context, atlas: &LcdGlyphAtlas, width: u32, height: u32) {
        if self.glyph_count == 0 {
            return;
        }

        unsafe {
            // LCD compositing is done entirely in shader, so disable GL blend
            // (output pre-composited FG/BG colors directly)
            gl.disable(glow::BLEND);

            self.shader.bind(gl);

            let projection = shader::ortho_projection(width as f32, height as f32);
            self.shader.set_projection(gl, &projection);

            // Gamma correction value (1.0 = normal, 1.1-1.25 = thinner/tighter)
            self.shader.set_gamma(gl, self.gamma);

            // Stem darkening (0.0 = disabled, 0.05-0.15 = thicker)
            self.shader.set_stem_darkening(gl, self.stem_darkening);

            // Contrast boost (1.0 = normal, 1.1-1.3 = sharper)
            self.shader.set_contrast(gl, self.contrast);

            // Color fringe reduction (0.0 = disabled, 0.3-0.5 = enabled)
            self.shader.set_fringe_reduction(gl, self.fringe_reduction);

            // BGR subpixel order
            self.shader.set_subpixel_bgr(gl, self.subpixel_bgr);

            // Bind texture
            atlas.bind(gl, 0);
            self.shader.set_atlas_unit(gl, 0);

            gl.bind_vertex_array(Some(self.vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));

            let vertex_bytes: &[u8] = bytemuck_cast_slice(&self.vertices);
            gl.buffer_sub_data_u8_slice(glow::ARRAY_BUFFER, 0, vertex_bytes);

            gl.draw_elements(
                glow::TRIANGLES,
                (self.glyph_count * INDICES_PER_GLYPH) as i32,
                glow::UNSIGNED_SHORT,
                0,
            );

            gl.bind_vertex_array(None);
            // Re-enable blend for other renderers
            gl.enable(glow::BLEND);
            gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
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

/// Compile shader and link program
fn compile_program(
    gl: &glow::Context,
    vertex_src: &str,
    fragment_src: &str,
) -> Result<glow::Program> {
    unsafe {
        let vs = compile_shader(gl, glow::VERTEX_SHADER, vertex_src)?;
        let fs = compile_shader(gl, glow::FRAGMENT_SHADER, fragment_src)?;

        let program = gl
            .create_program()
            .map_err(|e| anyhow!("Failed to create program: {}", e))?;

        gl.attach_shader(program, vs);
        gl.attach_shader(program, fs);
        gl.link_program(program);

        if !gl.get_program_link_status(program) {
            let log = gl.get_program_info_log(program);
            gl.delete_program(program);
            gl.delete_shader(vs);
            gl.delete_shader(fs);
            return Err(anyhow!("Shader link failed: {}", log));
        }

        gl.delete_shader(vs);
        gl.delete_shader(fs);

        Ok(program)
    }
}

fn compile_shader(gl: &glow::Context, shader_type: u32, source: &str) -> Result<glow::Shader> {
    unsafe {
        let shader = gl
            .create_shader(shader_type)
            .map_err(|e| anyhow!("Failed to create shader: {}", e))?;

        gl.shader_source(shader, source);
        gl.compile_shader(shader);

        if !gl.get_shader_compile_status(shader) {
            let log = gl.get_shader_info_log(shader);
            gl.delete_shader(shader);
            let type_name = match shader_type {
                glow::VERTEX_SHADER => "vertex",
                glow::FRAGMENT_SHADER => "fragment",
                _ => "unknown",
            };
            return Err(anyhow!("{} shader compile failed: {}", type_name, log));
        }

        Ok(shader)
    }
}

use super::bytemuck_cast_slice;
