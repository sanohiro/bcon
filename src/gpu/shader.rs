//! Shader management
//!
//! GLSL ES 3.00 shader compilation and linking

use anyhow::{anyhow, Result};
use glow::HasContext;
use log::info;

/// Text rendering vertex shader (GLSL ES 3.00)
///
/// Input:
///   a_pos: Vertex position (pixels)
///   a_uv:  Texture coordinates
///   a_color: Text color (RGBA)
/// Uniform:
///   u_projection: Orthographic projection matrix
const TEXT_VERTEX_SHADER: &str = r#"#version 300 es
precision highp float;

layout(location = 0) in vec2 a_pos;
layout(location = 1) in vec2 a_uv;
layout(location = 2) in vec4 a_color;

uniform mat4 u_projection;

out vec2 v_uv;
out vec4 v_color;

void main() {
    gl_Position = u_projection * vec4(a_pos, 0.0, 1.0);
    v_uv = a_uv;
    v_color = a_color;
}
"#;

/// Text rendering fragment shader (grayscale)
///
/// Sample glyph atlas R(alpha) channel and perform
/// high-quality text compositing with gamma correction and premultiplied alpha
const TEXT_FRAGMENT_SHADER: &str = r#"#version 300 es
precision highp float;

in vec2 v_uv;
in vec4 v_color;

uniform sampler2D u_atlas;
uniform float u_gamma;  // Gamma correction (1.0 = none, 1.2-1.8 for weight adjustment)

out vec4 frag_color;

void main() {
    float alpha = texture(u_atlas, v_uv).r;

    // Gamma correction: adjust text weight
    // gamma < 1.0 = thicker, gamma > 1.0 = thinner
    alpha = pow(alpha, u_gamma);

    float final_alpha = v_color.a * alpha;

    // Premultiplied alpha: prevent muddy text edges
    frag_color = vec4(v_color.rgb * final_alpha, final_alpha);
}
"#;

/// LCD subpixel rendering fragment shader
///
/// Sample RGB texture and apply independent alpha values
/// to each subpixel (ClearType-style)
#[allow(dead_code)]
const TEXT_FRAGMENT_SHADER_LCD: &str = r#"#version 300 es
precision highp float;

in vec2 v_uv;
in vec4 v_color;

uniform sampler2D u_atlas;
uniform float u_gamma;

out vec4 frag_color;

void main() {
    // Each RGB channel is subpixel alpha value
    vec3 coverage = texture(u_atlas, v_uv).rgb;

    // Gamma correction
    coverage = pow(coverage, vec3(u_gamma));

    // Multiply text color by coverage
    vec3 rgb = v_color.rgb * coverage;

    // Overall alpha is max coverage (for background compositing)
    float alpha = max(max(coverage.r, coverage.g), coverage.b) * v_color.a;

    // Premultiplied alpha
    frag_color = vec4(rgb * v_color.a, alpha);
}
"#;

/// Linear color space compositing fragment shader
///
/// Convert sRGB to linear, composite in linear space, then convert back to sRGB
/// This makes text appear with natural weight on dark backgrounds
#[allow(dead_code)]
const TEXT_FRAGMENT_SHADER_LINEAR: &str = r#"#version 300 es
precision highp float;

in vec2 v_uv;
in vec4 v_color;

uniform sampler2D u_atlas;
uniform float u_gamma;

out vec4 frag_color;

// sRGB to linear conversion
vec3 srgb_to_linear(vec3 srgb) {
    // Simplified: approximate with pow(x, 2.2)
    // Precise: x <= 0.04045 ? x/12.92 : pow((x+0.055)/1.055, 2.4)
    return pow(srgb, vec3(2.2));
}

// Linear to sRGB conversion
vec3 linear_to_srgb(vec3 linear) {
    // Simplified: approximate with pow(x, 1/2.2)
    return pow(linear, vec3(1.0 / 2.2));
}

void main() {
    float alpha = texture(u_atlas, v_uv).r;

    // Text weight adjustment (gamma in linear space)
    alpha = pow(alpha, u_gamma);

    float final_alpha = v_color.a * alpha;

    // Convert input color to linear space
    vec3 linear_color = srgb_to_linear(v_color.rgb);

    // Apply premultiplied alpha in linear space
    vec3 premult_linear = linear_color * final_alpha;

    // Convert back to sRGB (safe even if framebuffer is sRGB)
    vec3 premult_srgb = linear_to_srgb(premult_linear);

    frag_color = vec4(premult_srgb, final_alpha);
}
"#;

