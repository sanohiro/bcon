//! VT escape sequence parser
//!
//! Implements vte crate's Perform trait
//! and applies parsed results to Grid.

use std::sync::Arc;

use log::{info, trace, warn};
use vte::{Params, Perform};

use super::grid::{CellAttrs, Color, CursorStyle, Grid, Hyperlink, UnderlineStyle};
use super::sixel::SixelDecoder;
use super::{DcsHandler, ImageRegistry, TerminalImage};

/// vte::Perform implementation
/// Holds reference to Grid and directly applies parsed results
pub struct Performer<'a> {
    pub grid: &'a mut Grid,
    pub clipboard: &'a mut String,
    pub pty_response: Vec<u8>,
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
    ) -> Self {
        Self {
            grid,
            clipboard,
            pty_response: Vec::new(),
            dcs_handler,
            images,
            cell_width,
            cell_height,
            current_dir,
            clipboard_path,
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
            ('A', []) => {
                // CUU - Cursor Up
                let n = if param0 == 0 { 1 } else { param0 as usize };
                self.grid.move_cursor_up(n);
            }
            ('B', []) => {
                // CUD - Cursor Down
                let n = if param0 == 0 { 1 } else { param0 as usize };
                self.grid.move_cursor_down(n);
            }
            ('C', []) => {
                // CUF - Cursor Forward
                let n = if param0 == 0 { 1 } else { param0 as usize };
                self.grid.move_cursor_forward(n);
            }
            ('D', []) => {
                // CUB - Cursor Backward
                let n = if param0 == 0 { 1 } else { param0 as usize };
                self.grid.move_cursor_backward(n);
            }
            ('E', []) => {
                // CNL - Cursor Next Line
                let n = if param0 == 0 { 1 } else { param0 as usize };
                self.grid.move_cursor_down(n);
                self.grid.carriage_return();
            }
            ('F', []) => {
                // CPL - Cursor Previous Line
                let n = if param0 == 0 { 1 } else { param0 as usize };
                self.grid.move_cursor_up(n);
                self.grid.carriage_return();
            }
            ('H' | 'f', []) => {
                // CUP - Cursor Position
                let row = if param0 == 0 { 1 } else { param0 as usize };
                let col = flat_params
                    .get(1)
                    .and_then(|p| p.first().copied())
                    .map(|v| if v == 0 { 1 } else { v as usize })
                    .unwrap_or(1);
                self.grid.move_cursor_to(row, col);
            }
            ('G', []) => {
                // CHA - Cursor Horizontal Absolute
                let col = if param0 == 0 { 1 } else { param0 as usize };
                self.grid.move_cursor_to(self.grid.cursor_row + 1, col);
            }
            ('d', []) => {
                // VPA - Vertical Position Absolute
                let row = if param0 == 0 { 1 } else { param0 as usize };
                self.grid.move_cursor_to(row, self.grid.cursor_col + 1);
            }
            ('J', []) => {
                // ED - Erase in Display
                self.grid.erase_in_display(param0);
            }
            ('K', []) => {
                // EL - Erase in Line
                self.grid.erase_in_line(param0);
            }
            ('L', []) => {
                // IL - Insert Line
                let n = if param0 == 0 { 1 } else { param0 as usize };
                self.grid.insert_lines(n);
            }
            ('M', []) => {
                // DL - Delete Line
                let n = if param0 == 0 { 1 } else { param0 as usize };
                self.grid.delete_lines(n);
            }
            ('P', []) => {
                // DCH - Delete Character
                let n = if param0 == 0 { 1 } else { param0 as usize };
                self.grid.delete_chars(n);
            }
            ('@', []) => {
                // ICH - Insert Character
                let n = if param0 == 0 { 1 } else { param0 as usize };
                self.grid.insert_chars(n);
            }
            ('X', []) => {
                // ECH - Erase Character
                let n = if param0 == 0 { 1 } else { param0 as usize };
                self.grid.erase_chars(n);
            }
            ('S', []) => {
                // SU - Scroll Up
                let n = if param0 == 0 { 1 } else { param0 as usize };
                self.grid.scroll_up(n);
            }
            ('T', []) => {
                // SD - Scroll Down
                let n = if param0 == 0 { 1 } else { param0 as usize };
                self.grid.scroll_down(n);
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
                        self.pty_response = b"\x1b[0n".to_vec();
                    }
                    6 => {
                        // Cursor position report: ESC [ row ; col R
                        let row = self.grid.cursor_row + 1;
                        let col = self.grid.cursor_col + 1;
                        self.pty_response = format!("\x1b[{};{}R", row, col).into_bytes();
                    }
                    _ => {}
                }
            }
            ('c', []) | ('c', [b'?']) => {
                // DA1 - Primary Device Attributes
                // Report VT220 compatible + feature flags
                // 62: VT220, 1: 132 columns, 4: Sixel, 22: ANSI color, 29: ANSI text locator (mouse)
                self.pty_response = b"\x1b[?62;1;4;22;29c".to_vec();
            }
            ('c', [b'>']) => {
                // DA2 - Secondary Device Attributes
                // >Pp;Pv;Pc c (Pp=terminal type, Pv=firmware version, Pc=ROM number)
                // 1: VT220, 100: bcon version 0.1.0 (encoded as 100), 0: ROM
                self.pty_response = b"\x1b[>1;100;0c".to_vec();
            }
            ('c', [b'=']) => {
                // DA3 - Tertiary Device Attributes (Unit ID)
                // =XXXXXXXX ST (hex unit id)
                self.pty_response = b"\x1bP!|00000000\x1b\\".to_vec();
            }
            ('q', [b'>']) => {
                // XTVERSION - Terminal version query
                // DCS > | Pt ST
                self.pty_response = b"\x1bP>|bcon 0.1.0\x1b\\".to_vec();
            }
            ('b', []) => {
                // REP - Repeat preceding character
                let n = if param0 == 0 { 1 } else { param0 as usize };
                self.grid.repeat_char(n);
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
                    self.grid.modify_other_keys = level.min(2);
                    trace!("modifyOtherKeys: level={}", self.grid.modify_other_keys);
                }
            }
            ('u', [b'>']) => {
                // Kitty keyboard: Push mode
                // CSI > flags u
                self.grid.kitty_keyboard_flags = param0 as u32;
                trace!("Kitty keyboard: push flags={}", param0);
            }
            ('u', [b'<']) => {
                // Kitty keyboard: Pop mode
                self.grid.kitty_keyboard_flags = 0;
                trace!("Kitty keyboard: pop");
            }
            ('u', [b'?']) => {
                // Kitty keyboard: Query mode
                // Response: CSI ? flags u
                let flags = self.grid.kitty_keyboard_flags;
                self.pty_response = format!("\x1b[?{}u", flags).into_bytes();
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
                    1 => self.grid.kitty_keyboard_flags = flags, // set
                    2 => self.grid.kitty_keyboard_flags |= flags, // or
                    3 => self.grid.kitty_keyboard_flags &= !flags, // not
                    _ => {}
                }
                trace!("Kitty keyboard: set flags={} mode={}", flags, mode);
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
                // DECSET (Set Private Mode)
                self.handle_decset(param0, true);
            }
            ('l', [b'?']) => {
                // DECRST (Reset Private Mode)
                self.handle_decset(param0, false);
            }
            ('q', [b' ']) => {
                // DECSCUSR - Set Cursor Style
                let style = param0;
                match style {
                    0 | 1 => {
                        // 0: default, 1: blinking block
                        self.grid.cursor_style = CursorStyle::Block;
                        self.grid.cursor_blink = true;
                    }
                    2 => {
                        // Steady block
                        self.grid.cursor_style = CursorStyle::Block;
                        self.grid.cursor_blink = false;
                    }
                    3 => {
                        // Blinking underline
                        self.grid.cursor_style = CursorStyle::Underline;
                        self.grid.cursor_blink = true;
                    }
                    4 => {
                        // Steady underline
                        self.grid.cursor_style = CursorStyle::Underline;
                        self.grid.cursor_blink = false;
                    }
                    5 => {
                        // Blinking bar
                        self.grid.cursor_style = CursorStyle::Bar;
                        self.grid.cursor_blink = true;
                    }
                    6 => {
                        // Steady bar
                        self.grid.cursor_style = CursorStyle::Bar;
                        self.grid.cursor_blink = false;
                    }
                    _ => {}
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
                        self.pty_response = format!(
                            "\x1b[4;{};{}t",
                            height_px, width_px
                        ).into_bytes();
                    }
                    16 => {
                        // Report cell size in pixels
                        // Response: CSI 6 ; height ; width t
                        trace!("CSI 16 t: responding cell {}x{}", self.cell_width, self.cell_height);
                        self.pty_response = format!(
                            "\x1b[6;{};{}t",
                            self.cell_height, self.cell_width
                        ).into_bytes();
                    }
                    18 => {
                        // Report window size in characters
                        // Response: CSI 8 ; rows ; cols t
                        trace!("CSI 18 t: responding {}x{} chars", self.grid.cols(), self.grid.rows());
                        self.pty_response = format!(
                            "\x1b[8;{};{}t",
                            self.grid.rows(), self.grid.cols()
                        ).into_bytes();
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
                *self.grid = Grid::new(cols, rows);
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
        trace!("DCS hook: action='{}', intermediates={:?}, params={:?}",
            action, intermediates, params.iter().map(|p| p.to_vec()).collect::<Vec<_>>());

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
                buffer.push(byte);
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
            "52" => self.handle_osc_52(params),
            "7" => self.handle_osc_7(params),
            "8" => self.handle_osc_8(params),
            "10" => self.handle_osc_10(params),
            "11" => self.handle_osc_11(params),
            "133" => self.handle_osc_133(params),
            _ => {
                trace!("Unhandled OSC: cmd={}", cmd);
            }
        }
    }
}

