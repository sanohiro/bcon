//! LCD subpixel rendering text renderer (Instanced version)
//!
//! High-quality text rendering with FreeType + RGB atlas + LCD shader
//! Uses GPU instancing: 1 instance = 1 glyph (66% less data transfer vs 4-vertex approach)

use anyhow::{anyhow, Result};
use glow::HasContext;
use log::info;

use crate::font::lcd_atlas::{GlyphInfo, LcdGlyphAtlas};
use crate::gpu::shader;

/// Instanced LCD vertex shader
/// gl_VertexID determines corner (0-3), instance data provides position/UV/colors
const LCD_VERTEX_SHADER: &str = r#"#version 300 es
precision highp float;

// Per-instance attributes
layout(location = 0) in vec4 a_pos_size;     // xy = position, zw = size
layout(location = 1) in vec4 a_uv_pos_size;  // xy = uv position, zw = uv size
layout(location = 2) in vec4 a_fg_color;
layout(location = 3) in vec3 a_bg_color;
layout(location = 4) in float a_lcd_disable; // 1.0 = use grayscale AA

uniform mat4 u_projection;

out vec2 v_uv;
out vec4 v_fg_color;
out vec3 v_bg_color;
out float v_lcd_disable;

void main() {
    // gl_VertexID: 0=TL, 1=TR, 2=BL, 3=BR (triangle strip order)
    vec2 corner = vec2(
        float(gl_VertexID & 1),        // x: 0,1,0,1
        float((gl_VertexID >> 1) & 1)  // y: 0,0,1,1
    );

    vec2 pos = a_pos_size.xy + corner * a_pos_size.zw;
    vec2 uv = a_uv_pos_size.xy + corner * a_uv_pos_size.zw;

    gl_Position = u_projection * vec4(pos, 0.0, 1.0);
    v_uv = uv;
    v_fg_color = a_fg_color;
    v_bg_color = a_bg_color;
    v_lcd_disable = a_lcd_disable;
}
"#;

const LCD_FRAGMENT_SHADER: &str = r#"#version 300 es
precision highp float;

in vec2 v_uv;
in vec4 v_fg_color;
in vec3 v_bg_color;
in float v_lcd_disable;

uniform sampler2D u_atlas;
uniform float u_gamma;
uniform float u_stem_darkening;
uniform float u_contrast;
uniform float u_fringe_reduction;
uniform int u_subpixel_bgr;

out vec4 frag_color;

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
    vec3 coverage = texture(u_atlas, v_uv).rgb;

    if (u_subpixel_bgr == 1) {
        coverage = coverage.bgr;
    }

    if (u_fringe_reduction > 0.0) {
        float avg = (coverage.r + coverage.g + coverage.b) / 3.0;
        coverage = mix(coverage, vec3(avg), u_fringe_reduction);
    }

    coverage = pow(coverage, vec3(u_gamma));

    if (u_contrast != 1.0) {
        coverage = clamp((coverage - 0.5) * u_contrast + 0.5, 0.0, 1.0);
    }

    if (u_stem_darkening > 0.0) {
        float avg_cov = (coverage.r + coverage.g + coverage.b) / 3.0;
        float darken = smoothstep(0.0, 0.5, avg_cov) * u_stem_darkening;
        coverage = min(coverage + darken, vec3(1.0));
    }

    // Per-instance LCD disable: use grayscale AA for colored backgrounds
    // Use max instead of avg to preserve sharpness (especially for thin strokes like 'r')
    float gray = max(max(coverage.r, coverage.g), coverage.b);
    coverage = mix(coverage, vec3(gray), v_lcd_disable);

    vec3 fg_linear = srgb_to_linear(v_fg_color.rgb);
    vec3 bg_linear = srgb_to_linear(v_bg_color);

    float bg_luma = dot(bg_linear, vec3(0.2126, 0.7152, 0.0722));
    float dynamic_boost = mix(1.0, 0.95, bg_luma);
    coverage *= dynamic_boost;

    vec3 blended_linear = mix(bg_linear, fg_linear, coverage * v_fg_color.a);
    vec3 blended_srgb = linear_to_srgb(blended_linear);

    float alpha = max(max(coverage.r, coverage.g), coverage.b) * v_fg_color.a;
    frag_color = vec4(blended_srgb, alpha);
}
"#;

/// LCD shader for instanced rendering
pub struct LcdShaderInstanced {
    program: glow::Program,
    pub u_projection: glow::UniformLocation,
    pub u_atlas: glow::UniformLocation,
    pub u_gamma: glow::UniformLocation,
    pub u_stem_darkening: glow::UniformLocation,
    pub u_contrast: glow::UniformLocation,
    pub u_fringe_reduction: glow::UniformLocation,
    pub u_subpixel_bgr: glow::UniformLocation,
}

