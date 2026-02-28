//! Terminal emulation
//!
//! Core module integrating PTY, VT parser, and character grid
//! to form the terminal emulator.

#![allow(dead_code)]

pub mod grid;
pub mod kitty;
pub mod parser;
pub mod pty;
pub mod sixel;

use std::collections::{HashMap, VecDeque};

use anyhow::Result;
use log::{info, trace};

use grid::{Cell, Grid};
use kitty::KittyDecoder;
use parser::Performer;
use pty::Pty;
use sixel::SixelDecoder;

/// Read buffer size
const READ_BUF_SIZE: usize = 4096;

/// Maximum notification history entries
const MAX_NOTIFICATIONS: usize = 100;

/// Notification from OSC 9 / OSC 99
#[derive(Debug, Clone)]
pub struct Notification {
    /// OSC 99 identifier (i= parameter)
    pub id: Option<String>,
    /// Title (OSC 99 p=title / OSC 9 message)
    pub title: String,
    /// Body text (OSC 99 p=body)
    pub body: String,
    /// Urgency: 0=low, 1=normal, 2=critical
    pub urgency: u8,
    /// When the notification was received
    pub timestamp: std::time::Instant,
}

/// Progress state from OSC 9;4
#[derive(Debug, Clone)]
pub struct NotificationProgress {
    /// 0=stop, 1=normal, 2=error, 3=indeterminate, 4=warning
    pub state: u8,
    /// 0-100 percent
    pub percent: u8,
}

/// Maximum APC buffer size (4MB - for Kitty graphics images)
/// Prevents memory exhaustion from malicious/corrupt input
const MAX_APC_BUFFER_SIZE: usize = 4 * 1024 * 1024;

/// Generate default clipboard file path
/// Uses XDG_RUNTIME_DIR if available, otherwise /tmp
/// Includes PID to make it unique per instance
fn default_clipboard_path() -> String {
    let pid = std::process::id();
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        format!("{}/bcon_clipboard_{}", runtime_dir, pid)
    } else {
        format!("/tmp/bcon_clipboard_{}", pid)
    }
}

/// Check if buffer contains ESC _ (APC start sequence)
/// Manual loop is faster than windows(2).any() for small patterns
#[inline]
fn has_esc_underscore(buf: &[u8]) -> bool {
    if buf.len() < 2 {
        return false;
    }
    for i in 0..buf.len() - 1 {
        if buf[i] == 0x1B && buf[i + 1] == b'_' {
            return true;
        }
    }
    false
}

/// Copy mode state
pub struct CopyModeState {
    /// Copy mode cursor row (display coordinates)
    pub cursor_row: usize,
    /// Copy mode cursor column
    pub cursor_col: usize,
    /// Selection active flag
    pub selecting: bool,
    /// Selection start position (when selecting=true)
    pub anchor_row: usize,
    pub anchor_col: usize,
}

impl CopyModeState {
    pub fn new(cursor_row: usize, cursor_col: usize) -> Self {
        Self {
            cursor_row,
            cursor_col,
            selecting: false,
            anchor_row: 0,
            anchor_col: 0,
        }
    }

    /// Start/toggle selection
    pub fn toggle_selection(&mut self) {
        if self.selecting {
            self.selecting = false;
        } else {
            self.selecting = true;
            self.anchor_row = self.cursor_row;
            self.anchor_col = self.cursor_col;
        }
    }

    /// Return current selection range as Selection
    pub fn get_selection(&self) -> Option<Selection> {
        if self.selecting {
            Some(Selection {
                anchor_row: self.anchor_row,
                anchor_col: self.anchor_col,
                end_row: self.cursor_row,
                end_col: self.cursor_col,
            })
        } else {
            None
        }
    }
}

/// Search state
pub struct SearchState {
    /// Search query
    pub query: String,
    /// Match position list: (row, start_col, end_col)
    /// row is absolute row including scrollback (0 = start of scrollback)
    pub matches: Vec<(usize, usize, usize)>,
    /// Current match index
    pub current_match: usize,
    /// Matches grouped by row for fast lookup (row -> [(start_col, end_col, match_index)])
    row_matches: HashMap<usize, Vec<(usize, usize, usize)>>,
}

impl SearchState {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            matches: Vec::new(),
            current_match: 0,
            row_matches: HashMap::new(),
        }
    }

    /// Build row_matches index from matches
    fn build_row_index(&mut self) {
        self.row_matches.clear();
        for (idx, &(row, start, end)) in self.matches.iter().enumerate() {
            self.row_matches
                .entry(row)
                .or_insert_with(|| Vec::with_capacity(4))
                .push((start, end, idx));
        }
    }

    /// Move to next match
    pub fn next_match(&mut self) {
        if !self.matches.is_empty() {
            self.current_match = (self.current_match + 1) % self.matches.len();
        }
    }

    /// Move to previous match
    pub fn prev_match(&mut self) {
        if !self.matches.is_empty() {
            self.current_match = if self.current_match == 0 {
                self.matches.len() - 1
            } else {
                self.current_match - 1
            };
        }
    }
}

/// Text selection range (display coordinates)
pub struct Selection {
    /// Selection start point (anchor)
    pub anchor_row: usize,
    pub anchor_col: usize,
    /// Selection end point (current position)
    pub end_row: usize,
    pub end_col: usize,
}

impl Selection {
    /// Return normalized range (guarantees start <= end)
    /// Both anchor_col and end_col are stored as inclusive cell positions.
    /// This method returns (sr, sc, er, ec) where sc is inclusive and ec is EXCLUSIVE.
    pub fn normalized(&self) -> (usize, usize, usize, usize) {
        if (self.anchor_row, self.anchor_col) <= (self.end_row, self.end_col) {
            (
                self.anchor_row,
                self.anchor_col,
                self.end_row,
                self.end_col + 1,
            )
        } else {
            (
                self.end_row,
                self.end_col,
                self.anchor_row,
                self.anchor_col + 1,
            )
        }
    }

    /// Get column range for a specific row (returns None if row not in selection)
    /// This is more efficient than calling contains() for every column.
    /// Returns (start_col, end_col) where end_col is exclusive.
    #[inline]
    pub fn cols_for_row(&self, row: usize, max_cols: usize) -> Option<(usize, usize)> {
        let (sr, sc, er, ec) = self.normalized();
        if row < sr || row > er {
            return None;
        }
        let start = if row == sr { sc } else { 0 };
        let end = if row == er {
            ec.min(max_cols)
        } else {
            max_cols
        };
        if start >= end {
            return None;
        }
        Some((start, end))
    }

    /// Check if specified cell is within selection range
    pub fn contains(&self, row: usize, col: usize) -> bool {
        let (sr, sc, er, ec) = self.normalized();
        if row < sr || row > er {
            return false;
        }
        if row == sr && row == er {
            return col >= sc && col < ec;
        }
        if row == sr {
            return col >= sc;
        }
        if row == er {
            return col < ec;
        }
        true // Middle rows are fully selected
    }
}

/// DCS sequence handler
pub enum DcsHandler {
    /// Sixel graphics
    Sixel(SixelDecoder),
    /// XTGETTCAP (terminal capability query)
    XtGetTcap(Vec<u8>),
    /// DECRQSS (request selection or setting)
    Decrqss(Vec<u8>),
}

/// Animation state for animated images
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum AnimationState {
    #[default]
    Stopped,
    Loading,
    Running,
}

/// Single animation frame
#[derive(Debug, Clone)]
pub struct ImageFrame {
    /// Frame number (1-based)
    pub number: u32,
    /// Frame width (can be smaller than image)
    pub width: u32,
    /// Frame height
    pub height: u32,
    /// X offset within image
    pub x: u32,
    /// Y offset within image
    pub y: u32,
    /// Gap to next frame (milliseconds)
    pub gap: u32,
    /// Frame data (RGBA)
    pub data: Vec<u8>,
}

