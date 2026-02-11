//! UI drawing renderer
//!
//! Draw UI elements like candidate windows
//! using SDF rounded rectangle shader

use anyhow::{anyhow, Result};
use glow::HasContext;
use log::info;

use crate::gpu::shader::{self, UiShader};

/// Per-vertex data: pos(2) + center(2) + half_size(2) + radius(1) + color(4) = 11 floats
const VERTEX_FLOATS: usize = 11;
/// 1 rectangle = 4 vertices
const VERTICES_PER_RECT: usize = 4;
/// 1 rectangle = 6 indices (2 triangles)
const INDICES_PER_RECT: usize = 6;
/// Maximum rectangles per batch
const MAX_RECTS: usize = 256;

/// UI renderer (SDF rounded rectangle drawing)
pub struct UiRenderer {
    shader: UiShader,
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    ebo: glow::Buffer,
    /// Vertex buffer (CPU side)
    vertices: Vec<f32>,
    /// Current number of rectangles in buffer
    rect_count: usize,
}

impl UiRenderer {
    /// Create UI renderer
    pub fn new(gl: &glow::Context) -> Result<Self> {
        let shader = UiShader::new(gl)?;

        unsafe {
            // VAO
            let vao = gl
                .create_vertex_array()
                .map_err(|e| anyhow!("Failed to create UI VAO: {}", e))?;
            gl.bind_vertex_array(Some(vao));

            // VBO
            let vbo = gl
                .create_buffer()
                .map_err(|e| anyhow!("Failed to create UI VBO: {}", e))?;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            let vbo_size = MAX_RECTS * VERTICES_PER_RECT * VERTEX_FLOATS * 4;
            gl.buffer_data_size(glow::ARRAY_BUFFER, vbo_size as i32, glow::DYNAMIC_DRAW);

            // EBO (index buffer)
            let ebo = gl
                .create_buffer()
                .map_err(|e| anyhow!("Failed to create UI EBO: {}", e))?;
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(ebo));

            // Pre-generate index data
            let mut indices: Vec<u16> = Vec::with_capacity(MAX_RECTS * INDICES_PER_RECT);
            for i in 0..MAX_RECTS as u16 {
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

            // Set vertex attributes
            let stride = (VERTEX_FLOATS * 4) as i32; // 44 bytes

            // a_pos: location=0, vec2 (offset 0)
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, stride, 0);

            // a_center: location=1, vec2 (offset 8)
            gl.enable_vertex_attrib_array(1);
            gl.vertex_attrib_pointer_f32(1, 2, glow::FLOAT, false, stride, 8);

            // a_half_size: location=2, vec2 (offset 16)
            gl.enable_vertex_attrib_array(2);
            gl.vertex_attrib_pointer_f32(2, 2, glow::FLOAT, false, stride, 16);

            // a_radius: location=3, float (offset 24)
            gl.enable_vertex_attrib_array(3);
            gl.vertex_attrib_pointer_f32(3, 1, glow::FLOAT, false, stride, 24);

            // a_color: location=4, vec4 (offset 28)
            gl.enable_vertex_attrib_array(4);
            gl.vertex_attrib_pointer_f32(4, 4, glow::FLOAT, false, stride, 28);

            gl.bind_vertex_array(None);

            info!("UI renderer initialized");

            Ok(Self {
                shader,
                vao,
                vbo,
                ebo,
                vertices: Vec::with_capacity(MAX_RECTS * VERTICES_PER_RECT * VERTEX_FLOATS),
                rect_count: 0,
            })
        }
    }

    /// Clear draw buffer
    pub fn begin(&mut self) {
        self.vertices.clear();
        self.rect_count = 0;
    }

    /// Add rounded rectangle to draw buffer
    pub fn push_rounded_rect(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        color: [f32; 4],
    ) {
        if self.rect_count >= MAX_RECTS {
            return;
        }

        let cx = x + w / 2.0;
        let cy = y + h / 2.0;
        let hw = w / 2.0;
        let hh = h / 2.0;
        let [r, g, b, a] = color;

        // 4 vertices in one extend (reduces function call overhead)
        // Each vertex: pos(2) + center(2) + half_size(2) + radius(1) + color(4) = 11 floats
        #[rustfmt::skip]
        self.vertices.extend_from_slice(&[
            x,     y,     cx, cy, hw, hh, radius, r, g, b, a,  // top-left
            x + w, y,     cx, cy, hw, hh, radius, r, g, b, a,  // top-right
            x + w, y + h, cx, cy, hw, hh, radius, r, g, b, a,  // bottom-right
            x,     y + h, cx, cy, hw, hh, radius, r, g, b, a,  // bottom-left
        ]);

        self.rect_count += 1;
    }

    /// Add rounded rectangle with drop shadow to draw buffer
    ///
    /// Shadow is added first (lower layer) then main rectangle.
    pub fn push_shadow_rounded_rect(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        color: [f32; 4],
        shadow_offset: f32,
        shadow_color: [f32; 4],
    ) {
        // Shadow (slightly larger, offset)
        self.push_rounded_rect(
            x + shadow_offset,
            y + shadow_offset,
            w + shadow_offset,
            h + shadow_offset,
            radius + 2.0,
            shadow_color,
        );
        // Main rectangle
        self.push_rounded_rect(x, y, w, h, radius, color);
    }

    /// Upload buffer contents to GPU and draw
    pub fn flush(&self, gl: &glow::Context, width: u32, height: u32) {
        if self.rect_count == 0 {
            return;
        }

        unsafe {
            // Enable blending
            gl.enable(glow::BLEND);
            gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);

            // Bind shader
            self.shader.bind(gl);

            // Set orthographic projection matrix
            let projection = shader::ortho_projection(width as f32, height as f32);
            self.shader.set_projection(gl, &projection);

            // Bind VAO and upload vertex data
            gl.bind_vertex_array(Some(self.vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));

            let vertex_bytes: &[u8] = bytemuck_cast_slice(&self.vertices);
            gl.buffer_sub_data_u8_slice(glow::ARRAY_BUFFER, 0, vertex_bytes);

            // Draw
            gl.draw_elements(
                glow::TRIANGLES,
                (self.rect_count * INDICES_PER_RECT) as i32,
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

/// Convert &[T] to &[u8]
fn bytemuck_cast_slice<T>(slice: &[T]) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(
            slice.as_ptr() as *const u8,
            slice.len() * std::mem::size_of::<T>(),
        )
    }
}
