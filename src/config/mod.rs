//! Configuration file management
//!
//! Loads TOML configuration files and provides application settings.
//! Default config path: ~/.config/bcon/config.toml

use anyhow::{Context, Result};
use log::info;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[cfg(target_os = "linux")]
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
#[cfg(target_os = "linux")]
use std::path::Path;
#[cfg(target_os = "linux")]
use std::sync::mpsc;

/// Application settings
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Font settings
    pub font: FontConfig,
    /// Path settings
    pub paths: PathConfig,
    /// Keybind settings
    pub keybinds: KeybindConfig,
    /// Appearance settings
    pub appearance: AppearanceConfig,
    /// Terminal settings
    pub terminal: TerminalConfig,
}

/// Font settings
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FontConfig {
    /// Main font path (searches system fonts if empty)
    pub main: String,
    /// CJK font path (searches system fonts if empty)
    pub cjk: String,
    /// Emoji font path (searches system fonts if empty)
    pub emoji: String,
    /// Font size
    pub size: f32,
    /// Rendering mode: "grayscale" (default) or "lcd"
    /// LCD mode provides high-quality subpixel rendering,
    /// but is not suitable for rotated/scaled displays
    pub render_mode: String,
    /// LCD filter: "none" | "default" | "light" | "legacy" | "custom"
    /// Adjusts sharpness vs fringe tradeoff
    pub lcd_filter: String,
    /// LCD custom weights (5-tap FIR filter)
    /// Only used when lcd_filter = "custom"
    /// Example: [0x10, 0x40, 0x60, 0x40, 0x10] (balanced)
    pub lcd_weights: Option<[u8; 5]>,
    /// LCD subpixel order: "rgb" | "bgr" | "vrgb" | "vbgr" | "auto"
    /// Match panel's subpixel arrangement (mismatch causes color fringe)
    pub lcd_subpixel: String,
    /// LCD gamma correction (1.0 = standard, 1.1-1.25 = thinner/tighter)
    /// Higher values make text thinner/sharper
    pub lcd_gamma: f32,
    /// LCD stem darkening (0.0 = disabled, 0.05-0.15 = enabled)
    /// Makes thin strokes thicker. 0.0 recommended (iTerm2-like sharpness)
    pub lcd_stem_darkening: f32,
    /// Subpixel phase rendering (true = enabled)
    /// Places characters with 1/3 pixel precision for more natural spacing
    /// Increases memory usage but significantly improves quality
    pub lcd_subpixel_positioning: bool,
    /// Hinting mode: "normal" | "light" | "none"
    /// normal: crisp, slightly bold (Windows-like)
    /// light: natural curves, slightly thin (recommended)
    /// none: no hinting (macOS-like, most natural)
    pub lcd_hinting: String,
    /// Contrast enhancement (1.0 = standard, 1.1-1.3 = sharp)
    /// Makes edges crisper. Too high looks unnatural
    pub lcd_contrast: f32,
    /// Color fringe reduction (0.0 = disabled, 0.1-0.3 = enabled)
    /// Useful when rainbow colors are visible at text edges
    pub lcd_fringe_reduction: f32,
}

/// Path settings
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PathConfig {
    /// Screenshot save directory
    pub screenshot_dir: String,
    /// Clipboard file path
    pub clipboard_file: String,
}

/// Appearance settings
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppearanceConfig {
    /// Background color (RRGGBB)
    pub background: String,
    /// Foreground color (RRGGBB)
    pub foreground: String,
    /// Cursor color (RRGGBB)
    pub cursor: String,
    /// Selection color (RRGGBB)
    pub selection: String,
    /// Cursor opacity (0.0-1.0)
    pub cursor_opacity: f32,
}

/// Terminal settings
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalConfig {
    /// Scrollback line count
    pub scrollback_lines: usize,
    /// Bell type ("visual", "none")
    pub bell: String,
    /// TERM environment variable
    pub term_env: String,
    /// List of apps that auto-disable IME
    /// When foreground process name is in this list, IME is automatically disabled
    pub ime_disabled_apps: Vec<String>,
}

