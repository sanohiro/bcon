//! Image renderer
//!
//! Draw RGBA images (e.g., Sixel) on GPU.

use std::collections::HashMap;

use anyhow::{anyhow, Result};
use glow::HasContext;
use log::info;

use crate::terminal::TerminalImage;

use super::shader::ortho_projection;

/// Image drawing vertex shader (GLSL ES 3.00)
const IMAGE_VERTEX_SHADER: &str = r#"#version 300 es
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

/// Image drawing fragment shader
const IMAGE_FRAGMENT_SHADER: &str = r#"#version 300 es
precision mediump float;

in vec2 v_uv;

uniform sampler2D u_image;

out vec4 frag_color;

void main() {
    frag_color = texture(u_image, v_uv);
}
"#;

/// Per-vertex data: position(2) + UV(2) = 4 floats
const VERTEX_FLOATS: usize = 4;
/// 1 image = 4 vertices
const VERTICES_PER_IMAGE: usize = 4;
/// 1 image = 6 indices (2 triangles)
const INDICES_PER_IMAGE: usize = 6;
/// Maximum images per batch
const MAX_IMAGES: usize = 64;
/// Maximum cached textures (LRU eviction when exceeded)
const MAX_CACHED_TEXTURES: usize = 128;

/// Image shader
struct ImageShader {
    program: glow::Program,
    u_projection: glow::UniformLocation,
    u_image: glow::UniformLocation,
}

impl ImageShader {
    fn new(gl: &glow::Context) -> Result<Self> {
        let program = compile_program(gl, IMAGE_VERTEX_SHADER, IMAGE_FRAGMENT_SHADER)?;

        let u_projection = unsafe {
            gl.get_uniform_location(program, "u_projection")
                .ok_or_else(|| anyhow!("u_projection uniform not found (image)"))?
        };
        let u_image = unsafe {
            gl.get_uniform_location(program, "u_image")
                .ok_or_else(|| anyhow!("u_image uniform not found"))?
        };

        Ok(Self {
            program,
            u_projection,
            u_image,
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

    fn set_image_unit(&self, gl: &glow::Context, unit: i32) {
        unsafe {
            gl.uniform_1_i32(Some(&self.u_image), unit);
        }
    }

    fn destroy(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_program(self.program);
        }
    }
}

/// Image draw info (for batching)
struct DrawCall {
    /// Image ID
    id: u32,
    /// Draw X coordinate (pixels)
    x: f32,
    /// Draw Y coordinate (pixels)
    y: f32,
    /// Draw width (pixels)
    w: f32,
    /// Draw height (pixels)
    h: f32,
}

/// Image renderer
pub struct ImageRenderer {
    shader: ImageShader,
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    ebo: glow::Buffer,
    /// Texture cache (image ID -> texture)
    textures: HashMap<u32, glow::Texture>,
    /// LRU order (most recently used at end)
    lru_order: Vec<u32>,
    /// Draw queue
    draw_queue: Vec<DrawCall>,
}

impl ImageRenderer {
    /// Create ImageRenderer
    pub fn new(gl: &glow::Context) -> Result<Self> {
        let shader = ImageShader::new(gl)?;

        unsafe {
            // VAO
            let vao = gl
                .create_vertex_array()
                .map_err(|e| anyhow!("Failed to create VAO (image): {}", e))?;
            gl.bind_vertex_array(Some(vao));

            // VBO
            let vbo = gl
                .create_buffer()
                .map_err(|e| anyhow!("Failed to create VBO (image): {}", e))?;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            let vbo_size = MAX_IMAGES * VERTICES_PER_IMAGE * VERTEX_FLOATS * 4;
            gl.buffer_data_size(glow::ARRAY_BUFFER, vbo_size as i32, glow::DYNAMIC_DRAW);

            // EBO
            let ebo = gl
                .create_buffer()
                .map_err(|e| anyhow!("Failed to create EBO (image): {}", e))?;
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(ebo));

            // Pre-generate index data
            let mut indices: Vec<u16> = Vec::with_capacity(MAX_IMAGES * INDICES_PER_IMAGE);
            for i in 0..MAX_IMAGES as u16 {
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

            info!("ImageRenderer initialized");

            Ok(Self {
                shader,
                vao,
                vbo,
                ebo,
                textures: HashMap::new(),
                lru_order: Vec::with_capacity(MAX_CACHED_TEXTURES),
                draw_queue: Vec::new(),
            })
        }
    }

    /// Upload image texture
    pub fn upload_image(&mut self, gl: &glow::Context, image: &TerminalImage) {
        if self.textures.contains_key(&image.id) {
            // Already cached - update LRU order
            self.touch_lru(image.id);
            return;
        }

        // Evict oldest textures if cache is full
        while self.textures.len() >= MAX_CACHED_TEXTURES && !self.lru_order.is_empty() {
            let oldest_id = self.lru_order.remove(0);
            if let Some(texture) = self.textures.remove(&oldest_id) {
                unsafe {
                    gl.delete_texture(texture);
                }
                log::debug!("Evicted texture id={} (LRU)", oldest_id);
            }
        }

        unsafe {
            let texture = match gl.create_texture() {
                Ok(t) => t,
                Err(e) => {
                    log::warn!("Failed to create texture: {}", e);
                    return;
                }
            };

            gl.bind_texture(glow::TEXTURE_2D, Some(texture));

            // Upload as RGBA texture
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA as i32,
                image.width as i32,
                image.height as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                Some(&image.data),
            );

            // Texture parameters
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

            gl.bind_texture(glow::TEXTURE_2D, None);

            self.textures.insert(image.id, texture);
            self.lru_order.push(image.id);
            info!(
                "Image texture uploaded: id={} {}x{} (cache: {}/{})",
                image.id,
                image.width,
                image.height,
                self.textures.len(),
                MAX_CACHED_TEXTURES
            );
        }
    }

    /// Update LRU order (move id to end = most recently used)
    fn touch_lru(&mut self, id: u32) {
        if let Some(pos) = self.lru_order.iter().position(|&x| x == id) {
            self.lru_order.remove(pos);
            self.lru_order.push(id);
        }
    }

    /// Check if texture exists
    pub fn has_texture(&self, id: u32) -> bool {
        self.textures.contains_key(&id)
    }

    /// Clear draw queue
    pub fn begin(&mut self) {
        self.draw_queue.clear();
    }

    /// Add image to draw queue
    pub fn draw(&mut self, id: u32, x: f32, y: f32, w: f32, h: f32) {
        if self.draw_queue.len() >= MAX_IMAGES {
            return;
        }
        self.draw_queue.push(DrawCall { id, x, y, w, h });
    }

    /// Flush draw queue
    pub fn flush(&mut self, gl: &glow::Context, screen_width: u32, screen_height: u32) {
        if self.draw_queue.is_empty() {
            return;
        }

        unsafe {
            // Enable blending
            gl.enable(glow::BLEND);
            gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);

            // Bind shader
            self.shader.bind(gl);

            // Set orthographic projection matrix
            let projection = ortho_projection(screen_width as f32, screen_height as f32);
            self.shader.set_projection(gl, &projection);
            self.shader.set_image_unit(gl, 0);

            // Bind VAO
            gl.bind_vertex_array(Some(self.vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));

            // Draw per image (cannot batch due to different textures)
            for call in &self.draw_queue {
                let texture = match self.textures.get(&call.id) {
                    Some(t) => *t,
                    None => continue,
                };

                // Generate vertex data
                let x = call.x;
                let y = call.y;
                let w = call.w;
                let h = call.h;

                // 4 vertices: top-left, top-right, bottom-right, bottom-left
                #[rustfmt::skip]
                let vertices: [f32; 16] = [
                    x,     y,     0.0, 0.0,  // top-left
                    x + w, y,     1.0, 0.0,  // top-right
                    x + w, y + h, 1.0, 1.0,  // bottom-right
                    x,     y + h, 0.0, 1.0,  // bottom-left
                ];

                let vertex_bytes = bytemuck_cast_slice(&vertices);
                gl.buffer_sub_data_u8_slice(glow::ARRAY_BUFFER, 0, vertex_bytes);

                // Bind texture
                gl.active_texture(glow::TEXTURE0);
                gl.bind_texture(glow::TEXTURE_2D, Some(texture));

                // Draw
                gl.draw_elements(glow::TRIANGLES, 6, glow::UNSIGNED_SHORT, 0);
            }

            gl.bind_vertex_array(None);
            gl.disable(glow::BLEND);
        }
    }

