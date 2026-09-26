//! Configuration file management
//!
//! Loads TOML configuration files and provides application settings.
//! Default config path: ~/.config/bcon/config.toml

#![allow(dead_code)]

use anyhow::{Context, Result};
use log::{info, warn};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::constants::{LCD_DEFAULT_CONTRAST, LCD_DEFAULT_GAMMA};

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
    /// Color scheme settings (ANSI 16 colors)
    pub colors: ColorsConfig,
    /// Terminal settings
    pub terminal: TerminalConfig,
    /// Keyboard settings
    pub keyboard: KeyboardInputConfig,
    /// Mouse settings
    pub mouse: MouseConfig,
    /// DRM settings
    pub drm: DrmConfig,
    /// Display settings
    pub display: DisplayOutputConfig,
    /// Notification settings
    pub notifications: NotificationConfig,
    /// Security settings
    pub security: SecurityConfig,
}

/// Font settings
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FontConfig {
    /// Main font: family name or file path (searches system fonts if empty)
    pub main: String,
    /// Symbol/icon font: family name or file path (searches system fonts if empty)
    pub symbols: String,
    /// CJK font: family name or file path (searches system fonts if empty)
    pub cjk: String,
    /// Emoji font: family name or file path (searches system fonts if empty)
    pub emoji: String,
    /// Font size
    pub size: f32,
    /// Rendering mode: "lcd" (default) or "grayscale"
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
    /// Display scale factor (1.0, 1.25, 1.5, 1.75, 2.0)
    /// Used for HiDPI displays. Font size is multiplied by this value.
    pub scale: f32,
}

/// Color scheme settings (ANSI 16 colors)
/// Colors are specified as RRGGBB hex strings
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorsConfig {
    /// Color 0: Black
    pub black: String,
    /// Color 1: Red
    pub red: String,
    /// Color 2: Green
    pub green: String,
    /// Color 3: Yellow
    pub yellow: String,
    /// Color 4: Blue
    pub blue: String,
    /// Color 5: Magenta
    pub magenta: String,
    /// Color 6: Cyan
    pub cyan: String,
    /// Color 7: White
    pub white: String,
    /// Color 8: Bright Black (Gray)
    pub bright_black: String,
    /// Color 9: Bright Red
    pub bright_red: String,
    /// Color 10: Bright Green
    pub bright_green: String,
    /// Color 11: Bright Yellow
    pub bright_yellow: String,
    /// Color 12: Bright Blue
    pub bright_blue: String,
    /// Color 13: Bright Magenta
    pub bright_magenta: String,
    /// Color 14: Bright Cyan
    pub bright_cyan: String,
    /// Color 15: Bright White
    pub bright_white: String,
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
    /// Enable fcitx5 IME (Japanese input)
    /// When true, bcon will auto-start D-Bus session and fcitx5 if needed
    pub ime: bool,
    /// List of apps that auto-disable IME
    /// When foreground process name is in this list, IME is automatically disabled
    pub ime_disabled_apps: Vec<String>,
}

/// Keyboard input settings
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct KeyboardInputConfig {
    /// Key repeat delay in milliseconds (default: 400)
    pub repeat_delay: u64,
    /// Key repeat rate in milliseconds (default: 30)
    pub repeat_rate: u64,
    /// XKB keyboard model (empty = default)
    pub xkb_model: String,
    /// XKB keyboard layout (e.g., "us", "jp", empty = default)
    pub xkb_layout: String,
    /// XKB keyboard variant (empty = default)
    pub xkb_variant: String,
    /// XKB keyboard options (e.g., "ctrl:nocaps", empty = default)
    pub xkb_options: String,
}

impl Default for KeyboardInputConfig {
    fn default() -> Self {
        Self {
            repeat_delay: 400,
            repeat_rate: 30,
            xkb_model: String::new(),
            xkb_layout: String::new(),
            xkb_variant: String::new(),
            xkb_options: String::new(),
        }
    }
}

/// Mouse settings
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MouseConfig {
    /// Mouse cursor speed multiplier (default: 1.0)
    /// Values > 1.0 make the cursor faster, < 1.0 slower.
    /// Recommended: 1.5–2.0 for 4K displays.
    pub speed: f64,
    /// Enable tap-to-click on touchpads (default: true)
    pub tap_to_click: bool,
    /// Enable natural (reverse) scrolling on touchpads (default: false)
    pub natural_scroll: bool,
    /// Disable touchpad while typing (default: true)
    pub disable_while_typing: bool,
}

impl Default for MouseConfig {
    fn default() -> Self {
        Self {
            speed: 1.0,
            tap_to_click: true,
            natural_scroll: false,
            disable_while_typing: true,
        }
    }
}

/// DRM device settings
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DrmConfig {
    /// DRM device path: "auto" (default) or explicit path like "/dev/dri/card1"
    /// On Optimus laptops, set this to the Intel iGPU device
    pub device: String,
}

impl Default for DrmConfig {
    fn default() -> Self {
        Self {
            device: "auto".to_string(),
        }
    }
}

/// Display output settings
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplayOutputConfig {
    /// Prefer external monitors over internal (eDP) display
    /// When true, external monitors (HDMI, DisplayPort, DVI, VGA) take priority
    /// When false, use first connected display (default behavior)
    pub prefer_external: bool,
    /// Auto-switch to external monitor when connected
    /// When true, automatically switch display on hotplug connect
    pub auto_switch: bool,
    /// Display rotation in clockwise degrees: 0, 90, 180, or 270
    /// Useful for tablets and portrait monitors where the panel reports
    /// a fixed native orientation different from how the user holds it.
    /// Implemented via a GL projection-matrix transform (no hardware dependency).
    pub rotation: u32,
}

impl Default for DisplayOutputConfig {
    fn default() -> Self {
        Self {
            prefer_external: true,
            auto_switch: true,
            rotation: 0,
        }
    }
}

impl DisplayOutputConfig {
    /// Returns the rotation clamped to one of the valid values (0/90/180/270).
    /// Logs a warning for invalid input and falls back to 0.
    pub fn normalized_rotation(&self) -> u32 {
        match self.rotation {
            0 | 90 | 180 | 270 => self.rotation,
            other => {
                warn!(
                    "Invalid display.rotation = {} (must be 0, 90, 180, or 270). Using 0.",
                    other
                );
                0
            }
        }
    }
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
    /// Reset terminal modes (default: "ctrl+shift+escape") - reset enhanced input modes
    #[serde(deserialize_with = "deserialize_keybind")]
    pub reset_terminal: Vec<String>,
    /// Notification panel toggle (default: "ctrl+shift+n")
    #[serde(deserialize_with = "deserialize_keybind")]
    pub notification_panel: Vec<String>,
    /// Notification mute toggle (default: "ctrl+shift+m")
    #[serde(deserialize_with = "deserialize_keybind")]
    pub notification_mute: Vec<String>,

    // === Pane management ===