/// Keybind settings
/// Each keybind can be a single key ("ctrl+c") or multiple keys (["ctrl+c", "alt+w"])
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct KeybindConfig {
    /// Copy (default: "ctrl+shift+c")
    #[serde(deserialize_with = "deserialize_keybind")]
    pub copy: Vec<String>,
    /// Paste (default: "ctrl+shift+v")
    #[serde(deserialize_with = "deserialize_keybind")]
    pub paste: Vec<String>,
    /// Screenshot (default: "ctrl+shift+s")
    #[serde(deserialize_with = "deserialize_keybind")]
    pub screenshot: Vec<String>,
    /// Search (default: "ctrl+shift+f")
    #[serde(deserialize_with = "deserialize_keybind")]
    pub search: Vec<String>,
    /// Copy mode (default: "ctrl+shift+space")
    #[serde(deserialize_with = "deserialize_keybind")]
    pub copy_mode: Vec<String>,
    /// Font increase (default: "ctrl+plus")
    #[serde(deserialize_with = "deserialize_keybind")]
    pub font_increase: Vec<String>,
    /// Font decrease (default: "ctrl+minus")
    #[serde(deserialize_with = "deserialize_keybind")]
    pub font_decrease: Vec<String>,
    /// Font reset (default: "ctrl+0")
    #[serde(deserialize_with = "deserialize_keybind")]
    pub font_reset: Vec<String>,
    /// Scroll up (default: "shift+pageup")
    #[serde(deserialize_with = "deserialize_keybind")]
    pub scroll_up: Vec<String>,
    /// Scroll down (default: "shift+pagedown")
    #[serde(deserialize_with = "deserialize_keybind")]
    pub scroll_down: Vec<String>,
    /// IME toggle (default: "ctrl+shift+j") - enable/disable IME at bcon level
    #[serde(deserialize_with = "deserialize_keybind")]
    pub ime_toggle: Vec<String>,
}

/// Keybind deserializer: accepts string or array
fn deserialize_keybind<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{self, Visitor};

    struct KeybindVisitor;

    impl<'de> Visitor<'de> for KeybindVisitor {
        type Value = Vec<String>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a string or array of strings")
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(vec![value.to_string()])
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: de::SeqAccess<'de>,
        {
            let mut keys = Vec::new();
            while let Some(key) = seq.next_element::<String>()? {
                keys.push(key);
            }
            Ok(keys)
        }
    }

    deserializer.deserialize_any(KeybindVisitor)
}

impl Default for Config {
    fn default() -> Self {
        Self {
            font: FontConfig::default(),
            paths: PathConfig::default(),
            keybinds: KeybindConfig::default(),
            appearance: AppearanceConfig::default(),
            terminal: TerminalConfig::default(),
        }
    }
}

impl Default for FontConfig {
    fn default() -> Self {
        Self {
            main: String::new(),
            cjk: String::new(),
            emoji: String::new(),
            size: 18.0, // Recommended for console readability
            render_mode: "lcd".to_string(), // LCD subpixel rendering (high quality)
            lcd_filter: "default".to_string(), // Sharp (less blur than light)
            lcd_weights: None,
            lcd_subpixel: "rgb".to_string(), // For common panels
            lcd_gamma: 1.15,          // Thinner/tighter look (iTerm2-like)
            lcd_stem_darkening: 0.0,  // Disabled (sharpness priority)
            lcd_subpixel_positioning: true, // 1/3px phase rendering (highest quality)
            lcd_hinting: "light".to_string(), // Natural curves, thin (recommended)
            lcd_contrast: 1.15,       // Contrast enhancement
            lcd_fringe_reduction: 0.1, // Light fringe reduction
        }
    }
}

impl Default for PathConfig {
    fn default() -> Self {
        // Clipboard path is unique per instance
        let pid = std::process::id();
        let clipboard_file = if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
            format!("{}/bcon_clipboard_{}", runtime_dir, pid)
        } else {
            format!("/tmp/bcon_clipboard_{}", pid)
        };

        Self {
            // Use ~ to be expanded at runtime based on the logged-in user
            screenshot_dir: "~".to_string(),
            clipboard_file,
        }
    }
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            background: "000000".to_string(),
            foreground: "ffffff".to_string(),
            cursor: "ffffff".to_string(),
            selection: "4d7aa8".to_string(),
            cursor_opacity: 0.5,
        }
    }
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            scrollback_lines: 10000,
            bell: "visual".to_string(),
            term_env: "xterm-256color".to_string(),
            // Empty by default - uncomment in config for CJK/IME users
            ime_disabled_apps: vec![],
        }
    }
}

