//! VT escape sequence parser
//!
//! Implements vte crate's Perform trait and applies parsed results to Grid.
//!
//! ## References
//! - ECMA-48: Control Functions for Coded Character Sets
//! - VT100/VT220: <https://vt100.net/docs/>
//! - Xterm Control Sequences: <https://invisible-island.net/xterm/ctlseqs/ctlseqs.html>
//! - Kitty Keyboard Protocol: <https://sw.kovidgoyal.net/kitty/keyboard-protocol/>

use std::sync::Arc;

use log::{debug, info, trace, warn};
use vte::{Params, Perform};

// ============================================================================
// Constants
// ============================================================================

/// Maximum XTGETTCAP buffer size (64KB)
const MAX_XTGETTCAP_BUFFER: usize = 64 * 1024;

// ============================================================================
// Helper functions
// ============================================================================

/// Convert CSI parameter to usize with default value.
/// CSI parameters treat 0 as "default" (usually 1).
#[inline]
const fn param_or_default(param: u16, default: usize) -> usize {
    if param == 0 {
        default
    } else {
        param as usize
    }
}

/// Get cursor style and blink state from DECSCUSR parameter.
/// Returns (CursorStyle, blink) or None for invalid parameter.
#[inline]
const fn cursor_style_from_decscusr(param: u16) -> Option<(CursorStyle, bool)> {
    match param {
        0 | 1 => Some((CursorStyle::Block, true)), // Default / blinking block
        2 => Some((CursorStyle::Block, false)),    // Steady block
        3 => Some((CursorStyle::Underline, true)), // Blinking underline
        4 => Some((CursorStyle::Underline, false)), // Steady underline
        5 => Some((CursorStyle::Bar, true)),       // Blinking bar
        6 => Some((CursorStyle::Bar, false)),      // Steady bar
        _ => None,
    }
}

use super::grid::{CellAttrs, Color, CursorStyle, Grid, Hyperlink, UnderlineStyle};
use super::sixel::SixelDecoder;
use super::{AnimationState, DcsHandler, ImageRegistry, Notification, NotificationProgress, TerminalImage};

use std::collections::HashMap;

