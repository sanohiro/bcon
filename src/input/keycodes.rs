//! evdev keycode constants
//!
//! Consolidates all evdev key constants used throughout bcon.
//! These are Linux input event codes from <linux/input-event-codes.h>.

#![allow(dead_code)]

// ============================================================================
// Modifier Keys
// ============================================================================

/// Left Control key
pub const KEY_LEFTCTRL: u32 = 29;

/// Right Control key
pub const KEY_RIGHTCTRL: u32 = 97;

/// Left Shift key
pub const KEY_LEFTSHIFT: u32 = 42;

/// Right Shift key
pub const KEY_RIGHTSHIFT: u32 = 54;

/// Left Alt key
pub const KEY_LEFTALT: u32 = 56;

/// Right Alt key (AltGr on some keyboards)
pub const KEY_RIGHTALT: u32 = 100;

// ============================================================================
// Navigation Keys
// ============================================================================

/// Left arrow key
pub const KEY_LEFT: u32 = 105;

/// Right arrow key
pub const KEY_RIGHT: u32 = 106;

/// Up arrow key
pub const KEY_UP: u32 = 103;

/// Down arrow key
pub const KEY_DOWN: u32 = 108;

/// Home key
pub const KEY_HOME: u32 = 102;

/// End key
pub const KEY_END: u32 = 107;

/// Page Up key
pub const KEY_PAGEUP: u32 = 104;

/// Page Down key
pub const KEY_PAGEDOWN: u32 = 109;

/// Insert key
pub const KEY_INSERT: u32 = 110;

/// Delete key
pub const KEY_DELETE: u32 = 111;

// ============================================================================
// Function Keys
// ============================================================================

/// F1 key
pub const KEY_F1: u32 = 59;

/// F2 key
pub const KEY_F2: u32 = 60;

/// F3 key
pub const KEY_F3: u32 = 61;

/// F4 key
pub const KEY_F4: u32 = 62;

/// F5 key
pub const KEY_F5: u32 = 63;

/// F6 key
pub const KEY_F6: u32 = 64;

/// F7 key
pub const KEY_F7: u32 = 65;

/// F8 key
pub const KEY_F8: u32 = 66;

/// F9 key
pub const KEY_F9: u32 = 67;

/// F10 key
pub const KEY_F10: u32 = 68;

/// F11 key
pub const KEY_F11: u32 = 87;

/// F12 key
pub const KEY_F12: u32 = 88;

// ============================================================================
// Mouse Buttons (BTN_* from linux/input-event-codes.h)
// ============================================================================

/// Left mouse button
pub const BTN_LEFT: u32 = 0x110;

/// Right mouse button
pub const BTN_RIGHT: u32 = 0x111;

/// Middle mouse button
pub const BTN_MIDDLE: u32 = 0x112;

// ============================================================================
// Helper Functions
// ============================================================================

/// Check if keycode is a modifier key
#[inline]
pub const fn is_modifier_key(keycode: u32) -> bool {
    matches!(
        keycode,
        KEY_LEFTSHIFT | KEY_RIGHTSHIFT | KEY_LEFTCTRL | KEY_RIGHTCTRL | KEY_LEFTALT | KEY_RIGHTALT
    )
}

/// Check if keycode is a Shift key
#[inline]
pub const fn is_shift_key(keycode: u32) -> bool {
    keycode == KEY_LEFTSHIFT || keycode == KEY_RIGHTSHIFT
}

/// Check if keycode is a Ctrl key
#[inline]
pub const fn is_ctrl_key(keycode: u32) -> bool {
    keycode == KEY_LEFTCTRL || keycode == KEY_RIGHTCTRL
}

/// Check if keycode is an Alt key
#[inline]
pub const fn is_alt_key(keycode: u32) -> bool {
    keycode == KEY_LEFTALT || keycode == KEY_RIGHTALT
}

/// Convert function key code to function key number (1-12)
/// Returns None if not a function key
#[inline]
pub const fn function_key_number(keycode: u32) -> Option<u8> {
    match keycode {
        KEY_F1 => Some(1),
        KEY_F2 => Some(2),
        KEY_F3 => Some(3),
        KEY_F4 => Some(4),
        KEY_F5 => Some(5),
        KEY_F6 => Some(6),
        KEY_F7 => Some(7),
        KEY_F8 => Some(8),
        KEY_F9 => Some(9),
        KEY_F10 => Some(10),
        KEY_F11 => Some(11),
        KEY_F12 => Some(12),
        _ => None,
    }
}