impl Default for KeybindConfig {
    fn default() -> Self {
        Self::default_preset()
    }
}

impl KeybindConfig {
    /// Default preset (standard terminal keybindings)
    pub fn default_preset() -> Self {
        Self {
            copy: vec!["ctrl+shift+c".to_string()],
            paste: vec!["ctrl+shift+v".to_string()],
            screenshot: vec!["ctrl+shift+s".to_string(), "printscreen".to_string()],
            search: vec!["ctrl+shift+f".to_string()],
            copy_mode: vec!["ctrl+shift+space".to_string()],
            font_increase: vec!["ctrl+plus".to_string()],
            font_decrease: vec!["ctrl+minus".to_string()],
            font_reset: vec!["ctrl+0".to_string()],
            scroll_up: vec!["shift+pageup".to_string()],
            scroll_down: vec!["shift+pagedown".to_string()],
            ime_toggle: vec!["ctrl+shift+j".to_string()],
        }
    }

    /// Emacs preset (Emacs-like scroll)
    /// Ctrl+Shift based for safety, only scroll is Emacs-like
    /// (Ctrl+S/Y/V/Space conflicts with bash/zsh/tmux, so avoided)
    pub fn emacs_preset() -> Self {
        Self {
            copy: vec!["ctrl+shift+c".to_string()],
            paste: vec!["ctrl+shift+v".to_string()],
            screenshot: vec!["ctrl+shift+s".to_string(), "printscreen".to_string()],
            search: vec!["ctrl+shift+f".to_string()],
            copy_mode: vec!["ctrl+shift+space".to_string()],
            font_increase: vec!["ctrl+plus".to_string()],
            font_decrease: vec!["ctrl+minus".to_string()],
            font_reset: vec!["ctrl+0".to_string()],
            scroll_up: vec!["alt+shift+v".to_string()], // M-S-v (safe Emacs-like)
            scroll_down: vec!["alt+shift+n".to_string()], // M-S-n (safe Emacs-like)
            ime_toggle: vec!["ctrl+shift+j".to_string()],
        }
    }

    /// Vim preset (Vim-like scroll)
    /// Ctrl+Shift based for safety, only scroll is Vim-like
    /// (Ctrl+U/D conflicts with bash line deletion, so avoided)
    pub fn vim_preset() -> Self {
        Self {
            copy: vec!["ctrl+shift+c".to_string()],
            paste: vec!["ctrl+shift+v".to_string()],
            screenshot: vec!["ctrl+shift+s".to_string(), "printscreen".to_string()],
            search: vec!["ctrl+shift+f".to_string()],
            copy_mode: vec!["ctrl+shift+space".to_string()],
            font_increase: vec!["ctrl+plus".to_string()],
            font_decrease: vec!["ctrl+minus".to_string()],
            font_reset: vec!["ctrl+0".to_string()],
            scroll_up: vec!["ctrl+shift+u".to_string()], // Ctrl+Shift+U (safe Vim-like)
            scroll_down: vec!["ctrl+shift+d".to_string()], // Ctrl+Shift+D (safe Vim-like)
            ime_toggle: vec!["ctrl+shift+j".to_string()],
        }
    }
}

impl Config {
    /// System-wide config path
    const SYSTEM_CONFIG_PATH: &'static str = "/etc/bcon/config.toml";

    /// Load configuration file
    ///
    /// Priority:
    /// 1. Path specified by BCON_CONFIG environment variable
    /// 2. ~/.config/bcon/config.toml (user config)
    /// 3. /etc/bcon/config.toml (system config)
    /// 4. Built-in defaults
    pub fn load() -> Self {
        // 1. BCON_CONFIG environment variable
        if let Ok(path) = std::env::var("BCON_CONFIG") {
            if let Ok(config) = Self::load_from_file(&path) {
                info!("Loaded config from BCON_CONFIG: {}", path);
                return config;
            }
        }

        // 2. User config: ~/.config/bcon/config.toml
        if let Some(config_dir) = dirs::config_dir() {
            let config_path = config_dir.join("bcon").join("config.toml");
            if config_path.exists() {
                if let Ok(config) = Self::load_from_file(config_path.to_string_lossy().as_ref()) {
                    info!("Loaded config: {}", config_path.display());
                    return config;
                }
            }
        }

        // 3. System config: /etc/bcon/config.toml
        let system_config = std::path::Path::new(Self::SYSTEM_CONFIG_PATH);
        if system_config.exists() {
            if let Ok(config) = Self::load_from_file(Self::SYSTEM_CONFIG_PATH) {
                info!("Loaded system config: {}", Self::SYSTEM_CONFIG_PATH);
                return config;
            }
        }

        // 4. Built-in defaults
        info!("Using built-in default config");
        Self::default()
    }

