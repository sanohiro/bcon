//! Font loading and glyph atlas management
//!
//! Handles:
//! - TTF/OTF font loading (fontdue / freetype)
//! - Text shaping (rustybuzz)
//! - Glyph atlas texture generation
//! - Unicode width calculation
//! - Color emoji support (CBDT/CBLC)
//! - LCD subpixel rendering (freetype)

pub mod atlas;
pub mod emoji;
pub mod fontconfig;
pub mod freetype;
pub mod lcd_atlas;
pub mod shaper;

// Re-export for convenience (allow dead_code since these are library exports)
#[allow(unused_imports)]
pub use fontconfig::{load_cjk_font_fc, load_emoji_font_fc, load_system_font_fc, resolve_font, FontFinder};
#[allow(unused_imports)]
pub use freetype::{FtFont, FtGlyph, LcdMode};
#[allow(unused_imports)]
pub use lcd_atlas::LcdGlyphAtlas;