/// Parse OSC color value
/// Supports formats:
/// - rgb:RRRR/GGGG/BBBB (X11 format, 16-bit per component)
/// - rgb:RR/GG/BB (X11 format, 8-bit per component)
/// - #RRGGBB (hex format)
/// - #RGB (short hex format)
fn parse_osc_color(data: &[u8]) -> Option<(u8, u8, u8)> {
    let s = std::str::from_utf8(data).ok()?;

    if let Some(hex) = s.strip_prefix('#') {
        // #RRGGBB or #RGB format
        match hex.len() {
            6 => {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                Some((r, g, b))
            }
            3 => {
                let r = u8::from_str_radix(&hex[0..1], 16).ok()? * 17;
                let g = u8::from_str_radix(&hex[1..2], 16).ok()? * 17;
                let b = u8::from_str_radix(&hex[2..3], 16).ok()? * 17;
                Some((r, g, b))
            }
            _ => None,
        }
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

/// vte::Perform implementation
/// Holds reference to Grid and directly applies parsed results
pub struct Performer<'a> {
    pub grid: &'a mut Grid,
    pub clipboard: &'a mut String,
    /// PTY response buffer (borrowed, not owned)
    pub pty_response: &'a mut Vec<u8>,
    pub dcs_handler: &'a mut Option<DcsHandler>,
    pub images: &'a mut ImageRegistry,
    /// Cell width (pixels)
    cell_width: u32,
    /// Cell height (pixels)
    cell_height: u32,
    /// Current directory (OSC 7)
    pub current_dir: &'a mut Option<String>,
    /// Clipboard file path (OSC 52)
    clipboard_path: &'a str,
    /// Notification history
    pub notifications: &'a mut Vec<Notification>,
    /// Active progress bar state
    pub active_progress: &'a mut Option<NotificationProgress>,
    /// Pending (incomplete) OSC 99 notifications
    pub pending_notifications: &'a mut HashMap<String, Notification>,
    /// Whether notifications are enabled
    pub notifications_enabled: &'a bool,
}

impl<'a> Performer<'a> {
    pub fn new(
        grid: &'a mut Grid,
        clipboard: &'a mut String,
        dcs_handler: &'a mut Option<DcsHandler>,
        images: &'a mut ImageRegistry,
        cell_width: u32,
        cell_height: u32,
        current_dir: &'a mut Option<String>,
        clipboard_path: &'a str,
        pty_response: &'a mut Vec<u8>,
        notifications: &'a mut Vec<Notification>,
        active_progress: &'a mut Option<NotificationProgress>,
        pending_notifications: &'a mut HashMap<String, Notification>,
        notifications_enabled: &'a bool,
    ) -> Self {
        Self {
            grid,
            clipboard,
            pty_response,
            dcs_handler,
            images,
            cell_width,
            cell_height,
            current_dir,
            clipboard_path,
            notifications,
            active_progress,
            pending_notifications,
            notifications_enabled,
        }
    }
}

impl<'a> Perform for Performer<'a> {
    /// Handle printable character
    fn print(&mut self, c: char) {
        self.grid.put_char(c);
    }

    /// Handle C0/C1 control character
    fn execute(&mut self, byte: u8) {
        match byte {
            0x08 => self.grid.backspace(), // BS
            0x09 => self.grid.tab(),       // HT
            0x0A | 0x0B | 0x0C => {
                // LF, VT, FF
                self.grid.linefeed();
            }
            0x0D => self.grid.carriage_return(),     // CR
            0x0E => self.grid.shift_out(),           // SO - activate G1 charset
            0x0F => self.grid.shift_in(),            // SI - activate G0 charset
            0x07 => self.grid.bell_triggered = true, // BEL
            _ => {
                trace!("Unhandled control character: 0x{:02x}", byte);
            }
        }
    }

    /// Handle CSI sequence
    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], _ignore: bool, action: char) {
        // Convert parameters to flat array (supports sub-parameters)
        let flat_params: Vec<Vec<u16>> = params.iter().map(|p| p.to_vec()).collect();

        // First parameter (with default value)
        let param0 = flat_params
            .first()
            .and_then(|p| p.first().copied())
            .unwrap_or(0);

        match (action, intermediates) {
            // Cursor movement
            // === Cursor Movement (CUU/CUD/CUF/CUB/CNL/CPL) ===
            ('A', []) => {
                // CUU - Cursor Up Ps times (default 1)
                self.grid.move_cursor_up(param_or_default(param0, 1));
            }
            ('B', []) => {
                // CUD - Cursor Down Ps times (default 1)
                self.grid.move_cursor_down(param_or_default(param0, 1));
            }
            ('C', []) => {
                // CUF - Cursor Forward Ps times (default 1)
                self.grid.move_cursor_forward(param_or_default(param0, 1));
            }
            ('D', []) => {
                // CUB - Cursor Backward Ps times (default 1)
                self.grid.move_cursor_backward(param_or_default(param0, 1));
            }
            ('E', []) => {
                // CNL - Cursor Next Line Ps times (default 1)
                self.grid.move_cursor_down(param_or_default(param0, 1));
                self.grid.carriage_return();
            }
            ('F', []) => {
                // CPL - Cursor Previous Line Ps times (default 1)
                self.grid.move_cursor_up(param_or_default(param0, 1));
                self.grid.carriage_return();
            }
            ('H' | 'f', []) => {
                // CUP - Cursor Position (row ; col, 1-based, default 1;1)
                let row = param_or_default(param0, 1);
                let col = flat_params
                    .get(1)
                    .and_then(|p| p.first().copied())
                    .map(|v| param_or_default(v, 1))
                    .unwrap_or(1);
                self.grid.move_cursor_to(row, col);
            }
            ('G', []) => {
                // CHA - Cursor Horizontal Absolute (column, 1-based)
                self.grid
                    .move_cursor_to(self.grid.cursor_row + 1, param_or_default(param0, 1));
            }
            ('d', []) => {
                // VPA - Vertical Position Absolute (row, 1-based)
                self.grid
                    .move_cursor_to(param_or_default(param0, 1), self.grid.cursor_col + 1);
            }
            ('J', []) => {
                // ED - Erase in Display
                self.grid.erase_in_display(param0);
            }
            ('K', []) => {
                // EL - Erase in Line
                self.grid.erase_in_line(param0);
            }
            // === Line/Character Operations ===
            ('L', []) => {
                // IL - Insert Ps blank lines (default 1)
                self.grid.insert_lines(param_or_default(param0, 1));
            }
            ('M', []) => {
                // DL - Delete Ps lines (default 1)
                self.grid.delete_lines(param_or_default(param0, 1));
            }
            ('P', []) => {
                // DCH - Delete Ps characters (default 1)
                self.grid.delete_chars(param_or_default(param0, 1));
            }
            ('@', []) => {
                // ICH - Insert Ps blank characters (default 1)
                self.grid.insert_chars(param_or_default(param0, 1));
            }
            ('X', []) => {
                // ECH - Erase Ps characters (default 1)
                self.grid.erase_chars(param_or_default(param0, 1));
            }
            // === Scrolling ===
            ('S', []) => {
                // SU - Scroll Up Ps lines (default 1)
                self.grid.scroll_up(param_or_default(param0, 1));
            }
            ('T', []) => {
                // SD - Scroll Down Ps lines (default 1)
                self.grid.scroll_down(param_or_default(param0, 1));
            }
            ('s', []) => {
                // SCOSC - Save Cursor Position
                self.grid.save_cursor();
            }
            ('u', []) => {
                // SCORC - Restore Cursor Position
                self.grid.restore_cursor();
            }
            ('n', []) => {
                // DSR - Device Status Report
                match param0 {
                    5 => {
                        // Status report: report normal operation
                        self.pty_response.extend_from_slice(b"\x1b[0n");
                    }
                    6 => {
                        // Cursor position report: ESC [ row ; col R
                        let row = self.grid.cursor_row + 1;
                        let col = self.grid.cursor_col + 1;
                        self.pty_response
                            .extend_from_slice(format!("\x1b[{};{}R", row, col).as_bytes());
                    }
                    _ => {}
                }
            }
            ('c', []) | ('c', [b'?']) => {
                // DA1 - Primary Device Attributes
                // Report VT220 compatible + feature flags
                // 62: VT220, 1: 132 columns, 4: Sixel, 22: ANSI color, 29: ANSI text locator (mouse)
                log::debug!("DA1 query: responding with device attributes");
                self.pty_response.extend_from_slice(b"\x1b[?62;1;4;22;29c");
            }
            ('c', [b'>']) => {
                // DA2 - Secondary Device Attributes
                // >Pp;Pv;Pc c (Pp=terminal type, Pv=firmware version, Pc=ROM number)
                // 1: VT220, 100: bcon version 0.1.0 (encoded as 100), 0: ROM
                self.pty_response.extend_from_slice(b"\x1b[>1;100;0c");
            }
            ('c', [b'=']) => {
                // DA3 - Tertiary Device Attributes (Unit ID)
                // =XXXXXXXX ST (hex unit id)
                self.pty_response
                    .extend_from_slice(b"\x1bP!|00000000\x1b\\");
            }
            ('q', [b'>']) => {
                // XTVERSION - Terminal version query
                // DCS > | Pt ST
                self.pty_response
                    .extend_from_slice(b"\x1bP>|bcon 0.1.0\x1b\\");
            }
            ('b', []) => {
                // REP - Repeat preceding graphic character Ps times (default 1)
                self.grid.repeat_char(param_or_default(param0, 1));
            }
            ('m', []) => {
                // SGR - Select Graphic Rendition
                self.handle_sgr(&flat_params);
            }
            ('m', [b'>']) => {
                // modifyOtherKeys mode: CSI > 4 ; Pv m
                if param0 == 4 {
                    let level = flat_params
                        .get(1)
                        .and_then(|p| p.first().copied())
                        .unwrap_or(0) as u8;
                    self.grid.keyboard.modify_other_keys = level.min(2);
                    trace!(
                        "modifyOtherKeys: level={}",
                        self.grid.keyboard.modify_other_keys
                    );
                }
            }
            ('u', [b'>']) => {
                // Kitty keyboard: Push mode
                // CSI > flags u
                // Push current flags onto stack, then set new flags
                self.grid.keyboard.kitty_push(param0 as u32);
                trace!(
                    "Kitty keyboard: push flags={} (stack depth={})",
                    param0,
                    self.grid.keyboard.kitty_stack.len()
                );
            }
            ('u', [b'<']) => {
                // Kitty keyboard: Pop mode
                // CSI < count u - pop 'count' entries (default 1)
                let count = param0 as u16;
                self.grid.keyboard.kitty_pop(count);
                trace!(
                    "Kitty keyboard: pop count={} (now flags={}, stack depth={})",
                    if count == 0 { 1 } else { count },
                    self.grid.keyboard.kitty_flags,
                    self.grid.keyboard.kitty_stack.len()
                );
            }
            ('u', [b'?']) => {
                // Kitty keyboard: Query mode
                // Response: CSI ? flags u
                let flags = self.grid.keyboard.kitty_flags;
                let response = format!("\x1b[?{}u", flags);
                log::debug!(
                    "Kitty keyboard query: responding with flags={} ({:?})",
                    flags,
                    response
                );
                self.pty_response.extend_from_slice(response.as_bytes());
            }
            ('u', [b'=']) => {
                // Kitty keyboard: Set mode
                // CSI = flags ; mode u
                let flags = param0 as u32;
                let mode = flat_params
                    .get(1)
                    .and_then(|p| p.first().copied())
                    .unwrap_or(1);
                match mode {
                    1 => self.grid.keyboard.kitty_flags = flags,   // set
                    2 => self.grid.keyboard.kitty_flags |= flags,  // or
                    3 => self.grid.keyboard.kitty_flags &= !flags, // not
                    _ => {}
                }
                trace!("Kitty keyboard: set flags={} mode={}", flags, mode);
            }
            ('g', []) => {
                // TBC - Tabulation Clear
                // 0: clear at current column, 3: clear all
                self.grid.clear_tab_stop(param0);
            }
            ('p', [b'!']) => {
                // DECSTR - Soft Terminal Reset
                self.grid.soft_reset();
            }
            ('h', []) => {
                // SM - Set Mode (ANSI modes)
                match param0 {
                    4 => {
                        // IRM - Insert/Replace Mode
                        self.grid.modes.insert_mode = true;
                    }
                    _ => {
                        trace!("Unhandled SM mode: {}", param0);
                    }
                }
            }
            ('l', []) => {
                // RM - Reset Mode (ANSI modes)
                match param0 {
                    4 => {
                        // IRM - Insert/Replace Mode
                        self.grid.modes.insert_mode = false;
                    }
                    _ => {
                        trace!("Unhandled RM mode: {}", param0);
                    }
                }
            }
            ('p', [b'?', b'$']) | ('p', [b'$']) => {
                // DECRQM - DEC Request Mode
                // CSI ? Ps $ p → response: CSI ? Ps ; Pm $ y
                // Pm: 0=not recognized, 1=set, 2=reset, 3=permanently set, 4=permanently reset
                let is_dec = intermediates.contains(&b'?');
                if is_dec {
                    let pm = match self.grid.is_mode_set(param0) {
                        Some(true) => 1,  // Set
                        Some(false) => 2, // Reset
                        None => 0,        // Not recognized
                    };
                    let response = format!("\x1b[?{};{}$y", param0, pm);
                    self.pty_response.extend_from_slice(response.as_bytes());
                } else {
                    // ANSI mode query
                    let pm = match param0 {
                        4 => {
                            if self.grid.modes.insert_mode {
                                1
                            } else {
                                2
                            }
                        }
                        _ => 0,
                    };
                    let response = format!("\x1b[{};{}$y", param0, pm);
                    self.pty_response.extend_from_slice(response.as_bytes());
                }
            }
            ('r', []) => {
                // DECSTBM - Set Top and Bottom Margins
                let top = param0 as usize;
                let bottom = flat_params
                    .get(1)
                    .and_then(|p| p.first().copied())
                    .unwrap_or(0) as usize;
                self.grid.set_scroll_region(top, bottom);
            }
            ('h', [b'?']) => {
                // DECSET (Set Private Mode) - supports multiple params: CSI ? Pm ; Pm ; ... h
                for p in &flat_params {
                    if let Some(&mode) = p.first() {
                        self.handle_decset(mode, true);
                    }
                }
            }
            ('l', [b'?']) => {
                // DECRST (Reset Private Mode) - supports multiple params: CSI ? Pm ; Pm ; ... l
                for p in &flat_params {
                    if let Some(&mode) = p.first() {
                        self.handle_decset(mode, false);
                    }
                }
            }
            ('q', [b' ']) => {
                // DECSCUSR - Set Cursor Style (Ps SP q)
                // 0/1: blinking block, 2: steady block, 3/4: underline, 5/6: bar
                if let Some((style, blink)) = cursor_style_from_decscusr(param0) {
                    self.grid.cursor.style = style;
                    self.grid.cursor.blink = blink;
                }
            }
            ('t', []) => {
                // XTWINOPS - Window manipulation
                trace!("CSI t: param0={}", param0);
                match param0 {
                    14 => {
                        // Report window size in pixels
                        // Response: CSI 4 ; height ; width t
                        let width_px = self.grid.cols() as u32 * self.cell_width;
                        let height_px = self.grid.rows() as u32 * self.cell_height;
                        trace!("CSI 14 t: responding {}x{} pixels", width_px, height_px);
                        self.pty_response.extend_from_slice(
                            format!("\x1b[4;{};{}t", height_px, width_px).as_bytes(),
                        );
                    }
                    16 => {
                        // Report cell size in pixels
                        // Response: CSI 6 ; height ; width t
                        trace!(
                            "CSI 16 t: responding cell {}x{}",
                            self.cell_width,
                            self.cell_height
                        );
                        self.pty_response.extend_from_slice(
                            format!("\x1b[6;{};{}t", self.cell_height, self.cell_width).as_bytes(),
                        );
                    }
                    18 => {
                        // Report window size in characters
                        // Response: CSI 8 ; rows ; cols t
                        trace!(
                            "CSI 18 t: responding {}x{} chars",
                            self.grid.cols(),
                            self.grid.rows()
                        );
                        self.pty_response.extend_from_slice(
                            format!("\x1b[8;{};{}t", self.grid.rows(), self.grid.cols()).as_bytes(),
                        );
                    }
                    _ => {
                        trace!("XTWINOPS: unsupported operation {}", param0);
                    }
                }
            }
            _ => {
                trace!(
                    "Unhandled CSI: action='{}', intermediates={:?}, params={:?}",
                    action,
                    intermediates,
                    flat_params
                );
            }
        }
    }

    /// Handle escape sequence
    fn esc_dispatch(&mut self, intermediates: &[u8], _ignore: bool, byte: u8) {
        match (byte, intermediates) {
            (b'D', []) => {
                // IND - Index (move cursor down 1 line, with scroll)
                self.grid.linefeed();
            }
            (b'E', []) => {
                // NEL - Next Line
                self.grid.carriage_return();
                self.grid.linefeed();
            }
            (b'M', []) => {
                // RI - Reverse Index (move cursor up 1 line)
                self.grid.reverse_index();
            }
            (b'c', []) => {
                // RIS - Full Reset
                let cols = self.grid.cols();
                let rows = self.grid.rows();
                let max_scrollback = self.grid.max_scrollback;
                *self.grid = Grid::with_scrollback(cols, rows, max_scrollback);
            }
            (b'7', []) => {
                // DECSC - Save Cursor
                self.grid.save_dec_cursor();
            }
            (b'8', []) => {
                // DECRC - Restore Cursor
                self.grid.restore_dec_cursor();
            }
            (b'H', []) => {
                // HTS - Horizontal Tab Set
                self.grid.set_tab_stop();
            }
            // SCS - Select Character Set (G0)
            (b'B', [b'(']) => {
                // ESC ( B → ASCII
                self.grid.set_charset_g0(super::grid::Charset::Ascii);
            }
            (b'0', [b'(']) => {
                // ESC ( 0 → DEC Special Graphics (line drawing)
                self.grid.set_charset_g0(super::grid::Charset::DecSpecial);
            }
            // SCS - Select Character Set (G1)
            (b'B', [b')']) => {
                // ESC ) B → ASCII
                self.grid.set_charset_g1(super::grid::Charset::Ascii);
            }
            (b'0', [b')']) => {
                // ESC ) 0 → DEC Special Graphics
                self.grid.set_charset_g1(super::grid::Charset::DecSpecial);
            }
            // Other charset designators - treat as ASCII
            (_, [b'(']) | (_, [b')']) => {
                trace!(
                    "Unhandled SCS designator: 0x{:02x} intermediates={:?}",
                    byte,
                    intermediates
                );
            }
            _ => {
                trace!(
                    "Unhandled ESC: byte=0x{:02x}, intermediates={:?}",
                    byte,
                    intermediates
                );
            }
        }
    }

    /// DCS sequence start
    fn hook(&mut self, params: &Params, intermediates: &[u8], _ignore: bool, action: char) {
        trace!(
            "DCS hook: action='{}', intermediates={:?}, params={:?}",
            action,
            intermediates,
            params.iter().map(|p| p.to_vec()).collect::<Vec<_>>()
        );

        match (action, intermediates) {
            // Sixel: DCS q or DCS P q
            ('q', []) | ('q', [b'0'..=b'9']) => {
                info!("Sixel DCS started");
                *self.dcs_handler = Some(DcsHandler::Sixel(SixelDecoder::new()));
            }
            // XTGETTCAP: DCS + q Pt ST
            ('q', [b'+']) => {
                trace!("XTGETTCAP query started");
                *self.dcs_handler = Some(DcsHandler::XtGetTcap(Vec::new()));
            }
            // DECRQSS: DCS $ q Pt ST
            ('q', [b'$']) => {
                trace!("DECRQSS query started");
                *self.dcs_handler = Some(DcsHandler::Decrqss(Vec::new()));
            }
            _ => {}
        }
    }

    /// Data within DCS sequence
    fn put(&mut self, byte: u8) {
        match self.dcs_handler {
            Some(DcsHandler::Sixel(ref mut decoder)) => {
                decoder.push(byte);
            }
            Some(DcsHandler::XtGetTcap(ref mut buffer)) => {
                if buffer.len() < MAX_XTGETTCAP_BUFFER {
                    buffer.push(byte);
                }
            }
            Some(DcsHandler::Decrqss(ref mut buffer)) => {
                if buffer.len() < 256 {
                    buffer.push(byte);
                }
            }
            None => {}
        }
    }

    /// DCS sequence end
    fn unhook(&mut self) {
        if let Some(handler) = self.dcs_handler.take() {
            match handler {
                DcsHandler::XtGetTcap(buffer) => {
                    // Generate XTGETTCAP response
                    self.handle_xtgettcap(&buffer);
                }
                DcsHandler::Decrqss(buffer) => {
                    // Generate DECRQSS response
                    self.handle_decrqss(&buffer);
                }
                DcsHandler::Sixel(decoder) => {
                    // Decode complete, register image
                    let id = self.images.next_id;
                    if let Some(sixel_img) = decoder.finish(id) {
                        info!(
                            "Sixel image decode complete: {}x{} (id={})",
                            sixel_img.width, sixel_img.height, sixel_img.id
                        );
                        // Convert SixelImage to TerminalImage
                        let term_img = TerminalImage {
                            id: sixel_img.id,
                            width: sixel_img.width,
                            height: sixel_img.height,
                            data: sixel_img.data,
                            frames: Vec::new(),
                            animation_state: AnimationState::Stopped,
                            current_frame: 0,
                            loop_count: 0,
                            current_loop: 0,
                        };
                        let img_id = self.images.insert(term_img);
                        // Place image on grid
                        if let Some(image) = self.images.get(img_id) {
                            self.grid.place_image(
                                img_id,
                                image.width,
                                image.height,
                                self.cell_width,
                                self.cell_height,
                                false, // Sixel always moves cursor
                            );
                        }
                    }
                }
            }
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        if params.is_empty() {
            return;
        }

        let cmd = std::str::from_utf8(params[0]).unwrap_or("");
        trace!("OSC dispatch: cmd={}, params.len()={}", cmd, params.len());
        match cmd {
            "0" | "1" | "2" => self.handle_osc_title(params),
            "4" => self.handle_osc_4(params),
            "7" => self.handle_osc_7(params),
            "8" => self.handle_osc_8(params),
            "10" => self.handle_osc_10(params),
            "11" => self.handle_osc_11(params),
            "12" => self.handle_osc_12(params),
            "52" => self.handle_osc_52(params),
            "104" => self.handle_osc_104(params),
            "110" => {
                // Reset foreground color to default
                self.grid.colors.fg = None;
                self.grid.mark_all_dirty();
            }
            "111" => {
                // Reset background color to default
                self.grid.colors.bg = None;
                self.grid.mark_all_dirty();
            }
            "112" => {
                // Reset cursor color to default
                self.grid.colors.cursor = None;
            }
            "9" => self.handle_osc_9(params),
            "99" => self.handle_osc_99(params),
            "133" => self.handle_osc_133(params),
            _ => {
                trace!("Unhandled OSC: cmd={}", cmd);
            }
        }
    }
}