    /// Load settings from specified path
    fn load_from_file(path: &str) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path))?;
        let config: Config = toml::from_str(&content)
            .with_context(|| format!("Failed to parse config file: {}", path))?;
        Ok(config)
    }

    /// Write config to file (for template generation)
    ///
    /// preset: can specify multiple comma-separated
    /// Examples: "default", "emacs,japanese", "vim,jp"
    ///
    /// Special presets:
    /// - "system" - Write to /etc/bcon/config.toml instead of user config
    pub fn write_config_with_preset(preset: &str) -> Result<PathBuf> {
        // Process multiple comma-separated presets
        let presets: Vec<&str> = preset.split(',').map(|s| s.trim()).collect();

        // Check if writing to system config
        let use_system_path = presets.contains(&"system");

        let config_path = if use_system_path {
            let system_dir = std::path::Path::new("/etc/bcon");
            std::fs::create_dir_all(system_dir)?;
            system_dir.join("config.toml")
        } else {
            let config_dir =
                dirs::config_dir().ok_or_else(|| anyhow::anyhow!("Config directory not found"))?;
            let bcon_dir = config_dir.join("bcon");
            std::fs::create_dir_all(&bcon_dir)?;
            bcon_dir.join("config.toml")
        };

        let mut keybinds = KeybindConfig::default_preset();
        let mut include_japanese = false;

        for p in &presets {
            match *p {
                "japanese" | "jp" => {
                    include_japanese = true;
                }
                "emacs" => {
                    keybinds = KeybindConfig::emacs_preset();
                }
                "vim" => {
                    keybinds = KeybindConfig::vim_preset();
                }
                _ => {
                    // "default", "system", or unknown - use default keybinds
                }
            }
        }

        // Generate preset name (exclude "system" from display)
        let display_presets: Vec<&str> = presets
            .iter()
            .filter(|&&p| p != "system")
            .copied()
            .collect();
        let preset_name = if display_presets.is_empty() {
            "default".to_string()
        } else {
            display_presets.join(" + ")
        };

        // Config path for display in template
        let config_path_display = if use_system_path {
            "/etc/bcon/config.toml"
        } else {
            "~/.config/bcon/config.toml"
        };

        // Serialize only keybinds (font settings use system defaults)
        let keybinds_toml = toml::to_string_pretty(&keybinds)?;

        // Japanese/CJK section (only if preset includes japanese)
        let japanese_section = if include_japanese {
            r#"
[font]
cjk = "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc"

[terminal]
ime_disabled_apps = ["vim", "nvim", "vi", "vimdiff", "emacs", "nano", "less", "man", "htop", "top"]
"#
        } else {
            ""
        };

        let template = format!(
            r#"# bcon configuration file
# Config path: {config_path_display}
# Keybind preset: {preset_name}
#
# Font settings are commented out by default.
# bcon will automatically find system fonts via fontconfig.

[keybinds]
{keybinds_toml}{japanese_section}
# =============================================================================
# Font Configuration (Optional)
# =============================================================================
# By default, bcon automatically finds fonts via fontconfig.
# Uncomment and customize if you want to use specific fonts.
#
# Recommended monospace fonts:
#   1. FiraCode - ligature support, great for coding
#   2. JetBrains Mono - designed for developers
#   3. Hack - clean and readable
#   4. DejaVu Sans Mono - widely available
#
# [font]
# size = 18.0
# main = "/usr/share/fonts/truetype/firacode/FiraCode-Regular.ttf"
# cjk = "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc"
# emoji = "/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf"
#
# Install on Ubuntu/Debian:
#   sudo apt install fonts-firacode fonts-noto-color-emoji fonts-noto-cjk

# =============================================================================
# LCD Subpixel Rendering (High Quality Text Rendering)
# =============================================================================
# Default settings are optimized for iTerm2-like sharp rendering.
# These are the built-in defaults - only uncomment if you need to change them.
#
# [font]
# render_mode = "lcd"           # "lcd" (high quality) or "grayscale"
# lcd_filter = "default"        # Sharp (less blurry than light)
# lcd_subpixel = "rgb"          # Adjust to match your panel
# lcd_gamma = 1.15              # Thinner/tighter appearance (1.0-1.25)
# lcd_stem_darkening = 0.0      # Disabled recommended (for sharpness)
# lcd_contrast = 1.15           # Contrast boost
# lcd_fringe_reduction = 0.1    # Light fringe reduction
# lcd_subpixel_positioning = true  # 1/3px precision (highest quality)
# lcd_hinting = "light"         # Natural curves (recommended)
#
# Troubleshooting:
#   Color fringe appears -> Try lcd_subpixel = "bgr"
#   Blurry text          -> lcd_gamma = 1.2
#   Text too thick       -> lcd_gamma = 1.15
#   Vertical monitor     -> lcd_subpixel = "vrgb" or "vbgr"

# =============================================================================
# Appearance (Optional)
# =============================================================================
# [appearance]
# background = "000000"
# foreground = "ffffff"
# cursor = "ffffff"
# selection = "44475a"

# =============================================================================
# Terminal Settings (Optional)
# =============================================================================
# [terminal]
# scrollback_lines = 10000
"#
        );

        std::fs::write(&config_path, template)?;
        Ok(config_path)
    }

    /// Write default config to file
    #[allow(dead_code)]
    pub fn write_default_config() -> Result<PathBuf> {
        Self::write_config_with_preset("default")
    }
}

