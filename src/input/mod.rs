//! Input handling
//!
//! Manage keyboard input.
//! - Raw input via TTY stdin (fallback for SSH development)
//! - Direct input via evdev + xkbcommon (for DRM console)
//! - fcitx5 D-Bus IME integration (Japanese input)

pub mod evdev;
pub mod ime;
pub mod keyboard;

pub use evdev::{
    EvdevKeyboard, KeyboardConfig, MouseEvent, BTN_LEFT, BTN_MIDDLE, BTN_RIGHT,
    keysym_to_bytes_with_mods,
};
pub use keyboard::Keyboard;