impl<'a> Performer<'a> {
    /// Handle DECSET (CSI ? Pm h) and DECRST (CSI ? Pm l) sequences.
    ///
    /// These control DEC private modes. DECSET enables, DECRST disables.
    /// Reference: <https://invisible-island.net/xterm/ctlseqs/ctlseqs.html#h3-Functions-using-CSI-_-which-begin-with-CSI>
    fn handle_decset(&mut self, mode: u16, enable: bool) {
        match mode {
            // === DEC VT Modes ===
            1 => {
                // DECCKM: Application Cursor Keys
                // When set, cursor keys send application sequences (ESC O A/B/C/D)
                // instead of normal sequences (ESC [ A/B/C/D)
                self.grid.modes.application_cursor_keys = enable;
            }
            5 => {
                // DECSCNM: Screen Mode (reverse video)
                // When set, swap foreground/background for entire screen
                if self.grid.modes.reverse_video != enable {
                    self.grid.modes.reverse_video = enable;
                    self.grid.mark_all_dirty();
                }
            }
            6 => {
                // DECOM: Origin Mode
                // When set, cursor addressing is relative to scroll region
                self.grid.modes.origin_mode = enable;
                // Move cursor to origin on mode change
                if enable {
                    let (top, _) = self.grid.scroll_region();
                    self.grid.cursor_row = top;
                } else {
                    self.grid.cursor_row = 0;
                }
                self.grid.cursor_col = 0;
            }
            7 => {
                // DECAWM: Auto-wrap Mode
                // When set, text wraps to next line at right margin
                self.grid.modes.auto_wrap = enable;
            }
            25 => {
                // DECTCEM: Text Cursor Enable Mode
                // When set, cursor is visible
                self.grid.modes.cursor_visible = enable;
            }

            // === Xterm Extensions ===
            1047 => {
                // Alternate Screen Buffer (without save/restore cursor)
                if enable {
                    self.grid.enter_alternate_screen_1047();
                } else {
                    self.grid.leave_alternate_screen_1047();
                }
            }
            1048 => {
                // Save/Restore Cursor (equivalent to DECSC/DECRC)
                if enable {
                    self.grid.save_dec_cursor();
                } else {
                    self.grid.restore_dec_cursor();
                }
            }
            1049 => {
                // Alternate Screen Buffer (with save/restore cursor)
                // Used by vim, less, etc. to preserve main screen content
                if enable {
                    self.grid.enter_alternate_screen();
                } else {
                    self.grid.leave_alternate_screen();
                }
            }

            // === Mouse Tracking (mutually exclusive) ===
            1000 | 1002 | 1003 => {
                use super::grid::MouseMode;
                let new_mode = if enable {
                    match mode {
                        1000 => MouseMode::X10,         // X10 compatibility (button press only)
                        1002 => MouseMode::ButtonEvent, // Report button press/release/motion with button
                        1003 => MouseMode::AnyEvent,    // Report all motion events
                        _ => unreachable!(),
                    }
                } else {
                    MouseMode::None
                };
                debug!(
                    "Mouse mode: ?{} {} → {:?}",
                    mode,
                    if enable { "h" } else { "l" },
                    new_mode
                );
                self.grid.modes.mouse_mode = new_mode;
            }
            1006 => {
                // SGR Extended Mouse Mode
                // Uses CSI < Pb ; Px ; Py M/m format (supports coordinates > 223)
                self.grid.modes.mouse_sgr = enable;
            }
            1016 => {
                // SGR-Pixels Mouse Mode
                // Like 1006 but reports pixel coordinates instead of cell coordinates
                self.grid.modes.mouse_sgr_pixels = enable;
            }

            // === Modern Extensions ===
            1004 => {
                // Focus Event Mode
                // When set, terminal sends CSI I on focus-in, CSI O on focus-out
                self.grid.modes.send_focus_events = enable;
            }
            2004 => {
                // Bracketed Paste Mode
                // Wraps pasted text with CSI 200~ and CSI 201~
                self.grid.modes.bracketed_paste = enable;
            }
            2026 => {
                // Synchronized Update Mode (iTerm2/Kitty extension)
                // Defers rendering until mode is disabled, reducing flicker
                self.grid.modes.synchronized_update = enable;
            }
            _ => {
                trace!("Unhandled DEC private mode: {} = {}", mode, enable);
            }
        }
    }

