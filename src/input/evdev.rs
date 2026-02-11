//! evdev input handling
//!
//! Use libinput + xkbcommon to read keyboard/mouse events directly from /dev/input/eventN.
//! Handle physical input on DRM console.

#![allow(dead_code)]

use anyhow::{anyhow, Result};
use input::event::keyboard::{KeyState, KeyboardEventTrait};
use input::event::pointer::{Axis, ButtonState, PointerScrollEvent};
use input::event::{Event, PointerEvent};
use input::{Libinput, LibinputInterface};
use log::{debug, info, warn};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::{AsRawFd, OwnedFd};
use std::path::Path;
use std::time::{Duration, Instant};
use xkbcommon::xkb;
use xkbcommon::xkb::keysyms;

#[cfg(all(target_os = "linux", feature = "seatd"))]
use std::cell::RefCell;
#[cfg(all(target_os = "linux", feature = "seatd"))]
use std::rc::Rc;
#[cfg(all(target_os = "linux", feature = "seatd"))]
use crate::session::SeatSession;

use crate::config::KeyboardInputConfig;

/// LibinputInterface implementation for libinput
struct InputInterface;

impl LibinputInterface for InputInterface {
    fn open_restricted(&mut self, path: &Path, flags: i32) -> std::result::Result<OwnedFd, i32> {
        let f = OpenOptions::new()
            .read(true)
            .write((flags & libc::O_WRONLY != 0) || (flags & libc::O_RDWR != 0))
            .custom_flags(flags & !libc::O_WRONLY & !libc::O_RDWR & !libc::O_RDONLY)
            .open(path)
            .map_err(|e| {
                warn!("Cannot open device: {:?}: {}", path, e);
                e.raw_os_error().unwrap_or(-libc::ENOENT)
            })?;
        Ok(OwnedFd::from(f))
    }

    fn close_restricted(&mut self, fd: OwnedFd) {
        drop(fd);
    }
}

/// LibinputInterface implementation using libseat for device access
#[cfg(all(target_os = "linux", feature = "seatd"))]
struct SeatInputInterface {
    session: Rc<RefCell<SeatSession>>,
}

#[cfg(all(target_os = "linux", feature = "seatd"))]
impl LibinputInterface for SeatInputInterface {
    fn open_restricted(&mut self, path: &Path, _flags: i32) -> std::result::Result<OwnedFd, i32> {
        let mut session = self.session.borrow_mut();
        match session.open_device(path) {
            Ok(device) => Ok(device.fd),
            Err(e) => {
                warn!("libseat: Cannot open device {:?}: {}", path, e);
                Err(-libc::EACCES)
            }
        }
    }

    fn close_restricted(&mut self, fd: OwnedFd) {
        // Device is closed when OwnedFd is dropped
        drop(fd);
    }
}

/// Raw key event to send to IME
#[derive(Clone)]
pub struct RawKeyEvent {
    pub keysym: u32,
    /// evdev keycode (no xkb offset)
    pub keycode: u32,
    /// xkb modifier state (STATE_MODS_EFFECTIVE)
    pub xkb_state: u32,
    /// xkbcommon UTF-8 output
    pub utf8: String,
    /// true=press, false=release
    pub is_press: bool,
    /// Shift key pressed
    pub mods_shift: bool,
    /// Ctrl key pressed
    pub mods_ctrl: bool,
    /// Alt key pressed
    pub mods_alt: bool,
}

/// Mouse event
#[derive(Debug, Clone)]
pub enum MouseEvent {
    /// Mouse move (absolute coordinates after accumulation)
    Move { x: f64, y: f64 },
    /// Button press
    ButtonPress { button: u32, x: f64, y: f64 },
    /// Button release
    ButtonRelease { button: u32, x: f64, y: f64 },
    /// Scroll (negative=up, positive=down)
    Scroll { delta: f64, x: f64, y: f64 },
}

/// libinput mouse button numbers
pub const BTN_LEFT: u32 = 0x110;
pub const BTN_RIGHT: u32 = 0x111;
pub const BTN_MIDDLE: u32 = 0x112;

// evdev modifier keycodes
const KEY_LEFTCTRL: u32 = 29;
const KEY_RIGHTCTRL: u32 = 97;
const KEY_LEFTSHIFT: u32 = 42;
const KEY_RIGHTSHIFT: u32 = 54;
const KEY_LEFTALT: u32 = 56;
const KEY_RIGHTALT: u32 = 100;