impl LcdShaderInstanced {
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

        info!("LCD instanced shader compiled");
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

/// Per-instance data: pos_size(4) + uv_pos_size(4) + fg(4) + bg(3) + lcd_disable(1) = 16 floats
const INSTANCE_FLOATS: usize = 16;
/// Maximum glyphs per batch (32K supports 4K displays)
const MAX_GLYPHS: usize = 32768;

/// LCD text renderer using GPU instancing
pub struct LcdTextRendererInstanced {
    shader: LcdShaderInstanced,
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    instances: Vec<f32>,
    glyph_count: usize,
    default_bg: [f32; 3],
    subpixel_bgr: bool,
    gamma: f32,
    stem_darkening: f32,
    contrast: f32,
    fringe_reduction: f32,
}

impl LcdTextRendererInstanced {
    pub fn new(gl: &glow::Context) -> Result<Self> {
        let shader = LcdShaderInstanced::new(gl)?;

        unsafe {
            let vao = gl
                .create_vertex_array()
                .map_err(|e| anyhow!("Failed to create VAO: {}", e))?;
            gl.bind_vertex_array(Some(vao));

            let vbo = gl
                .create_buffer()
                .map_err(|e| anyhow!("Failed to create VBO: {}", e))?;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            let vbo_size = MAX_GLYPHS * INSTANCE_FLOATS * 4;
            gl.buffer_data_size(glow::ARRAY_BUFFER, vbo_size as i32, glow::DYNAMIC_DRAW);

            let stride = (INSTANCE_FLOATS * 4) as i32;

            // a_pos_size: location=0, vec4 (position xy, size wh)
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 4, glow::FLOAT, false, stride, 0);
            gl.vertex_attrib_divisor(0, 1); // per-instance

            // a_uv_pos_size: location=1, vec4 (uv position, uv size)
            gl.enable_vertex_attrib_array(1);
            gl.vertex_attrib_pointer_f32(1, 4, glow::FLOAT, false, stride, 16);
            gl.vertex_attrib_divisor(1, 1);

            // a_fg_color: location=2, vec4
            gl.enable_vertex_attrib_array(2);
            gl.vertex_attrib_pointer_f32(2, 4, glow::FLOAT, false, stride, 32);
            gl.vertex_attrib_divisor(2, 1);

            // a_bg_color: location=3, vec3
            gl.enable_vertex_attrib_array(3);
            gl.vertex_attrib_pointer_f32(3, 3, glow::FLOAT, false, stride, 48);
            gl.vertex_attrib_divisor(3, 1);

            // a_lcd_disable: location=4, float
            gl.enable_vertex_attrib_array(4);
            gl.vertex_attrib_pointer_f32(4, 1, glow::FLOAT, false, stride, 60);
            gl.vertex_attrib_divisor(4, 1);

            gl.bind_vertex_array(None);

            info!("LCD instanced text renderer initialized (66% less GPU transfer)");

            Ok(Self {
                shader,
                vao,
                vbo,
                instances: Vec::with_capacity(MAX_GLYPHS * INSTANCE_FLOATS),
                glyph_count: 0,
                default_bg: [0.0, 0.0, 0.0],
                subpixel_bgr: false,
                gamma: 1.15,
                stem_darkening: 0.0,
                contrast: 1.15,
                fringe_reduction: 0.1,
            })
        }
    }

    pub fn set_bg_color(&mut self, r: f32, g: f32, b: f32) {
        self.default_bg = [r, g, b];
    }

    pub fn set_subpixel_bgr(&mut self, bgr: bool) {
        self.subpixel_bgr = bgr;
    }

    pub fn set_gamma(&mut self, gamma: f32) {
        self.gamma = gamma;
    }

    pub fn set_stem_darkening(&mut self, strength: f32) {
        self.stem_darkening = strength;
    }

    pub fn set_contrast(&mut self, contrast: f32) {
        self.contrast = contrast;
    }

    pub fn set_fringe_reduction(&mut self, reduction: f32) {
        self.fringe_reduction = reduction;
    }

    pub fn begin(&mut self) {
        self.instances.clear();
        self.glyph_count = 0;
    }

    /// Add a character (using default background)
    pub fn push_char(&mut self, ch: char, x: f32, y: f32, fg: [f32; 4], atlas: &LcdGlyphAtlas) {
        self.push_char_with_bg(ch, x, y, fg, self.default_bg, 0.0, atlas);
    }