    /// Handle SGR (Select Graphic Rendition) sequences (CSI Pm m).
    ///
    /// Controls text attributes like bold, italic, colors, etc.
    /// Multiple parameters can be combined: CSI 1;31;40 m = bold + red fg + black bg
    ///
    /// Reference: <https://en.wikipedia.org/wiki/ANSI_escape_code#SGR>
    fn handle_sgr(&mut self, params: &[Vec<u16>]) {
        // No parameters = SGR 0 (reset all attributes)
        if params.is_empty() {
            self.grid.reset_attrs();
            return;
        }

        let mut iter = params.iter().peekable();

        while let Some(param) = iter.next() {
            // Check for sub-parameters (colon-separated)
            if param.len() > 1 {
                self.handle_sgr_subparams(param);
                continue;
            }

            let code = param[0];
            match code {
                0 => self.grid.reset_attrs(),
                1 => self.grid.set_attr(CellAttrs::BOLD),
                2 => self.grid.set_attr(CellAttrs::DIM),
                3 => self.grid.set_attr(CellAttrs::ITALIC),
                4 => self.grid.set_underline_style(UnderlineStyle::Single),
                5 => self.grid.set_attr(CellAttrs::BLINK),
                7 => self.grid.set_attr(CellAttrs::INVERSE),
                8 => self.grid.set_attr(CellAttrs::HIDDEN),
                9 => self.grid.set_attr(CellAttrs::STRIKE),
                21 => self.grid.clear_attr(CellAttrs::BOLD),
                22 => {
                    self.grid.clear_attr(CellAttrs::BOLD);
                    self.grid.clear_attr(CellAttrs::DIM);
                }
                23 => self.grid.clear_attr(CellAttrs::ITALIC),
                24 => self.grid.set_underline_style(UnderlineStyle::None),
                25 => self.grid.clear_attr(CellAttrs::BLINK),
                27 => self.grid.clear_attr(CellAttrs::INVERSE),
                28 => self.grid.clear_attr(CellAttrs::HIDDEN),
                29 => self.grid.clear_attr(CellAttrs::STRIKE),
                // Foreground color (standard 8 colors)
                30..=37 => self.grid.set_fg(Color::Indexed((code - 30) as u8)),
                38 => {
                    // Extended foreground color: 38;5;n (256 color) or 38;2;r;g;b (True Color)
                    if let Some(color) = self.parse_extended_color(&mut iter) {
                        self.grid.set_fg(color);
                    }
                }
                39 => self.grid.set_fg(Color::Default),
                // Background color (standard 8 colors)
                40..=47 => self.grid.set_bg(Color::Indexed((code - 40) as u8)),
                48 => {
                    // Extended background color: 48;5;n (256 color) or 48;2;r;g;b (True Color)
                    if let Some(color) = self.parse_extended_color(&mut iter) {
                        self.grid.set_bg(color);
                    }
                }
                49 => self.grid.set_bg(Color::Default),
                // Overline
                53 => self.grid.set_attr(CellAttrs::OVERLINE),
                55 => self.grid.clear_attr(CellAttrs::OVERLINE),
                // Underline color
                58 => {
                    // Extended underline color: 58;5;n (256 color) or 58;2;r;g;b (True Color)
                    if let Some(color) = self.parse_extended_color(&mut iter) {
                        self.grid.set_underline_color(color);
                    }
                }
                59 => self.grid.reset_underline_color(),
                // Foreground color (bright 8 colors)
                90..=97 => self.grid.set_fg(Color::Indexed((code - 90 + 8) as u8)),
                // Background color (bright 8 colors)
                100..=107 => self.grid.set_bg(Color::Indexed((code - 100 + 8) as u8)),
                _ => {
                    trace!("Unhandled SGR: {}", code);
                }
            }
        }
    }