/// evdev input management (keyboard + mouse)
pub struct EvdevKeyboard {
    /// libinput context
    input: Libinput,
    /// xkbcommon keyboard state
    xkb_state: xkb::State,
    /// libinput raw fd (for future poll/epoll integration)
    #[allow(dead_code)]
    fd: i32,
    /// Shift physical key pressed tracking
    shift_pressed: bool,
    /// Ctrl physical key pressed tracking
    ctrl_pressed: bool,
    /// Alt physical key pressed tracking
    alt_pressed: bool,
    /// Mouse X coordinate (pixels)
    mouse_x: f64,
    /// Mouse Y coordinate (pixels)
    mouse_y: f64,
    /// Screen width (for mouse coordinate clamping)
    screen_width: f64,
    /// Screen height (for mouse coordinate clamping)
    screen_height: f64,
    /// Scroll accumulator (accumulate until reaching line units)
    scroll_accum: f64,
    /// Held keys: keycode -> (press time, next repeat time, RawKeyEvent)
    held_keys: HashMap<u32, (Instant, Instant, RawKeyEvent)>,
    /// Key repeat delay (ms)
    repeat_delay_ms: u64,
    /// Key repeat rate (ms)
    repeat_rate_ms: u64,
}

impl EvdevKeyboard {
    /// Initialize evdev input
    ///
    /// Scan /dev/input/event* and add devices to libinput.
    /// Set up keymap with xkbcommon.
    /// screen_width/height used for mouse coordinate clamping.
    pub fn new(screen_width: u32, screen_height: u32, kb_config: &KeyboardInputConfig) -> Result<Self> {
        // Create libinput context
        let mut input = Libinput::new_from_path(InputInterface);

        // Scan and add devices from /dev/input/event*
        let mut device_count = 0;
        for entry in std::fs::read_dir("/dev/input")
            .map_err(|e| anyhow!("Cannot scan /dev/input: {}", e))?
        {
            let entry = entry?;
            let path = entry.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with("event") {
                let path_str = path.to_str().unwrap_or("");
                if let Some(_device) = input.path_add_device(path_str) {
                    debug!("Input device added: {}", path_str);
                    device_count += 1;
                }
            }
        }

        if device_count == 0 {
            return Err(anyhow!(
                "No input devices found. Check permissions for /dev/input/event*."
            ));
        }

        info!("evdev: {} input devices added", device_count);

        // Get libinput fd
        let fd = input.as_raw_fd();

        // Set fd to non-blocking
        let flags = nix::fcntl::fcntl(fd, nix::fcntl::FcntlArg::F_GETFL)
            .map_err(|e| anyhow!("F_GETFL failed: {}", e))?;
        let mut flags = nix::fcntl::OFlag::from_bits_truncate(flags);
        flags.insert(nix::fcntl::OFlag::O_NONBLOCK);
        nix::fcntl::fcntl(fd, nix::fcntl::FcntlArg::F_SETFL(flags))
            .map_err(|e| anyhow!("F_SETFL failed: {}", e))?;

        // Initialize xkbcommon with config settings
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);

        // Use config values or empty string for default
        let rules = "";  // Always use default rules
        let model = if kb_config.xkb_model.is_empty() { "" } else { &kb_config.xkb_model };
        let layout = if kb_config.xkb_layout.is_empty() { "" } else { &kb_config.xkb_layout };
        let variant = if kb_config.xkb_variant.is_empty() { "" } else { &kb_config.xkb_variant };
        let options = if kb_config.xkb_options.is_empty() { None } else { Some(kb_config.xkb_options.clone()) };
        let options_for_error = options.clone();

        let keymap = xkb::Keymap::new_from_names(
            &context,
            rules,
            model,
            layout,
            variant,
            options,
            xkb::COMPILE_NO_FLAGS,
        )
        .ok_or_else(|| anyhow!("Failed to create xkb keymap (model={}, layout={}, variant={}, options={:?})",
            model, layout, variant, options_for_error))?;

        let xkb_state = xkb::State::new(&keymap);

        info!("evdev keyboard initialized (layout={}, repeat_delay={}ms, repeat_rate={}ms)",
            if layout.is_empty() { "default" } else { layout },
            kb_config.repeat_delay, kb_config.repeat_rate);