    /// Split pane right (default: "ctrl+shift+enter")
    #[serde(deserialize_with = "deserialize_keybind")]
    pub split_right: Vec<String>,
    /// Split pane down (default: "ctrl+shift+d")
    #[serde(deserialize_with = "deserialize_keybind")]
    pub split_down: Vec<String>,
    /// Close pane (default: "ctrl+shift+w")
    #[serde(deserialize_with = "deserialize_keybind")]
    pub close_pane: Vec<String>,
    /// Navigate pane left
    #[serde(deserialize_with = "deserialize_keybind")]
    pub pane_left: Vec<String>,
    /// Navigate pane right
    #[serde(deserialize_with = "deserialize_keybind")]
    pub pane_right: Vec<String>,
    /// Navigate pane up
    #[serde(deserialize_with = "deserialize_keybind")]
    pub pane_up: Vec<String>,
    /// Navigate pane down
    #[serde(deserialize_with = "deserialize_keybind")]
    pub pane_down: Vec<String>,
    /// Resize pane left
    #[serde(deserialize_with = "deserialize_keybind")]
    pub resize_left: Vec<String>,
    /// Resize pane right
    #[serde(deserialize_with = "deserialize_keybind")]
    pub resize_right: Vec<String>,
    /// Resize pane up
    #[serde(deserialize_with = "deserialize_keybind")]
    pub resize_up: Vec<String>,
    /// Resize pane down
    #[serde(deserialize_with = "deserialize_keybind")]
    pub resize_down: Vec<String>,
    /// Zoom toggle (default: "ctrl+shift+z")
    #[serde(deserialize_with = "deserialize_keybind")]
    pub zoom_pane: Vec<String>,

    // === Tab management ===

    /// New tab (default: "ctrl+shift+t")
    #[serde(deserialize_with = "deserialize_keybind")]
    pub new_tab: Vec<String>,
    /// Close tab (default: "ctrl+shift+q")
    #[serde(deserialize_with = "deserialize_keybind")]
    pub close_tab: Vec<String>,
    /// Next tab (default: "ctrl+pagedown")
    #[serde(deserialize_with = "deserialize_keybind")]
    pub next_tab: Vec<String>,
    /// Previous tab (default: "ctrl+pageup")
    #[serde(deserialize_with = "deserialize_keybind")]
    pub prev_tab: Vec<String>,
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
            colors: ColorsConfig::default(),
            terminal: TerminalConfig::default(),
            keyboard: KeyboardInputConfig::default(),
            mouse: MouseConfig::default(),
            drm: DrmConfig::default(),
            display: DisplayOutputConfig::default(),
            notifications: NotificationConfig::default(),
            security: SecurityConfig::default(),
        }
    }
}

impl Default for FontConfig {
    fn default() -> Self {
        Self {
            main: String::new(),
            symbols: String::new(),
            cjk: String::new(),
            emoji: String::new(),
            size: 16.0,
            render_mode: "lcd".to_string(), // LCD subpixel rendering (high quality)
            lcd_filter: "default".to_string(), // Sharp (less blur than light)
            lcd_weights: None,
            lcd_subpixel: "rgb".to_string(),  // For common panels
            lcd_gamma: LCD_DEFAULT_GAMMA,      // Thinner/tighter look (iTerm2-like)
            lcd_stem_darkening: 0.0,          // Disabled (sharpness priority)
            lcd_subpixel_positioning: true,   // 1/3px phase rendering (highest quality)
            lcd_hinting: "light".to_string(), // Natural curves, thin (recommended)
            lcd_contrast: LCD_DEFAULT_CONTRAST, // Contrast enhancement
            lcd_fringe_reduction: 0.1,        // Light fringe reduction
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
            scale: 1.0,
        }
    }
}

impl AppearanceConfig {
    /// Parse hex color string (RRGGBB) to normalized RGB tuple (0.0-1.0)
    pub fn parse_hex_color(hex: &str) -> (f32, f32, f32) {
        let hex = hex.trim_start_matches('#');
        if hex.len() >= 6 {
            if let (Ok(r), Ok(g), Ok(b)) = (
                u8::from_str_radix(&hex[0..2], 16),
                u8::from_str_radix(&hex[2..4], 16),
                u8::from_str_radix(&hex[4..6], 16),
            ) {
                return (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
            }
        }
        // Fallback to black
        (0.0, 0.0, 0.0)
    }

    /// Get background color as normalized RGB
    pub fn background_rgb(&self) -> (f32, f32, f32) {
        Self::parse_hex_color(&self.background)
    }

    /// Get foreground color as normalized RGB
    pub fn foreground_rgb(&self) -> (f32, f32, f32) {
        Self::parse_hex_color(&self.foreground)
    }

    /// Get cursor color as normalized RGB
    pub fn cursor_rgb(&self) -> (f32, f32, f32) {
        Self::parse_hex_color(&self.cursor)
    }

    /// Get selection color as normalized RGB
    pub fn selection_rgb(&self) -> (f32, f32, f32) {
        Self::parse_hex_color(&self.selection)
    }
}

/// Modern color scheme defaults (inspired by One Dark / Catppuccin)
/// Muted, pleasant colors that are easy on the eyes
impl Default for ColorsConfig {
    fn default() -> Self {
        Self {
            // Normal colors (muted)
            black: "282c34".to_string(),   // Soft black
            red: "e06c75".to_string(),     // Muted red
            green: "98c379".to_string(),   // Soft green
            yellow: "e5c07b".to_string(),  // Warm yellow
            blue: "61afef".to_string(),    // Soft blue
            magenta: "c678dd".to_string(), // Soft purple
            cyan: "56b6c2".to_string(),    // Soft cyan
            white: "abb2bf".to_string(),   // Light gray

            // Bright colors (slightly more vibrant)
            bright_black: "5c6370".to_string(),   // Gray
            bright_red: "e06c75".to_string(),     // Same red (already vibrant)
            bright_green: "98c379".to_string(),   // Same green
            bright_yellow: "e5c07b".to_string(),  // Same yellow
            bright_blue: "61afef".to_string(),    // Same blue
            bright_magenta: "c678dd".to_string(), // Same magenta
            bright_cyan: "56b6c2".to_string(),    // Same cyan
            bright_white: "ffffff".to_string(),   // Pure white
        }
    }
}

impl ColorsConfig {
    /// Parse hex color string to [f32; 4] RGBA
    fn parse_color(hex: &str) -> [f32; 4] {
        let hex = hex.trim_start_matches('#');
        if hex.len() >= 6 {
            if let (Ok(r), Ok(g), Ok(b)) = (
                u8::from_str_radix(&hex[0..2], 16),
                u8::from_str_radix(&hex[2..4], 16),
                u8::from_str_radix(&hex[4..6], 16),
            ) {
                return [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0];
            }
        }
        [1.0, 1.0, 1.0, 1.0] // Fallback: white
    }

    /// Get ANSI 16 colors as array of [f32; 4] for shader use
    pub fn to_palette(&self) -> [[f32; 4]; 16] {
        [
            Self::parse_color(&self.black),
            Self::parse_color(&self.red),
            Self::parse_color(&self.green),
            Self::parse_color(&self.yellow),
            Self::parse_color(&self.blue),
            Self::parse_color(&self.magenta),
            Self::parse_color(&self.cyan),
            Self::parse_color(&self.white),
            Self::parse_color(&self.bright_black),
            Self::parse_color(&self.bright_red),
            Self::parse_color(&self.bright_green),
            Self::parse_color(&self.bright_yellow),
            Self::parse_color(&self.bright_blue),
            Self::parse_color(&self.bright_magenta),
            Self::parse_color(&self.bright_cyan),
            Self::parse_color(&self.bright_white),
        ]
    }
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            scrollback_lines: 10000,
            bell: "visual".to_string(),
            term_env: "xterm-256color".to_string(),
            ime: false,
            // Empty by default - uncomment in config for CJK/IME users
            ime_disabled_apps: vec![],
        }
    }
}