    /// Parse extended color (semicolon-separated)
    /// Format: 38;5;n or 38;2;r;g;b
    fn parse_extended_color(
        &self,
        iter: &mut std::iter::Peekable<std::slice::Iter<'_, Vec<u16>>>,
    ) -> Option<Color> {
        let mode = iter.next()?.first().copied()?;
        match mode {
            5 => {
                // 256 colors: 38;5;n
                let idx = iter.next()?.first().copied()?;
                Some(Color::Indexed(idx as u8))
            }
            2 => {
                // True Color: 38;2;r;g;b
                let r = iter.next()?.first().copied()? as u8;
                let g = iter.next()?.first().copied()? as u8;
                let b = iter.next()?.first().copied()? as u8;
                Some(Color::Rgb(r, g, b))
            }
            _ => None,
        }
    }

    /// Handle sub-parameters (colon-separated)
    /// Examples: 38:2:r:g:b (SGR True Color, colon format)
    ///           4:3 (curly underline)
    fn handle_sgr_subparams(&mut self, subparams: &[u16]) {
        if subparams.is_empty() {
            return;
        }

        match subparams[0] {
            4 => {
                // Underline style: 4:x (CSI 4:x m)
                let style = subparams.get(1).copied().unwrap_or(1);
                let underline_style = match style {
                    0 => UnderlineStyle::None,
                    1 => UnderlineStyle::Single,
                    2 => UnderlineStyle::Double,
                    3 => UnderlineStyle::Curly,
                    4 => UnderlineStyle::Dotted,
                    5 => UnderlineStyle::Dashed,
                    _ => UnderlineStyle::Single, // Unknown style falls back to single
                };
                self.grid.set_underline_style(underline_style);
            }
            38 => {
                // Foreground color
                if let Some(color) = self.parse_colon_color(subparams) {
                    self.grid.set_fg(color);
                }
            }
            48 => {
                // Background color
                if let Some(color) = self.parse_colon_color(subparams) {
                    self.grid.set_bg(color);
                }
            }
            58 => {
                // Underline color: 58:2:r:g:b or 58:5:n
                if let Some(color) = self.parse_colon_color(subparams) {
                    self.grid.set_underline_color(color);
                }
            }
            _ => {
                trace!("Unhandled SGR sub-parameters: {:?}", subparams);
            }
        }
    }