/// Generic image data (shared by Sixel, Kitty)
#[derive(Debug)]
pub struct TerminalImage {
    pub id: u32,
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>, // RGBA - root frame (frame 1)
    /// Additional frames for animation (frame 2+)
    pub frames: Vec<ImageFrame>,
    /// Animation state
    pub animation_state: AnimationState,
    /// Current frame index (0 = root frame)
    pub current_frame: u32,
    /// Loop count (0 = infinite)
    pub loop_count: u32,
    /// Current loop iteration
    pub current_loop: u32,
}

/// Maximum total image memory (512MB)
const MAX_TOTAL_IMAGE_BYTES: usize = 512 * 1024 * 1024;

/// Maximum number of images in registry
const MAX_IMAGE_COUNT: usize = 256;

/// Image registry (manages Sixel, Kitty, etc. images)
pub struct ImageRegistry {
    /// Image map (ID -> TerminalImage)
    images: HashMap<u32, TerminalImage>,
    /// Next ID to assign
    pub next_id: u32,
    /// Total tracked image memory in bytes
    total_bytes: usize,
}

/// Calculate memory usage of a TerminalImage
fn image_byte_size(image: &TerminalImage) -> usize {
    image.data.len() + image.frames.iter().map(|f| f.data.len()).sum::<usize>()
}

impl ImageRegistry {
    /// Create a new registry
    pub fn new() -> Self {
        Self {
            images: HashMap::new(),
            next_id: 1,
            total_bytes: 0,
        }
    }

    /// Register image and return its ID.
    /// Evicts oldest images (by lowest ID) if count or memory limits are exceeded.
    pub fn insert(&mut self, image: TerminalImage) -> u32 {
        let id = image.id;
        let new_size = image_byte_size(&image);

        // If replacing existing image, subtract old size first
        if let Some(old) = self.images.remove(&id) {
            self.total_bytes = self.total_bytes.saturating_sub(image_byte_size(&old));
        }

        // Evict oldest images while over limits
        while (self.images.len() >= MAX_IMAGE_COUNT
            || self.total_bytes + new_size > MAX_TOTAL_IMAGE_BYTES)
            && !self.images.is_empty()
        {
            // Find the smallest ID (oldest image)
            let oldest_id = match self.images.keys().min().copied() {
                Some(id) => id,
                None => break,
            };
            if let Some(removed) = self.images.remove(&oldest_id) {
                let removed_size = image_byte_size(&removed);
                self.total_bytes = self.total_bytes.saturating_sub(removed_size);
                log::trace!(
                    "ImageRegistry: evicted image {} ({} bytes) to stay within limits",
                    oldest_id,
                    removed_size
                );
            }
        }

        self.total_bytes += new_size;
        self.images.insert(id, image);
        if id >= self.next_id {
            self.next_id = id + 1;
        }
        id
    }

    /// Get image by ID
    pub fn get(&self, id: u32) -> Option<&TerminalImage> {
        self.images.get(&id)
    }

    /// Get mutable image by ID.
    /// After mutating the image (e.g. adding frames), call `update_tracking(id)`
    /// to keep total_bytes accurate.
    pub fn get_mut(&mut self, id: u32) -> Option<&mut TerminalImage> {
        self.images.get_mut(&id)
    }

    /// Recalculate total_bytes and evict oldest images if over limits.
    /// Call after mutating images via get_mut() (e.g. frame add/replace).
    pub fn enforce_limits(&mut self) {
        self.total_bytes = self.images.values().map(image_byte_size).sum();
        while (self.images.len() > MAX_IMAGE_COUNT || self.total_bytes > MAX_TOTAL_IMAGE_BYTES)
            && !self.images.is_empty()
        {
            let oldest_id = match self.images.keys().min().copied() {
                Some(id) => id,
                None => break,
            };
            if let Some(removed) = self.images.remove(&oldest_id) {
                let removed_size = image_byte_size(&removed);
                self.total_bytes = self.total_bytes.saturating_sub(removed_size);
                log::trace!(
                    "ImageRegistry: evicted image {} ({} bytes) after frame update",
                    oldest_id,
                    removed_size
                );
            }
        }
    }

    /// Remove image by ID
    pub fn remove(&mut self, id: u32) -> Option<TerminalImage> {
        let removed = self.images.remove(&id);
        if let Some(ref img) = removed {
            self.total_bytes = self.total_bytes.saturating_sub(image_byte_size(img));
        }
        removed
    }

    /// Remove all images
    pub fn clear(&mut self) {
        self.images.clear();
        self.total_bytes = 0;
    }

    /// Check if image exists by ID
    pub fn contains(&self, id: u32) -> bool {
        self.images.contains_key(&id)
    }
}

impl Default for ImageRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// APC parse state
enum ApcState {
    /// Normal state
    Normal,
    /// ESC detected
    Escape,
    /// ESC _ detected, collecting APC data
    InApc,
    /// Waiting for APC termination (ESC detected)
    ApcEscape,
}

/// Terminal emulator
pub struct Terminal {
    /// Character grid
    pub grid: Grid,
    /// VT parser
    vt_parser: vte::Parser,
    /// PTY
    pty: Pty,
    /// Read buffer
    read_buf: Vec<u8>,
    /// Scroll offset (0=live, >0=viewing history)
    pub scroll_offset: usize,
    /// Text selection range
    pub selection: Option<Selection>,
    /// Internal clipboard
    pub clipboard: String,
    /// Image registry (Sixel, Kitty, etc.)
    pub images: ImageRegistry,
    /// DCS sequence handler (during Sixel parsing)
    pub dcs_handler: Option<DcsHandler>,
    /// Kitty graphics decoder (during APC parsing)
    pub kitty_decoder: Option<KittyDecoder>,
    /// APC parse state
    apc_state: ApcState,
    /// APC data buffer
    apc_buffer: Vec<u8>,
    /// Cell width (pixels, for image placement calculation)
    cell_width: u32,
    /// Cell height (pixels, for image placement calculation)
    cell_height: u32,
    /// Search state (None = search mode OFF)
    pub search: Option<SearchState>,
    /// Copy mode state (None = normal mode)
    pub copy_mode: Option<CopyModeState>,
    /// Clipboard file path
    clipboard_path: String,
    /// Current directory (OSC 7)
    pub current_directory: Option<String>,
    /// PTY response buffer (reused across parser calls)
    pty_response: Vec<u8>,
    /// Image IDs whose GPU textures need re-upload (image data changed for existing ID)
    pub dirty_image_ids: Vec<u32>,
    /// Notification history (oldest first, max MAX_NOTIFICATIONS)
    pub notifications: VecDeque<Notification>,
    /// Monotonically increasing counter for notification toast detection
    pub notification_seq: u64,
    /// Whether notifications are enabled (config: notifications.enabled)
    pub notifications_enabled: bool,
    /// Current progress bar state (OSC 9;4)
    pub active_progress: Option<NotificationProgress>,
    /// Pending OSC 99 notifications (incomplete, keyed by id)
    pub pending_notifications: HashMap<String, Notification>,
    /// Allow Kitty graphics remote file/shm transfers (from config)
    pub allow_kitty_remote: bool,
}

impl Terminal {
    /// Initialize terminal and spawn shell
    pub fn new(cols: usize, rows: usize) -> Result<Self> {
        Self::with_scrollback(cols, rows, 10000, "xterm-256color")
    }

    /// Initialize terminal with custom scrollback size and TERM setting
    pub fn with_scrollback(
        cols: usize,
        rows: usize,
        max_scrollback: usize,
        term_env: &str,
    ) -> Result<Self> {
        Self::with_scrollback_env(cols, rows, max_scrollback, term_env, &[])
    }