/// Compiled shader program
pub struct TextShader {
    program: glow::Program,
    pub u_projection: glow::UniformLocation,
    pub u_atlas: glow::UniformLocation,
    pub u_gamma: glow::UniformLocation,
}

impl TextShader {
    /// Compile and link text rendering shader (grayscale version)
    pub fn new(gl: &glow::Context) -> Result<Self> {
        Self::new_internal(gl, TEXT_FRAGMENT_SHADER, "grayscale")
    }

    /// Compile and link LCD subpixel rendering shader
    #[allow(dead_code)]
    pub fn new_lcd(gl: &glow::Context) -> Result<Self> {
        Self::new_internal(gl, TEXT_FRAGMENT_SHADER_LCD, "LCD subpixel")
    }

    /// Compile and link linear color space compositing shader
    ///
    /// Convert sRGB to linear, composite in linear space, then convert back to sRGB
    /// Text appears with natural weight on dark backgrounds
    #[allow(dead_code)]
    pub fn new_linear(gl: &glow::Context) -> Result<Self> {
        Self::new_internal(gl, TEXT_FRAGMENT_SHADER_LINEAR, "linear color space")
    }

    fn new_internal(gl: &glow::Context, fragment_shader: &str, mode_name: &str) -> Result<Self> {
        let program = compile_program(gl, TEXT_VERTEX_SHADER, fragment_shader)?;

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

        info!("Text shader compiled ({})", mode_name);
        Ok(Self {
            program,
            u_projection,
            u_atlas,
            u_gamma,
        })
    }

    /// Activate the shader
    pub fn bind(&self, gl: &glow::Context) {
        unsafe {
            gl.use_program(Some(self.program));
        }
    }

    /// Set orthographic projection matrix
    pub fn set_projection(&self, gl: &glow::Context, matrix: &[f32; 16]) {
        unsafe {
            gl.uniform_matrix_4_f32_slice(Some(&self.u_projection), false, matrix);
        }
    }

    /// Set atlas texture unit
    pub fn set_atlas_unit(&self, gl: &glow::Context, unit: i32) {
        unsafe {
            gl.uniform_1_i32(Some(&self.u_atlas), unit);
        }
    }

    /// Set gamma correction value
    ///
    /// gamma < 1.0 makes text thicker, gamma > 1.0 makes it thinner
    /// Recommended: 1.0-1.4 (1.2 is default)
    pub fn set_gamma(&self, gl: &glow::Context, gamma: f32) {
        unsafe {
            gl.uniform_1_f32(Some(&self.u_gamma), gamma);
        }
    }

    #[allow(dead_code)]
    pub fn program(&self) -> glow::Program {
        self.program
    }

    /// Release resources
    pub fn destroy(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_program(self.program);
        }
    }
}

// === Curly underline shader (SDF + smoothstep) ===

/// Curly underline vertex shader
const CURLY_VERTEX_SHADER: &str = r#"#version 300 es
precision highp float;

layout(location = 0) in vec2 a_pos;
layout(location = 1) in vec4 a_rect;   // x, y, width, height (run rectangle)
layout(location = 2) in vec4 a_color;
layout(location = 3) in vec4 a_params; // amplitude, wavelength, thickness, base_y

uniform mat4 u_projection;

out vec2 v_pos;
out vec4 v_rect;
out vec4 v_color;
out vec4 v_params;

void main() {
    gl_Position = u_projection * vec4(a_pos, 0.0, 1.0);
    v_pos = a_pos;
    v_rect = a_rect;
    v_color = a_color;
    v_params = a_params;
}
"#;

/// Curly underline fragment shader
/// Calculate distance to sine wave using SDF, anti-alias with smoothstep
const CURLY_FRAGMENT_SHADER: &str = r#"#version 300 es
precision highp float;

in vec2 v_pos;
in vec4 v_rect;
in vec4 v_color;
in vec4 v_params;

out vec4 frag_color;

void main() {
    float amplitude = v_params.x;
    float wavelength = v_params.y;
    float thickness = v_params.z;
    float base_y = v_params.w;

    // Use run-based phase (continuous across cells)
    float phase = ((v_pos.x - v_rect.x) / wavelength) * 6.28318530718; // 2*PI
    float wave_y = base_y - amplitude * sin(phase);

    float dist = abs(v_pos.y - wave_y);

    // Treat thickness as line width, reduce AA for less width variation
    float halfThick = thickness * 0.5;
    float aa = max(fwidth(dist), 0.35);
    float alpha = 1.0 - smoothstep(halfThick - aa, halfThick + aa, dist);

    frag_color = vec4(v_color.rgb, v_color.a * alpha);
}
"#;