    /// Parse colon-separated color
    /// Format: 38:5:n or 38:2:r:g:b (also supports 38:2:colorspace:r:g:b)
    fn parse_colon_color(&self, subparams: &[u16]) -> Option<Color> {
        if subparams.len() < 3 {
            return None;
        }

        match subparams[1] {
            5 => {
                // 256 colors: 38:5:n
                Some(Color::Indexed(subparams.get(2).copied()? as u8))
            }
            2 => {
                // True Color
                if subparams.len() >= 6 {
                    // 38:2:colorspace:r:g:b (ignore colorspace)
                    Some(Color::Rgb(
                        subparams[3] as u8,
                        subparams[4] as u8,
                        subparams[5] as u8,
                    ))
                } else if subparams.len() >= 5 {
                    // 38:2:r:g:b
                    Some(Color::Rgb(
                        subparams[2] as u8,
                        subparams[3] as u8,
                        subparams[4] as u8,
                    ))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// OSC 52 (clipboard operation) handler
    /// Format: ESC ] 52 ; <selection> ; <base64-data> ST
    fn handle_osc_52(&mut self, params: &[&[u8]]) {
        // Max payload size (10MB base64 = ~7.5MB decoded)
        const MAX_OSC52_PAYLOAD: usize = 10 * 1024 * 1024;

        // params[0] = "52", params[1] = selection type (e.g. "c"), params[2] = data
        trace!(
            "OSC 52: params.len()={}, params={:?}",
            params.len(),
            params
                .iter()
                .map(|p| String::from_utf8_lossy(p).to_string())
                .collect::<Vec<_>>()
        );
        if params.len() < 3 {
            trace!("OSC 52: params not enough, returning");
            return;
        }

        let data = params[2];

        // Security: reject oversized payloads
        if data.len() > MAX_OSC52_PAYLOAD {
            warn!("OSC 52: payload too large ({} bytes), ignoring", data.len());
            return;
        }

        if data == b"?" {
            // Query: respond with current clipboard contents in base64
            use std::fmt::Write;
            let encoded = base64_encode(self.clipboard.as_bytes());
            let mut response = String::new();
            write!(&mut response, "\x1b]52;c;{}\x1b\\", encoded).ok();
            self.pty_response.extend_from_slice(response.as_bytes());
        } else {
            // Set: decode base64 and store in clipboard
            if let Some(decoded) = base64_decode(data) {
                if let Ok(text) = String::from_utf8(decoded) {
                    *self.clipboard = text.clone();
                    let _ = std::fs::write(self.clipboard_path, &text);
                    trace!("OSC 52: clipboard set ({} chars)", text.len());
                }
            }
        }
    }

    /// OSC 7 (current directory) handler
    /// Format: ESC ] 7 ; file://hostname/path ST
    fn handle_osc_7(&mut self, params: &[&[u8]]) {
        if params.len() < 2 {
            return;
        }

        let uri = String::from_utf8_lossy(params[1]);
        if let Some(path) = uri.strip_prefix("file://") {
            // Extract path part from hostname/path
            if let Some(pos) = path.find('/') {
                let dir_path = &path[pos..];
                *self.current_dir = Some(dir_path.to_string());
                trace!("OSC 7: current directory = {}", dir_path);
            }
        }
    }

    /// OSC 8 (hyperlink) handler
    /// Format: ESC ] 8 ; params ; URI ST
    fn handle_osc_8(&mut self, params: &[&[u8]]) {
        // params[0] = "8", params[1] = link params (e.g. "id=xxx"), params[2] = URI
        let uri = if params.len() > 2 {
            String::from_utf8_lossy(params[2]).to_string()
        } else if params.len() > 1 {
            // If params is in "8;URI" format
            String::from_utf8_lossy(params[1]).to_string()
        } else {
            String::new()
        };

        if uri.is_empty() {
            // Link end
            self.grid.current_hyperlink = None;
        } else {
            // Link start
            let id = if params.len() > 1 {
                String::from_utf8_lossy(params[1])
                    .split(';')
                    .find_map(|p| p.strip_prefix("id="))
                    .map(|s| s.to_string())
            } else {
                None
            };

            self.grid.current_hyperlink = Some(Arc::new(Hyperlink { id, url: uri }));
        }
    }

    /// OSC 10 (foreground color query/set) handler
    /// Format: ESC ] 10 ; ? ST (query)
    /// Format: ESC ] 10 ; rgb:RRRR/GGGG/BBBB ST (set)
    /// Format: ESC ] 10 ; #RRGGBB ST (set)
    fn handle_osc_10(&mut self, params: &[&[u8]]) {
        let param = if params.len() > 1 {
            params[1]
        } else {
            return;
        };

        if param == b"?" {
            // Query: return current foreground color
            let (r, g, b) = self.grid.colors.fg.unwrap_or((0xff, 0xff, 0xff));
            // X11 format: rgb:RRRR/GGGG/BBBB (16bit, duplicate 8bit value)
            let response = format!(
                "\x1b]10;rgb:{:02x}{:02x}/{:02x}{:02x}/{:02x}{:02x}\x1b\\",
                r, r, g, g, b, b
            );
            self.pty_response.extend_from_slice(response.as_bytes());
        } else {
            // Set: parse color value
            if let Some(rgb) = parse_osc_color(param) {
                self.grid.colors.fg = Some(rgb);
                trace!("OSC 10: foreground color set to {:?}", rgb);
            }
        }
    }

    /// OSC 11 (background color query/set) handler
    /// Format: ESC ] 11 ; ? ST (query)
    /// Format: ESC ] 11 ; rgb:RRRR/GGGG/BBBB ST (set)
    /// Format: ESC ] 11 ; #RRGGBB ST (set)
    fn handle_osc_11(&mut self, params: &[&[u8]]) {
        let param = if params.len() > 1 {
            params[1]
        } else {
            return;
        };

        if param == b"?" {
            // Query: return current background color
            let (r, g, b) = self.grid.colors.bg.unwrap_or((0x00, 0x00, 0x00));
            // X11 format: rgb:RRRR/GGGG/BBBB (16bit, duplicate 8bit value)
            let response = format!(
                "\x1b]11;rgb:{:02x}{:02x}/{:02x}{:02x}/{:02x}{:02x}\x1b\\",
                r, r, g, g, b, b
            );
            self.pty_response.extend_from_slice(response.as_bytes());
        } else {
            // Set: parse color value
            if let Some(rgb) = parse_osc_color(param) {
                self.grid.colors.bg = Some(rgb);
                trace!("OSC 11: background color set to {:?}", rgb);
            }
        }
    }

    /// OSC 133 - Shell Integration (iTerm2/FinalTerm compatible)
    fn handle_osc_133(&mut self, params: &[&[u8]]) {
        if params.len() < 2 {
            return;
        }

        let marker = std::str::from_utf8(params[1]).unwrap_or("");
        match marker {
            "A" => {
                // Prompt started (fresh line)
                trace!("Shell integration: prompt started");
                self.grid.shell.prompt_row = Some(self.grid.cursor_row);
            }
            "B" => {
                // Prompt ended, command input starts
                trace!("Shell integration: prompt ended, command input starts");
            }
            "C" => {
                // Command started (user pressed enter)
                trace!("Shell integration: command execution started");
                self.grid.shell.command_row = Some(self.grid.cursor_row);
            }
            _ if marker.starts_with("D") => {
                // Command finished: D or D;exit_code
                let exit_code = if marker.len() > 1 {
                    marker[2..].parse::<i32>().ok()
                } else if params.len() > 2 {
                    std::str::from_utf8(params[2])
                        .ok()
                        .and_then(|s| s.parse::<i32>().ok())
                } else {
                    None
                };
                trace!(
                    "Shell integration: command finished, exit_code={:?}",
                    exit_code
                );
                self.grid.shell.last_exit_code = exit_code;
                self.grid.shell.command_row = None;
            }
            _ => {
                trace!("Unhandled OSC 133: marker={}", marker);
            }
        }
    }

    /// OSC 0/1/2 (window title) handler
    /// Format: ESC ] 0 ; title ST (icon + title)
    /// Format: ESC ] 2 ; title ST (title only)
    fn handle_osc_title(&mut self, params: &[&[u8]]) {
        if params.len() < 2 {
            return;
        }
        let title = String::from_utf8_lossy(params[1]).to_string();
        self.grid.window_title = Some(title.clone());
        trace!("OSC title: {}", title);
    }

    /// OSC 4 (color palette) handler
    /// Format: ESC ] 4 ; index ; color ST (set)
    /// Format: ESC ] 4 ; index ; ? ST (query)
    fn handle_osc_4(&mut self, params: &[&[u8]]) {
        // OSC 4 can have multiple index;color pairs: ESC]4;idx1;color1;idx2;color2 ST
        // vte splits on ';', so params = ["4", idx1, color1, idx2, color2, ...]
        if params.len() < 3 {
            return;
        }

        let mut i = 1;
        while i + 1 < params.len() {
            let index_str = std::str::from_utf8(params[i]).unwrap_or("");
            let index: u8 = match index_str.parse::<u16>() {
                Ok(v) if v < 256 => v as u8,
                _ => { i += 2; continue; }
            };

            let data = params[i + 1];
            if data == b"?" {
                // Query: respond with current color
                let (r, g, b) = self.grid.get_palette_color(index);
                let response = format!(
                    "\x1b]4;{};rgb:{:02x}{:02x}/{:02x}{:02x}/{:02x}{:02x}\x1b\\",
                    index, r, r, g, g, b, b
                );
                self.pty_response.extend_from_slice(response.as_bytes());
            } else {
                // Set: parse color value
                if let Some((r, g, b)) = parse_osc_color(data) {
                    self.grid.set_palette_color(index, r, g, b);
                    trace!("OSC 4: palette[{}] = ({},{},{})", index, r, g, b);
                } else {
                    trace!("OSC 4: failed to parse color for palette[{}]: {:?}",
                          index, std::str::from_utf8(data));
                }
            }
            i += 2;
        }
    }

    /// OSC 12 (cursor color query/set) handler
    fn handle_osc_12(&mut self, params: &[&[u8]]) {
        let param = if params.len() > 1 {
            params[1]
        } else {
            return;
        };

        if param == b"?" {
            // Query: return current cursor color
            let (r, g, b) = self.grid.colors.cursor.unwrap_or((0xff, 0xff, 0xff));
            let response = format!(
                "\x1b]12;rgb:{:02x}{:02x}/{:02x}{:02x}/{:02x}{:02x}\x1b\\",
                r, r, g, g, b, b
            );
            self.pty_response.extend_from_slice(response.as_bytes());
        } else {
            // Set: parse color value
            if let Some(rgb) = parse_osc_color(param) {
                self.grid.colors.cursor = Some(rgb);
                trace!("OSC 12: cursor color set to {:?}", rgb);
            }
        }
    }

    /// OSC 104 (reset palette color) handler
    /// Format: ESC ] 104 ST (reset all)
    /// Format: ESC ] 104 ; index ST (reset one)
    fn handle_osc_104(&mut self, params: &[&[u8]]) {
        if params.len() < 2 {
            // Reset all palette colors
            self.grid.reset_palette_color(None);
        } else {
            let index_str = std::str::from_utf8(params[1]).unwrap_or("");
            if let Ok(idx) = index_str.parse::<u16>() {
                if idx < 256 {
                    self.grid.reset_palette_color(Some(idx as u8));
                }
            } else {
                // No valid index = reset all
                self.grid.reset_palette_color(None);
            }
        }
    }

    /// OSC 9 (iTerm2 notification / ConEmu progress) handler
    /// Note: vte crate splits on ';', so params are already separated:
    ///   OSC 9 ; message ST          → params = ["9", "message"]
    ///   OSC 9 ; 4 ; state ; pct ST  → params = ["9", "4", "state", "pct"]
    ///   OSC 9 ; 4 ST                → params = ["9", "4"]
    fn handle_osc_9(&mut self, params: &[&[u8]]) {
        if params.len() < 2 {
            return;
        }

        let first = std::str::from_utf8(params[1]).unwrap_or("");

        // Progress: params[1] == "4"
        if first == "4" {
            if params.len() >= 4 {
                // OSC 9;4;state;percent → params = ["9","4","state","percent"]
                let state = std::str::from_utf8(params[2])
                    .ok()
                    .and_then(|s| s.parse::<u8>().ok())
                    .unwrap_or(0);
                let percent = std::str::from_utf8(params[3])
                    .ok()
                    .and_then(|s| s.parse::<u8>().ok())
                    .unwrap_or(0)
                    .min(100);
                if state == 0 {
                    *self.active_progress = None;
                    debug!("OSC 9: progress stopped (state=0)");
                } else {
                    *self.active_progress = Some(NotificationProgress { state, percent });
                    debug!("OSC 9: progress state={} percent={}%", state, percent);
                }
            } else {
                // OSC 9;4 → stop progress
                *self.active_progress = None;
                debug!("OSC 9: progress stopped");
            }
            return;
        }

        // Regular notification — rejoin remaining params with ';' (message may contain ';')
        let mut title = first.to_string();
        for p in &params[2..] {
            title.push(';');
            title.push_str(&String::from_utf8_lossy(p));
        }
        info!("OSC 9: notification '{}'", title);
        self.push_notification(Notification {
            id: None,
            title,
            body: String::new(),
            urgency: 1,
            timestamp: std::time::Instant::now(),
        });
    }

    /// OSC 99 (Kitty notification protocol) handler
    /// Format: OSC 99 ; metadata ; payload ST
    /// Metadata: colon-separated key=value pairs (e.g., "i=id:d=1:p=title")
    fn handle_osc_99(&mut self, params: &[&[u8]]) {
        if params.len() < 2 {
            return;
        }

        // Parse metadata from params[1]
        let meta_str = String::from_utf8_lossy(params[1]);
        let payload = if params.len() > 2 {
            String::from_utf8_lossy(params[2]).to_string()
        } else {
            String::new()
        };

        // Parse key=value pairs from metadata
        let mut id: Option<String> = None;
        let mut done = true; // d=1 is default (complete)
        let mut payload_type = String::from("title"); // p=title is default
        let mut urgency: u8 = 1;

        for kv in meta_str.split(':') {
            if let Some((key, value)) = kv.split_once('=') {
                match key {
                    "i" => id = Some(value.to_string()),
                    "d" => done = value != "0",
                    "p" => payload_type = value.to_string(),
                    "u" => urgency = value.parse().unwrap_or(1).min(2),
                    "e" => {
                        // e=1 means close notification with this id
                        if value == "1" {
                            if let Some(ref close_id) = id {
                                self.pending_notifications.remove(close_id);
                            }
                            return;
                        }
                    }
                    _ => {}
                }
            }
        }

        // Handle query (p=?)
        if payload_type == "?" {
            // Report capabilities per Kitty notification spec:
            //   metadata: i=<id>:p=?  (echo back query marker)
            //   payload:  p=title,body:a=focus:o=always:u=0,1,2
            let resp_id = id.as_deref().unwrap_or("");
            let response = format!(
                "\x1b]99;i={}:p=?;p=title,body:a=focus:o=always:u=0,1,2\x1b\\",
                resp_id
            );
            self.pty_response.extend_from_slice(response.as_bytes());
            debug!("OSC 99: query response sent");
            return;
        }

        if !done {
            // Incomplete notification: store/update in pending
            let notif_id = id.clone().unwrap_or_default();
            let entry = self.pending_notifications.entry(notif_id).or_insert_with(|| Notification {
                id: id.clone(),
                title: String::new(),
                body: String::new(),
                urgency,
                timestamp: std::time::Instant::now(),
            });
            match payload_type.as_str() {
                "title" => entry.title = payload,
                "body" => entry.body = payload,
                _ => {}
            }
            debug!("OSC 99: pending notification updated (id={:?})", id);
        } else {
            // Complete notification
            let notif_id = id.clone().unwrap_or_default();

            // Check if there's a pending notification to complete
            let mut notif = if let Some(mut pending) = self.pending_notifications.remove(&notif_id) {
                // Update with final payload
                match payload_type.as_str() {
                    "title" => pending.title = payload,
                    "body" => pending.body = payload,
                    _ => {}
                }
                pending.urgency = urgency;
                pending
            } else {
                // New complete notification
                let mut n = Notification {
                    id: id.clone(),
                    title: String::new(),
                    body: String::new(),
                    urgency,
                    timestamp: std::time::Instant::now(),
                };
                match payload_type.as_str() {
                    "title" => n.title = payload,
                    "body" => n.body = payload,
                    _ => {}
                }
                n
            };

            // Default title if empty
            if notif.title.is_empty() && !notif.body.is_empty() {
                notif.title = notif.body.clone();
                notif.body = String::new();
            }

            info!("OSC 99: notification '{}' (id={:?})", notif.title, notif.id);
            self.push_notification(notif);
        }
    }

    /// Push a notification to the history list (capped at MAX_NOTIFICATIONS)
    fn push_notification(&mut self, notif: Notification) {
        if !*self.notifications_enabled {
            return;
        }
        self.notifications.push(notif);
        if self.notifications.len() > super::MAX_NOTIFICATIONS {
            self.notifications.remove(0);
        }
    }

    /// XTGETTCAP (DCS + q) handler
    /// Query: DCS + q Pt ST (Pt is hex-encoded capability name, ; separated)
    /// Response: DCS 1 + r Pt = Pv ST (capability exists) or DCS 0 + r Pt ST (not found)
    fn handle_xtgettcap(&mut self, buffer: &[u8]) {
        let query_str = String::from_utf8_lossy(buffer);
        trace!("XTGETTCAP: query={}", query_str);

        let mut response = Vec::new();

        // Multiple queries are separated by ;
        for hex_cap in query_str.split(';') {
            if hex_cap.is_empty() {
                continue;
            }

            // Hex decode to get capability name
            let cap_name = hex_decode(hex_cap.as_bytes());
            let cap_str = String::from_utf8_lossy(&cap_name);
            trace!("XTGETTCAP: capability={}", cap_str);

            // Get capability response value
            if let Some(value) = self.get_termcap_value(&cap_str) {
                // Capability exists: DCS 1 + r Pt = Pv ST
                response.extend_from_slice(b"\x1bP1+r");
                response.extend_from_slice(hex_cap.as_bytes());
                response.push(b'=');
                response.extend_from_slice(&hex_encode(value.as_bytes()));
                response.extend_from_slice(b"\x1b\\");
            } else {
                // Capability not found: DCS 0 + r Pt ST
                response.extend_from_slice(b"\x1bP0+r");
                response.extend_from_slice(hex_cap.as_bytes());
                response.extend_from_slice(b"\x1b\\");
            }
        }

        if !response.is_empty() {
            trace!("XTGETTCAP: sending {} bytes response", response.len());
            self.pty_response.extend_from_slice(&response);
        }
    }

    /// DECRQSS (DCS $ q) handler
    /// Query: DCS $ q Pt ST (Pt is the setting to query)
    /// Response: DCS Ps $ r Pt ST (Ps=1 valid, Ps=0 invalid)
    fn handle_decrqss(&mut self, buffer: &[u8]) {
        let query = std::str::from_utf8(buffer).unwrap_or("");
        trace!("DECRQSS: query={:?}", query);

        let response = match query {
            // SGR (Select Graphic Rendition)
            "m" => {
                // Report current SGR state as "0m" (simplified - always report reset)
                // A full implementation would serialize the current pen state
                Some("\x1bP1$r0m\x1b\\".to_string())
            }
            // DECSTBM (scroll region)
            "r" => {
                let (top, bottom) = self.grid.scroll_region_1based();
                Some(format!("\x1bP1$r{};{}r\x1b\\", top, bottom))
            }
            // DECSCUSR (cursor style)
            " q" => {
                let style_code = match (self.grid.cursor.style, self.grid.cursor.blink) {
                    (CursorStyle::Block, true) => 1,
                    (CursorStyle::Block, false) => 2,
                    (CursorStyle::Underline, true) => 3,
                    (CursorStyle::Underline, false) => 4,
                    (CursorStyle::Bar, true) => 5,
                    (CursorStyle::Bar, false) => 6,
                };
                Some(format!("\x1bP1$r{} q\x1b\\", style_code))
            }
            _ => {
                // Not recognized
                trace!("DECRQSS: unknown setting {:?}", query);
                Some(format!("\x1bP0$r\x1b\\"))
            }
        };

        if let Some(resp) = response {
            self.pty_response.extend_from_slice(resp.as_bytes());
        }
    }

    /// Return termcap/terminfo capability value
    fn get_termcap_value(&self, cap: &str) -> Option<String> {
        match cap {
            // Terminal name
            "TN" => Some("bcon".to_string()),
            // True color support (RGB)
            "RGB" => Some("1".to_string()),
            // Tc (tmux-style true color flag)
            "Tc" => Some("".to_string()),
            // Number of colors
            "Co" | "colors" => Some("256".to_string()),
            // Sixel support
            "sixel" => Some("1".to_string()),
            // Set underline style (kitty extension)
            "Smulx" => Some("\x1b[4:%p1%dm".to_string()),
            // Set underline color
            "Setulc" => Some("\x1b[58:2::%p1%d:%p2%d:%p3%dm".to_string()),
            // Cursor style (DECSCUSR)
            "Ss" => Some("\x1b[%p1%d q".to_string()),
            // Reset cursor style
            "Se" => Some("\x1b[2 q".to_string()),
            // OSC 52 clipboard (Ms = set selection)
            "Ms" => Some("\x1b]52;%p1%s;%p2%s\x1b\\".to_string()),
            // Bracketed paste
            "BE" => Some("\x1b[?2004h".to_string()),
            "BD" => Some("\x1b[?2004l".to_string()),
            "PS" => Some("\x1b[200~".to_string()),
            "PE" => Some("\x1b[201~".to_string()),
            // Synchronized output (CSI ? 2026 h/l)
            "Sync" => Some("".to_string()),
            // Focus events (CSI ? 1004 h/l)
            "focus" => Some("".to_string()),
            // Kitty keyboard protocol version
            "fullkbd" => Some("1".to_string()),
            // Unsupported capability
            _ => {
                trace!("XTGETTCAP: unknown capability: {}", cap);
                None
            }
        }
    }
}

// ========== Hex encode/decode (for XTGETTCAP) ==========

fn hex_encode(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len() * 2);
    for &byte in input {
        output.push(b"0123456789ABCDEF"[(byte >> 4) as usize]);
        output.push(b"0123456789ABCDEF"[(byte & 0x0F) as usize]);
    }
    output
}

fn hex_decode(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len() / 2);
    let mut iter = input.iter();
    while let (Some(&hi), Some(&lo)) = (iter.next(), iter.next()) {
        let hi = match hi {
            b'0'..=b'9' => hi - b'0',
            b'A'..=b'F' => hi - b'A' + 10,
            b'a'..=b'f' => hi - b'a' + 10,
            _ => continue,
        };
        let lo = match lo {
            b'0'..=b'9' => lo - b'0',
            b'A'..=b'F' => lo - b'A' + 10,
            b'a'..=b'f' => lo - b'a' + 10,
            _ => continue,
        };
        output.push((hi << 4) | lo);
    }
    output
}

// ========== Base64 encode/decode ==========

const BASE64_TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(input: &[u8]) -> String {
    let mut output = String::with_capacity((input.len() + 2) / 3 * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;

        output.push(BASE64_TABLE[((triple >> 18) & 0x3F) as usize] as char);
        output.push(BASE64_TABLE[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            output.push(BASE64_TABLE[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            output.push('=');
        }
        if chunk.len() > 2 {
            output.push(BASE64_TABLE[(triple & 0x3F) as usize] as char);
        } else {
            output.push('=');
        }
    }
    output
}

fn base64_decode(input: &[u8]) -> Option<Vec<u8>> {
    let mut output = Vec::with_capacity(input.len() * 3 / 4);
    let mut buf: u32 = 0;
    let mut bits: u32 = 0;

    for &byte in input {
        let val = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\n' | b'\r' | b' ' => continue,
            _ => return None,
        };
        buf = (buf << 6) | val as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    Some(output)
}
