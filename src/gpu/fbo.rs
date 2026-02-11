//! Framebuffer Object (FBO) for dirty region rendering
//!
//! Caches rendered content and only updates dirty regions

use anyhow::{anyhow, Result};
use glow::HasContext;
use log::info;

/// Vertex shader for FBO blit (full screen quad)
const BLIT_VERTEX_SHADER: &str = r#"#version 300 es
precision mediump float;

// Full screen quad vertices (clip space)
const vec2 positions[4] = vec2[](
    vec2(-1.0, -1.0),
    vec2( 1.0, -1.0),
    vec2( 1.0,  1.0),
    vec2(-1.0,  1.0)
);

const vec2 texcoords[4] = vec2[](
    vec2(0.0, 0.0),
    vec2(1.0, 0.0),
    vec2(1.0, 1.0),
    vec2(0.0, 1.0)
);

out vec2 v_uv;

void main() {
    gl_Position = vec4(positions[gl_VertexID], 0.0, 1.0);
    v_uv = texcoords[gl_VertexID];
}
"#;

/// Fragment shader for FBO blit
const BLIT_FRAGMENT_SHADER: &str = r#"#version 300 es
precision mediump float;

in vec2 v_uv;
uniform sampler2D u_texture;
out vec4 frag_color;

void main() {
    frag_color = texture(u_texture, v_uv);
}
"#;

/// FBO for caching rendered content
pub struct Fbo {
    framebuffer: glow::Framebuffer,
    texture: glow::Texture,
    width: u32,
    height: u32,
    blit_program: glow::Program,
    blit_vao: glow::VertexArray,
    u_texture: glow::UniformLocation,
}

impl Fbo {
    /// Create FBO with specified size
    pub fn new(gl: &glow::Context, width: u32, height: u32) -> Result<Self> {
        unsafe {
            // Create texture for color attachment
            let texture = gl
                .create_texture()
                .map_err(|e| anyhow!("Failed to create FBO texture: {}", e))?;
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                width as i32,
                height as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                None,
            );
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
            gl.bind_texture(glow::TEXTURE_2D, None);

            // Create framebuffer
            let framebuffer = gl
                .create_framebuffer()
                .map_err(|e| anyhow!("Failed to create FBO: {}", e))?;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(texture),
                0,
            );

            // Check framebuffer status
            let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
            if status != glow::FRAMEBUFFER_COMPLETE {
                gl.delete_framebuffer(framebuffer);
                gl.delete_texture(texture);
                return Err(anyhow!("FBO incomplete: status={}", status));
            }
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);

            // Create blit shader program
            let blit_program = compile_program(gl, BLIT_VERTEX_SHADER, BLIT_FRAGMENT_SHADER)?;
            let u_texture = gl
                .get_uniform_location(blit_program, "u_texture")
                .ok_or_else(|| anyhow!("u_texture uniform not found"))?;

            // Create empty VAO for vertex-less rendering
            let blit_vao = gl
                .create_vertex_array()
                .map_err(|e| anyhow!("Failed to create blit VAO: {}", e))?;

            info!("FBO created: {}x{}", width, height);

            Ok(Self {
                framebuffer,
                texture,
                width,
                height,
                blit_program,
                blit_vao,
                u_texture,
            })
        }
    }

    /// Bind FBO for rendering
    pub fn bind(&self, gl: &glow::Context) {
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.framebuffer));
            gl.viewport(0, 0, self.width as i32, self.height as i32);
        }
    }

    /// Unbind FBO (return to default framebuffer)
    pub fn unbind(&self, gl: &glow::Context) {
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        }
    }

    /// Clear entire FBO
    pub fn clear(&self, gl: &glow::Context, r: f32, g: f32, b: f32, a: f32) {
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.framebuffer));
            gl.clear_color(r, g, b, a);
            gl.clear(glow::COLOR_BUFFER_BIT);
        }
    }

    /// Clear specific row range (uses scissor test)
    pub fn clear_rows(
        &self,
        gl: &glow::Context,
        start_row: usize,
        end_row: usize,
        cell_height: f32,
        margin_y: f32,
        r: f32,
        g: f32,
        b: f32,
    ) {
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.framebuffer));
            gl.enable(glow::SCISSOR_TEST);

            let y = (margin_y + start_row as f32 * cell_height) as i32;
            let height = ((end_row - start_row + 1) as f32 * cell_height) as i32;

            // OpenGL scissor Y is from bottom, need to flip
            let flipped_y = self.height as i32 - y - height;
            gl.scissor(0, flipped_y, self.width as i32, height);

            gl.clear_color(r, g, b, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);

            gl.disable(glow::SCISSOR_TEST);
        }
    }

    /// Blit FBO to screen (default framebuffer)
    pub fn blit_to_screen(&self, gl: &glow::Context, screen_width: u32, screen_height: u32) {
        unsafe {
            // Bind default framebuffer
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.viewport(0, 0, screen_width as i32, screen_height as i32);

            // Use blit shader
            gl.use_program(Some(self.blit_program));
            gl.uniform_1_i32(Some(&self.u_texture), 0);

            // Bind FBO texture
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.texture));

            // Draw full screen quad
            gl.bind_vertex_array(Some(self.blit_vao));
            gl.draw_arrays(glow::TRIANGLE_FAN, 0, 4);

            gl.bind_vertex_array(None);
            gl.bind_texture(glow::TEXTURE_2D, None);
        }
    }

    /// Resize FBO
    #[allow(dead_code)]
    pub fn resize(&mut self, gl: &glow::Context, width: u32, height: u32) -> Result<()> {
        if width == self.width && height == self.height {
            return Ok(());
        }

        unsafe {
            // Resize texture
            gl.bind_texture(glow::TEXTURE_2D, Some(self.texture));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                width as i32,
                height as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                None,
            );
            gl.bind_texture(glow::TEXTURE_2D, None);
        }

        self.width = width;
        self.height = height;
        info!("FBO resized: {}x{}", width, height);

        Ok(())
    }

    /// Get FBO size
    #[allow(dead_code)]
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Release resources
    pub fn destroy(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_framebuffer(self.framebuffer);
            gl.delete_texture(self.texture);
            gl.delete_program(self.blit_program);
            gl.delete_vertex_array(self.blit_vao);
        }
    }
}

/// Compile shader program
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
            .map_err(|e| anyhow!("Failed to create program (fbo): {}", e))?;

        gl.attach_shader(program, vs);
        gl.attach_shader(program, fs);
        gl.link_program(program);

        if !gl.get_program_link_status(program) {
            let log = gl.get_program_info_log(program);
            gl.delete_program(program);
            gl.delete_shader(vs);
            gl.delete_shader(fs);
            return Err(anyhow!("Shader link failed (fbo): {}", log));
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
            .map_err(|e| anyhow!("Failed to create shader (fbo): {}", e))?;

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
                "{} shader compile failed (fbo): {}",
                type_name,
                log
            ));
        }

        Ok(shader)
    }
}