/// Notification settings
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NotificationConfig {
    /// Enable notification system (OSC 9/99). Default: true
    pub enabled: bool,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// Security settings
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SecurityConfig {
    /// Allow Kitty graphics protocol remote file/shm transfers (t=f, t=t, t=s).
    /// Default: true (matches kitty's default behavior).
    /// When enabled, temp file paths are restricted to /tmp/ and /dev/shm/.
    /// Set to false to restrict to direct base64 transfer (t=d) only.
    ///
    /// NOTE: bcon runs as root for DRM access, so t=f can read any file.
    /// This is the same trust model as kitty/foot/other terminal emulators.
    /// If you want to harden against malicious escape sequences, set to false.
    pub allow_kitty_remote: bool,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            allow_kitty_remote: true,
        }
    }
}

impl Default for KeybindConfig {
    fn default() -> Self {
        Self::default_preset()
    }
}

impl KeybindConfig {
    /// Default pane/tab keybinds (shared across all presets)
    fn default_pane_keybinds() -> PaneTabKeybinds {
        PaneTabKeybinds {
            split_right: vec!["ctrl+shift+enter".to_string()],
            split_down: vec!["ctrl+shift+d".to_string()],
            close_pane: vec!["ctrl+shift+w".to_string()],
            pane_left: vec!["ctrl+shift+left".to_string()],
            pane_right: vec!["ctrl+shift+right".to_string()],
            pane_up: vec!["ctrl+shift+up".to_string()],
            pane_down: vec!["ctrl+shift+down".to_string()],
            resize_left: vec!["ctrl+shift+alt+left".to_string()],
            resize_right: vec!["ctrl+shift+alt+right".to_string()],
            resize_up: vec!["ctrl+shift+alt+up".to_string()],
            resize_down: vec!["ctrl+shift+alt+down".to_string()],
            zoom_pane: vec!["ctrl+shift+z".to_string()],
            new_tab: vec!["ctrl+shift+t".to_string()],
            close_tab: vec!["ctrl+shift+q".to_string()],
            next_tab: vec!["ctrl+pagedown".to_string()],
            prev_tab: vec!["ctrl+pageup".to_string()],
        }
    }

    /// Vim preset pane navigation (hjkl)
    fn vim_pane_keybinds() -> PaneTabKeybinds {
        let mut kb = Self::default_pane_keybinds();
        kb.pane_left = vec!["ctrl+shift+h".to_string(), "ctrl+shift+left".to_string()];
        kb.pane_right = vec!["ctrl+shift+l".to_string(), "ctrl+shift+right".to_string()];
        kb.pane_up = vec!["ctrl+shift+k".to_string(), "ctrl+shift+up".to_string()];
        kb.pane_down = vec!["ctrl+shift+j".to_string(), "ctrl+shift+down".to_string()];
        kb.resize_left = vec!["ctrl+shift+alt+h".to_string(), "ctrl+shift+alt+left".to_string()];
        kb.resize_right = vec!["ctrl+shift+alt+l".to_string(), "ctrl+shift+alt+right".to_string()];
        kb.resize_up = vec!["ctrl+shift+alt+k".to_string(), "ctrl+shift+alt+up".to_string()];
        kb.resize_down = vec!["ctrl+shift+alt+j".to_string(), "ctrl+shift+alt+down".to_string()];
        // Vim preset uses Ctrl+Shift+D for scroll down, so use Ctrl+Shift+Backslash for split down
        kb.split_down = vec!["ctrl+shift+\\".to_string()];
        kb
    }

    /// Default preset (standard terminal keybindings)
    pub fn default_preset() -> Self {
        let pane = Self::default_pane_keybinds();
        Self {
            copy: vec!["ctrl+shift+c".to_string()],
            paste: vec!["ctrl+shift+v".to_string()],
            screenshot: vec!["ctrl+shift+s".to_string(), "printscreen".to_string()],
            search: vec!["ctrl+shift+f".to_string()],
            copy_mode: vec!["ctrl+shift+space".to_string()],
            font_increase: vec!["ctrl+=".to_string(), "ctrl+shift+=".to_string()],
            font_decrease: vec!["ctrl+minus".to_string(), "ctrl+shift+minus".to_string()],
            font_reset: vec!["ctrl+0".to_string()],
            scroll_up: vec!["shift+pageup".to_string()],
            scroll_down: vec!["shift+pagedown".to_string()],
            ime_toggle: vec!["ctrl+shift+j".to_string()],
            reset_terminal: vec!["ctrl+shift+escape".to_string()],
            notification_panel: vec!["ctrl+shift+n".to_string()],
            notification_mute: vec!["ctrl+shift+m".to_string()],
            split_right: pane.split_right,
            split_down: pane.split_down,
            close_pane: pane.close_pane,
            pane_left: pane.pane_left,
            pane_right: pane.pane_right,
            pane_up: pane.pane_up,
            pane_down: pane.pane_down,
            resize_left: pane.resize_left,
            resize_right: pane.resize_right,
            resize_up: pane.resize_up,
            resize_down: pane.resize_down,
            zoom_pane: pane.zoom_pane,
            new_tab: pane.new_tab,
            close_tab: pane.close_tab,
            next_tab: pane.next_tab,
            prev_tab: pane.prev_tab,
        }
    }

    /// Emacs preset (Emacs-like scroll)
    pub fn emacs_preset() -> Self {
        let mut pane = Self::default_pane_keybinds();
        // Default close_pane (ctrl+shift+w) conflicts with Emacs copy
        pane.close_pane = vec!["alt+shift+w".to_string()];
        Self {
            copy: vec!["ctrl+shift+w".to_string()],
            paste: vec!["ctrl+shift+y".to_string()],
            screenshot: vec!["printscreen".to_string()],
            search: vec!["ctrl+shift+s".to_string()],
            copy_mode: vec!["ctrl+shift+m".to_string()],
            font_increase: vec!["ctrl+=".to_string(), "ctrl+shift+=".to_string()],
            font_decrease: vec!["ctrl+minus".to_string(), "ctrl+shift+minus".to_string()],
            font_reset: vec!["ctrl+0".to_string()],
            scroll_up: vec!["alt+shift+v".to_string()],
            scroll_down: vec!["alt+shift+n".to_string()],
            ime_toggle: vec!["ctrl+shift+j".to_string()],
            reset_terminal: vec!["ctrl+shift+escape".to_string()],
            notification_panel: vec!["ctrl+shift+n".to_string()],
            notification_mute: vec!["alt+shift+m".to_string()],
            split_right: pane.split_right,
            split_down: pane.split_down,
            close_pane: pane.close_pane,
            pane_left: pane.pane_left,
            pane_right: pane.pane_right,
            pane_up: pane.pane_up,
            pane_down: pane.pane_down,
            resize_left: pane.resize_left,
            resize_right: pane.resize_right,
            resize_up: pane.resize_up,
            resize_down: pane.resize_down,
            zoom_pane: pane.zoom_pane,
            new_tab: pane.new_tab,
            close_tab: pane.close_tab,
            next_tab: pane.next_tab,
            prev_tab: pane.prev_tab,
        }
    }

