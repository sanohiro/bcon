//! Emoji renderer
//!
//! Draws color emoji from RGBA texture

use anyhow::{anyhow, Result};
use glow::HasContext;
use log::info;

use crate::font::emoji::EmojiAtlas;

use super::shader::ortho_projection;

/// Vertex shader for emoji rendering (GLSL ES 3.00)
const EMOJI_VERTEX_SHADER: &str = r#"#version 300 es
precision mediump float;

layout(location = 0) in vec2 a_pos;
layout(location = 1) in vec2 a_uv;

uniform mat4 u_projection;

out vec2 v_uv;

void main() {
    gl_Position = u_projection * vec4(a_pos, 0.0, 1.0);
    v_uv = a_uv;
}
"#;

/// Fragment shader for emoji rendering
/// Samples from SRGB8_ALPHA8 texture (automatically linearized)
const EMOJI_FRAGMENT_SHADER: &str = r#"#version 300 es
precision mediump float;

in vec2 v_uv;

uniform sampler2D u_texture;

out vec4 frag_color;

// Linear -> sRGB conversion (for framebuffer output)
float linear_to_srgb(float c) {
    return c <= 0.0031308 ? c * 12.92 : 1.055 * pow(c, 1.0/2.4) - 0.055;
}

void main() {
    // SRGB8_ALPHA8 texture: automatically linearized when sampled
    vec4 texel = texture(u_texture, v_uv);
    vec3 linear_color = texel.rgb;

    // Increase saturation slightly for vibrancy
    float luma = dot(linear_color, vec3(0.2126, 0.7152, 0.0722));
    linear_color = mix(vec3(luma), linear_color, 1.1);
    linear_color = clamp(linear_color, 0.0, 1.0);

    // Convert linear -> sRGB (framebuffer doesn't support sRGB)
    vec3 srgb_color = vec3(
        linear_to_srgb(linear_color.r),
        linear_to_srgb(linear_color.g),
        linear_to_srgb(linear_color.b)
    );

    frag_color = vec4(srgb_color, texel.a);
}
"#;

/// Vertex data: position(2) + UV(2) = 4 floats
const VERTEX_FLOATS: usize = 4;
/// 1 character = 4 vertices
const VERTICES_PER_GLYPH: usize = 4;
/// 1 character = 6 indices (2 triangles)
const INDICES_PER_GLYPH: usize = 6;
/// Maximum characters per batch
const MAX_GLYPHS: usize = 1024;

/// Emoji shader
struct EmojiShader {
    program: glow::Program,
    u_projection: glow::UniformLocation,
    u_texture: glow::UniformLocation,
}

impl EmojiShader {
    fn new(gl: &glow::Context) -> Result<Self> {
        let program = compile_program(gl, EMOJI_VERTEX_SHADER, EMOJI_FRAGMENT_SHADER)?;

        let u_projection = unsafe {
            gl.get_uniform_location(program, "u_projection")
                .ok_or_else(|| anyhow!("u_projection uniform not found (emoji)"))?
        };
        let u_texture = unsafe {
            gl.get_uniform_location(program, "u_texture")
                .ok_or_else(|| anyhow!("u_texture uniform not found (emoji)"))?
        };

        Ok(Self {
            program,
            u_projection,
            u_texture,
        })
    }

    fn bind(&self, gl: &glow::Context) {
        unsafe {
            gl.use_program(Some(self.program));
        }
    }

    fn set_projection(&self, gl: &glow::Context, matrix: &[f32; 16]) {
        unsafe {
            gl.uniform_matrix_4_f32_slice(Some(&self.u_projection), false, matrix);
        }
    }

    fn set_texture_unit(&self, gl: &glow::Context, unit: i32) {
        unsafe {
            gl.uniform_1_i32(Some(&self.u_texture), unit);
        }
    }

    fn destroy(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_program(self.program);
        }
    }
}

/// Emoji renderer
pub struct EmojiRenderer {
    shader: EmojiShader,
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    ebo: glow::Buffer,
    /// Vertex buffer (CPU side)
    vertices: Vec<f32>,
    /// Current character count in buffer
    glyph_count: usize,
}

impl EmojiRenderer {
    /// Create EmojiRenderer
    pub fn new(gl: &glow::Context) -> Result<Self> {
        let shader = EmojiShader::new(gl)?;

        unsafe {
            // VAO
            let vao = gl
                .create_vertex_array()
                .map_err(|e| anyhow!("Failed to create VAO (emoji): {}", e))?;
            gl.bind_vertex_array(Some(vao));

            // VBO
            let vbo = gl
                .create_buffer()
                .map_err(|e| anyhow!("Failed to create VBO (emoji): {}", e))?;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            let vbo_size = MAX_GLYPHS * VERTICES_PER_GLYPH * VERTEX_FLOATS * 4;
            gl.buffer_data_size(glow::ARRAY_BUFFER, vbo_size as i32, glow::DYNAMIC_DRAW);

            // EBO
            let ebo = gl
                .create_buffer()
                .map_err(|e| anyhow!("Failed to create EBO (emoji): {}", e))?;
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(ebo));

            // Pre-generate index data
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
            let index_bytes = bytemuck_cast_slice(&indices);
            gl.buffer_data_u8_slice(glow::ELEMENT_ARRAY_BUFFER, index_bytes, glow::STATIC_DRAW);

            // Set vertex attributes
            let stride = (VERTEX_FLOATS * 4) as i32;

            // a_pos: location=0, vec2
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, stride, 0);