/// Fix f32 serialization precision issues
/// Round floating point in TOML output to 2 decimal places
fn fix_float_precision(content: &str) -> String {
    let mut result = content.to_string();

    // Simple replacement without regex
    // Replace common f32 imprecise values with correct values
    let replacements = [
        // 1.15 variants
        ("1.149999976158142", "1.15"),
        ("1.1499999761581421", "1.15"),
        // 1.1 variants
        ("1.100000023841858", "1.1"),
        ("1.1000000238418579", "1.1"),
        // 0.15 variants
        ("0.15000000596046448", "0.15"),
        ("0.1500000059604645", "0.15"),
        // 0.1 variants
        ("0.10000000149011612", "0.1"),
        ("0.1000000014901161", "0.1"),
        // 0.5 variants
        ("0.5", "0.5"),
        // 18.0 variants
        ("18.0", "18.0"),
    ];

    for (from, to) in replacements {
        result = result.replace(from, to);
    }

    result
}

/// Convert color string (RRGGBB) to [f32; 4]
#[allow(dead_code)]
pub fn parse_color(hex: &str) -> [f32; 4] {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return [1.0, 1.0, 1.0, 1.0]; // Default: white
    }

    let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(255);
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(255);
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(255);

    [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0]
}

/// Parse multiple keybindings
#[derive(Debug, Clone, Default)]
pub struct ParsedKeybinds {
    pub bindings: Vec<ParsedKeybind>,
}

impl ParsedKeybinds {
    /// Parse from string array
    pub fn parse(keys: &[String]) -> Self {
        Self {
            bindings: keys.iter().map(|s| ParsedKeybind::parse(s)).collect(),
        }
    }

    /// Check if any keybind matches
    pub fn matches(&self, ctrl: bool, shift: bool, alt: bool, keycode: u32, keysym: u32) -> bool {
        self.bindings
            .iter()
            .any(|kb| kb.matches(ctrl, shift, alt, keycode, keysym))
    }
}

/// Parse keybind string
/// Example: "ctrl+shift+c" -> (ctrl: true, shift: true, key: "c")
#[derive(Debug, Clone, Default)]
pub struct ParsedKeybind {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub key: String,
}

impl ParsedKeybind {
    pub fn parse(s: &str) -> Self {
        let lowercase = s.to_lowercase();
        let parts: Vec<&str> = lowercase.split('+').collect();
        let mut result = Self::default();

        for part in &parts {
            match *part {
                "ctrl" | "control" => result.ctrl = true,
                "shift" => result.shift = true,
                "alt" => result.alt = true,
                other => result.key = other.to_string(),
            }
        }

        result
    }