impl<'a> Performer<'a> {
    /// DECSET/DECRST handling
    fn handle_decset(&mut self, mode: u16, enable: bool) {
        match mode {
            1 => {
                // DECCKM - Application Cursor Keys
                self.grid.application_cursor_keys = enable;
            }
            7 => {
                // DECAWM - Auto-wrap Mode
                self.grid.auto_wrap = enable;
            }
            25 => {
                // DECTCEM - Text Cursor Enable Mode
                self.grid.cursor_visible = enable;
            }
            1049 => {
                // Alternate Screen Buffer
                if enable {
                    self.grid.enter_alternate_screen();
                } else {
                    self.grid.leave_alternate_screen();
                }
            }
            1000 => {
                // X10 Mouse Tracking
                self.grid.mouse_mode = if enable {
                    super::grid::MouseMode::X10
                } else {
                    super::grid::MouseMode::None
                };
            }
            1002 => {
                // Button Event Mouse Tracking
                self.grid.mouse_mode = if enable {
                    super::grid::MouseMode::ButtonEvent
                } else {
                    super::grid::MouseMode::None
                };
            }
            1003 => {
                // Any Event Mouse Tracking
                self.grid.mouse_mode = if enable {
                    super::grid::MouseMode::AnyEvent
                } else {
                    super::grid::MouseMode::None
                };
            }
            1006 => {
                // SGR Extended Mouse Mode
                self.grid.mouse_sgr = enable;
            }
            2004 => {
                // Bracketed Paste Mode
                self.grid.bracketed_paste = enable;
            }
            1004 => {
                // Focus Event Mode
                self.grid.send_focus_events = enable;
            }
            2026 => {
                // Synchronized Update Mode
                self.grid.synchronized_update = enable;
            }
            _ => {
                trace!("Unhandled DEC private mode: {} = {}", mode, enable);
            }
        }
    }