    /// Add a character with specific background and LCD disable flag
    pub fn push_char_with_bg(
        &mut self,
        ch: char,
        x: f32,
        y: f32,
        fg: [f32; 4],
        bg: [f32; 3],
        lcd_disable: f32,
        atlas: &LcdGlyphAtlas,
    ) {
        if self.glyph_count >= MAX_GLYPHS {
            return;
        }

        let x_frac = x.fract();
        let x_frac = if x_frac < 0.0 { x_frac + 1.0 } else { x_frac };

        let glyph = match atlas.get_glyph_phased(ch, x_frac) {
            Some(g) => g,
            None => return,
        };

        let phase_offset = atlas.phase_offset(x_frac);

        // Fix underscore position: don't let it go too far below baseline
        if ch == '_' && glyph.bearing_y < 0.0 {
            let min_bearing = -(atlas.ascent * 0.06);
            let adjusted_bearing = glyph.bearing_y.max(min_bearing);
            self.push_glyph_info_with_adjusted_bearing(
                glyph,
                x + phase_offset,
                y,
                adjusted_bearing,
                fg,
                bg,
                lcd_disable,
            );
        } else {
            self.push_glyph_info(glyph, x + phase_offset, y, fg, bg, lcd_disable);
        }
    }

    /// Add text string (using default background, LCD enabled)
    #[allow(dead_code)]
    pub fn push_text(&mut self, text: &str, x: f32, y: f32, fg: [f32; 4], atlas: &LcdGlyphAtlas) {
        self.push_text_with_bg_lcd(text, x, y, fg, self.default_bg, 0.0, atlas);
    }

    /// Add text string with specific background (LCD enabled)
    pub fn push_text_with_bg(
        &mut self,
        text: &str,
        x: f32,
        y: f32,
        fg: [f32; 4],
        bg: [f32; 3],
        atlas: &LcdGlyphAtlas,
    ) {
        self.push_text_with_bg_lcd(text, x, y, fg, bg, 0.0, atlas);
    }

    /// Add text string with specific background and LCD disable flag
    pub fn push_text_with_bg_lcd(
        &mut self,
        text: &str,
        x: f32,
        y: f32,
        fg: [f32; 4],
        bg: [f32; 3],
        lcd_disable: f32,
        atlas: &LcdGlyphAtlas,
    ) {
        let mut cursor_x = x;
        for ch in text.chars() {
            let x_frac = cursor_x.fract();
            let x_frac = if x_frac < 0.0 { x_frac + 1.0 } else { x_frac };

            if let Some(glyph) = atlas.get_glyph_phased(ch, x_frac) {
                let phase_offset = atlas.phase_offset(x_frac);

                // Fix underscore position: don't let it go too far below baseline
                // Clamp bearing_y to at most ~6% of ascent below baseline (scales with font size)
                if ch == '_' && glyph.bearing_y < 0.0 {
                    let min_bearing = -(atlas.ascent * 0.06);
                    let adjusted_bearing = glyph.bearing_y.max(min_bearing);
                    self.push_glyph_info_with_adjusted_bearing(
                        glyph,
                        cursor_x + phase_offset,
                        y,
                        adjusted_bearing,
                        fg,
                        bg,
                        lcd_disable,
                    );
                } else {
                    self.push_glyph_info(glyph, cursor_x + phase_offset, y, fg, bg, lcd_disable);
                }
                cursor_x += glyph.advance;
            } else {
                cursor_x += atlas.cell_width;
            }
        }
    }

    /// Add glyph info as instance data
    fn push_glyph_info(
        &mut self,
        glyph: &GlyphInfo,
        x: f32,
        y: f32,
        fg: [f32; 4],
        bg: [f32; 3],
        lcd_disable: f32,
    ) {
        self.push_glyph_info_internal(glyph, x, y, glyph.bearing_y, fg, bg, lcd_disable);
    }

    /// Add glyph with adjusted bearing_y (for underscore position fix)
    fn push_glyph_info_with_adjusted_bearing(
        &mut self,
        glyph: &GlyphInfo,
        x: f32,
        y: f32,
        bearing_y_override: f32,
        fg: [f32; 4],
        bg: [f32; 3],
        lcd_disable: f32,
    ) {
        self.push_glyph_info_internal(glyph, x, y, bearing_y_override, fg, bg, lcd_disable);
    }