    /// Check if matches evdev keycode and modifiers
    pub fn matches(&self, ctrl: bool, shift: bool, alt: bool, keycode: u32, keysym: u32) -> bool {
        if self.ctrl != ctrl || self.shift != shift || self.alt != alt {
            return false;
        }

        // Determine keycode/keysym from key name
        // Special keys use keysym, normal keys use keycode
        match self.key.as_str() {
            // Alphabet (evdev keycode)
            "a" => keycode == 30,
            "b" => keycode == 48,
            "c" => keycode == 46,
            "d" => keycode == 32,
            "e" => keycode == 18,
            "f" => keycode == 33,
            "g" => keycode == 34,
            "h" => keycode == 35,
            "i" => keycode == 23,
            "j" => keycode == 36,
            "k" => keycode == 37,
            "l" => keycode == 38,
            "m" => keycode == 50,
            "n" => keycode == 49,
            "o" => keycode == 24,
            "p" => keycode == 25,
            "q" => keycode == 16,
            "r" => keycode == 19,
            "s" => keycode == 31,
            "t" => keycode == 20,
            "u" => keycode == 22,
            "v" => keycode == 47,
            "w" => keycode == 17,
            "x" => keycode == 45,
            "y" => keycode == 21,
            "z" => keycode == 44,
            // Numbers
            "0" => keycode == 11,
            "1" => keycode == 2,
            "2" => keycode == 3,
            "3" => keycode == 4,
            "4" => keycode == 5,
            "5" => keycode == 6,
            "6" => keycode == 7,
            "7" => keycode == 8,
            "8" => keycode == 9,
            "9" => keycode == 10,
            // Special keys (evdev keycode)
            "space" => keycode == 57,
            "plus" | "=" => keycode == 13,
            "minus" | "-" => keycode == 12,
            "enter" | "return" => keycode == 28,
            "tab" => keycode == 15,
            "escape" | "esc" => keycode == 1,
            "backspace" => keycode == 14,
            "insert" | "ins" => keycode == 110,
            "delete" | "del" => keycode == 111,
            "printscreen" | "print" | "prtsc" | "sysrq" => keycode == 99,
            "scrolllock" => keycode == 70,
            "pause" | "break" => keycode == 119,
            // Function keys
            "f1" => keycode == 59,
            "f2" => keycode == 60,
            "f3" => keycode == 61,
            "f4" => keycode == 62,
            "f5" => keycode == 63,
            "f6" => keycode == 64,
            "f7" => keycode == 65,
            "f8" => keycode == 66,
            "f9" => keycode == 67,
            "f10" => keycode == 68,
            "f11" => keycode == 87,
            "f12" => keycode == 88,
            // Navigation keys (use keysym)
            "pageup" => keysym == 0xff55,   // XK_Page_Up
            "pagedown" => keysym == 0xff56, // XK_Page_Down
            "home" => keysym == 0xff50,     // XK_Home
            "end" => keysym == 0xff57,      // XK_End
            "up" => keysym == 0xff52,       // XK_Up
            "down" => keysym == 0xff54,     // XK_Down
            "left" => keysym == 0xff51,     // XK_Left
            "right" => keysym == 0xff53,    // XK_Right
            _ => false,
        }
    }
}

/// Config file change watcher (Linux only)
#[cfg(target_os = "linux")]
pub struct ConfigWatcher {
    _watcher: RecommendedWatcher,
    rx: mpsc::Receiver<()>,
}

#[cfg(target_os = "linux")]
impl ConfigWatcher {
    /// Start watching config file
    pub fn new(config_path: &Path) -> Result<Self> {
        let (tx, rx) = mpsc::channel();

        let mut watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
            if let Ok(event) = res {
                // Only detect Modify events
                if event.kind.is_modify() {
                    let _ = tx.send(());
                }
            }
        })?;

        watcher.watch(config_path, RecursiveMode::NonRecursive)?;

        Ok(Self {
            _watcher: watcher,
            rx,
        })
    }

    /// Check if config file was modified (non-blocking)
    pub fn check_reload(&self) -> bool {
        self.rx.try_recv().is_ok()
    }
}

/// Get default config file path
pub fn default_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("bcon").join("config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_color() {
        let color = parse_color("ff0000");
        assert!((color[0] - 1.0).abs() < 0.01);
        assert!(color[1].abs() < 0.01);
        assert!(color[2].abs() < 0.01);
    }

    #[test]
    fn test_parse_keybind() {
        let kb = ParsedKeybind::parse("ctrl+shift+c");
        assert!(kb.ctrl);
        assert!(kb.shift);
        assert_eq!(kb.key, "c");
    }
}