        Ok(Self {
            input,
            xkb_state,
            fd,
            shift_pressed: false,
            ctrl_pressed: false,
            alt_pressed: false,
            mouse_x: screen_width as f64 / 2.0,
            mouse_y: screen_height as f64 / 2.0,
            screen_width: screen_width as f64,
            screen_height: screen_height as f64,
            scroll_accum: 0.0,
            held_keys: HashMap::new(),
            repeat_delay_ms: kb_config.repeat_delay,
            repeat_rate_ms: kb_config.repeat_rate,
        })
    }

    /// Initialize evdev input with libseat session
    ///
    /// Uses libseat for device access (no root required).
    #[cfg(all(target_os = "linux", feature = "seatd"))]
    pub fn new_with_seat(
        screen_width: u32,
        screen_height: u32,
        session: Rc<RefCell<SeatSession>>,
        kb_config: &KeyboardInputConfig,
    ) -> Result<Self> {
        // Create libinput context with seat-based interface
        let interface = SeatInputInterface {
            session: session.clone(),
        };
        let mut input = Libinput::new_from_path(interface);

        // Scan and add devices from /dev/input/event*
        let mut device_count = 0;
        for entry in std::fs::read_dir("/dev/input")
            .map_err(|e| anyhow!("Cannot scan /dev/input: {}", e))?
        {
            let entry = entry?;
            let path = entry.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with("event") {
                let path_str = path.to_str().unwrap_or("");
                if let Some(_device) = input.path_add_device(path_str) {
                    debug!("Input device added via libseat: {}", path_str);
                    device_count += 1;
                }
            }
        }

        if device_count == 0 {
            return Err(anyhow!(
                "No input devices found via libseat. Check seatd/logind permissions."
            ));
        }

        info!("evdev: {} input devices added via libseat", device_count);

        // Get libinput fd
        let fd = input.as_raw_fd();

        // Set fd to non-blocking
        let flags = nix::fcntl::fcntl(fd, nix::fcntl::FcntlArg::F_GETFL)
            .map_err(|e| anyhow!("F_GETFL failed: {}", e))?;
        let mut flags = nix::fcntl::OFlag::from_bits_truncate(flags);
        flags.insert(nix::fcntl::OFlag::O_NONBLOCK);
        nix::fcntl::fcntl(fd, nix::fcntl::FcntlArg::F_SETFL(flags))
            .map_err(|e| anyhow!("F_SETFL failed: {}", e))?;

        // Initialize xkbcommon with config settings
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);

        // Use config values or empty string for default
        let rules = "";  // Always use default rules
        let model = if kb_config.xkb_model.is_empty() { "" } else { &kb_config.xkb_model };
        let layout = if kb_config.xkb_layout.is_empty() { "" } else { &kb_config.xkb_layout };
        let variant = if kb_config.xkb_variant.is_empty() { "" } else { &kb_config.xkb_variant };
        let options = if kb_config.xkb_options.is_empty() { None } else { Some(kb_config.xkb_options.clone()) };
        let options_for_error = options.clone();

        let keymap = xkb::Keymap::new_from_names(
            &context,
            rules,
            model,
            layout,
            variant,
            options,
            xkb::COMPILE_NO_FLAGS,
        )
        .ok_or_else(|| anyhow!("Failed to create xkb keymap (model={}, layout={}, variant={}, options={:?})",
            model, layout, variant, options_for_error))?;

        let xkb_state = xkb::State::new(&keymap);

        info!("evdev keyboard initialized with libseat (layout={}, repeat_delay={}ms, repeat_rate={}ms)",
            if layout.is_empty() { "default" } else { layout },
            kb_config.repeat_delay, kb_config.repeat_rate);

        Ok(Self {
            input,
            xkb_state,
            fd,
            shift_pressed: false,
            ctrl_pressed: false,
            alt_pressed: false,
            mouse_x: screen_width as f64 / 2.0,
            mouse_y: screen_height as f64 / 2.0,
            screen_width: screen_width as f64,
            screen_height: screen_height as f64,
            scroll_accum: 0.0,
            held_keys: HashMap::new(),
            repeat_delay_ms: kb_config.repeat_delay,
            repeat_rate_ms: kb_config.repeat_rate,
        })
    }

    /// Return libinput fd (for poll)
    #[allow(dead_code)]
    pub fn fd(&self) -> i32 {
        self.fd
    }

    /// Process events and return bytes to forward to PTY
    #[allow(dead_code)]
    pub fn process_events(&mut self) -> Vec<u8> {
        let mut output = Vec::new();

        if let Err(e) = self.input.dispatch() {
            warn!("libinput dispatch error: {}", e);
            return output;
        }

        while let Some(event) = self.input.next() {
            if let Event::Keyboard(kb_event) = event {
                if let input::event::KeyboardEvent::Key(key_event) = kb_event {
                    let evdev_code = key_event.key();
                    // evdev keycode -> xkb keycode (evdev + 8)
                    let xkb_keycode = xkb::Keycode::new(evdev_code + 8);
                    let key_state = key_event.key_state();

                    // Get keysym and UTF-8 (before state update)
                    let sym = self.xkb_state.key_get_one_sym(xkb_keycode);
                    let utf8 = self.xkb_state.key_get_utf8(xkb_keycode);

                    // Update xkb state
                    let direction = match key_state {
                        KeyState::Pressed => xkb::KeyDirection::Down,
                        KeyState::Released => xkb::KeyDirection::Up,
                    };
                    self.xkb_state.update_key(xkb_keycode, direction);

                    // Process only key presses (releases only update state)
                    if key_state == KeyState::Pressed {
                        let bytes = keysym_to_bytes(sym, &utf8);
                        output.extend_from_slice(&bytes);
                    }
                }
            }
        }

        output
    }

    /// Return raw key events and mouse events
    ///
    /// Keyboard: Return `RawKeyEvent` (for input via IME)
    /// Mouse: Return `MouseEvent` (for selection and scrolling)
    pub fn process_raw_events(&mut self) -> (Vec<RawKeyEvent>, Vec<MouseEvent>) {
        let mut key_events = Vec::new();
        let mut mouse_events = Vec::new();

        if let Err(e) = self.input.dispatch() {
            warn!("libinput dispatch error: {}", e);
            return (key_events, mouse_events);
        }

        while let Some(event) = self.input.next() {
            match event {
                Event::Keyboard(kb_event) => {
                    if let input::event::KeyboardEvent::Key(key_event) = kb_event {
                        let evdev_code = key_event.key();
                        let xkb_keycode = xkb::Keycode::new(evdev_code + 8);
                        let key_state = key_event.key_state();

                        // Get keysym and UTF-8 (before state update)
                        let sym = self.xkb_state.key_get_one_sym(xkb_keycode);
                        let utf8 = self.xkb_state.key_get_utf8(xkb_keycode);

                        // Get modifier state
                        let mods = self.xkb_state.serialize_mods(xkb::STATE_MODS_EFFECTIVE);

                        // Track physical state of modifier keys
                        match (evdev_code, key_state) {
                            (KEY_LEFTSHIFT | KEY_RIGHTSHIFT, KeyState::Pressed) => {
                                self.shift_pressed = true
                            }
                            (KEY_LEFTSHIFT | KEY_RIGHTSHIFT, KeyState::Released) => {
                                self.shift_pressed = false
                            }
                            (KEY_LEFTCTRL | KEY_RIGHTCTRL, KeyState::Pressed) => {
                                self.ctrl_pressed = true
                            }
                            (KEY_LEFTCTRL | KEY_RIGHTCTRL, KeyState::Released) => {
                                self.ctrl_pressed = false
                            }
                            (KEY_LEFTALT | KEY_RIGHTALT, KeyState::Pressed) => {
                                self.alt_pressed = true
                            }
                            (KEY_LEFTALT | KEY_RIGHTALT, KeyState::Released) => {
                                self.alt_pressed = false
                            }
                            _ => {}
                        }

                        // Update xkb state
                        let direction = match key_state {
                            KeyState::Pressed => xkb::KeyDirection::Down,
                            KeyState::Released => xkb::KeyDirection::Up,
                        };
                        self.xkb_state.update_key(xkb_keycode, direction);

                        let raw_event = RawKeyEvent {
                            keysym: sym.raw(),
                            keycode: evdev_code,
                            xkb_state: mods,
                            utf8,
                            is_press: key_state == KeyState::Pressed,
                            mods_shift: self.shift_pressed,
                            mods_ctrl: self.ctrl_pressed,
                            mods_alt: self.alt_pressed,
                        };

                        // Key repeat tracking (modifiers don't repeat)
                        let is_modifier = matches!(
                            evdev_code,
                            KEY_LEFTSHIFT | KEY_RIGHTSHIFT |
                            KEY_LEFTCTRL | KEY_RIGHTCTRL |
                            56 | 100 |  // Alt
                            125 | 126 // Super
                        );

                        if key_state == KeyState::Pressed {
                            if !is_modifier {
                                let now = Instant::now();
                                let first_repeat = now + Duration::from_millis(self.repeat_delay_ms);
                                self.held_keys
                                    .insert(evdev_code, (now, first_repeat, raw_event.clone()));
                            }
                        } else {
                            self.held_keys.remove(&evdev_code);
                        }

                        key_events.push(raw_event);
                    }
                }
                Event::Pointer(ptr_event) => {
                    match ptr_event {
                        PointerEvent::Motion(m) => {
                            // Accumulate relative movement
                            self.mouse_x += m.dx();
                            self.mouse_y += m.dy();
                            // Clamp to screen bounds
                            self.mouse_x = self.mouse_x.clamp(0.0, self.screen_width - 1.0);
                            self.mouse_y = self.mouse_y.clamp(0.0, self.screen_height - 1.0);
                            mouse_events.push(MouseEvent::Move {
                                x: self.mouse_x,
                                y: self.mouse_y,
                            });
                        }
                        PointerEvent::MotionAbsolute(m) => {
                            // Absolute coordinates (touchpad, tablet, etc.)
                            self.mouse_x =
                                m.absolute_x_transformed(self.screen_width as u32) as f64;
                            self.mouse_y =
                                m.absolute_y_transformed(self.screen_height as u32) as f64;
                            mouse_events.push(MouseEvent::Move {
                                x: self.mouse_x,
                                y: self.mouse_y,
                            });
                        }
                        PointerEvent::Button(b) => {
                            let button = b.button();
                            debug!("Mouse button: {} state={:?}", button, b.button_state());
                            match b.button_state() {
                                ButtonState::Pressed => {
                                    mouse_events.push(MouseEvent::ButtonPress {
                                        button,
                                        x: self.mouse_x,
                                        y: self.mouse_y,
                                    });
                                }
                                ButtonState::Released => {
                                    mouse_events.push(MouseEvent::ButtonRelease {
                                        button,
                                        x: self.mouse_x,
                                        y: self.mouse_y,
                                    });
                                }
                            }
                        }
                        PointerEvent::ScrollWheel(s) => {
                            // Wheel: 1 notch = 3 lines scroll
                            if s.has_axis(Axis::Vertical) {
                                let raw = s.scroll_value_v120(Axis::Vertical);
                                let lines = (raw / 120.0) * 3.0;
                                self.scroll_accum += lines;
                            }
                        }
                        PointerEvent::ScrollFinger(s) => {
                            // Touchpad: convert pixel value to lines, increase sensitivity
                            if s.has_axis(Axis::Vertical) {
                                let raw = s.scroll_value(Axis::Vertical);
                                self.scroll_accum += raw / 15.0;
                            }
                        }
                        PointerEvent::ScrollContinuous(s) => {
                            if s.has_axis(Axis::Vertical) {
                                let raw = s.scroll_value(Axis::Vertical);
                                self.scroll_accum += raw / 15.0;
                            }
                        }
                        other => {
                            debug!("Unhandled pointer event: {:?}", other);
                        }
                    }
                }
                _ => {}
            }
        }

        // Emit accumulated scroll as event (even small values for smoothness)
        if self.scroll_accum.abs() >= 0.1 {
            mouse_events.push(MouseEvent::Scroll {
                delta: self.scroll_accum,
                x: self.mouse_x,
                y: self.mouse_y,
            });
            self.scroll_accum = 0.0;
        }

        // Generate key repeats
        let now = Instant::now();
        let repeat_interval = Duration::from_millis(self.repeat_rate_ms);
        for (_keycode, (_, next_repeat, event)) in self.held_keys.iter_mut() {
            if now >= *next_repeat {
                // Generate repeat event
                let mut repeat_event = event.clone();
                repeat_event.is_press = true; // Treat as press
                                              // Reflect current modifier state
                repeat_event.mods_shift = self.shift_pressed;
                repeat_event.mods_ctrl = self.ctrl_pressed;
                key_events.push(repeat_event);
                // Update next repeat time
                *next_repeat = now + repeat_interval;
            }
        }

        (key_events, mouse_events)
    }

    /// Get current mouse position
    #[allow(dead_code)]
    pub fn mouse_position(&self) -> (f64, f64) {
        (self.mouse_x, self.mouse_y)
    }
}