    /// Initialize terminal with custom scrollback, TERM setting, and extra environment variables
    pub fn with_scrollback_env(
        cols: usize,
        rows: usize,
        max_scrollback: usize,
        term_env: &str,
        extra_env: &[(&str, &str)],
    ) -> Result<Self> {
        let grid = Grid::with_scrollback(cols, rows, max_scrollback);
        let vt_parser = vte::Parser::new();
        let pty = Pty::spawn_with_env(cols as u16, rows as u16, term_env, extra_env)?;

        Ok(Self {
            grid,
            vt_parser,
            pty,
            read_buf: vec![0u8; READ_BUF_SIZE],
            scroll_offset: 0,
            selection: None,
            clipboard: String::new(),
            images: ImageRegistry::new(),
            dcs_handler: None,
            kitty_decoder: None,
            apc_state: ApcState::Normal,
            apc_buffer: Vec::new(),
            cell_width: 0,
            cell_height: 0,
            search: None,
            copy_mode: None,
            clipboard_path: default_clipboard_path(),
            current_directory: None,
            pty_response: Vec::with_capacity(256),
            dirty_image_ids: Vec::new(),
            notifications: VecDeque::new(),
            notification_seq: 0,
            notifications_enabled: true,
            active_progress: None,
            pending_notifications: HashMap::new(),
            allow_kitty_remote: false,
        })
    }

    /// Set cell size (for image placement calculation)
    pub fn set_cell_size(&mut self, width: u32, height: u32) {
        self.cell_width = width;
        self.cell_height = height;

        // Notify PTY of pixel size (clamp to u16::MAX for safety)
        let cols = (self.grid.cols() as u32).min(u16::MAX as u32) as u16;
        let rows = (self.grid.rows() as u32).min(u16::MAX as u32) as u16;
        let xpixel = (cols as u32).saturating_mul(width).min(u16::MAX as u32) as u16;
        let ypixel = (rows as u32).saturating_mul(height).min(u16::MAX as u32) as u16;
        if let Err(e) = self.pty.resize_with_pixels(cols, rows, xpixel, ypixel) {
            log::warn!("Failed to set PTY pixel size: {}", e);
        }
    }

    /// Set clipboard file path
    pub fn set_clipboard_path(&mut self, path: &str) {
        self.clipboard_path = path.to_string();
    }

    /// Get the home directory of the logged-in user (child process owner)
    pub fn user_home_dir(&self) -> Option<String> {
        self.pty.child_home_dir()
    }

    /// Get cell width
    #[allow(dead_code)]
    pub fn cell_width(&self) -> u32 {
        self.cell_width
    }

    /// Get cell height
    #[allow(dead_code)]
    pub fn cell_height(&self) -> u32 {
        self.cell_height
    }

    // ========== Dirty tracking ==========

    /// Check if any row needs redraw
    #[inline]
    pub fn has_dirty_rows(&self) -> bool {
        self.grid.has_dirty_rows()
    }

    /// Clear all dirty flags (call after rendering)
    #[inline]
    pub fn clear_dirty(&mut self) {
        self.grid.clear_dirty();
    }

    /// Mark all rows as dirty (for full screen redraw)
    #[inline]
    pub fn mark_all_dirty(&mut self) {
        self.grid.mark_all_dirty();
    }

    /// Resize terminal (when font size changes)
    pub fn resize(&mut self, new_cols: usize, new_rows: usize) {
        info!(
            "Terminal resize: {}x{} -> {}x{}",
            self.grid.cols(),
            self.grid.rows(),
            new_cols,
            new_rows
        );

        // Resize grid
        self.grid.resize(new_cols, new_rows);

        // Resize PTY (with pixel size, clamp to u16::MAX)
        let xpixel = (new_cols as u32)
            .saturating_mul(self.cell_width)
            .min(u16::MAX as u32) as u16;
        let ypixel = (new_rows as u32)
            .saturating_mul(self.cell_height)
            .min(u16::MAX as u32) as u16;
        if let Err(e) = self.pty.resize_with_pixels(
            (new_cols as u32).min(u16::MAX as u32) as u16,
            (new_rows as u32).min(u16::MAX as u32) as u16,
            xpixel,
            ypixel,
        ) {
            log::warn!("PTY resize failed: {}", e);
        }

        // Reset scroll offset
        self.scroll_offset = 0;

        // Clear selection
        self.selection = None;

        // Clear image placements (cell size changed)
        self.images.clear();
        self.grid.image_placements.clear();
    }

    /// Process PTY output and update Grid
    ///
    /// Returns: number of bytes read (0 if no data)
    pub fn process_pty_output(&mut self) -> Result<usize> {
        let n = self.pty.read(&mut self.read_buf)?;
        if n == 0 {
            return Ok(0);
        }

        trace!("PTY read: {} bytes", n);

        // Fast path: if already in APC state or buffer contains ESC _, use slow path
        // This handles the rare APC (Kitty graphics) case
        // Use manual loop instead of windows(2).any() to avoid iterator overhead
        // Include ApcState::Escape to handle ESC at buffer boundary (ESC in prev buffer, _ in this one)
        let has_apc = matches!(
            self.apc_state,
            ApcState::Escape | ApcState::InApc | ApcState::ApcEscape
        ) || has_esc_underscore(&self.read_buf[..n]);

        if has_apc {
            // Slow path: byte-by-byte for APC handling
            self.process_pty_output_slow(n);
        } else {
            // Fast path: single Performer for all bytes
            self.process_pty_output_fast(n);
        }

        Ok(n)
    }

    /// Fast path: process all bytes with single Performer (no APC)
    fn process_pty_output_fast(&mut self, n: usize) {
        self.pty_response.clear();

        let mut performer = Performer::new(
            &mut self.grid,
            &mut self.clipboard,
            &mut self.dcs_handler,
            &mut self.images,
            self.cell_width,
            self.cell_height,
            &mut self.current_directory,
            &self.clipboard_path,
            &mut self.pty_response,
            &mut self.notifications,
            &mut self.notification_seq,
            &mut self.active_progress,
            &mut self.pending_notifications,
            &self.notifications_enabled,
        );

        for i in 0..n {
            self.vt_parser.advance(&mut performer, self.read_buf[i]);
        }

        drop(performer);

        if !self.pty_response.is_empty() {
            log::trace!("PTY response: {} bytes", self.pty_response.len());
            self.write_response(&self.pty_response);
        }

        // Track if buffer ends with ESC for cross-buffer APC detection
        // If next buffer starts with '_', we need slow path
        if n > 0 && self.read_buf[n - 1] == 0x1B {
            self.apc_state = ApcState::Escape;
        }
    }

    /// Slow path: byte-by-byte processing with APC state machine
    fn process_pty_output_slow(&mut self, n: usize) {
        self.pty_response.clear();
        for i in 0..n {
            let byte = self.read_buf[i];

            match self.apc_state {
                ApcState::Normal => {
                    if byte == 0x1B {
                        self.apc_state = ApcState::Escape;
                    } else {
                        self.process_byte_with_vte(byte);
                    }
                }
                ApcState::Escape => {
                    if byte == b'_' {
                        self.apc_state = ApcState::InApc;
                        self.apc_buffer.clear();
                    } else {
                        self.apc_state = ApcState::Normal;
                        self.process_byte_with_vte(0x1B);
                        self.process_byte_with_vte(byte);
                    }
                }
                ApcState::InApc => {
                    if byte == 0x1B {
                        self.apc_state = ApcState::ApcEscape;
                    } else if byte == 0x9C {
                        self.process_apc();
                        self.apc_state = ApcState::Normal;
                    } else if self.apc_buffer.len() < MAX_APC_BUFFER_SIZE {
                        self.apc_buffer.push(byte);
                    }
                }
                ApcState::ApcEscape => {
                    if byte == b'\\' {
                        self.process_apc();
                        self.apc_state = ApcState::Normal;
                    } else {
                        self.apc_buffer.push(0x1B);
                        if byte == 0x1B {
                            self.apc_state = ApcState::ApcEscape;
                        } else {
                            self.apc_buffer.push(byte);
                            self.apc_state = ApcState::InApc;
                        }
                    }
                }
            }
        }
    }