    /// Internal: add glyph info as instance data
    fn push_glyph_info_internal(
        &mut self,
        glyph: &GlyphInfo,
        x: f32,
        y: f32,
        bearing_y: f32,
        fg: [f32; 4],
        bg: [f32; 3],
        lcd_disable: f32,
    ) {
        if glyph.width == 0 || glyph.height == 0 {
            return;
        }

        // Position and size
        let gx = (x + glyph.bearing_x).floor() + 0.5;
        let gy = (y - bearing_y).round();

        let gw = glyph.width as f32;
        let gh = glyph.height as f32;

        // UV position and size
        let u0 = glyph.uv_x;
        let v0 = glyph.uv_y;
        let uw = glyph.uv_w;
        let vh = glyph.uv_h;

        let [fr, fg_c, fb, fa] = fg;
        let [br, bg_c, bb] = bg;

        // 16 floats per instance
        #[rustfmt::skip]
        self.instances.extend_from_slice(&[
            gx, gy, gw, gh,           // pos_size
            u0, v0, uw, vh,           // uv_pos_size
            fr, fg_c, fb, fa,         // fg_color
            br, bg_c, bb,             // bg_color
            lcd_disable,              // lcd_disable flag
        ]);

        self.glyph_count += 1;
    }

    /// Add glyph scaled to specific size (for Powerline characters)
    #[allow(dead_code)]
    pub fn push_glyph_scaled(
        &mut self,
        glyph: &GlyphInfo,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        fg: [f32; 4],
        bg: [f32; 3],
        lcd_disable: f32,
        _atlas: &LcdGlyphAtlas,
    ) {
        if self.glyph_count >= MAX_GLYPHS {
            return;
        }
        if glyph.width == 0 || glyph.height == 0 {
            return;
        }

        // UV coordinates from glyph
        let u0 = glyph.uv_x;
        let v0 = glyph.uv_y;
        let uw = glyph.uv_w;
        let vh = glyph.uv_h;

        let [fr, fg_c, fb, fa] = fg;
        let [br, bg_c, bb] = bg;

        // Draw at specified position and size (scaled)
        #[rustfmt::skip]
        self.instances.extend_from_slice(&[
            x, y, w, h,               // pos_size (scaled)
            u0, v0, uw, vh,           // uv_pos_size
            fr, fg_c, fb, fa,         // fg_color
            br, bg_c, bb,             // bg_color
            lcd_disable,              // lcd_disable flag
        ]);

        self.glyph_count += 1;
    }

    /// Add rectangle (using default background)
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

    /// Add rectangle with specific background
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

        #[rustfmt::skip]
        self.instances.extend_from_slice(&[
            x, y, w, h,               // pos_size
            u, v, 0.0, 0.0,           // uv_pos_size (size=0 for solid)
            fr, fg_c, fb, fa,         // fg_color
            br, bg_c, bb,             // bg_color
            0.0,                      // lcd_disable (not used for rects)
        ]);

        self.glyph_count += 1;
    }

    /// Upload and draw
    pub fn flush(&self, gl: &glow::Context, atlas: &LcdGlyphAtlas, width: u32, height: u32) {
        if self.glyph_count == 0 {
            return;
        }

        unsafe {
            gl.disable(glow::BLEND);

            self.shader.bind(gl);

            let projection = shader::ortho_projection(width as f32, height as f32);
            self.shader.set_projection(gl, &projection);
            self.shader.set_gamma(gl, self.gamma);
            self.shader.set_stem_darkening(gl, self.stem_darkening);
            self.shader.set_contrast(gl, self.contrast);
            self.shader.set_fringe_reduction(gl, self.fringe_reduction);
            self.shader.set_subpixel_bgr(gl, self.subpixel_bgr);

            atlas.bind(gl, 0);
            self.shader.set_atlas_unit(gl, 0);

            gl.bind_vertex_array(Some(self.vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));

            let instance_bytes: &[u8] = bytemuck_cast_slice(&self.instances);
            gl.buffer_sub_data_u8_slice(glow::ARRAY_BUFFER, 0, instance_bytes);

            // Draw 4 vertices per instance (triangle strip)
            gl.draw_arrays_instanced(glow::TRIANGLE_STRIP, 0, 4, self.glyph_count as i32);

            gl.bind_vertex_array(None);
            gl.enable(glow::BLEND);
            gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
        }
    }

    pub fn destroy(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_vertex_array(self.vao);
            gl.delete_buffer(self.vbo);
        }
        self.shader.destroy(gl);
    }
}

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

fn bytemuck_cast_slice<T>(slice: &[T]) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(
            slice.as_ptr() as *const u8,
            slice.len() * std::mem::size_of::<T>(),
        )
    }
}