    /// Delete texture
    #[allow(dead_code)]
    pub fn remove_texture(&mut self, gl: &glow::Context, id: u32) {
        if let Some(texture) = self.textures.remove(&id) {
            unsafe {
                gl.delete_texture(texture);
            }
            // Remove from LRU order
            if let Some(pos) = self.lru_order.iter().position(|&x| x == id) {
                self.lru_order.remove(pos);
            }
        }
    }

    /// Clear all cached textures (call after GPU state loss, e.g., suspend/resume)
    /// Images will be re-uploaded from terminal's image cache on next render
    pub fn invalidate_all(&mut self, gl: &glow::Context) {
        unsafe {
            for texture in self.textures.values() {
                gl.delete_texture(*texture);
            }
        }
        self.textures.clear();
        self.lru_order.clear();
        log::info!("ImageRenderer: all textures invalidated");
    }

    /// Release resources
    pub fn destroy(&self, gl: &glow::Context) {
        unsafe {
            for texture in self.textures.values() {
                gl.delete_texture(*texture);
            }
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
            .map_err(|e| anyhow!("Failed to create program (image): {}", e))?;

        gl.attach_shader(program, vs);
        gl.attach_shader(program, fs);
        gl.link_program(program);

        if !gl.get_program_link_status(program) {
            let log = gl.get_program_info_log(program);
            gl.delete_program(program);
            gl.delete_shader(vs);
            gl.delete_shader(fs);
            return Err(anyhow!("Shader link failed (image): {}", log));
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
            .map_err(|e| anyhow!("Failed to create shader (image): {}", e))?;

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
                "{} shader compile failed (image): {}",
                type_name,
                log
            ));
        }

        Ok(shader)
    }
}

/// Convert &[T] to &[u8]
fn bytemuck_cast_slice<T>(slice: &[T]) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(
            slice.as_ptr() as *const u8,
            slice.len() * std::mem::size_of::<T>(),
        )
    }
}