    /// Process single byte with vte parser (used in slow path)
    fn process_byte_with_vte(&mut self, byte: u8) {
        self.pty_response.clear();

        let mut performer = Performer::new(
            &mut self.grid,
            &mut self.clipboard,
            &mut self.dcs_handler,
            &mut self.images,
            self.cell_width,
            self.cell_height,
            &mut self.current_directory,
            &self.clipboard_path,
            &mut self.pty_response,
            &mut self.notifications,
            &mut self.notification_seq,
            &mut self.active_progress,
            &mut self.pending_notifications,
            &self.notifications_enabled,
        );
        self.vt_parser.advance(&mut performer, byte);

        if !self.pty_response.is_empty() {
            log::trace!("PTY response: {} bytes", self.pty_response.len());
            self.write_response(&self.pty_response);
        }
    }

    /// Process APC sequence
    fn process_apc(&mut self) {
        // Kitty graphics: ESC _ G ... ST
        if self.apc_buffer.is_empty() || self.apc_buffer[0] != b'G' {
            return;
        }

        let payload = &self.apc_buffer[1..];
        log::info!("Kitty APC received: {} bytes payload", payload.len());

        // Create new decoder if none exists
        if self.kitty_decoder.is_none() {
            self.kitty_decoder = Some(KittyDecoder::new());
        }

        if let Some(ref mut decoder) = self.kitty_decoder {
            let (done, response) = decoder.process(payload);

            // Send response if any
            if let Some(resp) = response {
                self.write_response(&resp);
            }

            if done {
                self.finish_kitty_decode();
            }
        }
    }