    /// Vim preset (Vim-like scroll + hjkl pane navigation)
    pub fn vim_preset() -> Self {
        let pane = Self::vim_pane_keybinds();
        Self {
            copy: vec!["ctrl+shift+c".to_string()],
            paste: vec!["ctrl+shift+v".to_string()],
            screenshot: vec!["ctrl+shift+s".to_string(), "printscreen".to_string()],
            search: vec!["ctrl+shift+f".to_string()],
            copy_mode: vec!["ctrl+shift+space".to_string()],
            font_increase: vec!["ctrl+=".to_string(), "ctrl+shift+=".to_string()],
            font_decrease: vec!["ctrl+minus".to_string(), "ctrl+shift+minus".to_string()],
            font_reset: vec!["ctrl+0".to_string()],
            scroll_up: vec!["ctrl+shift+u".to_string()],
            scroll_down: vec!["ctrl+shift+d".to_string()],
            ime_toggle: vec!["ctrl+shift+j".to_string()],
            reset_terminal: vec!["ctrl+shift+escape".to_string()],
            notification_panel: vec!["ctrl+shift+n".to_string()],
            notification_mute: vec!["ctrl+shift+m".to_string()],
            split_right: pane.split_right,
            split_down: pane.split_down,
            close_pane: pane.close_pane,
            pane_left: pane.pane_left,
            pane_right: pane.pane_right,
            pane_up: pane.pane_up,
            pane_down: pane.pane_down,
            resize_left: pane.resize_left,
            resize_right: pane.resize_right,
            resize_up: pane.resize_up,
            resize_down: pane.resize_down,
            zoom_pane: pane.zoom_pane,
            new_tab: pane.new_tab,
            close_tab: pane.close_tab,
            next_tab: pane.next_tab,
            prev_tab: pane.prev_tab,
        }
    }
}

/// Internal helper for pane/tab keybind defaults
struct PaneTabKeybinds {
    split_right: Vec<String>,
    split_down: Vec<String>,
    close_pane: Vec<String>,
    pane_left: Vec<String>,
    pane_right: Vec<String>,
    pane_up: Vec<String>,
    pane_down: Vec<String>,
    resize_left: Vec<String>,
    resize_right: Vec<String>,
    resize_up: Vec<String>,
    resize_down: Vec<String>,
    zoom_pane: Vec<String>,
    new_tab: Vec<String>,
    close_tab: Vec<String>,
    next_tab: Vec<String>,
    prev_tab: Vec<String>,
}

/// Merge `overlay` into `base` as a TOML value tree.
///
/// Tables are merged recursively, so that keys present only in `base` are
/// preserved while keys present in `overlay` override (or add to) `base`.
/// Any non-table slot — scalars, arrays, and missing-vs-present options —
/// is replaced wholesale by the overlay value (no array append, no scalar mix).
fn merge_value(base: &mut toml::Value, overlay: toml::Value) {
    use toml::Value;
    match (base, overlay) {
        (Value::Table(b), Value::Table(o)) => {
            for (k, v) in o {
                match b.get_mut(&k) {
                    Some(existing) => merge_value(existing, v),
                    None => {
                        b.insert(k, v);
                    }
                }
            }
        }
        (slot, overlay_val) => {
            *slot = overlay_val;
        }
    }
}

/// Read a TOML file from disk and parse it into a generic `toml::Value`.
///
/// Returned as `toml::Value` (not `Config`) so the caller can feed it into
/// [`merge_value`] before coercing the merged tree back into [`Config`] —
/// this preserves field-level missing-vs-present semantics across layers.
fn parse_layer(path: &std::path::Path) -> Result<toml::Value> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read config layer: {}", path.display()))?;
    let value: toml::Value = toml::from_str(&content)
        .with_context(|| format!("Failed to parse config layer: {}", path.display()))?;
    Ok(value)
}

/// Where `Config::write_config_with_preset` should write the generated file.
///
/// Decoupled from the preset list (`vim`/`emacs`/`jp`) so that the caller
/// must declare the destination explicitly and cannot accidentally route a
/// `sudo bcon --init-config=vim` invocation to `/root/.config/bcon/`. The
/// previous string-based dispatch silently routed by `dirs::config_dir()`
/// of the calling user, which produced the root-shadow trap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteTarget {
    /// User config: `dirs::config_dir().join("bcon/config.toml")`.
    User,
    /// System config: `/etc/bcon/config.toml`.
    System,
    /// An explicit absolute path (debug / test / unusual deployments).
    Path(std::path::PathBuf),
}

/// Expand a leading `~/` to the current user's home directory.
///
/// Anything else (including bare `~` without a slash) is returned as-is.
fn expand_tilde(s: &str) -> std::path::PathBuf {
    if let Some(rest) = s.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    std::path::PathBuf::from(s)
}

/// Parse the value of `--init-config[=ARG]` into a `(target, presets)` pair.
///
/// `arg` is the raw text after `=` (or the empty string for the bare
/// `--init-config` form). The first comma-separated token is interpreted
/// as a destination target if and only if it is exactly `"system"`,
/// exactly `"user"`, or starts with `/` (absolute path) or `~/` (home
/// expansion). All remaining tokens are treated as preset names
/// (`default`, `vim`, `emacs`, `japanese`/`jp`). When the first token is
/// not a target token, the destination defaults to `User` so that the
/// legacy form `--init-config=vim,jp` keeps writing to
/// `~/.config/bcon/config.toml`.
pub fn parse_init_config_arg(arg: &str) -> (WriteTarget, Vec<String>) {
    let tokens: Vec<&str> = arg
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();

    let (target, preset_tokens): (WriteTarget, &[&str]) = match tokens.first() {
        Some(&"system") => (WriteTarget::System, &tokens[1..]),
        Some(&"user") => (WriteTarget::User, &tokens[1..]),
        Some(t) if t.starts_with('/') || t.starts_with("~/") => {
            (WriteTarget::Path(expand_tilde(t)), &tokens[1..])
        }
        _ => (WriteTarget::User, &tokens[..]),
    };

    let presets = preset_tokens.iter().map(|s| s.to_string()).collect();
    (target, presets)
}

impl Config {
    /// System-wide config path
    const SYSTEM_CONFIG_PATH: &'static str = "/etc/bcon/config.toml";

    /// Load configuration file
    ///
    /// Get the path that would be used for loading config
    /// Returns None if using built-in defaults
    pub fn config_path() -> Option<std::path::PathBuf> {
        // 1. BCON_CONFIG environment variable
        if let Ok(path) = std::env::var("BCON_CONFIG") {
            let p = std::path::Path::new(&path);
            if p.exists() {
                return Some(p.to_path_buf());
            }
        }

        // 2. User config: ~/.config/bcon/config.toml
        if let Some(config_dir) = dirs::config_dir() {
            let config_path = config_dir.join("bcon").join("config.toml");
            if config_path.exists() {
                return Some(config_path);
            }
        }

        // 3. System config: /etc/bcon/config.toml
        let system_config = std::path::Path::new(Self::SYSTEM_CONFIG_PATH);
        if system_config.exists() {
            return Some(system_config.to_path_buf());
        }

        None
    }