// evdev keycodes for F1-F12
const KEY_F1: u32 = 59;
const KEY_F2: u32 = 60;
const KEY_F3: u32 = 61;
const KEY_F4: u32 = 62;
const KEY_F5: u32 = 63;
const KEY_F6: u32 = 64;
const KEY_F7: u32 = 65;
const KEY_F8: u32 = 66;
const KEY_F9: u32 = 67;
const KEY_F10: u32 = 68;
const KEY_F11: u32 = 87;
const KEY_F12: u32 = 88;

/// Check if key event is a VT switch request (Ctrl+Alt+Fn)
/// Returns the target VT number (1-12) if it is a VT switch
pub fn check_vt_switch(event: &RawKeyEvent) -> Option<u16> {
    // Only on key press with Ctrl+Alt held
    if !event.is_press || !event.mods_ctrl || !event.mods_alt {
        return None;
    }

    // Map F1-F12 to VT 1-12
    match event.keycode {
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

/// Convert keysym to terminal escape sequence
pub fn keysym_to_bytes(sym: xkb::Keysym, utf8: &str) -> Vec<u8> {
    let raw = sym.raw();
    match raw {
        // Control keys
        _ if raw == keysyms::KEY_Return || raw == keysyms::KEY_KP_Enter => vec![b'\r'],
        _ if raw == keysyms::KEY_BackSpace => vec![0x7f],
        _ if raw == keysyms::KEY_Tab => vec![b'\t'],
        _ if raw == keysyms::KEY_Escape => vec![0x1b],

        // Cursor keys
        _ if raw == keysyms::KEY_Up => b"\x1b[A".to_vec(),
        _ if raw == keysyms::KEY_Down => b"\x1b[B".to_vec(),
        _ if raw == keysyms::KEY_Right => b"\x1b[C".to_vec(),
        _ if raw == keysyms::KEY_Left => b"\x1b[D".to_vec(),

        // Navigation
        _ if raw == keysyms::KEY_Home => b"\x1b[H".to_vec(),
        _ if raw == keysyms::KEY_End => b"\x1b[F".to_vec(),
        _ if raw == keysyms::KEY_Insert => b"\x1b[2~".to_vec(),
        _ if raw == keysyms::KEY_Delete => b"\x1b[3~".to_vec(),
        _ if raw == keysyms::KEY_Page_Up => b"\x1b[5~".to_vec(),
        _ if raw == keysyms::KEY_Page_Down => b"\x1b[6~".to_vec(),

        // Function keys
        _ if raw == keysyms::KEY_F1 => b"\x1bOP".to_vec(),
        _ if raw == keysyms::KEY_F2 => b"\x1bOQ".to_vec(),
        _ if raw == keysyms::KEY_F3 => b"\x1bOR".to_vec(),
        _ if raw == keysyms::KEY_F4 => b"\x1bOS".to_vec(),
        _ if raw == keysyms::KEY_F5 => b"\x1b[15~".to_vec(),
        _ if raw == keysyms::KEY_F6 => b"\x1b[17~".to_vec(),
        _ if raw == keysyms::KEY_F7 => b"\x1b[18~".to_vec(),
        _ if raw == keysyms::KEY_F8 => b"\x1b[19~".to_vec(),
        _ if raw == keysyms::KEY_F9 => b"\x1b[20~".to_vec(),
        _ if raw == keysyms::KEY_F10 => b"\x1b[21~".to_vec(),
        _ if raw == keysyms::KEY_F11 => b"\x1b[23~".to_vec(),
        _ if raw == keysyms::KEY_F12 => b"\x1b[24~".to_vec(),

        // Ignore modifier keys (don't generate characters by themselves)
        _ if raw == keysyms::KEY_Shift_L
            || raw == keysyms::KEY_Shift_R
            || raw == keysyms::KEY_Control_L
            || raw == keysyms::KEY_Control_R
            || raw == keysyms::KEY_Alt_L
            || raw == keysyms::KEY_Alt_R
            || raw == keysyms::KEY_Super_L
            || raw == keysyms::KEY_Super_R
            || raw == keysyms::KEY_Caps_Lock
            || raw == keysyms::KEY_Num_Lock =>
        {
            vec![]
        }

        // Normal characters: use xkbcommon UTF-8 output directly
        _ => {
            if !utf8.is_empty() {
                utf8.as_bytes().to_vec()
            } else {
                vec![]
            }
        }
    }
}

/// Convert keysym only to PTY bytes (for IME ForwardKey)
///
/// Same mapping as `keysym_to_bytes()` but generates UTF-8 string
/// from keysym value only without referencing xkb state.
pub fn keysym_to_bytes_from_sym(keysym: u32) -> Vec<u8> {
    let sym = xkb::Keysym::new(keysym);
    // Get UTF-8 string using xkbcommon's keysym_to_utf8
    let utf8 = xkb::keysym_to_utf8(sym);
    let result = keysym_to_bytes(sym, &utf8);
    // Filter NUL bytes (can occur with invalid keysyms)
    result.into_iter().filter(|&b| b != 0).collect()
}

/// Terminal keyboard configuration
#[derive(Debug, Clone, Copy, Default)]
pub struct KeyboardConfig {
    /// DECCKM: Application cursor keys mode
    pub application_cursor_keys: bool,
    /// modifyOtherKeys level (0=off, 1=most, 2=all)
    pub modify_other_keys: u8,
    /// Kitty keyboard protocol flags
    pub kitty_flags: u32,
}

/// Modifier key bitmask (xterm compatible)
fn modifier_code(ctrl: bool, alt: bool, shift: bool) -> u8 {
    let mut code = 1u8; // Base value
    if shift {
        code += 1;
    }
    if alt {
        code += 2;
    }
    if ctrl {
        code += 4;
    }
    code
}

/// Convert keysym to bytes considering modifiers and terminal settings
///
/// - DECCKM: Application cursor mode (SS3 format)
/// - Modifiers: CSI 1;{mod}X format
/// - Kitty keyboard: CSI u format
pub fn keysym_to_bytes_with_mods(
    sym: xkb::Keysym,
    utf8: &str,
    ctrl: bool,
    alt: bool,
    shift: bool,
    config: &KeyboardConfig,
) -> Vec<u8> {
    let raw = sym.raw();
    let has_mods = ctrl || alt || shift;
    let mod_code = modifier_code(ctrl, alt, shift);

    // Kitty keyboard protocol
    // flags & 1 = disambiguate escape codes
    // flags & 8 = report all keys as escape codes (CSI u for everything)
    let kitty_all_keys = config.kitty_flags & 8 != 0;
    let kitty_disambiguate = config.kitty_flags & 1 != 0;
    if kitty_disambiguate || kitty_all_keys {
        if let Some(bytes) = encode_kitty_keyboard(raw, ctrl, alt, shift, kitty_all_keys, utf8) {
            return bytes;
        }
    }

    // Cursor keys
    if let Some(cursor_char) = match raw {
        _ if raw == keysyms::KEY_Up => Some(b'A'),
        _ if raw == keysyms::KEY_Down => Some(b'B'),
        _ if raw == keysyms::KEY_Right => Some(b'C'),
        _ if raw == keysyms::KEY_Left => Some(b'D'),
        _ if raw == keysyms::KEY_Home => Some(b'H'),
        _ if raw == keysyms::KEY_End => Some(b'F'),
        _ => None,
    } {
        if has_mods {
            // CSI 1;{mod}X
            return format!("\x1b[1;{}{}", mod_code, cursor_char as char).into_bytes();
        } else if config.application_cursor_keys {
            // SS3 X (application mode)
            return vec![0x1b, b'O', cursor_char];
        } else {
            // CSI X (normal mode)
            return vec![0x1b, b'[', cursor_char];
        }
    }

    // Insert/Delete/PageUp/PageDown
    if let Some((code, suffix)) = match raw {
        _ if raw == keysyms::KEY_Insert => Some((2, b'~')),
        _ if raw == keysyms::KEY_Delete => Some((3, b'~')),
        _ if raw == keysyms::KEY_Page_Up => Some((5, b'~')),
        _ if raw == keysyms::KEY_Page_Down => Some((6, b'~')),
        _ => None,
    } {
        if has_mods {
            return format!("\x1b[{};{}{}", code, mod_code, suffix as char).into_bytes();
        } else {
            return format!("\x1b[{}~", code).into_bytes();
        }
    }

    // Function keys (F1-F12)
    if let Some(fkey_seq) = encode_function_key(raw, has_mods, mod_code) {
        return fkey_seq;
    }

    // Ctrl + alphabet (A-Z, a-z)
    if ctrl && !alt && !shift {
        // ASCII control character (Ctrl+A = 0x01, Ctrl+Z = 0x1A)
        let ch = utf8.chars().next().unwrap_or('\0');
        if ch.is_ascii_alphabetic() {
            let ctrl_code = (ch.to_ascii_uppercase() as u8) - b'A' + 1;
            return vec![ctrl_code];
        }
    }

    // Alt + character (ESC prefix)
    if alt && !ctrl {
        let base = keysym_to_bytes(sym, utf8);
        if !base.is_empty() {
            let mut result = vec![0x1b];
            result.extend(base);
            return result;
        }
    }

    // modifyOtherKeys: send modified characters as CSI 27;{mod};{code}~ format
    // Level 1: only for keys that would otherwise be ambiguous
    // Level 2: for all modified keys
    if config.modify_other_keys >= 1 && has_mods {
        let ch = utf8.chars().next().unwrap_or('\0');
        if ch.is_ascii_graphic() {
            // Level 1: only Ctrl+letter and a few special cases
            // Level 2: all modified printable characters
            let should_encode = if config.modify_other_keys >= 2 {
                true
            } else {
                // Level 1: Ctrl+letter (excluding Ctrl+C, Ctrl+Z, etc. that have standard meanings)
                ctrl && !alt && ch.is_ascii_alphabetic()
            };
            if should_encode {
                return format!("\x1b[27;{};{}~", mod_code, ch as u32).into_bytes();
            }
        }
    }

    // Default: normal processing without modifiers
    keysym_to_bytes(sym, utf8)
}

/// Function key escape sequences
fn encode_function_key(raw: u32, has_mods: bool, mod_code: u8) -> Option<Vec<u8>> {
    // F1-F4: SS3 format (no modifiers) or CSI 1;{mod}P/Q/R/S (with modifiers)
    let f1_4 = match raw {
        _ if raw == keysyms::KEY_F1 => Some(b'P'),
        _ if raw == keysyms::KEY_F2 => Some(b'Q'),
        _ if raw == keysyms::KEY_F3 => Some(b'R'),
        _ if raw == keysyms::KEY_F4 => Some(b'S'),
        _ => None,
    };
    if let Some(ch) = f1_4 {
        if has_mods {
            return Some(format!("\x1b[1;{}{}", mod_code, ch as char).into_bytes());
        } else {
            return Some(vec![0x1b, b'O', ch]);
        }
    }

    // F5-F12: CSI {code}~ format
    let f5_12 = match raw {
        _ if raw == keysyms::KEY_F5 => Some(15),
        _ if raw == keysyms::KEY_F6 => Some(17),
        _ if raw == keysyms::KEY_F7 => Some(18),
        _ if raw == keysyms::KEY_F8 => Some(19),
        _ if raw == keysyms::KEY_F9 => Some(20),
        _ if raw == keysyms::KEY_F10 => Some(21),
        _ if raw == keysyms::KEY_F11 => Some(23),
        _ if raw == keysyms::KEY_F12 => Some(24),
        _ => None,
    };
    if let Some(code) = f5_12 {
        if has_mods {
            return Some(format!("\x1b[{};{}~", code, mod_code).into_bytes());
        } else {
            return Some(format!("\x1b[{}~", code).into_bytes());
        }
    }

    None
}

/// Kitty keyboard protocol encoding (CSI u format)
/// all_keys: if true (flags & 8), report all keys including plain letters
fn encode_kitty_keyboard(raw: u32, ctrl: bool, alt: bool, shift: bool, all_keys: bool, utf8_input: &str) -> Option<Vec<u8>> {
    // Basic keycode mapping
    let unicode = match raw {
        _ if raw == keysyms::KEY_Escape => Some(27),
        _ if raw == keysyms::KEY_Return || raw == keysyms::KEY_KP_Enter => Some(13),
        _ if raw == keysyms::KEY_Tab => Some(9),
        _ if raw == keysyms::KEY_BackSpace => Some(127),
        _ if raw == keysyms::KEY_Up => Some(57352),      // KITTY_KEY_UP
        _ if raw == keysyms::KEY_Down => Some(57353),    // KITTY_KEY_DOWN
        _ if raw == keysyms::KEY_Left => Some(57351),    // KITTY_KEY_LEFT
        _ if raw == keysyms::KEY_Right => Some(57350),   // KITTY_KEY_RIGHT
        _ if raw == keysyms::KEY_Home => Some(57345),    // KITTY_KEY_HOME
        _ if raw == keysyms::KEY_End => Some(57346),     // KITTY_KEY_END
        _ if raw == keysyms::KEY_Insert => Some(57348),  // KITTY_KEY_INSERT
        _ if raw == keysyms::KEY_Delete => Some(57349),  // KITTY_KEY_DELETE
        _ if raw == keysyms::KEY_Page_Up => Some(57354), // KITTY_KEY_PAGE_UP
        _ if raw == keysyms::KEY_Page_Down => Some(57355), // KITTY_KEY_PAGE_DOWN
        _ if raw == keysyms::KEY_F1 => Some(57364),
        _ if raw == keysyms::KEY_F2 => Some(57365),
        _ if raw == keysyms::KEY_F3 => Some(57366),
        _ if raw == keysyms::KEY_F4 => Some(57367),
        _ if raw == keysyms::KEY_F5 => Some(57368),
        _ if raw == keysyms::KEY_F6 => Some(57369),
        _ if raw == keysyms::KEY_F7 => Some(57370),
        _ if raw == keysyms::KEY_F8 => Some(57371),
        _ if raw == keysyms::KEY_F9 => Some(57372),
        _ if raw == keysyms::KEY_F10 => Some(57373),
        _ if raw == keysyms::KEY_F11 => Some(57374),
        _ if raw == keysyms::KEY_F12 => Some(57375),
        _ => {
            // Normal Unicode characters
            utf8_input.chars().next().map(|c| c as u32)
        }
    };

    let code = unicode?;
    let mod_code = modifier_code(ctrl, alt, shift);

    // For normal printable characters without modifiers:
    // - If all_keys mode: encode as CSI u
    // - Otherwise: return None to use normal character output
    if mod_code == 1 && code < 57345 {
        if all_keys {
            return Some(format!("\x1b[{}u", code).into_bytes());
        }
        return None;
    }

    // CSI {code};{mod}u format
    if mod_code > 1 {
        Some(format!("\x1b[{};{}u", code, mod_code).into_bytes())
    } else {
        Some(format!("\x1b[{}u", code).into_bytes())
    }
}