    /// Finish Kitty decode processing
    fn finish_kitty_decode(&mut self) {
        use kitty::{make_response, KittyAction};

        if let Some(decoder) = self.kitty_decoder.take() {
            let params = decoder.params();
            let action = params.action;
            let quiet = params.quiet;
            let no_cursor_move = params.do_not_move_cursor;
            let id = if params.id != 0 {
                params.id
            } else {
                self.images.next_id
            };

            log::info!(
                "Kitty finish_decode: action={:?}, id={}, quiet={}, C={}, size={}x{}",
                action,
                id,
                quiet,
                if no_cursor_move { 1 } else { 0 },
                params.width,
                params.height
            );

            match action {
                KittyAction::Delete => {
                    if let Some(target) = params.delete_target {
                        match target {
                            'a' | 'A' => {
                                self.images.clear();
                                self.grid.image_placements.clear();
                            }
                            'i' | 'I' => {
                                self.images.remove(id);
                                self.grid.image_placements.retain(|p| p.id != id);
                            }
                            _ => {}
                        }
                    }
                }
                KittyAction::Display => {
                    if let Some(image) = self.images.get(id) {
                        self.grid.place_image(
                            id,
                            image.width,
                            image.height,
                            self.cell_width,
                            self.cell_height,
                            no_cursor_move,
                        );
                    }
                }
                KittyAction::Query => {
                    // a=q is for protocol support detection - always return OK
                    if quiet < 2 {
                        let resp = make_response(id, true, "");
                        self.write_response(&resp);
                    }
                }
                // Actions that produce decode results
                KittyAction::Transmit
                | KittyAction::TransmitAndDisplay
                | KittyAction::Frame
                | KittyAction::Compose
                | KittyAction::Animation => {
                    match decoder.finish(self.images.next_id, self.allow_kitty_remote) {
                        Ok(result) => {
                            self.handle_kitty_result(result, action, quiet);
                        }
                        Err(e) => {
                            log::warn!("Kitty decode error: {}", e);
                            if quiet < 2 {
                                let resp = make_response(id, false, &e);
                                self.write_response(&resp);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Handle Kitty decode result
    fn handle_kitty_result(
        &mut self,
        result: kitty::KittyDecodeResult,
        action: kitty::KittyAction,
        quiet: u8,
    ) {
        use kitty::{make_response, KittyAction, KittyDecodeResult};

        match result {
            KittyDecodeResult::Image(kitty_img) => {
                let no_cursor_move = kitty_img.do_not_move_cursor;
                info!(
                    "Kitty image decode complete: {}x{} (id={}, C={})",
                    kitty_img.width,
                    kitty_img.height,
                    kitty_img.id,
                    if no_cursor_move { 1 } else { 0 }
                );
                let term_img = TerminalImage {
                    id: kitty_img.id,
                    width: kitty_img.width,
                    height: kitty_img.height,
                    data: kitty_img.data,
                    frames: Vec::new(),
                    animation_state: AnimationState::Stopped,
                    current_frame: 0,
                    loop_count: 0,
                    current_loop: 0,
                };
                let img_id = term_img.id;
                let width = term_img.width;
                let height = term_img.height;

                // If image with same ID already exists, mark texture for re-upload
                if self.images.get(img_id).is_some() {
                    self.dirty_image_ids.push(img_id);
                    // Remove old placements for this ID (will be replaced)
                    self.grid.image_placements.retain(|p| p.id != img_id);
                }
                self.images.insert(term_img);

                if action == KittyAction::TransmitAndDisplay {
                    self.grid.place_image(
                        img_id,
                        width,
                        height,
                        self.cell_width,
                        self.cell_height,
                        no_cursor_move,
                    );
                }

                if quiet < 2 {
                    let resp = make_response(img_id, true, "");
                    log::info!(
                        "Kitty graphics: sending OK response for id={} ({} bytes)",
                        img_id,
                        resp.len()
                    );
                    self.write_response(&resp);
                }
            }
            KittyDecodeResult::Frame(frame_data) => {
                info!(
                    "Kitty frame decode: image_id={}, frame={}, size={}x{}",
                    frame_data.image_id,
                    frame_data.frame_number,
                    frame_data.width,
                    frame_data.height
                );

                // Find the image and add the frame
                if let Some(image) = self.images.get_mut(frame_data.image_id) {
                    let frame = ImageFrame {
                        number: frame_data.frame_number,
                        width: frame_data.width,
                        height: frame_data.height,
                        x: frame_data.x,
                        y: frame_data.y,
                        gap: frame_data.gap,
                        data: frame_data.data,
                    };

                    // Frame numbering: 1 = root frame (stored in image.data)
                    // 2+ = extra frames (stored in image.frames)
                    if frame_data.frame_number == 1 {
                        // Replace root frame
                        image.data = frame.data;
                    } else {
                        // Add or replace extra frame
                        let frame_idx = (frame_data.frame_number - 2) as usize;
                        if frame_idx < image.frames.len() {
                            image.frames[frame_idx] = frame;
                        } else {
                            // Extend frames array
                            let gap_frame_size = (image.width as usize)
                                .saturating_mul(image.height as usize)
                                .saturating_mul(4);
                            if gap_frame_size > 256 * 1024 * 1024 {
                                log::warn!(
                                    "Kitty: gap frame too large ({}), skipping",
                                    gap_frame_size
                                );
                            } else {
                                while image.frames.len() < frame_idx {
                                    // Fill gaps with empty frames
                                    image.frames.push(ImageFrame {
                                        number: image.frames.len() as u32 + 2,
                                        width: image.width,
                                        height: image.height,
                                        x: 0,
                                        y: 0,
                                        gap: 40,
                                        data: vec![0u8; gap_frame_size],
                                    });
                                }
                                image.frames.push(frame);
                            }
                        }
                    }

                    // Recalculate tracked memory after frame mutation
                    self.images.enforce_limits();

                    if quiet < 2 {
                        let resp = make_response(frame_data.image_id, true, "");
                        self.write_response(&resp);
                    }
                } else {
                    log::warn!("Kitty frame: image {} not found", frame_data.image_id);
                    if quiet < 2 {
                        let resp =
                            make_response(frame_data.image_id, false, "ENOENT:image not found");
                        self.write_response(&resp);
                    }
                }
            }
            KittyDecodeResult::Compose(cmd) => {
                info!(
                    "Kitty compose: image_id={}, src_frame={} -> dst_frame={}",
                    cmd.image_id, cmd.src_frame, cmd.dst_frame
                );

                if let Some(image) = self.images.get_mut(cmd.image_id) {
                    // Get source and destination frame data
                    let result = compose_frames(
                        image,
                        cmd.src_frame,
                        cmd.dst_frame,
                        cmd.src_x,
                        cmd.src_y,
                        cmd.dst_x,
                        cmd.dst_y,
                        cmd.width,
                        cmd.height,
                        cmd.compose_mode,
                    );

                    if quiet < 2 {
                        let resp = if result.is_ok() {
                            make_response(cmd.image_id, true, "")
                        } else {
                            make_response(cmd.image_id, false, &result.unwrap_err())
                        };
                        self.write_response(&resp);
                    }
                } else {
                    if quiet < 2 {
                        let resp = make_response(cmd.image_id, false, "ENOENT:image not found");
                        self.write_response(&resp);
                    }
                }
            }
            KittyDecodeResult::Animation(cmd) => {
                info!(
                    "Kitty animation: image_id={}, state={}, frame={}, current={}",
                    cmd.image_id, cmd.state, cmd.frame_number, cmd.current_frame
                );

                if let Some(image) = self.images.get_mut(cmd.image_id) {
                    // Update animation state
                    if cmd.state > 0 {
                        image.animation_state = match cmd.state {
                            1 => AnimationState::Stopped,
                            2 => AnimationState::Loading,
                            3 => AnimationState::Running,
                            _ => image.animation_state,
                        };
                    }

                    // Update current frame
                    if cmd.current_frame > 0 {
                        image.current_frame = cmd.current_frame - 1; // Convert to 0-based
                    }

                    // Update loop count
                    if cmd.loop_count > 0 {
                        image.loop_count = cmd.loop_count;
                    }

                    // Update gap for specific frame
                    if cmd.frame_number > 0 && cmd.gap >= 0 {
                        if cmd.frame_number == 1 {
                            // Root frame gap would need separate storage
                            // For now, skip
                        } else {
                            let frame_idx = (cmd.frame_number - 2) as usize;
                            if frame_idx < image.frames.len() {
                                image.frames[frame_idx].gap = cmd.gap as u32;
                            }
                        }
                    }

                    if quiet < 2 {
                        let resp = make_response(cmd.image_id, true, "");
                        self.write_response(&resp);
                    }
                } else {
                    if quiet < 2 {
                        let resp = make_response(cmd.image_id, false, "ENOENT:image not found");
                        self.write_response(&resp);
                    }
                }
            }
        }
    }
}

/// Compose (copy) rectangle from one frame to another
fn compose_frames(
    image: &mut TerminalImage,
    src_frame: u32,
    dst_frame: u32,
    src_x: u32,
    src_y: u32,
    dst_x: u32,
    dst_y: u32,
    width: u32,
    height: u32,
    compose_mode: u8,
) -> Result<(), String> {
    // Get source data
    let src_data = if src_frame == 1 {
        &image.data
    } else {
        let idx = (src_frame - 2) as usize;
        if idx >= image.frames.len() {
            return Err(format!("ENOENT:source frame {} not found", src_frame));
        }
        &image.frames[idx].data
    };

    // Bounds check: prevent u32 underflow when src/dst exceed image dimensions
    if src_x >= image.width
        || src_y >= image.height
        || dst_x >= image.width
        || dst_y >= image.height
    {
        return Ok(());
    }

    // Copy source rectangle
    let img_width = image.width as usize;
    let copy_width = width.min(image.width - src_x).min(image.width - dst_x) as usize;
    let copy_height = height.min(image.height - src_y).min(image.height - dst_y) as usize;

    let alloc_size = copy_width.saturating_mul(copy_height).saturating_mul(4);
    if alloc_size > 256 * 1024 * 1024 {
        return Err(format!(
            "compose_frames: allocation too large ({})",
            alloc_size
        ));
    }
    let mut copied_data = vec![0u8; alloc_size];
    for row in 0..copy_height {
        let src_row_start = ((src_y as usize + row) * img_width + src_x as usize) * 4;
        let dst_row_start = row * copy_width * 4;
        copied_data[dst_row_start..dst_row_start + copy_width * 4]
            .copy_from_slice(&src_data[src_row_start..src_row_start + copy_width * 4]);
    }

    // Get destination data and apply
    let dst_data = if dst_frame == 1 {
        &mut image.data
    } else {
        let idx = (dst_frame - 2) as usize;
        if idx >= image.frames.len() {
            return Err(format!("ENOENT:destination frame {} not found", dst_frame));
        }
        &mut image.frames[idx].data
    };

    // Compose: 0 = alpha blend, 1 = overwrite
    for row in 0..copy_height {
        let src_row_start = row * copy_width * 4;
        let dst_row_start = ((dst_y as usize + row) * img_width + dst_x as usize) * 4;

        for col in 0..copy_width {
            let src_idx = src_row_start + col * 4;
            let dst_idx = dst_row_start + col * 4;

            if compose_mode == 1 {
                // Overwrite
                dst_data[dst_idx..dst_idx + 4].copy_from_slice(&copied_data[src_idx..src_idx + 4]);
            } else {
                // Alpha blend
                let src_a = copied_data[src_idx + 3] as u32;
                if src_a == 255 {
                    dst_data[dst_idx..dst_idx + 4]
                        .copy_from_slice(&copied_data[src_idx..src_idx + 4]);
                } else if src_a > 0 {
                    let dst_a = dst_data[dst_idx + 3] as u32;
                    let out_a = src_a + dst_a * (255 - src_a) / 255;
                    if out_a > 0 {
                        for c in 0..3 {
                            let src_c = copied_data[src_idx + c] as u32;
                            let dst_c = dst_data[dst_idx + c] as u32;
                            dst_data[dst_idx + c] = ((src_c * src_a
                                + dst_c * dst_a * (255 - src_a) / 255)
                                / out_a) as u8;
                        }
                        dst_data[dst_idx + 3] = out_a as u8;
                    }
                }
            }
        }
    }

    Ok(())
}

impl Terminal {
    /// Write terminal response to PTY (DSR, device attributes, etc.)
    /// Uses write_all to ensure complete delivery
    fn write_response(&self, data: &[u8]) {
        if let Err(e) = self.pty.write_all(data) {
            log::warn!("PTY response write failed: {}", e);
        }
    }

    /// Write data to PTY (for keyboard input forwarding)
    pub fn write_to_pty(&self, data: &[u8]) -> Result<usize> {
        self.pty.write(data)
    }

    /// Check if child process is alive
    pub fn is_alive(&self) -> bool {
        self.pty.is_alive()
    }

    /// Get foreground process name
    pub fn foreground_process_name(&self) -> Option<String> {
        self.pty.foreground_process_name()
    }

    /// Reset enhanced input modes (called by user via Ctrl+Shift+Escape)
    pub fn reset_enhanced_modes(&mut self) {
        self.grid.reset_enhanced_modes();
    }

    // ========== Scrollback control ==========

    /// Scroll towards history (upward)
    pub fn scroll_back(&mut self, n: usize) {
        let max = self.grid.scrollback_len();
        let new_offset = (self.scroll_offset + n).min(max);
        if new_offset != self.scroll_offset {
            self.scroll_offset = new_offset;
            self.grid.mark_all_dirty(); // Redraw all rows when scrolling
        }
    }

    /// Scroll towards live (downward)
    pub fn scroll_forward(&mut self, n: usize) {
        let new_offset = self.scroll_offset.saturating_sub(n);
        if new_offset != self.scroll_offset {
            self.scroll_offset = new_offset;
            self.grid.mark_all_dirty(); // Redraw all rows when scrolling
        }
    }

    /// Return to live position
    pub fn scroll_to_bottom(&mut self) {
        if self.scroll_offset != 0 {
            self.scroll_offset = 0;
            self.grid.mark_all_dirty(); // Redraw all rows when returning to live
        }
    }

    // ========== Text selection & clipboard ==========

    /// Get text in selection range
    pub fn get_selection_text(&self) -> String {
        let sel = match &self.selection {
            Some(s) => s,
            None => return String::new(),
        };
        let (sr, sc, er, ec) = sel.normalized();
        let mut result = String::new();
        let cols = self.grid.cols();

        for row in sr..=er {
            let col_start = if row == sr { sc } else { 0 };
            let col_end = if row == er { ec.min(cols) } else { cols };
            for col in col_start..col_end {
                let cell = self.display_cell(row, col);
                if cell.width == 0 {
                    continue; // Wide character continuation cell
                }
                if !cell.grapheme.is_empty() {
                    result.push_str(&cell.grapheme);
                }
            }
            // Trim trailing whitespace, add newline
            if row < er {
                let trimmed = result.trim_end().len();
                result.truncate(trimmed);
                result.push('\n');
            }
        }
        let trimmed = result.trim_end().len();
        result.truncate(trimmed);
        result
    }

    /// Set clipboard (internal buffer + write to file)
    pub fn set_clipboard(&mut self, text: &str) {
        self.clipboard = text.to_string();
        if let Err(e) = std::fs::write(&self.clipboard_path, text) {
            log::warn!("Failed to write clipboard file: {}", e);
        }
    }

    /// Copy selection to clipboard
    pub fn copy_selection(&mut self) {
        let text = self.get_selection_text();
        if !text.is_empty() {
            self.set_clipboard(&text);
            info!("Clipboard: {} characters copied", text.len());
        }
    }

    /// Double click: word selection
    pub fn select_word(&mut self, row: usize, col: usize) {
        let cols = self.grid.cols();

        // Get cell at click position
        let mut click_col = col;
        let cell = self.grid.cell(row, col);

        // If clicked on continuation cell (width=0), find the original cell
        if cell.width == 0 {
            while click_col > 0 {
                click_col -= 1;
                let prev = self.grid.cell(row, click_col);
                if prev.width != 0 {
                    break;
                }
            }
        }

        let base_cell = self.grid.cell(row, click_col);

        // Don't select if whitespace/empty cell
        if base_cell.grapheme.is_empty() || base_cell.grapheme == " " {
            self.selection = None;
            return;
        }

        // Word boundary detection: treat everything except whitespace as word (simple version)
        fn is_word_boundary(grapheme: &str) -> bool {
            grapheme.is_empty() || grapheme == " " || grapheme == "\t"
        }

        let mut start_col = click_col;
        let mut end_col = click_col;

        // Search left
        while start_col > 0 {
            let prev = self.grid.cell(row, start_col - 1);
            if prev.width == 0 {
                start_col -= 1;
                continue;
            }
            if is_word_boundary(&prev.grapheme) {
                break;
            }
            start_col -= 1;
        }

        // Search right
        while end_col < cols - 1 {
            let next = self.grid.cell(row, end_col + 1);
            if next.width == 0 {
                end_col += 1;
                continue;
            }
            if is_word_boundary(&next.grapheme) {
                break;
            }
            end_col += 1;
        }

        // Both anchor_col and end_col are inclusive cell positions
        self.selection = Some(Selection {
            anchor_row: row,
            anchor_col: start_col,
            end_row: row,
            end_col: end_col,
        });
        self.copy_selection();
    }

    /// Triple click: line selection
    pub fn select_line(&mut self, row: usize) {
        let cols = self.grid.cols();
        self.selection = Some(Selection {
            anchor_row: row,
            anchor_col: 0,
            end_row: row,
            end_col: cols - 1,
        });
        self.copy_selection();
    }

    /// Send clipboard contents to PTY (paste)
    /// If bracketed_paste is enabled, wrap with \e[200~ and \e[201~
    pub fn paste_clipboard(&self) -> Result<()> {
        if !self.clipboard.is_empty() {
            if self.grid.modes.bracketed_paste {
                // Bracketed paste mode: use write_all for reliable delivery
                self.pty.write_all(b"\x1b[200~")?;
                self.pty.write_all(self.clipboard.as_bytes())?;
                self.pty.write_all(b"\x1b[201~")?;
            } else {
                self.pty.write_all(self.clipboard.as_bytes())?;
            }
        }
        Ok(())
    }

    /// Check if mouse mode is enabled
    pub fn mouse_mode_enabled(&self) -> bool {
        self.grid.modes.mouse_mode != grid::MouseMode::None
    }

    /// Encode and send mouse event to PTY
    /// cb: button code
    /// col, row: 0-indexed coordinates
    /// press: true for press (M), false for release (m) in SGR mode
    fn send_mouse_event(&self, cb: u8, col: usize, row: usize, press: bool) -> Result<()> {
        let cx = (col + 1).min(223);
        let cy = (row + 1).min(223);

        if self.grid.modes.mouse_sgr {
            let suffix = if press { 'M' } else { 'm' };
            let seq = format!("\x1b[<{};{};{}{}", cb, cx, cy, suffix);
            trace!(
                "Mouse event → PTY (SGR): cb={} col={} row={} {}",
                cb,
                cx,
                cy,
                suffix
            );
            self.pty.write(seq.as_bytes())?;
        } else {
            let bytes = [0x1b, b'[', b'M', cb + 32, (cx as u8) + 32, (cy as u8) + 32];
            trace!("Mouse event → PTY (X10): cb={} col={} row={}", cb, cx, cy);
            self.pty.write(&bytes)?;
        }
        Ok(())
    }

    /// Send mouse button press event to PTY
    /// button: 0=left, 1=middle, 2=right
    /// col, row: 0-indexed
    pub fn send_mouse_press(&self, button: u8, col: usize, row: usize) -> Result<()> {
        if self.grid.modes.mouse_mode == grid::MouseMode::None {
            return Ok(());
        }
        self.send_mouse_event(button, col, row, true)
    }

    /// Send mouse button release event to PTY
    pub fn send_mouse_release(&self, button: u8, col: usize, row: usize) -> Result<()> {
        if self.grid.modes.mouse_mode == grid::MouseMode::None {
            return Ok(());
        }

        if self.grid.modes.mouse_sgr {
            // SGR format uses original button with lowercase 'm'
            self.send_mouse_event(button, col, row, false)
        } else {
            // Normal format: button 3 means release
            self.send_mouse_event(3, col, row, true)
        }
    }

    /// Send mouse move event to PTY (in ButtonEvent/AnyEvent mode)
    pub fn send_mouse_move(&self, col: usize, row: usize, button_held: Option<u8>) -> Result<()> {
        // AnyEvent (1003) or ButtonEvent (1002) + button held only
        match self.grid.modes.mouse_mode {
            grid::MouseMode::AnyEvent => {}
            grid::MouseMode::ButtonEvent if button_held.is_some() => {}
            _ => return Ok(()),
        }

        // button code: 32+button if held, 35 if not
        let cb = match button_held {
            Some(b) => 32 + b, // motion + button
            None => 35,        // motion only
        };

        self.send_mouse_event(cb, col, row, true)
    }

    /// Send mouse wheel event to PTY
    /// delta: positive=down, negative=up
    pub fn send_mouse_wheel(&self, delta: i8, col: usize, row: usize) -> Result<()> {
        if self.grid.modes.mouse_mode == grid::MouseMode::None {
            return Ok(());
        }

        // button code: 64=up, 65=down
        let cb = if delta < 0 { 64 } else { 65 };
        self.send_mouse_event(cb, col, row, true)
    }

    /// Get cell for display row (considering scroll_offset)
    ///
    /// display_row: row on screen (0 = top of screen)
    /// scroll_offset = 0: return grid row as-is
    /// scroll_offset > 0: compose from scrollback + grid
    pub fn display_cell(&self, display_row: usize, col: usize) -> &Cell {
        if self.scroll_offset == 0 {
            return self.grid.cell(display_row, col);
        }

        let total_scrollback = self.grid.scrollback_len();
        let grid_rows = self.grid.rows();

        // Number of rows to fetch from scrollback out of total displayed rows
        let scrollback_rows_shown = self.scroll_offset.min(grid_rows);

        if display_row < scrollback_rows_shown {
            // Fetch from scrollback
            let sb_idx = total_scrollback - self.scroll_offset + display_row;
            if let Some(row) = self.grid.scrollback_row(sb_idx) {
                if col < row.len() {
                    return &row[col];
                }
            }
            return Cell::empty_ref();
        }

        // Fetch from grid row
        let grid_row = display_row - scrollback_rows_shown;
        if grid_row < grid_rows {
            self.grid.cell(grid_row, col)
        } else {
            Cell::empty_ref()
        }
    }

    /// Detect URL at specified position and return it
    /// Prioritize OSC 8 hyperlinks, otherwise detect from text pattern
    pub fn detect_url_at(&self, row: usize, col: usize) -> Option<String> {
        // First check OSC 8 hyperlink
        let cell = self.display_cell(row, col);
        if let Some(ref link) = cell.hyperlink {
            return Some(link.url.clone());
        }

        // Get entire line text
        let cols = self.grid.cols();
        let mut line_text = String::new();
        let mut col_to_byte: Vec<usize> = Vec::with_capacity(cols);

        for c in 0..cols {
            let cell = self.display_cell(row, c);
            if cell.width == 0 {
                col_to_byte.push(line_text.len());
                continue;
            }
            col_to_byte.push(line_text.len());
            if !cell.grapheme.is_empty() {
                line_text.push_str(&cell.grapheme);
            } else {
                line_text.push(' ');
            }
        }

        // Search for URL pattern (simple regex-like match)
        let url_starts = ["http://", "https://", "file://"];
        let url_chars =
            |c: char| -> bool { c.is_alphanumeric() || "-._~:/?#[]@!$&'()*+,;=%".contains(c) };

        let click_byte = col_to_byte.get(col).copied().unwrap_or(line_text.len());

        for prefix in &url_starts {
            let mut search_start = 0;
            while let Some(pos) = line_text[search_start..].find(prefix) {
                let start = search_start + pos;
                // Find URL end
                let end = line_text[start..]
                    .chars()
                    .take_while(|&c| url_chars(c))
                    .map(|c| c.len_utf8())
                    .sum::<usize>()
                    + start;

                // Check if click position is within this URL range
                if click_byte >= start && click_byte < end {
                    return Some(line_text[start..end].to_string());
                }

                search_start = start + prefix.len();
            }
        }

        None
    }

    /// Copy URL to clipboard
    ///
    /// On DRM console we cannot directly open a browser,
    /// so copy URL to clipboard and notify.
    pub fn copy_url_to_clipboard(&mut self, url: &str) {
        info!("URL copied to clipboard: {}", url);
        self.set_clipboard(url);
    }

    // ========== Search functionality ==========

    /// Start search mode
    pub fn start_search(&mut self) {
        self.search = Some(SearchState::new());
    }

    /// End search mode
    pub fn end_search(&mut self) {
        self.search = None;
    }

    /// Execute search (search entire scrollback + grid)
    pub fn execute_search(&mut self) {
        let query = match &self.search {
            Some(s) if !s.query.is_empty() => s.query.clone(),
            _ => return,
        };

        let mut matches = Vec::new();
        let cols = self.grid.cols();
        let grid_rows = self.grid.rows();
        let scrollback_len = self.grid.scrollback_len();

        // Build byte→char index once per line for O(1) byte-to-column lookup.
        // This avoids repeated O(n) chars().count() calls per match.
        let mut byte_to_char: Vec<usize> = Vec::new();

        /// Build byte-to-char-index mapping for a string.
        /// byte_to_char[byte_offset] = char_index at that byte offset.
        fn build_byte_to_char(s: &str, buf: &mut Vec<usize>) {
            buf.clear();
            buf.reserve(s.len() + 1);
            let mut char_idx = 0;
            for (byte_idx, _) in s.char_indices() {
                while buf.len() <= byte_idx {
                    buf.push(char_idx);
                }
                char_idx += 1;
            }
            // Fill remaining bytes (for slicing up to s.len())
            while buf.len() <= s.len() {
                buf.push(char_idx);
            }
        }

        // Search scrollback
        for sb_row in 0..scrollback_len {
            if let Some(row_cells) = self.grid.scrollback_row(sb_row) {
                let line_text: String = row_cells
                    .iter()
                    .filter(|c| c.width != 0)
                    .map(|c| {
                        if c.grapheme.is_empty() {
                            " "
                        } else {
                            c.grapheme.as_str()
                        }
                    })
                    .collect();

                build_byte_to_char(&line_text, &mut byte_to_char);

                let mut search_pos = 0;
                while let Some(pos) = line_text[search_pos..].find(&query) {
                    let start_byte = search_pos + pos;
                    let end_byte = start_byte + query.len();

                    let start_col = byte_to_char[start_byte];
                    let end_col = byte_to_char[end_byte];

                    matches.push((sb_row, start_col, end_col));
                    search_pos = start_byte + 1;
                }
            }
        }

        // Search grid
        for row in 0..grid_rows {
            let mut line_text = String::new();
            for col in 0..cols {
                let cell = self.grid.cell(row, col);
                if cell.width == 0 {
                    continue;
                }
                if !cell.grapheme.is_empty() {
                    line_text.push_str(&cell.grapheme);
                } else {
                    line_text.push(' ');
                }
            }

            build_byte_to_char(&line_text, &mut byte_to_char);

            let mut search_pos = 0;
            while let Some(pos) = line_text[search_pos..].find(&query) {
                let start_byte = search_pos + pos;
                let end_byte = start_byte + query.len();

                let start_col = byte_to_char[start_byte];
                let end_col = byte_to_char[end_byte];

                // Grid row is offset from scrollback_len
                matches.push((scrollback_len + row, start_col, end_col));
                search_pos = start_byte + 1;
            }
        }

        if let Some(ref mut s) = self.search {
            s.matches = matches;
            s.current_match = 0;
            s.build_row_index();
        }

        info!(
            "Search '{}': {} matches",
            query,
            self.search.as_ref().map(|s| s.matches.len()).unwrap_or(0)
        );
    }

    /// Scroll to current match position
    pub fn scroll_to_current_match(&mut self) {
        let (match_row, scrollback_len, grid_rows) = match &self.search {
            Some(s) if !s.matches.is_empty() => {
                let (row, _, _) = s.matches[s.current_match];
                (row, self.grid.scrollback_len(), self.grid.rows())
            }
            _ => return,
        };

        // Calculate scroll_offset to display match_row
        if match_row < scrollback_len {
            // In scrollback
            self.scroll_offset = scrollback_len - match_row;
            self.grid.mark_all_dirty();
        } else {
            // In grid
            let grid_row = match_row - scrollback_len;
            if grid_row < grid_rows {
                if self.scroll_offset != 0 {
                    self.scroll_offset = 0; // Visible in live view
                    self.grid.mark_all_dirty();
                }
            }
        }
    }

    /// Check if row has search matches (for highlighting)
    /// Returns: list of match ranges (start_col, end_col, is_current)
    pub fn get_search_matches_for_display_row(
        &self,
        display_row: usize,
    ) -> &[(usize, usize, usize)] {
        static EMPTY: [(usize, usize, usize); 0] = [];

        let search = match &self.search {
            Some(s) => s,
            None => return &EMPTY,
        };

        let scrollback_len = self.grid.scrollback_len();
        let grid_rows = self.grid.rows();

        // Convert display_row to absolute row
        let abs_row = if self.scroll_offset == 0 {
            scrollback_len + display_row
        } else {
            let scrollback_rows_shown = self.scroll_offset.min(grid_rows);
            if display_row < scrollback_rows_shown {
                scrollback_len - self.scroll_offset + display_row
            } else {
                scrollback_len + display_row - scrollback_rows_shown
            }
        };

        // O(1) lookup instead of O(n) linear scan
        search
            .row_matches
            .get(&abs_row)
            .map(|v| v.as_slice())
            .unwrap_or(&EMPTY)
    }

    /// Get current match index (for is_current check)
    pub fn current_search_match(&self) -> Option<usize> {
        self.search.as_ref().map(|s| s.current_match)
    }

    // ========== Copy mode ==========

    /// Enter copy mode
    pub fn enter_copy_mode(&mut self) {
        // Start copy mode from current cursor position
        let cursor_row = self.grid.cursor_row;
        let cursor_col = self.grid.cursor_col;
        self.copy_mode = Some(CopyModeState::new(cursor_row, cursor_col));
        info!("Copy mode started: row={}, col={}", cursor_row, cursor_col);
    }

    /// Exit copy mode
    pub fn exit_copy_mode(&mut self) {
        self.copy_mode = None;
        self.selection = None;
        info!("Copy mode ended");
    }

    /// Copy mode: move cursor
    pub fn copy_mode_move(&mut self, delta_row: isize, delta_col: isize) {
        if let Some(ref mut cm) = self.copy_mode {
            let rows = self.grid.rows();
            let cols = self.grid.cols();
            let max_scroll = self.grid.scrollback_len();

            // Row movement
            let new_row = (cm.cursor_row as isize + delta_row).max(0) as usize;

            // Scroll back if going past top
            if delta_row < 0 && cm.cursor_row == 0 && self.scroll_offset < max_scroll {
                self.scroll_offset += 1;
                self.grid.mark_all_dirty();
            }
            // Scroll forward if going past bottom
            else if delta_row > 0 && cm.cursor_row >= rows - 1 && self.scroll_offset > 0 {
                self.scroll_offset -= 1;
                self.grid.mark_all_dirty();
            } else {
                cm.cursor_row = new_row.min(rows - 1);
            }

            // Column movement
            let new_col = (cm.cursor_col as isize + delta_col).max(0) as usize;
            cm.cursor_col = new_col.min(cols - 1);

            // Update selection if selecting
            if cm.selecting {
                self.selection = cm.get_selection();
            }
        }
    }

    /// Copy mode: go to beginning
    pub fn copy_mode_goto_top(&mut self) {
        if let Some(ref mut cm) = self.copy_mode {
            let new_offset = self.grid.scrollback_len();
            if self.scroll_offset != new_offset {
                self.scroll_offset = new_offset;
                self.grid.mark_all_dirty();
            }
            cm.cursor_row = 0;
            cm.cursor_col = 0;
            if cm.selecting {
                self.selection = cm.get_selection();
            }
        }
    }

    /// Copy mode: go to end
    pub fn copy_mode_goto_bottom(&mut self) {
        if let Some(ref mut cm) = self.copy_mode {
            if self.scroll_offset != 0 {
                self.scroll_offset = 0;
                self.grid.mark_all_dirty();
            }
            cm.cursor_row = self.grid.rows() - 1;
            cm.cursor_col = 0;
            if cm.selecting {
                self.selection = cm.get_selection();
            }
        }
    }

    /// Copy mode: move half page up
    pub fn copy_mode_page_up(&mut self) {
        let half_page = self.grid.rows() / 2;
        for _ in 0..half_page {
            self.copy_mode_move(-1, 0);
        }
    }

    /// Copy mode: move half page down
    pub fn copy_mode_page_down(&mut self) {
        let half_page = self.grid.rows() / 2;
        for _ in 0..half_page {
            self.copy_mode_move(1, 0);
        }
    }

    /// Copy mode: toggle selection
    pub fn copy_mode_toggle_selection(&mut self) {
        if let Some(ref mut cm) = self.copy_mode {
            cm.toggle_selection();
            if cm.selecting {
                self.selection = cm.get_selection();
            } else {
                self.selection = None;
            }
        }
    }

    /// Copy mode: go to line start
    pub fn copy_mode_goto_line_start(&mut self) {
        if let Some(ref mut cm) = self.copy_mode {
            cm.cursor_col = 0;
            if cm.selecting {
                self.selection = cm.get_selection();
            }
        }
    }

    /// Copy mode: go to line end
    pub fn copy_mode_goto_line_end(&mut self) {
        if let Some(ref mut cm) = self.copy_mode {
            cm.cursor_col = self.grid.cols() - 1;
            if cm.selecting {
                self.selection = cm.get_selection();
            }
        }
    }

    /// Copy mode: move forward by word
    pub fn copy_mode_word_forward(&mut self) {
        // Get necessary values first to avoid borrow issues
        let (row, start_col, cols) = match self.copy_mode {
            Some(ref cm) => (cm.cursor_row, cm.cursor_col, self.grid.cols()),
            None => return,
        };

        let mut col = start_col;

        // If currently in a word, skip to end of word
        while col < cols - 1 {
            let cell = self.display_cell(row, col);
            if cell
                .grapheme
                .chars()
                .next()
                .map(|c| c.is_whitespace())
                .unwrap_or(true)
            {
                break;
            }
            col += 1;
        }
        // Skip whitespace
        while col < cols - 1 {
            let cell = self.display_cell(row, col);
            if !cell
                .grapheme
                .chars()
                .next()
                .map(|c| c.is_whitespace())
                .unwrap_or(true)
            {
                break;
            }
            col += 1;
        }

        // Apply result
        if let Some(ref mut cm) = self.copy_mode {
            cm.cursor_col = col;
            if cm.selecting {
                self.selection = cm.get_selection();
            }
        }
    }

    /// Copy mode: move backward by word
    pub fn copy_mode_word_backward(&mut self) {
        // Get necessary values first to avoid borrow issues
        let (row, start_col) = match self.copy_mode {
            Some(ref cm) => (cm.cursor_row, cm.cursor_col),
            None => return,
        };

        let mut col = start_col;

        // Skip whitespace
        while col > 0 {
            let cell = self.display_cell(row, col - 1);
            if !cell
                .grapheme
                .chars()
                .next()
                .map(|c| c.is_whitespace())
                .unwrap_or(true)
            {
                break;
            }
            col -= 1;
        }
        // Go to start of word
        while col > 0 {
            let cell = self.display_cell(row, col - 1);
            if cell
                .grapheme
                .chars()
                .next()
                .map(|c| c.is_whitespace())
                .unwrap_or(true)
            {
                break;
            }
            col -= 1;
        }

        // Apply result
        if let Some(ref mut cm) = self.copy_mode {
            cm.cursor_col = col;
            if cm.selecting {
                self.selection = cm.get_selection();
            }
        }
    }

    /// Copy mode: yank selection and exit
    pub fn copy_mode_yank(&mut self) {
        self.copy_selection();
        self.exit_copy_mode();
    }

    // ========== Focus events ==========

    /// Send focus event (CSI I / CSI O)
    #[allow(dead_code)]
    pub fn send_focus_event(&self, focused: bool) -> Result<()> {
        if self.grid.modes.send_focus_events {
            let seq = if focused { b"\x1b[I" } else { b"\x1b[O" };
            self.pty.write(seq)?;
        }
        Ok(())
    }

    // ========== Synchronized Update ==========

    /// Check if Synchronized Update mode is enabled
    #[allow(dead_code)]
    pub fn is_synchronized_update(&self) -> bool {
        self.grid.modes.synchronized_update
    }

    /// Process arbitrary output data (e.g., /etc/issue before login)
    ///
    /// This is similar to process_pty_output but takes external data.
    pub fn process_output(&mut self, data: &[u8]) {
        // Use fast path (single Performer) for external data
        // APC sequences are not expected in /etc/issue
        self.pty_response.clear();

        let mut performer = Performer::new(
            &mut self.grid,
            &mut self.clipboard,
            &mut self.dcs_handler,
            &mut self.images,
            self.cell_width,
            self.cell_height,
            &mut self.current_directory,
            &self.clipboard_path,
            &mut self.pty_response,
            &mut self.notifications,
            &mut self.notification_seq,
            &mut self.active_progress,
            &mut self.pending_notifications,
            &self.notifications_enabled,
        );

        for &byte in data {
            self.vt_parser.advance(&mut performer, byte);
        }
    }
}
