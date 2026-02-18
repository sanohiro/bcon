//! Color parsing utilities
//!
//! Consolidates hex color parsing from config and terminal modules.

#![allow(dead_code)]

/// Parse 6-digit hex color (e.g., "ff0000" -> (255, 0, 0))
/// Also supports 3-digit short format (e.g., "f00" -> (255, 0, 0))
/// Returns None on invalid input.
pub fn parse_hex_color(hex: &str) -> Option<(u8, u8, u8)> {
    let hex = hex.trim_start_matches('#');
    match hex.len() {
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some((r, g, b))
        }
        3 => {
            // Short format: expand F -> FF
            let r = u8::from_str_radix(&hex[0..1], 16).ok()? * 17;
            let g = u8::from_str_radix(&hex[1..2], 16).ok()? * 17;
            let b = u8::from_str_radix(&hex[2..3], 16).ok()? * 17;
            Some((r, g, b))
        }
        _ => None,
    }
}

/// Parse hex color string (RRGGBB) to normalized RGB tuple (0.0-1.0)
/// Returns (0.0, 0.0, 0.0) on invalid input.
pub fn parse_hex_color_to_f32(hex: &str) -> (f32, f32, f32) {
    match parse_hex_color(hex) {
        Some((r, g, b)) => (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0),
        None => (0.0, 0.0, 0.0),
    }
}

/// Parse hex color to [f32; 4] RGBA (alpha = 1.0)
/// Returns white [1.0, 1.0, 1.0, 1.0] on invalid input.
pub fn parse_hex_color_to_rgba(hex: &str) -> [f32; 4] {
    match parse_hex_color(hex) {
        Some((r, g, b)) => [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0],
        None => [1.0, 1.0, 1.0, 1.0],
    }
}

/// Parse OSC color specification formats.
///
/// Supported formats:
/// - rgb:RR/GG/BB (X11 format, 8-bit per component)
/// - rgb:RRRR/GGGG/BBBB (X11 format, 16-bit per component)
/// - #RRGGBB (hex format)
/// - #RGB (short hex format)
pub fn parse_osc_color(data: &[u8]) -> Option<(u8, u8, u8)> {
    let s = std::str::from_utf8(data).ok()?;

    if let Some(hex) = s.strip_prefix('#') {
        // #RRGGBB or #RGB format
        parse_hex_color(hex)
    } else if let Some(rgb) = s.strip_prefix("rgb:") {
        // rgb:RRRR/GGGG/BBBB or rgb:RR/GG/BB format
        let parts: Vec<&str> = rgb.split('/').collect();
        if parts.len() != 3 {
            return None;
        }

        // Parse each component (use high byte of 16-bit value)
        let parse_component = |s: &str| -> Option<u8> {
            let v = u16::from_str_radix(s, 16).ok()?;
            match s.len() {
                4 => Some((v >> 8) as u8), // 16-bit: use high byte
                2 => Some(v as u8),        // 8-bit: use as-is
                1 => Some((v as u8) * 17), // 4-bit: expand
                _ => None,
            }
        };

        let r = parse_component(parts[0])?;
        let g = parse_component(parts[1])?;
        let b = parse_component(parts[2])?;
        Some((r, g, b))
    } else {
        None
    }
}

/// Blend two colors with alpha compositing.
///
/// # Arguments
/// * `base` - Background color [r, g, b, a]
/// * `overlay` - Foreground color [r, g, b, a]
/// * `alpha` - Blend factor (0.0 = all base, 1.0 = all overlay)
///
/// # Returns
/// Blended color with base alpha preserved.
pub fn blend_colors(base: [f32; 4], overlay: [f32; 4], alpha: f32) -> [f32; 4] {
    [
        overlay[0] * alpha + base[0] * (1.0 - alpha),
        overlay[1] * alpha + base[1] * (1.0 - alpha),
        overlay[2] * alpha + base[2] * (1.0 - alpha),
        base[3],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hex_color() {
        assert_eq!(parse_hex_color("ff0000"), Some((255, 0, 0)));
        assert_eq!(parse_hex_color("00ff00"), Some((0, 255, 0)));
        assert_eq!(parse_hex_color("0000ff"), Some((0, 0, 255)));
        assert_eq!(parse_hex_color("#ff0000"), Some((255, 0, 0)));
        assert_eq!(parse_hex_color("f00"), Some((255, 0, 0)));
        assert_eq!(parse_hex_color("#f00"), Some((255, 0, 0)));
        assert_eq!(parse_hex_color("invalid"), None);
    }

    #[test]
    fn test_parse_osc_color() {
        assert_eq!(parse_osc_color(b"#ff0000"), Some((255, 0, 0)));
        assert_eq!(parse_osc_color(b"rgb:ff/00/00"), Some((255, 0, 0)));
        assert_eq!(parse_osc_color(b"rgb:ffff/0000/0000"), Some((255, 0, 0)));
    }
}