/// Curly underline shader
pub struct CurlyShader {
    program: glow::Program,
    pub u_projection: glow::UniformLocation,
}

impl CurlyShader {
    pub fn new(gl: &glow::Context) -> Result<Self> {
        let program = compile_program(gl, CURLY_VERTEX_SHADER, CURLY_FRAGMENT_SHADER)?;

        let u_projection = unsafe {
            gl.get_uniform_location(program, "u_projection")
                .ok_or_else(|| anyhow!("u_projection uniform not found (Curly)"))?
        };

        info!("Curly underline shader compiled");
        Ok(Self {
            program,
            u_projection,
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

    pub fn destroy(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_program(self.program);
        }
    }
}

// === UI shader (SDF rounded rectangle) ===

/// UI drawing vertex shader (GLSL ES 3.00)
///
/// Receive rectangle center, radius, corner radius via vertex attributes,
/// pass SDF calculation parameters to fragment shader.
const UI_VERTEX_SHADER: &str = r#"#version 300 es
precision mediump float;

layout(location = 0) in vec2 a_pos;
layout(location = 1) in vec2 a_center;
layout(location = 2) in vec2 a_half_size;
layout(location = 3) in float a_radius;
layout(location = 4) in vec4 a_color;

uniform mat4 u_projection;

out vec2 v_pos;
out vec2 v_center;
out vec2 v_half_size;
out float v_radius;
out vec4 v_color;

void main() {
    gl_Position = u_projection * vec4(a_pos, 0.0, 1.0);
    v_pos = a_pos;
    v_center = a_center;
    v_half_size = a_half_size;
    v_radius = a_radius;
    v_color = a_color;
}
"#;

/// UI drawing fragment shader
///
/// Calculate rounded rectangle using SDF, anti-alias with smoothstep.
const UI_FRAGMENT_SHADER: &str = r#"#version 300 es
precision mediump float;

in vec2 v_pos;
in vec2 v_center;
in vec2 v_half_size;
in float v_radius;
in vec4 v_color;

out vec4 frag_color;

float roundedRectSDF(vec2 p, vec2 halfSize, float radius) {
    vec2 d = abs(p) - halfSize + radius;
    return min(max(d.x, d.y), 0.0) + length(max(d, 0.0)) - radius;
}

void main() {
    float dist = roundedRectSDF(v_pos - v_center, v_half_size, v_radius);
    float aa = 1.0 - smoothstep(-1.0, 0.5, dist);
    frag_color = vec4(v_color.rgb, v_color.a * aa);
}
"#;

/// Compiled UI shader program (SDF rounded rectangle)
pub struct UiShader {
    program: glow::Program,
    pub u_projection: glow::UniformLocation,
}

impl UiShader {
    /// Compile and link UI shader
    pub fn new(gl: &glow::Context) -> Result<Self> {
        let program = compile_program(gl, UI_VERTEX_SHADER, UI_FRAGMENT_SHADER)?;

        let u_projection = unsafe {
            gl.get_uniform_location(program, "u_projection")
                .ok_or_else(|| anyhow!("u_projection uniform not found (UI)"))?
        };

        info!("UI shader compiled");
        Ok(Self {
            program,
            u_projection,
        })
    }

    /// Activate shader
    pub fn bind(&self, gl: &glow::Context) {
        unsafe {
            gl.use_program(Some(self.program));
        }
    }

    /// Set orthographic projection matrix
    pub fn set_projection(&self, gl: &glow::Context, matrix: &[f32; 16]) {
        unsafe {
            gl.uniform_matrix_4_f32_slice(Some(&self.u_projection), false, matrix);
        }
    }

    /// Release resources
    pub fn destroy(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_program(self.program);
        }
    }
}

/// Generate orthographic projection matrix (top-left origin)
///
/// Map pixel coordinates (0,0)-(width,height)
/// to NDC (-1,-1)-(1,1)
pub fn ortho_projection(width: f32, height: f32) -> [f32; 16] {
    let l = 0.0_f32;
    let r = width;
    let t = 0.0_f32; // top
    let b = height; // bottom
    let n = -1.0_f32;
    let f = 1.0_f32;

    // Column-major (OpenGL convention)
    [
        2.0 / (r - l),
        0.0,
        0.0,
        0.0,
        0.0,
        2.0 / (t - b),
        0.0,
        0.0,
        0.0,
        0.0,
        -2.0 / (f - n),
        0.0,
        -(r + l) / (r - l),
        -(t + b) / (t - b),
        -(f + n) / (f - n),
        1.0,
    ]
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

        // Shader objects no longer needed after linking
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