    /// Load configuration using the layered XDG model.
    ///
    /// Resolution order:
    /// 1. `BCON_CONFIG` environment variable, if set and the file exists,
    ///    is loaded as a single-file bypass (no merge with other layers).
    ///    Intended for debugging and testing — the override is exclusive.
    /// 2. Otherwise the layered merge applies: built-in defaults are
    ///    overlaid first by `/etc/bcon/config.toml` (site default,
    ///    typically installed by the package manager) and then by
    ///    `~/.config/bcon/config.toml` (per-user override). Tables merge
    ///    recursively; scalars and arrays are replaced wholesale by the
    ///    later layer. See [`Config::load_layered_from_paths`] for the
    ///    pure form used by tests.
    pub fn load() -> Self {
        if let Ok(p) = std::env::var("BCON_CONFIG") {
            let path = std::path::Path::new(&p);
            if path.exists() {
                match Self::load_from_file(&p) {
                    Ok(c) => {
                        info!("BCON_CONFIG override (bypassing layered merge): {}", p);
                        return c;
                    }
                    Err(e) => warn!("BCON_CONFIG load failed ({}): {}", p, e),
                }
            } else {
                warn!("BCON_CONFIG points to non-existent path: {}", p);
            }
        }

        let user_path = dirs::config_dir().map(|d| d.join("bcon").join("config.toml"));
        Self::load_layered_from_paths(
            std::path::Path::new(Self::SYSTEM_CONFIG_PATH),
            user_path.as_deref(),
        )
    }

    /// Load configuration by merging multiple layers in priority order:
    ///
    /// 1. Builtin defaults from [`Config::default`] (always present).
    /// 2. System layer at `system` path (e.g. `/etc/bcon/config.toml`).
    /// 3. User layer at `user` path (e.g. `~/.config/bcon/config.toml`).
    ///
    /// Each successive layer overlays its keys on top of the merged tree using
    /// [`merge_value`]: tables merge recursively, while scalars, arrays, and
    /// `Option` fields replace wholesale. Missing files are silently skipped
    /// in this happy-path form; richer diagnostics (warn-on-missing system,
    /// warn-on-malformed) are layered on in a later cycle.
    ///
    /// This function takes explicit paths instead of resolving them itself so
    /// it can be unit-tested with `tempfile::tempdir()`. The production
    /// dispatcher that wires `BCON_CONFIG` and XDG paths into this entry
    /// point will be added in a follow-up cycle; until then, callers like
    /// [`Config::load`] still use the first-found-exclusive path.
    pub(crate) fn load_layered_from_paths(
        system: &std::path::Path,
        user: Option<&std::path::Path>,
    ) -> Self {
        let default_cfg = Self::default();
        // Config::default() is TOML-serializable by construction; failure here
        // would be a programming error in a Default impl, not a runtime input
        // problem, so we panic instead of warn-and-fall-back.
        let mut merged: toml::Value = toml::Value::try_from(&default_cfg)
            .expect("Config::default must serialize cleanly");

        if system.exists() {
            match parse_layer(system) {
                Ok(v) => {
                    info!("Loaded system layer: {}", system.display());
                    merge_value(&mut merged, v);
                }
                Err(e) => warn!(
                    "Skipping malformed system layer {}: {}",
                    system.display(),
                    e
                ),
            }
        } else {
            warn!(
                "System config not found at {}. Running with builtin defaults only. \
                 If you installed bcon via a package manager, please reinstall. \
                 Otherwise create it with: sudo bcon --init-config=system",
                system.display()
            );
        }

        if let Some(user) = user {
            if user.exists() {
                match parse_layer(user) {
                    Ok(v) => {
                        info!("Loaded user layer: {}", user.display());
                        merge_value(&mut merged, v);
                    }
                    Err(e) => warn!(
                        "Skipping malformed user layer {}: {}",
                        user.display(),
                        e
                    ),
                }
            }
        }

        merged.try_into::<Config>().unwrap_or_else(|e| {
            warn!(
                "Merged config failed to coerce ({}); falling back to builtin.",
                e
            );
            default_cfg
        })
    }

    /// Serialize `Config::default()` to a TOML string suitable for installation
    /// at `/etc/bcon/config.toml` by package distributors.
    ///
    /// Maintainers should use this output as the byte-identical content of
    /// the package-shipped site-default config so that the runtime merge
    /// floor (`Config::default()`) and the distributed template never drift.
    /// CI in distribution repositories should assert
    /// `Config::default_template()` round-trips through `toml::from_str`
    /// back into the committed template file.
    pub fn default_template() -> String {
        toml::to_string_pretty(&Self::default())
            .expect("Config::default must serialize")
    }

