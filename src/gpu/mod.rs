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
pub mod renderer;
pub mod shader;
pub mod ui_renderer;

pub use context::{EglContext, GbmDevice, GbmSurface, GlEsVersion, GlRenderer};
pub use emoji_renderer::EmojiRenderer;
pub use fbo::Fbo;
pub use image_renderer::ImageRenderer;
pub use lcd_renderer::LcdTextRenderer;
pub use renderer::CurlyRenderer;
pub use ui_renderer::UiRenderer;