    /// SGR (Select Graphic Rendition) handling
    fn handle_sgr(&mut self, params: &[Vec<u16>]) {
        // No parameters -> reset
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
        trace!("OSC 52: params.len()={}, params={:?}",
            params.len(),
            params.iter().map(|p| String::from_utf8_lossy(p).to_string()).collect::<Vec<_>>()
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
            self.pty_response = response.into_bytes();
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

    /// OSC 10 (foreground color query) handler
    /// Format: ESC ] 10 ; ? ST
    fn handle_osc_10(&mut self, params: &[&[u8]]) {
        // Query: return foreground color on "?"
        let query = if params.len() > 1 {
            params[1]
        } else {
            return;
        };

        if query == b"?" {
            // Default foreground color (white)
            // X11 format: rgb:RRRR/GGGG/BBBB (16bit)
            let response = format!("\x1b]10;rgb:ffff/ffff/ffff\x1b\\");
            self.pty_response = response.into_bytes();
        }
    }

    /// OSC 11 (background color query) handler
    /// Format: ESC ] 11 ; ? ST
    fn handle_osc_11(&mut self, params: &[&[u8]]) {
        // Query: return background color on "?"
        let query = if params.len() > 1 {
            params[1]
        } else {
            return;
        };

        if query == b"?" {
            // Default background color (black)
            // X11 format: rgb:RRRR/GGGG/BBBB (16bit)
            let response = format!("\x1b]11;rgb:0000/0000/0000\x1b\\");
            self.pty_response = response.into_bytes();
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
                self.grid.shell_prompt_row = Some(self.grid.cursor_row);
            }
            "B" => {
                // Prompt ended, command input starts
                trace!("Shell integration: prompt ended, command input starts");
            }
            "C" => {
                // Command started (user pressed enter)
                trace!("Shell integration: command execution started");
                self.grid.shell_command_row = Some(self.grid.cursor_row);
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
                trace!("Shell integration: command finished, exit_code={:?}", exit_code);
                self.grid.shell_last_exit_code = exit_code;
                self.grid.shell_command_row = None;
            }
            _ => {
                trace!("Unhandled OSC 133: marker={}", marker);
            }
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
            self.pty_response = response;
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