    /// Load settings from specified path
    fn load_from_file(path: &str) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path))?;
        let config: Config = toml::from_str(&content)
            .with_context(|| format!("Failed to parse config file: {}", path))?;
        Ok(config)
    }

    /// Write a config template to disk for the given target and preset list.
    ///
    /// `target` decides where the file is written (`User` =
    /// `~/.config/bcon/config.toml`, `System` = `/etc/bcon/config.toml`,
    /// or an explicit `Path`). `presets` are the keybind/font preset
    /// names (`default`, `vim`, `emacs`, `japanese`/`jp`). The target is
    /// no longer mixed into the preset list; callers obtain a parsed
    /// `WriteTarget` via `parse_init_config_arg`.
    pub fn write_config_with_preset(
        target: &WriteTarget,
        presets: &[&str],
    ) -> Result<PathBuf> {
        let config_path = match target {
            WriteTarget::System => {
                let system_dir = std::path::Path::new("/etc/bcon");
                std::fs::create_dir_all(system_dir)?;
                system_dir.join("config.toml")
            }
            WriteTarget::User => {
                let config_dir = dirs::config_dir()
                    .ok_or_else(|| anyhow::anyhow!("Config directory not found"))?;
                let bcon_dir = config_dir.join("bcon");
                std::fs::create_dir_all(&bcon_dir)?;
                bcon_dir.join("config.toml")
            }
            WriteTarget::Path(p) => {
                if let Some(parent) = p.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                p.clone()
            }
        };

        let mut keybinds = KeybindConfig::default_preset();
        let mut include_japanese = false;

        for p in presets {
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
                    // "default" or unknown - use default keybinds
                }
            }
        }

        let preset_name = if presets.is_empty() {
            "default".to_string()
        } else {
            presets.join(" + ")
        };

        let config_path_display = match target {
            WriteTarget::System => "/etc/bcon/config.toml".to_string(),
            WriteTarget::User => "~/.config/bcon/config.toml".to_string(),
            WriteTarget::Path(p) => p.display().to_string(),
        };

        // Serialize only keybinds (font settings use system defaults)
        let keybinds_toml = toml::to_string_pretty(&keybinds)?;

        // Build font section with auto-detected font names via fontconfig
        // Detected fonts are active, undetected ones are commented out with hints
        let font_section = {
            let finder = crate::font::fontconfig::FontFinder::new().ok();
            let mut lines = vec!["[font]".to_string()];

            // Main monospace font
            match finder.as_ref().and_then(|f| f.find_monospace()) {
                Some(m) => lines.push(format!("main = \"{}\"", m.family)),
                None => lines.push("# main = \"\"                    # Monospace font name or path (e.g. \"FiraCode\")".to_string()),
            }

            // Symbols / Nerd Font
            let nerd_font = finder
                .as_ref()
                .and_then(|f| f.find_nerd_font())
                .map(|m| m.family)
                .or_else(detect_nerd_font_path);
            match nerd_font {
                Some(name) => lines.push(format!("symbols = \"{}\"", name)),
                None => lines.push("# symbols = \"\"                 # Nerd Font for icons (e.g. \"Hack Nerd Font Mono\")".to_string()),
            }

            // CJK font
            match finder.as_ref().and_then(|f| f.find_cjk()) {
                Some(m) => lines.push(format!("cjk = \"{}\"", m.family)),
                None => lines.push("# cjk = \"\"                     # CJK font for Japanese/Chinese/Korean (e.g. \"Noto Sans CJK JP\")".to_string()),
            }

            // Emoji font
            match finder.as_ref().and_then(|f| f.find_emoji()) {
                Some(m) => lines.push(format!("emoji = \"{}\"", m.family)),
                None => lines.push("# emoji = \"\"                   # Color emoji font (e.g. \"Noto Color Emoji\")".to_string()),
            }

            lines.join("\n") + "\n"
        };

        // Japanese/CJK terminal settings
        let terminal_section = if include_japanese {
            r#"
[terminal]
ime = true
ime_disabled_apps = ["vim", "nvim", "vi", "vimdiff", "emacs", "nano", "less", "man", "htop", "top"]
"#
            .to_string()
        } else {
            String::new()
        };

        let template = format!(
            r#"# bcon configuration file
# Config path: {config_path_display}
# Keybind preset: {preset_name}
#
# Font settings are commented out by default.
# bcon will automatically find system fonts via fontconfig.
# You can specify fonts by family name (e.g. "FiraCode") or file path.

[keybinds]
{keybinds_toml}
{font_section}{terminal_section}
# =============================================================================
# Font Configuration (Optional)
# =============================================================================
# By default, bcon automatically finds fonts via fontconfig.
# Uncomment and customize if you want to use specific fonts.
#
# You can specify fonts by family name OR file path:
#   main = "FiraCode"                    # by name (resolved via fontconfig)
#   main = "/usr/share/fonts/.../X.ttf"  # by path (direct)
#
# Recommended monospace fonts:
#   1. FiraCode - ligature support, great for coding
#   2. JetBrains Mono - designed for developers
#   3. Hack - clean and readable
#   4. DejaVu Sans Mono - widely available
#
# [font]
# size = 16.0
# main = "FiraCode"
# cjk = "Noto Sans CJK JP"
# emoji = "Noto Color Emoji"
# symbols = "Hack Nerd Font Mono"
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
# scale = 1.0               # Display scale (1.0, 1.25, 1.5, 1.75, 2.0 for HiDPI)

# =============================================================================
# Color Scheme (Optional - ANSI 16 colors)
# =============================================================================
# Default: One Dark inspired (modern, muted colors)
#
# [colors]
# black = "282c34"
# red = "e06c75"
# green = "98c379"
# yellow = "e5c07b"
# blue = "61afef"
# magenta = "c678dd"
# cyan = "56b6c2"
# white = "abb2bf"
# bright_black = "5c6370"
# bright_red = "e06c75"
# bright_green = "98c379"
# bright_yellow = "e5c07b"
# bright_blue = "61afef"
# bright_magenta = "c678dd"
# bright_cyan = "56b6c2"
# bright_white = "ffffff"
#
# Popular color schemes:
#
# --- Dracula ---
# black = "21222c", red = "ff5555", green = "50fa7b", yellow = "f1fa8c"
# blue = "bd93f9", magenta = "ff79c6", cyan = "8be9fd", white = "f8f8f2"
# bright_black = "6272a4", bright_red = "ff6e6e", bright_green = "69ff94"
# bright_yellow = "ffffa5", bright_blue = "d6acff", bright_magenta = "ff92df"
# bright_cyan = "a4ffff", bright_white = "ffffff"
#
# --- Nord ---
# black = "3b4252", red = "bf616a", green = "a3be8c", yellow = "ebcb8b"
# blue = "81a1c1", magenta = "b48ead", cyan = "88c0d0", white = "e5e9f0"
# bright_black = "4c566a", bright_red = "bf616a", bright_green = "a3be8c"
# bright_yellow = "ebcb8b", bright_blue = "81a1c1", bright_magenta = "b48ead"
# bright_cyan = "8fbcbb", bright_white = "eceff4"
#
# --- Gruvbox Dark ---
# black = "282828", red = "cc241d", green = "98971a", yellow = "d79921"
# blue = "458588", magenta = "b16286", cyan = "689d6a", white = "a89984"
# bright_black = "928374", bright_red = "fb4934", bright_green = "b8bb26"
# bright_yellow = "fabd2f", bright_blue = "83a598", bright_magenta = "d3869b"
# bright_cyan = "8ec07c", bright_white = "ebdbb2"

# =============================================================================
# Terminal Settings (Optional)
# =============================================================================
# [terminal]
# scrollback_lines = 10000

# =============================================================================
# Keyboard Settings (Optional)
# =============================================================================
# [keyboard]
# repeat_delay = 400        # Key repeat delay in milliseconds
# repeat_rate = 30          # Key repeat rate in milliseconds
# xkb_layout = "us"         # XKB keyboard layout (e.g., "us", "jp", "de")
# xkb_variant = ""          # XKB layout variant (e.g., "dvorak", "nodeadkeys")
# xkb_options = ""          # XKB options (e.g., "ctrl:nocaps", "compose:ralt")

# =============================================================================
# Mouse Settings (Optional)
# =============================================================================
# [mouse]
# speed = 1.0                # Cursor speed multiplier (default: 1.0)
#                            # Recommended: 1.5-2.0 for 4K displays
# tap_to_click = true        # Enable tap-to-click on touchpads
# natural_scroll = false     # Enable natural (reverse) scrolling
# disable_while_typing = true # Disable touchpad while typing

# =============================================================================
# Display Settings (Optional)
# =============================================================================
# [drm]
# device = "auto"           # "auto" (default) or explicit path: "/dev/dri/card1"
#                            # Optimus laptops: set to Intel iGPU device
#                            #   ls -l /dev/dri/card* to find available devices
#                            #   udevadm info -a /dev/dri/card1 | grep -i vendor

# [display]
# prefer_external = true    # Prefer external monitors (HDMI/DP) over internal (eDP)
# auto_switch = true        # Auto-switch to external monitor on hotplug connect
# rotation = 0              # Display rotation in clockwise degrees: 0, 90, 180, 270
#                           # For tablets / portrait monitors whose panel ships
#                           # in a different physical orientation than expected.
#                           # 0 = no rotation (default). Applied via GL projection
#                           # transform (no hardware dependency).
#                           # Caveat: LCD subpixel rendering assumes the panel's
#                           # physical subpixel layout, so when rotation is 90/270
#                           # switch font.render_mode to "grayscale" or set
#                           # font.lcd_subpixel = "vrgb"/"vbgr" to match.
#                           # Changing this at runtime requires a restart.
#
# Connector priority (when prefer_external = true):
#   HDMI > DisplayPort > DVI > VGA > eDP (internal)
#
# Use case: Laptop with external monitor
#   - Connect external: auto-switch to external display
#   - Disconnect: auto-fallback to internal display
"#
        );

        std::fs::write(&config_path, template)?;
        Ok(config_path)
    }

    /// Write default config to file
    #[allow(dead_code)]
    pub fn write_default_config() -> Result<PathBuf> {
        Self::write_config_with_preset(&WriteTarget::User, &[])
    }

    /// Get the config file path for a given write target (without writing).
    pub fn path_for_target(target: &WriteTarget) -> Result<PathBuf> {
        match target {
            WriteTarget::System => Ok(std::path::Path::new("/etc/bcon").join("config.toml")),
            WriteTarget::User => {
                let config_dir = dirs::config_dir()
                    .ok_or_else(|| anyhow::anyhow!("Config directory not found"))?;
                Ok(config_dir.join("bcon").join("config.toml"))
            }
            WriteTarget::Path(p) => Ok(p.clone()),
        }
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
        // notify emits absolute paths, including when BCON_CONFIG was relative.
        // Preserve symlink names so atomic replacement still matches this path.
        let config_path = if config_path.is_absolute() {
            config_path.to_path_buf()
        } else {
            std::env::current_dir()?.join(config_path)
        };
        let watched_config = config_path.clone();

        let mut watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
            if let Ok(event) = res {
                // The parent directory is watched to catch atomic renames, but
                // changes to sibling files must not reload the config. In
                // particular, a log in that directory would feed back forever.
                if !event.paths.iter().any(|path| path == &watched_config) {
                    return;
                }
                // Detect Modify, Create, and Rename events
                // (editors often save by writing to temp file then rename)
                use notify::EventKind;
                match event.kind {
                    EventKind::Modify(_) | EventKind::Create(_) => {
                        let _ = tx.send(());
                    }
                    _ => {}
                }
            }
        })?;

        // Watch the parent directory to catch rename operations
        let watch_path = config_path.parent().unwrap_or(&config_path);
        watcher.watch(watch_path, RecursiveMode::NonRecursive)?;

        Ok(Self {
            _watcher: watcher,
            rx,
        })
    }

    /// Check if config file was modified (non-blocking)
    pub fn check_reload(&self) -> bool {
        let mut changed = false;
        while self.rx.try_recv().is_ok() {
            changed = true;
        }
        changed
    }
}

