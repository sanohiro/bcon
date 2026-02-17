//! fontconfig integration
//!
//! Search and select system fonts

use anyhow::{anyhow, Result};
use fontconfig::Fontconfig;
use log::{info, warn};
use std::path::PathBuf;

/// Font search result
#[derive(Debug, Clone)]
pub struct FontMatch {
    /// Font file path
    pub path: PathBuf,
    /// Font name
    pub family: String,
}

/// Search fonts using fontconfig
pub struct FontFinder {
    fc: Fontconfig,
}

impl FontFinder {
    /// Initialize FontFinder
    pub fn new() -> Result<Self> {
        let fc = Fontconfig::new().ok_or_else(|| anyhow!("fontconfig initialization failed"))?;
        info!("fontconfig initialized");
        Ok(Self { fc })
    }

    /// Search by font name
    pub fn find_font(&self, family: &str) -> Option<FontMatch> {
        // Use fontconfig's find method
        if let Some(font) = self.fc.find(family, None) {
            return Some(FontMatch {
                path: font.path,
                family: font.name,
            });
        }
        None
    }

    /// Search for monospace font
    pub fn find_monospace(&self) -> Option<FontMatch> {
        // Fallback candidates
        let fallbacks = [
            "DejaVu Sans Mono",
            "Liberation Mono",
            "Noto Sans Mono",
            "Source Code Pro",
            "Inconsolata",
            "Courier New",
            "monospace",
        ];

        for name in fallbacks {
            if let Some(m) = self.find_font(name) {
                return Some(m);
            }
        }

        warn!("Monospace font not found");
        None
    }

    /// Search for CJK font
    pub fn find_cjk(&self) -> Option<FontMatch> {
        let candidates = [
            "Noto Sans CJK JP",
            "Noto Sans CJK",
            "Source Han Sans",
            "IPA Gothic",
            "IPAGothic",
            "VL Gothic",
            "Takao Gothic",
        ];

        for name in candidates {
            if let Some(m) = self.find_font(name) {
                return Some(m);
            }
        }

        warn!("CJK font not found");
        None
    }

    /// Search for color emoji font
    #[allow(dead_code)]
    pub fn find_emoji(&self) -> Option<FontMatch> {
        let candidates = [
            "Noto Color Emoji",
            "Apple Color Emoji",
            "Twemoji",
            "EmojiOne",
        ];

        for name in candidates {
            if let Some(m) = self.find_font(name) {
                return Some(m);
            }
        }

        warn!("Color emoji font not found");
        None
    }

    /// Search for Nerd Font (symbol/icon font)
    pub fn find_nerd_font(&self) -> Option<FontMatch> {
        let candidates = [
            // Nerd Font variants (most common)
            "Hack Nerd Font Mono",
            "Hack Nerd Font",
            "HackNerdFontMono",
            "HackNerdFont",
            "FiraCode Nerd Font Mono",
            "FiraCode Nerd Font",
            "FiraCodeNerdFontMono",
            "FiraCodeNerdFont",
            "JetBrainsMono Nerd Font Mono",
            "JetBrainsMono Nerd Font",
            "JetBrainsMonoNerdFontMono",
            "JetBrainsMonoNerdFont",
            "DejaVuSansMono Nerd Font Mono",
            "DejaVuSansMono Nerd Font",
            "DejaVuSansM Nerd Font Mono",
            "DejaVuSansM Nerd Font",
            "SauceCodePro Nerd Font Mono",
            "SauceCodePro Nerd Font",
            "Symbols Nerd Font Mono",
            "Symbols Nerd Font",
        ];

        for name in candidates {
            if let Some(m) = self.find_font(name) {
                return Some(m);
            }
        }

        None
    }
}

/// Load font file
pub fn load_font_file(path: &std::path::Path) -> Result<Vec<u8>> {
    std::fs::read(path).map_err(|e| anyhow!("Failed to read font file: {} ({})", path.display(), e))
}

/// Search and load system font using fontconfig
pub fn load_system_font_fc() -> Result<Vec<u8>> {
    let finder = FontFinder::new()?;

    if let Some(font_match) = finder.find_monospace() {
        info!(
            "System font (fontconfig): {} ({})",
            font_match.family,
            font_match.path.display()
        );
        return load_font_file(&font_match.path);
    }

    Err(anyhow!("Monospace font not found via fontconfig"))
}

/// Search and load CJK font using fontconfig
pub fn load_cjk_font_fc() -> Option<Vec<u8>> {
    let finder = match FontFinder::new() {
        Ok(f) => f,
        Err(e) => {
            warn!("fontconfig initialization failed: {:?}", e);
            return None;
        }
    };

    if let Some(font_match) = finder.find_cjk() {
        info!(
            "CJK font (fontconfig): {} ({})",
            font_match.family,
            font_match.path.display()
        );
        return load_font_file(&font_match.path).ok();
    }

    None
}

/// Search and load emoji font using fontconfig
#[allow(dead_code)]
pub fn load_emoji_font_fc() -> Option<Vec<u8>> {
    let finder = match FontFinder::new() {
        Ok(f) => f,
        Err(e) => {
            warn!("fontconfig initialization failed: {:?}", e);
            return None;
        }
    };

    if let Some(font_match) = finder.find_emoji() {
        info!(
            "Emoji font (fontconfig): {} ({})",
            font_match.family,
            font_match.path.display()
        );
        return load_font_file(&font_match.path).ok();
    }

    None
}

/// Search and load Nerd Font (symbol/icon font) using fontconfig
pub fn load_nerd_font_fc() -> Option<Vec<u8>> {
    let finder = match FontFinder::new() {
        Ok(f) => f,
        Err(e) => {
            warn!("fontconfig initialization failed: {:?}", e);
            return None;
        }
    };

    if let Some(font_match) = finder.find_nerd_font() {
        info!(
            "Nerd Font (fontconfig): {} ({})",
            font_match.family,
            font_match.path.display()
        );
        return load_font_file(&font_match.path).ok();
    }

    None
}

/// Find Nerd Font path (for config generation)
#[allow(dead_code)]
pub fn find_nerd_font_path() -> Option<String> {
    let finder = FontFinder::new().ok()?;
    let font_match = finder.find_nerd_font()?;
    Some(font_match.path.to_string_lossy().to_string())
}