            // a_uv: location=1, vec2
            gl.enable_vertex_attrib_array(1);
            gl.vertex_attrib_pointer_f32(1, 2, glow::FLOAT, false, stride, 8);

            gl.bind_vertex_array(None);

            info!("EmojiRenderer initialized");

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

    /// Clear buffer
    pub fn begin(&mut self) {
        self.vertices.clear();
        self.glyph_count = 0;
    }

    /// Add emoji to draw buffer
    pub fn push_emoji(
        &mut self,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        uv_x: f32,
        uv_y: f32,
        uv_w: f32,
        uv_h: f32,
    ) {
        if self.glyph_count >= MAX_GLYPHS {
            return;
        }

        // Round to integer pixels
        let x = x.round();
        let y = y.round();
        let w = width.round();
        let h = height.round();

        let u0 = uv_x;
        let v0 = uv_y;
        let u1 = uv_x + uv_w;
        let v1 = uv_y + uv_h;

        // 4 vertices in one extend (reduces function call overhead)
        #[rustfmt::skip]
        self.vertices.extend_from_slice(&[
            x,     y,     u0, v0,  // top-left
            x + w, y,     u1, v0,  // top-right
            x + w, y + h, u1, v1,  // bottom-right
            x,     y + h, u0, v1,  // bottom-left
        ]);

        self.glyph_count += 1;
    }

    /// Flush buffer and draw
    pub fn flush(
        &mut self,
        gl: &glow::Context,
        emoji_atlas: &EmojiAtlas,
        screen_width: u32,
        screen_height: u32,
    ) {
        if self.glyph_count == 0 {
            return;
        }

        let texture = match emoji_atlas.texture() {
            Some(t) => t,
            None => return,
        };

        unsafe {
            // Enable blending
            gl.enable(glow::BLEND);
            gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);

            // Bind shader
            self.shader.bind(gl);

            // Set orthographic projection matrix
            let projection = ortho_projection(screen_width as f32, screen_height as f32);
            self.shader.set_projection(gl, &projection);
            self.shader.set_texture_unit(gl, 0);

            // Bind texture
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));

            // Bind VAO
            gl.bind_vertex_array(Some(self.vao));

            // Upload vertex data
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            let vertex_bytes = bytemuck_cast_slice(&self.vertices);
            gl.buffer_sub_data_u8_slice(glow::ARRAY_BUFFER, 0, vertex_bytes);

            // Draw
            let index_count = (self.glyph_count * INDICES_PER_GLYPH) as i32;
            gl.draw_elements(glow::TRIANGLES, index_count, glow::UNSIGNED_SHORT, 0);

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

/// Compile shaders and link program
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
            .map_err(|e| anyhow!("Program creation failed (emoji): {}", e))?;

        gl.attach_shader(program, vs);
        gl.attach_shader(program, fs);
        gl.link_program(program);

        if !gl.get_program_link_status(program) {
            let log = gl.get_program_info_log(program);
            gl.delete_program(program);
            gl.delete_shader(vs);
            gl.delete_shader(fs);
            return Err(anyhow!("Shader link failed (emoji): {}", log));
        }

        gl.delete_shader(vs);
        gl.delete_shader(fs);

        Ok(program)
    }
}

/// Compile individual shader
fn compile_shader(gl: &glow::Context, shader_type: u32, source: &str) -> Result<glow::Shader> {
    unsafe {
        let shader = gl
            .create_shader(shader_type)
            .map_err(|e| anyhow!("Shader creation failed (emoji): {}", e))?;

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
            return Err(anyhow!(
                "{} shader compile failed (emoji): {}",
                type_name,
                log
            ));
        }

        Ok(shader)
    }
}

/// &[T] -> &[u8] conversion
///
/// # Safety
/// This is safe because:
/// - The pointer comes from a valid slice
/// - The size calculation cannot overflow (slice.len() is already bounded by allocation)
/// - The resulting byte slice refers to the same memory as the input slice
/// - T is expected to be a plain data type (no padding issues for our use case)
fn bytemuck_cast_slice<T>(slice: &[T]) -> &[u8] {
    // SAFETY: See function documentation above
    unsafe {
        std::slice::from_raw_parts(
            slice.as_ptr() as *const u8,
            slice.len() * std::mem::size_of::<T>(),
        )
    }
}