/// Get default config file path
pub fn default_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("bcon").join("config.toml"))
}

/// Detect Nerd Font path by checking common installation locations
pub fn detect_nerd_font_path() -> Option<String> {
    // Common Nerd Font paths on Debian/Ubuntu
    let candidates = [
        // Hack Nerd Font (apt: fonts-hack-nerd)
        "/usr/share/fonts/truetype/hack-nerd/HackNerdFontMono-Regular.ttf",
        "/usr/share/fonts/truetype/hack-nerd/HackNerdFont-Regular.ttf",
        "/usr/share/fonts/truetype/hack/HackNerdFontMono-Regular.ttf",
        // FiraCode Nerd Font
        "/usr/share/fonts/truetype/firacode-nerd/FiraCodeNerdFontMono-Regular.ttf",
        "/usr/share/fonts/truetype/firacode-nerd/FiraCodeNerdFont-Regular.ttf",
        // JetBrainsMono Nerd Font
        "/usr/share/fonts/truetype/jetbrains-mono-nerd/JetBrainsMonoNerdFontMono-Regular.ttf",
        "/usr/share/fonts/truetype/jetbrains-mono-nerd/JetBrainsMonoNerdFont-Regular.ttf",
    ];

    for path in candidates {
        if std::path::Path::new(path).exists() {
            info!("Detected Nerd Font: {}", path);
            return Some(path.to_string());
        }
    }

    // Check system-wide and user font directories for manually installed Nerd Fonts
    let nerd_font_patterns = [
        "HackNerdFont",
        "FiraCodeNerdFont",
        "JetBrainsMonoNerdFont",
        "DejaVuSansMNerdFont",
    ];

    let mut font_dirs: Vec<std::path::PathBuf> = vec![
        // System-wide (README recommends installing here)
        std::path::PathBuf::from("/usr/local/share/fonts"),
    ];

    // User fonts directory (~/.local/share/fonts)
    if let Some(data_dir) = dirs::data_dir() {
        font_dirs.push(data_dir.join("fonts"));
    }

    for font_dir in &font_dirs {
        if let Ok(entries) = std::fs::read_dir(font_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                for pattern in nerd_font_patterns {
                    if name.contains(pattern) && name.ends_with(".ttf") {
                        let path = entry.path().to_string_lossy().to_string();
                        info!("Detected Nerd Font: {}", path);
                        return Some(path);
                    }
                }
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn config_watcher_detects_relative_path_and_atomic_replace() {
        let cwd = std::env::current_dir().unwrap();
        let dir = tempfile::tempdir_in(&cwd).unwrap();
        let config_path = dir.path().join("config.toml");
        std::fs::write(&config_path, "[display]\nrotation = 0\n").unwrap();
        let watcher = ConfigWatcher::new(config_path.strip_prefix(&cwd).unwrap()).unwrap();

        let replacement = dir.path().join("replacement.toml");
        std::fs::write(&replacement, "[display]\nrotation = 90\n").unwrap();
        std::fs::rename(&replacement, &config_path).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !watcher.check_reload() {
            assert!(
                std::time::Instant::now() < deadline,
                "atomic save of relative config was not detected"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn config_watcher_ignores_sibling_log_updates() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let log_path = dir.path().join("bcon.log");
        std::fs::write(&config_path, "[display]\nrotation = 0\n").unwrap();
        let watcher = ConfigWatcher::new(&config_path).unwrap();

        std::fs::write(&log_path, "first log line\n").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(150));
        assert!(!watcher.check_reload(), "a sibling log triggered config reload");

        std::fs::write(&config_path, "[display]\nrotation = 90\n").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let mut detected = watcher.check_reload();
        while !detected && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
            detected = watcher.check_reload();
        }
        assert!(detected, "config edit was not detected");
    }

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

    #[test]
    fn test_merge_recursive_table() {
        // Tables are merged recursively: overlay's nested fields override base's
        // matching fields, while sibling keys not present in overlay are preserved.
        let mut base: toml::Value = toml::from_str(
            r#"
[font]
size = 14

[keybinds]
copy = "x"
"#,
        )
        .unwrap();

        let overlay: toml::Value = toml::from_str(
            r#"
[font]
size = 18
"#,
        )
        .unwrap();

        merge_value(&mut base, overlay);

        let font = base.get("font").and_then(|v| v.as_table()).unwrap();
        assert_eq!(font.get("size").and_then(|v| v.as_integer()), Some(18));

        let keybinds = base.get("keybinds").and_then(|v| v.as_table()).unwrap();
        assert_eq!(
            keybinds.get("copy").and_then(|v| v.as_str()),
            Some("x"),
            "sibling key absent in overlay must be preserved"
        );
    }

    #[test]
    fn test_merge_array_replaces() {
        // Arrays replace wholesale (no append/dedup). Keys missing in base
        // are inserted from overlay.
        let mut base: toml::Value = toml::from_str(
            r#"
items = ["a", "b"]
"#,
        )
        .unwrap();

        let overlay: toml::Value = toml::from_str(
            r#"
items = ["foo"]
lcd_weights = [10, 20, 30, 40, 50]
"#,
        )
        .unwrap();

        merge_value(&mut base, overlay);

        let items = base.get("items").and_then(|v| v.as_array()).unwrap();
        assert_eq!(items.len(), 1, "array must be replaced, not appended");
        assert_eq!(items[0].as_str(), Some("foo"));

        let weights = base.get("lcd_weights").and_then(|v| v.as_array()).unwrap();
        assert_eq!(weights.len(), 5);
        assert_eq!(weights[0].as_integer(), Some(10));
        assert_eq!(weights[4].as_integer(), Some(50));
    }

    #[test]
    fn test_load_multi_layer_merge() {
        // Multi-layer merge: builtin defaults <- system layer <- user layer.
        // The user layer must win on overlapping keys, while keys absent from
        // both files must fall back to Config::default() values transparently.
        use std::fs;

        let tmp = tempfile::tempdir().expect("tempdir must succeed");
        let system_path = tmp.path().join("etc-bcon-config.toml");
        let user_path = tmp.path().join("user-bcon-config.toml");

        fs::write(&system_path, "[font]\nsize = 20\n").unwrap();
        fs::write(&user_path, "[font]\nsize = 24\n").unwrap();

        let cfg = Config::load_layered_from_paths(&system_path, Some(&user_path));

        // User layer wins for the overlapping key.
        assert!(
            (cfg.font.size - 24.0).abs() < f32::EPSILON,
            "user layer must win on font.size, got {}",
            cfg.font.size
        );

        // Builtin transparency: an unrelated field equals Default::default().
        let default_terminal = TerminalConfig::default();
        assert_eq!(
            cfg.terminal.scrollback_lines, default_terminal.scrollback_lines,
            "fields absent from both layers must fall back to builtin default"
        );
    }

    #[test]
    fn test_load_no_system_layer() {
        // When the system layer file does not exist (and no user layer is
        // provided), the loader must degrade gracefully to builtin defaults.
        // No panic, no error return — just `Config::default()` equivalents.
        let tmp = tempfile::tempdir().expect("tempdir must succeed");
        let system_path = tmp.path().join("nonexistent-etc-bcon-config.toml");
        assert!(!system_path.exists(), "precondition: system path must not exist");

        let cfg = Config::load_layered_from_paths(&system_path, None);

        // Representative leaf fields should match Config::default() values.
        let default_font = FontConfig::default();
        assert!(
            (cfg.font.size - default_font.size).abs() < f32::EPSILON,
            "font.size must fall back to builtin default, got {}",
            cfg.font.size
        );

        let default_terminal = TerminalConfig::default();
        assert_eq!(
            cfg.terminal.scrollback_lines, default_terminal.scrollback_lines,
            "terminal.scrollback_lines must fall back to builtin default"
        );
    }

    #[test]
    fn test_load_malformed_user_layer() {
        // A malformed user layer must be skipped (with a warn log) so that the
        // effective config remains builtin + system. The system value must win
        // on the overlapping key, and the call must not panic.
        use std::fs;

        let tmp = tempfile::tempdir().expect("tempdir must succeed");
        let system_path = tmp.path().join("system.toml");
        let user_path = tmp.path().join("user.toml");

        fs::write(&system_path, "[font]\nsize = 20\n").unwrap();
        // Invalid TOML: bare punctuation cannot be parsed as a key/value or table.
        fs::write(&user_path, "!@#\n").unwrap();

        let cfg = Config::load_layered_from_paths(&system_path, Some(&user_path));

        // System layer's font.size must win because the user layer was skipped.
        assert!(
            (cfg.font.size - 20.0).abs() < f32::EPSILON,
            "system layer must win when user layer is malformed, got {}",
            cfg.font.size
        );
    }

    #[test]
    fn test_load_bcon_config_env_bypasses_layers() {
        // BCON_CONFIG, when set and pointing to an existing file, must bypass
        // the layered merge entirely and load that single file. Other layers
        // (system /etc, user ~/.config) are deliberately ignored even if they
        // exist. This is the debug/test override semantics.
        //
        // env::set_var/remove_var pollute the process; serialize against any
        // future env-touching tests using a static Mutex. Save and restore the
        // previous value so we do not leak test state into other tests or the
        // surrounding cargo invocation.
        use std::fs;
        use std::sync::Mutex;
        static ENV_LOCK: Mutex<()> = Mutex::new(());
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());

        let tmp = tempfile::tempdir().expect("tempdir must succeed");
        let bcon_config = tmp.path().join("override.toml");
        fs::write(&bcon_config, "[font]\nsize = 99\n").unwrap();

        let prev = std::env::var_os("BCON_CONFIG");
        std::env::set_var("BCON_CONFIG", &bcon_config);

        let cfg = Config::load();

        if let Some(p) = prev {
            std::env::set_var("BCON_CONFIG", p);
        } else {
            std::env::remove_var("BCON_CONFIG");
        }

        // BCON_CONFIG file's font.size = 99 must win even though no system or
        // user layer was set up; default for everything else stays builtin.
        assert!(
            (cfg.font.size - 99.0).abs() < f32::EPSILON,
            "BCON_CONFIG must bypass layers and override font.size, got {}",
            cfg.font.size
        );
    }

    #[test]
    fn test_default_template_round_trips() {
        // Config::default_template() must produce a TOML string that parses
        // cleanly back into a Config equivalent to Config::default(). This
        // is the upstream-side check that protects distributors against
        // silent drift between the Default impl and any package-shipped
        // /etc/bcon/config.toml template.
        let rendered = Config::default_template();
        let parsed: Config = toml::from_str(&rendered)
            .expect("default_template output must round-trip through toml::from_str");
        let default = Config::default();

        // Spot-check representative leaf fields across multiple sections.
        assert!((parsed.font.size - default.font.size).abs() < f32::EPSILON);
        assert_eq!(
            parsed.terminal.scrollback_lines,
            default.terminal.scrollback_lines
        );
        assert_eq!(parsed.keybinds.copy, default.keybinds.copy);
    }

    #[test]
    fn test_parse_init_config_arg_cases() {
        // Empty arg defaults to user with no presets (legacy bare --init-config).
        assert_eq!(
            parse_init_config_arg(""),
            (WriteTarget::User, Vec::<String>::new())
        );

        // Legacy backward-compat: tokens without a recognized target keyword
        // route to user (this is the form documented in README pre-Cycle 6).
        assert_eq!(
            parse_init_config_arg("vim,jp"),
            (
                WriteTarget::User,
                vec!["vim".to_string(), "jp".to_string()]
            )
        );

        // Explicit "system" target with no presets.
        assert_eq!(
            parse_init_config_arg("system"),
            (WriteTarget::System, Vec::<String>::new())
        );

        // Explicit "system" target plus presets — backward compat with the
        // original --init-config=system,vim,jp form.
        assert_eq!(
            parse_init_config_arg("system,vim,jp"),
            (
                WriteTarget::System,
                vec!["vim".to_string(), "jp".to_string()]
            )
        );

        // Explicit "user" token (Cycle 6 addition for unambiguous root use).
        assert_eq!(
            parse_init_config_arg("user,emacs"),
            (WriteTarget::User, vec!["emacs".to_string()])
        );

        // Absolute path token.
        assert_eq!(
            parse_init_config_arg("/tmp/x.toml,vim"),
            (
                WriteTarget::Path(std::path::PathBuf::from("/tmp/x.toml")),
                vec!["vim".to_string()]
            )
        );

        // Path token without trailing presets.
        assert_eq!(
            parse_init_config_arg("/tmp/x.toml"),
            (
                WriteTarget::Path(std::path::PathBuf::from("/tmp/x.toml")),
                Vec::<String>::new()
            )
        );

        // Whitespace-tolerant tokens: parse_init_config_arg trims each split.
        assert_eq!(
            parse_init_config_arg(" system , vim , jp "),
            (
                WriteTarget::System,
                vec!["vim".to_string(), "jp".to_string()]
            )
        );
    }

    #[test]
    fn test_expand_tilde_basics() {
        // Bare absolute path passes through.
        assert_eq!(
            expand_tilde("/etc/bcon/config.toml"),
            std::path::PathBuf::from("/etc/bcon/config.toml")
        );

        // ~/foo gets the home directory prepended (when one is detectable).
        if let Some(home) = dirs::home_dir() {
            assert_eq!(expand_tilde("~/.config/bcon/config.toml"),
                       home.join(".config/bcon/config.toml"));
        }
    }
}
