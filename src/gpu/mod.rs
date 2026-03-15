//! GPU rendering with OpenGL ES
//!
//! Handles:
//! - GBM device/surface creation
//! - EGL context creation (GBM platform)
//! - OpenGL ES rendering

pub mod context;
pub mod emoji_renderer;
pub mod fbo;
pub mod image_renderer;
pub mod lcd_renderer;
pub mod lcd_renderer_instanced;
pub mod renderer;
pub mod shader;
pub mod ui_renderer;

pub use context::{EglContext, GbmDevice, GbmSurface, GlRenderer};
pub use emoji_renderer::EmojiRenderer;
pub use fbo::Fbo;
pub use image_renderer::{image_key, ImageRenderer};
#[allow(unused_imports)]
pub use lcd_renderer::LcdTextRenderer;
#[allow(unused_imports)]
pub use lcd_renderer_instanced::LcdTextRendererInstanced;
pub use renderer::CurlyRenderer;
pub use ui_renderer::UiRenderer;

// ============================================================================
// Common GPU Constants
// ============================================================================

/// Vertices per quad (two triangles sharing two vertices)
pub const VERTICES_PER_QUAD: usize = 4;

/// Indices per quad (two triangles = 6 indices)
pub const INDICES_PER_QUAD: usize = 6;

/// Maximum quads for text renderers (supports 4K: ~240x135 cells)
pub const MAX_TEXT_QUADS: usize = 32768;

/// Cast a slice of any type to a byte slice (for OpenGL buffer uploads).
pub fn bytemuck_cast_slice<T>(slice: &[T]) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(
            slice.as_ptr() as *const u8,
            slice.len() * std::mem::size_of::<T>(),
        )
    }
}
