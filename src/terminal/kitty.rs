//! Kitty graphics protocol implementation
//!
//! Supports the Kitty terminal's graphics protocol for displaying images
//! inline in the terminal. Used by tools like ranger, viu, termpdf, etc.
//!
//! ## Protocol Overview
//!
//! Images are sent via APC (Application Program Command) sequences:
//! ```text
//! ESC _G <control-data>;<payload> ESC \
//! ```
//!
//! ### Transmission Actions (a=t/T)
//! - `a=t`: Transmit image data (direct/base64)
//! - `a=T`: Transmit with display
//! - `t=d`: Direct data in payload
//! - `t=f`: Read from file path
//! - `t=t`: Read from temp file
//! - `t=s`: Read from shared memory
//!
//! ### Display Actions (a=p)
//! - Place image at cursor position
//! - `i=<id>`: Image ID to display
//! - `c=<cols>`, `r=<rows>`: Size in cells
//!
//! ### Animation (a=f)
//! - Frame-based animation support
//! - `r=<frame>`: Frame number
//! - `z=<gap>`: Inter-frame delay in ms
//!
//! ## Data Formats (f=)
//! - 24: RGB (3 bytes/pixel)
//! - 32: RGBA (4 bytes/pixel, default)
//! - 100: PNG (decoded automatically)
//!
//! ## Reference
//! - <https://sw.kovidgoyal.net/kitty/graphics-protocol/>

use log::{info, trace, warn};

/// Maximum accumulated image data size (256MB)
/// Allows 8K RGBA images (7680x4320x4 = 132MB raw)
const MAX_IMAGE_DATA_SIZE: usize = 256 * 1024 * 1024;

/// Animation frame
#[derive(Debug, Clone)]
pub struct KittyFrame {
    /// Frame ID (1 = root frame)
    pub id: u32,
    /// Frame width
    pub width: u32,
    /// Frame height
    pub height: u32,
    /// X offset within image
    pub x: u32,
    /// Y offset within image
    pub y: u32,
    /// Gap to next frame in milliseconds
    pub gap: u32,
    /// Frame data (RGBA)
    pub data: Vec<u8>,
    /// Base frame ID (for delta frames, 0 = none)
    pub base_frame_id: u32,
    /// Background color for composition
    pub bgcolor: u32,
    /// Alpha blend mode (false = overwrite)
    pub alpha_blend: bool,
}

/// Kitty graphics image (with animation support)
#[derive(Debug)]
pub struct KittyImage {
    pub id: u32,
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>, // RGBA - root frame data (frame 1)
    /// Do not move cursor after display (C=1)
    pub do_not_move_cursor: bool,
    /// Display columns (c= parameter, 0 = auto-calculate from pixel size)
    pub display_cols: u32,
    /// Display rows (r= parameter, 0 = auto-calculate from pixel size)
    pub display_rows: u32,
    /// Z-index for layering
    pub z_index: i32,
    /// Source rect (x, y, w, h)
    pub src_x: u32,
    pub src_y: u32,
    pub src_w: u32,
    pub src_h: u32,
    /// Unicode virtual placement (U=1)
    pub unicode_placement: bool,
    /// Placement ID (p)
    pub placement_id: u32,
}

/// Frame data result from a=f action
#[derive(Debug)]
pub struct KittyFrameData {
    /// Image ID
    pub image_id: u32,
    /// Frame number (1-based)
    pub frame_number: u32,
    /// Frame width
    pub width: u32,
    /// Frame height
    pub height: u32,
    /// X offset
    pub x: u32,
    /// Y offset
    pub y: u32,
    /// Gap to next frame (ms)
    pub gap: u32,
    /// Frame data (RGBA)
    pub data: Vec<u8>,
    /// Base frame to copy from
    pub base_frame: u32,
    /// Compose mode (0=blend, 1=overwrite)
    pub compose_mode: u8,
    /// Background color
    pub bgcolor: u32,
}

/// Compose command parameters
#[derive(Debug)]
pub struct KittyComposeCmd {
    /// Image ID
    pub image_id: u32,
    /// Source frame number
    pub src_frame: u32,
    /// Destination frame number
    pub dst_frame: u32,
    /// Source X offset
    pub src_x: u32,
    /// Source Y offset
    pub src_y: u32,
    /// Destination X offset
    pub dst_x: u32,
    /// Destination Y offset
    pub dst_y: u32,
    /// Width to copy
    pub width: u32,
    /// Height to copy
    pub height: u32,
    /// Compose mode (0=blend, 1=overwrite)
    pub compose_mode: u8,
}

/// Animation control command
#[derive(Debug)]
pub struct KittyAnimationCmd {
    /// Image ID
    pub image_id: u32,
    /// Frame number to modify (0 = none)
    pub frame_number: u32,
    /// Frame to display (0 = no change)
    pub current_frame: u32,
    /// Animation state (0=no change, 1=stop, 2=loading, 3=running)
    pub state: u8,
    /// Loop count (0 = no change)
    pub loop_count: u32,
    /// Gap for frame_number (negative = no change)
    pub gap: i32,
}

/// Result of decoding a Kitty command
#[derive(Debug)]
pub enum KittyDecodeResult {
    /// New image (a=t, a=T)
    Image(KittyImage),
    /// Frame data (a=f)
    Frame(KittyFrameData),
    /// Compose command (a=c) - no data, just parameters
    Compose(KittyComposeCmd),
    /// Animation control (a=a) - no data, just parameters
    Animation(KittyAnimationCmd),
}

/// Kitty command action
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum KittyAction {
    /// Transmit only (a=t)
    Transmit,
    /// Transmit and display (a=T, default)
    TransmitAndDisplay,
    /// Display (a=p)
    Display,
    /// Delete (a=d)
    Delete,
    /// Query (a=q)
    Query,
    /// Frame load (a=f) - load frame data for animation
    Frame,
    /// Compose (a=c) - copy rectangle from one frame to another
    Compose,
    /// Animation control (a=a) - start/stop/control animation
    Animation,
}

impl Default for KittyAction {
    fn default() -> Self {
        KittyAction::TransmitAndDisplay
    }
}

/// Image format
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum KittyFormat {
    /// RGB (f=24)
    Rgb,
    /// RGBA (f=32)
    Rgba,
    /// PNG (f=100)
    Png,
}

impl Default for KittyFormat {
    fn default() -> Self {
        KittyFormat::Rgba
    }
}

/// Transmission medium
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum KittyTransmission {
    /// Direct (t=d)
    Direct,
    /// File (t=f)
    File,
    /// Shared memory (t=s)
    SharedMemory,
    /// Temporary file (t=t)
    TempFile,
}

impl Default for KittyTransmission {
    fn default() -> Self {
        KittyTransmission::Direct
    }
}

/// Kitty command parameters
#[derive(Debug, Default)]
pub struct KittyParams {
    /// Action (a)
    pub action: KittyAction,
    /// Format (f)
    pub format: KittyFormat,
    /// Transmission medium (t)
    pub transmission: KittyTransmission,
    /// Image ID (i)
    pub id: u32,
    /// Image number (I) - for identifying multiple images
    pub number: u32,
    /// Width (s)
    pub width: u32,
    /// Height (v)
    pub height: u32,
    /// Chunk continuation flag (m): 1=continue, 0=end
    pub more: bool,
    /// Compression (o): z=zlib
    pub compression: Option<char>,
    /// Quiet mode (q): 1=no response on success, 2=no response
    pub quiet: u8,
    /// Placement X (x)
    pub x: u32,
    /// Placement Y (y)
    pub y: u32,
    /// Cell width (c)
    pub cols: u32,
    /// Cell height (r)
    pub rows: u32,
    /// Z index (z)
    pub z_index: i32,
    /// Delete target (d): a=all, i=id, etc.
    pub delete_target: Option<char>,

    // Animation parameters
    /// Frame number (r) - 1-based frame index
    pub frame_number: u32,
    /// Other frame number (x) - for compose: destination frame
    pub other_frame_number: u32,
    /// Frame gap (z) - milliseconds between frames
    pub gap: i32,
    /// Compose mode (C) - 0=alpha blend, 1=overwrite
    pub compose_mode: u8,
    /// Animation state (s) - 1=stop, 2=loading, 3=running
    pub animation_state: u8,
    /// Loop count (v) - 0=infinite, n=loop n times
    pub loop_count: u32,
    /// Base frame (c) - frame to copy from for new frames
    pub base_frame: u32,
    /// Frame X offset (X) - where to place frame data
    pub frame_x: u32,
    /// Frame Y offset (Y) - where to place frame data
    pub frame_y: u32,
    /// Background color (Y) - for frame composition
    pub bgcolor: u32,
    /// Source rect width (w parameter, 0=full)
    pub src_w: u32,
    /// Source rect height (h parameter, 0=full)
    pub src_h: u32,
    /// Do not move cursor (C=1 for display actions)
    pub do_not_move_cursor: bool,
    /// Unicode virtual placement (U=1)
    pub unicode_placement: bool,
    /// Placement ID (p)
    pub placement_id: u32,
    /// Parent image ID for relative placement (P)
    pub parent_id: u32,
    /// Parent placement ID for relative placement (Q)
    pub parent_placement_id: u32,
    /// Horizontal offset from parent (H)
    pub rel_h: i32,
    /// Vertical offset from parent (V)
    pub rel_v: i32,
}

/// Kitty decoder
pub struct KittyDecoder {
    /// Accumulated decoded payload bytes.
    /// Each chunk's base64 payload is a self-contained unit (may end with `=`
    /// padding), so we decode per-chunk and append the resulting bytes here.
    payload_buffer: Vec<u8>,
    /// Current parameters
    params: KittyParams,
    /// Whether this is the first chunk
    first_chunk: bool,
}

impl KittyDecoder {
    pub fn new() -> Self {
        Self {
            payload_buffer: Vec::new(),
            params: KittyParams::default(),
            first_chunk: true,
        }
    }

    /// Process APC sequence
    /// Returns: (completion flag, response)
    pub fn process(&mut self, data: &[u8]) -> (bool, Option<Vec<u8>>) {
        // Parse data after 'G'
        // Format: key=value,key=value,...;base64data

        let data_str = match std::str::from_utf8(data) {
            Ok(s) => s,
            Err(_) => {
                warn!("Kitty: invalid UTF-8 in APC");
                return (true, None);
            }
        };

        // Separate parameters and payload by ';'
        let (params_str, payload) = match data_str.find(';') {
            Some(pos) => (&data_str[..pos], &data_str[pos + 1..]),
            None => (data_str, ""),
        };

        // Parse parameters if first chunk
        if self.first_chunk {
            self.parse_params(params_str);
            self.first_chunk = false;
        } else {
            // For subsequent chunks, only update m parameter
            for part in params_str.split(',') {
                if let Some((key, value)) = part.split_once('=') {
                    if key == "m" {
                        self.params.more = value == "1";
                    }
                }
            }
        }

        // Decode each chunk's base64 payload independently and append the
        // resulting bytes. Chunks are self-contained base64 units per the Kitty
        // protocol — e.g. chafa ends every chunk with `=` padding — so we must
        // NOT concatenate raw base64 across chunks (that would drop the
        // leftover bits of each chunk's final group into the next chunk's
        // bitstream and shift the decoded output).
        if !payload.is_empty() {
            let decoded = match base64_decode(payload.as_bytes()) {
                Some(d) => d,
                None => Vec::new(), // base64_decode is lenient, shouldn't reach here
            };
            if self.payload_buffer.len() + decoded.len() <= MAX_IMAGE_DATA_SIZE {
                self.payload_buffer.extend_from_slice(&decoded);
            } else {
                warn!(
                    "Kitty: image data exceeds {}MB limit, truncating",
                    MAX_IMAGE_DATA_SIZE / 1024 / 1024
                );
            }
        }

        // Not complete if chunk continues
        if self.params.more {
            return (false, None);
        }

        // Complete
        (true, None)
    }

    /// Parse parameters
    fn parse_params(&mut self, params_str: &str) {
        for part in params_str.split(',') {
            if let Some((key, value)) = part.split_once('=') {
                match key {
                    "a" => {
                        self.params.action = match value {
                            "t" => KittyAction::Transmit,
                            "T" => KittyAction::TransmitAndDisplay,
                            "p" => KittyAction::Display,
                            "d" => KittyAction::Delete,
                            "q" => KittyAction::Query,
                            "f" => KittyAction::Frame,
                            "c" => KittyAction::Compose,
                            "a" => KittyAction::Animation,
                            _ => KittyAction::TransmitAndDisplay,
                        };
                    }
                    "f" => {
                        self.params.format = match value {
                            "24" => KittyFormat::Rgb,
                            "32" => KittyFormat::Rgba,
                            "100" => KittyFormat::Png,
                            _ => KittyFormat::Rgba,
                        };
                    }
                    "t" => {
                        self.params.transmission = match value {
                            "d" => KittyTransmission::Direct,
                            "f" => KittyTransmission::File,
                            "s" => KittyTransmission::SharedMemory,
                            "t" => KittyTransmission::TempFile,
                            _ => KittyTransmission::Direct,
                        };
                    }
                    "i" => {
                        self.params.id = value.parse().unwrap_or(0);
                    }
                    "I" => {
                        self.params.number = value.parse().unwrap_or(0);
                    }
                    "s" => {
                        // s = width for images, animation_state for a=a
                        let v = value.parse().unwrap_or(0);
                        self.params.width = v;
                        self.params.animation_state = v as u8;
                    }
                    "v" => {
                        // v = height for images, loop_count for animation
                        let v = value.parse().unwrap_or(0);
                        self.params.height = v;
                        self.params.loop_count = v;
                    }
                    "w" => {
                        self.params.src_w = value.parse().unwrap_or(0);
                    }
                    "h" => {
                        self.params.src_h = value.parse().unwrap_or(0);
                    }
                    "m" => {
                        self.params.more = value == "1";
                    }
                    "o" => {
                        self.params.compression = value.chars().next();
                    }
                    "q" => {
                        self.params.quiet = value.parse().unwrap_or(0);
                    }
                    "x" => {
                        self.params.x = value.parse().unwrap_or(0);
                    }
                    "y" => {
                        self.params.y = value.parse().unwrap_or(0);
                    }
                    "c" => {
                        self.params.cols = value.parse().unwrap_or(0);
                    }
                    "r" => {
                        // r = rows for placement, frame_number for animation
                        let v = value.parse().unwrap_or(0);
                        self.params.rows = v;
                        self.params.frame_number = v;
                    }
                    "z" => {
                        // z = z_index for placement, gap for animation
                        let v: i32 = value.parse().unwrap_or(0);
                        self.params.z_index = v;
                        self.params.gap = v;
                    }
                    "d" => {
                        self.params.delete_target = value.chars().next();
                    }
                    // Animation-specific parameters (uppercase)
                    "X" => {
                        self.params.frame_x = value.parse().unwrap_or(0);
                    }
                    "Y" => {
                        // Y = frame_y for animation, bgcolor for certain commands
                        let v = value.parse().unwrap_or(0);
                        self.params.frame_y = v;
                        self.params.bgcolor = v;
                    }
                    "C" => {
                        let v: u8 = value.parse().unwrap_or(0);
                        self.params.compose_mode = v;
                        // C=1 also means "do not move cursor" for display actions
                        self.params.do_not_move_cursor = v == 1;
                    }
                    "U" => {
                        let v: u8 = value.parse().unwrap_or(0);
                        self.params.unicode_placement = v == 1;
                    }
                    "p" => {
                        self.params.placement_id = value.parse().unwrap_or(0);
                    }
                    "P" => {
                        self.params.parent_id = value.parse().unwrap_or(0);
                    }
                    "Q" => {
                        self.params.parent_placement_id = value.parse().unwrap_or(0);
                    }
                    "H" => {
                        self.params.rel_h = value.parse().unwrap_or(0);
                    }
                    "V" => {
                        self.params.rel_v = value.parse().unwrap_or(0);
                    }
                    _ => {
                        // For animation, 'c' also means other_frame_number
                        if key == "c" {
                            let v = value.parse().unwrap_or(0);
                            self.params.cols = v;
                            self.params.base_frame = v;
                            self.params.other_frame_number = v;
                        } else if key == "v" {
                            let v = value.parse().unwrap_or(0);
                            self.params.height = v;
                            self.params.loop_count = v;
                        } else {
                            trace!("Kitty: unknown param {}={}", key, value);
                        }
                    }
                }
            }
        }
    }

    /// Decode complete, generate result based on action
    /// allow_remote: if false, reject File/TempFile/SharedMemory transfers
    pub fn finish(self, next_id: u32, allow_remote: bool) -> Result<KittyDecodeResult, String> {
        let params = self.params;
        let id = if params.id != 0 { params.id } else { next_id };

        // payload_buffer already contains decoded bytes (chunks are decoded
        // on arrival in process()).
        let raw_data = self.payload_buffer;
        trace!("Kitty: total decoded payload = {} bytes", raw_data.len());

        // Handle non-data actions first
        match params.action {
            KittyAction::Delete => {
                return Err("delete action".to_string());
            }
            KittyAction::Compose => {
                // Compose command - no data payload needed
                return Ok(KittyDecodeResult::Compose(KittyComposeCmd {
                    image_id: id,
                    src_frame: params.frame_number,
                    dst_frame: params.other_frame_number,
                    src_x: params.frame_x,
                    src_y: params.frame_y,
                    dst_x: params.x,
                    dst_y: params.y,
                    width: params.width,
                    height: params.height,
                    compose_mode: params.compose_mode,
                }));
            }
            KittyAction::Animation => {
                // Animation control - no data payload needed
                return Ok(KittyDecodeResult::Animation(KittyAnimationCmd {
                    image_id: id,
                    frame_number: params.frame_number,
                    current_frame: params.other_frame_number,
                    state: params.animation_state,
                    loop_count: params.loop_count,
                    gap: params.gap,
                }));
            }
            _ => {}
        }

        // Actions that need data (Transmit, TransmitAndDisplay, Display, Frame, Query)
        let mut data = load_data_from_transmission(params.transmission, raw_data, allow_remote)?;

        // zlib decompression
        if params.compression == Some('z') {
            data = decompress_zlib(&data)?;
        }

        // For Frame action, return frame data
        if params.action == KittyAction::Frame {
            let (width, height, rgba) = decode_image_data(&params, data)?;
            return Ok(KittyDecodeResult::Frame(KittyFrameData {
                image_id: id,
                frame_number: params.frame_number,
                width,
                height,
                x: params.frame_x,
                y: params.frame_y,
                gap: if params.gap > 0 {
                    params.gap as u32
                } else {
                    40
                }, // default 40ms
                data: rgba,
                base_frame: params.base_frame,
                compose_mode: params.compose_mode,
                bgcolor: params.bgcolor,
            }));
        }

        // For image actions (Transmit, TransmitAndDisplay, Query)
        let (width, height, rgba) = decode_image_data(&params, data)?;

        trace!("Kitty: decoded image {}x{} (id={})", width, height, id);

        Ok(KittyDecodeResult::Image(KittyImage {
            id,
            width,
            height,
            data: rgba,
            do_not_move_cursor: params.do_not_move_cursor,
            display_cols: params.cols,
            display_rows: params.rows,
            z_index: params.z_index,
            src_x: params.x,
            src_y: params.y,
            src_w: params.src_w,
            src_h: params.src_h,
            unicode_placement: params.unicode_placement,
            placement_id: params.placement_id,
        }))
    }

    /// Get parameters
    pub fn params(&self) -> &KittyParams {
        &self.params
    }
}

/// Load data from transmission medium
/// allow_remote: if false, reject File/TempFile/SharedMemory (only Direct allowed)
fn load_data_from_transmission(
    transmission: KittyTransmission,
    raw_data: Vec<u8>,
    allow_remote: bool,
) -> Result<Vec<u8>, String> {
    match transmission {
        KittyTransmission::Direct => Ok(raw_data),
        KittyTransmission::File => {
            if !allow_remote {
                return Err("remote file transfer (t=f) disabled. \
                     Set [security] allow_kitty_remote = true in config to enable."
                    .to_string());
            }
            let path = String::from_utf8(raw_data).map_err(|_| "invalid file path encoding")?;
            let path = path.trim();
            info!("Kitty: reading image from file: {}", path);
            let data = std::fs::read(path)
                .map_err(|e| format!("failed to read file '{}': {}", path, e))?;
            if data.len() > MAX_IMAGE_DATA_SIZE {
                return Err(format!(
                    "file too large: {} bytes (max {})",
                    data.len(),
                    MAX_IMAGE_DATA_SIZE
                ));
            }
            Ok(data)
        }
        KittyTransmission::TempFile => {
            if !allow_remote {
                return Err("remote temp file transfer (t=t) disabled. \
                     Set [security] allow_kitty_remote = true in config to enable."
                    .to_string());
            }
            let path =
                String::from_utf8(raw_data).map_err(|_| "invalid temp file path encoding")?;
            let path = path.trim();
            // Validate canonical path is under /tmp/ or /dev/shm/ (prevent symlink traversal)
            match std::fs::canonicalize(path) {
                Ok(canonical) => {
                    let canonical_str = canonical.to_string_lossy();
                    if !canonical_str.starts_with("/tmp/")
                        && !canonical_str.starts_with("/dev/shm/")
                    {
                        return Err(format!(
                            "temp file path '{}' (resolved: {}) not under /tmp/ or /dev/shm/",
                            path, canonical_str
                        ));
                    }
                }
                Err(e) => {
                    return Err(format!(
                        "failed to resolve temp file path '{}': {}",
                        path, e
                    ));
                }
            }
            info!("Kitty: reading image from temp file: {}", path);
            let data = std::fs::read(path)
                .map_err(|e| format!("failed to read temp file '{}': {}", path, e))?;
            if let Err(e) = std::fs::remove_file(path) {
                warn!("Kitty: failed to remove temp file '{}': {}", path, e);
            }
            if data.len() > MAX_IMAGE_DATA_SIZE {
                return Err(format!(
                    "temp file too large: {} bytes (max {})",
                    data.len(),
                    MAX_IMAGE_DATA_SIZE
                ));
            }
            Ok(data)
        }
        KittyTransmission::SharedMemory => {
            if !allow_remote {
                return Err("shared memory transfer (t=s) disabled. \
                     Set [security] allow_kitty_remote = true in config to enable."
                    .to_string());
            }
            let name = String::from_utf8(raw_data).map_err(|_| "invalid shm name encoding")?;
            let name = name.trim();
            info!("Kitty: reading image from shared memory: {}", name);
            read_shared_memory(name)
        }
    }
}

/// Decode image data according to format
fn decode_image_data(params: &KittyParams, data: Vec<u8>) -> Result<(u32, u32, Vec<u8>), String> {
    match params.format {
        KittyFormat::Png => decode_png(&data),
        KittyFormat::Rgb => {
            let w = params.width;
            let h = params.height;
            if w == 0 || h == 0 {
                return Err("missing width/height for RGB".to_string());
            }
            let rgba = rgb_to_rgba(&data, w, h)?;
            Ok((w, h, rgba))
        }
        KittyFormat::Rgba => {
            let w = params.width;
            let h = params.height;
            if w == 0 || h == 0 {
                return Err("missing width/height for RGBA".to_string());
            }
            let expected_size = (w as usize)
                .checked_mul(h as usize)
                .and_then(|wh| wh.checked_mul(4))
                .ok_or_else(|| "RGBA dimensions too large".to_string())?;
            let mut data = data;
            if data.len() != expected_size {
                trace!(
                    "RGBA size: expected {}, got {} (delta {})",
                    expected_size, data.len(),
                    expected_size as i64 - data.len() as i64
                );
                data.resize(expected_size, 0);
            }
            Ok((w, h, data))
        }
    }
}

/// Base64 decode — lenient: skips invalid bytes instead of aborting.
/// Some mpv versions emit chunks with stray bytes; aborting the
/// entire frame on a single bad byte makes video unplayable.
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
            _ => continue, // skip invalid bytes instead of aborting
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

/// Recalculate PNG chunk CRCs for compatibility with non-conforming PNGs.
/// Many image generators produce PNGs with incorrect CRCs; real terminals
/// (Kitty, etc.) silently accept them. We do the same by fixing CRCs before decode.
fn fix_png_checksums(data: &[u8]) -> Vec<u8> {
    const PNG_SIG: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    if data.len() < 12 || data[..8] != PNG_SIG {
        return data.to_vec();
    }

    let mut fixed = Vec::with_capacity(data.len());
    fixed.extend_from_slice(&data[..8]); // PNG signature
    let mut any_fixed = false;

    let mut pos = 8;
    while pos + 12 <= data.len() {
        let chunk_len =
            u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
        if pos + 12 + chunk_len > data.len() {
            fixed.extend_from_slice(&data[pos..]);
            return fixed;
        }

        // Copy length + type + data
        fixed.extend_from_slice(&data[pos..pos + 8 + chunk_len]);

        // Recalculate CRC over type + data
        let mut crc = flate2::Crc::new();
        crc.update(&data[pos + 4..pos + 8 + chunk_len]);
        let correct_crc = crc.sum();
        fixed.extend_from_slice(&correct_crc.to_be_bytes());

        // Check if CRC was wrong
        let stored_crc = u32::from_be_bytes([
            data[pos + 8 + chunk_len],
            data[pos + 9 + chunk_len],
            data[pos + 10 + chunk_len],
            data[pos + 11 + chunk_len],
        ]);
        if stored_crc != correct_crc {
            any_fixed = true;
        }

        pos += 12 + chunk_len;
    }

    if any_fixed {
        trace!("Kitty: fixed PNG chunk CRCs");
    }

    fixed
}

/// PNG decode
fn decode_png(data: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    use image::io::Reader as ImageReader;
    use std::io::Cursor;

    // Fix CRCs for compatibility with non-conforming PNGs
    let data = fix_png_checksums(data);

    let reader = ImageReader::new(Cursor::new(&data))
        .with_guessed_format()
        .map_err(|e| format!("PNG format error: {}", e))?;

    let img = reader
        .decode()
        .map_err(|e| format!("PNG decode error: {}", e))?;

    let rgba = img.to_rgba8();
    let width = rgba.width();
    let height = rgba.height();
    let raw = rgba.into_raw();

    // Guard against decompression bombs: decoded RGBA can be far larger than compressed PNG
    if raw.len() > MAX_IMAGE_DATA_SIZE {
        return Err(format!(
            "decoded PNG too large: {}x{} = {} bytes (max {})",
            width,
            height,
            raw.len(),
            MAX_IMAGE_DATA_SIZE
        ));
    }

    Ok((width, height, raw))
}

/// Convert RGB to RGBA
fn rgb_to_rgba(data: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|wh| wh.checked_mul(3))
        .ok_or_else(|| "RGB dimensions too large".to_string())?;
    // Tolerate minor size differences — base64 chunk boundaries or
    // mpv version quirks can cause a few hundred bytes of drift.
    // Use whichever is smaller to avoid out-of-bounds reads.
    let usable = data.len().min(expected);
    if data.len() != expected {
        trace!(
            "RGB size: expected {}, got {} (delta {}), using {}",
            expected, data.len(),
            expected as i64 - data.len() as i64, usable
        );
    }

    let pixel_count = usable / 3;
    let rgba_size = pixel_count * 4;
    let mut rgba = Vec::with_capacity(rgba_size);
    for chunk in data[..pixel_count * 3].chunks_exact(3) {
        rgba.push(chunk[0]);
        rgba.push(chunk[1]);
        rgba.push(chunk[2]);
        rgba.push(255);
    }
    // Pad to expected RGBA size if data was short
    rgba.resize(width as usize * height as usize * 4, 0);
    Ok(rgba)
}

/// zlib decompression (with output size limit)
fn decompress_zlib(data: &[u8]) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut decoder = flate2::read::ZlibDecoder::new(data);
    let mut output = Vec::new();

    // Read in chunks with size limit to prevent memory exhaustion
    let mut buf = [0u8; 65536];
    loop {
        match decoder.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                output.extend_from_slice(&buf[..n]);
                if output.len() > MAX_IMAGE_DATA_SIZE {
                    return Err(format!(
                        "decompressed data too large (max {} bytes)",
                        MAX_IMAGE_DATA_SIZE
                    ));
                }
            }
            Err(e) => return Err(format!("zlib decompress error: {}", e)),
        }
    }
    Ok(output)
}

/// Read data from shared memory (POSIX shm)
fn read_shared_memory(name: &str) -> Result<Vec<u8>, String> {
    use std::io::Read;
    use std::os::unix::io::FromRawFd;

    // Get file descriptor with shm_open
    let c_name = std::ffi::CString::new(name).map_err(|_| "invalid shm name")?;

    let fd = unsafe { libc::shm_open(c_name.as_ptr(), libc::O_RDONLY, 0) };

    if fd < 0 {
        return Err(format!(
            "shm_open failed: {}",
            std::io::Error::last_os_error()
        ));
    }

    // Get file size
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    let stat_result = unsafe { libc::fstat(fd, &mut stat) };
    if stat_result < 0 {
        unsafe { libc::close(fd) };
        return Err(format!("fstat failed: {}", std::io::Error::last_os_error()));
    }

    let size = stat.st_size as usize;

    // Check size limit
    if size > MAX_IMAGE_DATA_SIZE {
        unsafe {
            libc::close(fd);
            libc::shm_unlink(c_name.as_ptr()); // Clean up shared memory
        }
        return Err(format!(
            "shared memory too large: {} bytes (max {})",
            size, MAX_IMAGE_DATA_SIZE
        ));
    }

    // Convert to File and read
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    let mut data = vec![0u8; size];
    file.read_exact(&mut data)
        .map_err(|e| format!("shm read failed: {}", e))?;

    // Delete shared memory
    unsafe {
        libc::shm_unlink(c_name.as_ptr());
    }

    Ok(data)
}

/// Generate Kitty response
pub fn make_response(id: u32, ok: bool, message: &str) -> Vec<u8> {
    if ok {
        format!("\x1b_Gi={};OK\x1b\\", id).into_bytes()
    } else {
        format!("\x1b_Gi={};{}\x1b\\", id, message).into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64_encode(data: &[u8]) -> String {
        const TBL: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
        let mut i = 0;
        while i + 3 <= data.len() {
            let n = ((data[i] as u32) << 16) | ((data[i + 1] as u32) << 8) | data[i + 2] as u32;
            out.push(TBL[((n >> 18) & 0x3f) as usize] as char);
            out.push(TBL[((n >> 12) & 0x3f) as usize] as char);
            out.push(TBL[((n >> 6) & 0x3f) as usize] as char);
            out.push(TBL[(n & 0x3f) as usize] as char);
            i += 3;
        }
        let rem = data.len() - i;
        if rem == 1 {
            let n = (data[i] as u32) << 16;
            out.push(TBL[((n >> 18) & 0x3f) as usize] as char);
            out.push(TBL[((n >> 12) & 0x3f) as usize] as char);
            out.push_str("==");
        } else if rem == 2 {
            let n = ((data[i] as u32) << 16) | ((data[i + 1] as u32) << 8);
            out.push(TBL[((n >> 18) & 0x3f) as usize] as char);
            out.push(TBL[((n >> 12) & 0x3f) as usize] as char);
            out.push(TBL[((n >> 6) & 0x3f) as usize] as char);
            out.push('=');
        }
        out
    }

    /// Reproduces the chafa-style chunked Kitty transmission where each chunk
    /// is a self-contained base64 unit ending in `=` padding. The bug: bcon
    /// used to concatenate raw base64 across chunks and skip `=` during decode,
    /// leaking 2 leftover bits from each chunk's final group into the next
    /// chunk and producing `num_chunks * 2 / 8` extra bytes in the output.
    #[test]
    fn chunked_base64_with_per_chunk_padding_decodes_exact() {
        let w = 32u32;
        let h = 16u32;
        let pixel_count = (w * h) as usize;
        // Distinct byte pattern so off-by-one errors become visible.
        let rgba: Vec<u8> = (0..pixel_count * 4).map(|i| (i & 0xff) as u8).collect();
        assert_eq!(rgba.len(), 32 * 16 * 4);

        // Chunk size chosen so each chunk encodes to a base64 unit ending in
        // `=` padding (512 bytes → 684 chars with one trailing `=`, matching
        // chafa's actual chunking).
        let chunk_bytes = 512;
        let chunks: Vec<&[u8]> = rgba.chunks(chunk_bytes).collect();
        let num_chunks = chunks.len();
        assert!(num_chunks >= 2);

        let mut decoder = KittyDecoder::new();
        // First APC: metadata only, no payload, m=1 (chafa-style).
        let first = format!("a=T,f=32,s={},v={},m=1", w, h);
        decoder.process(first.as_bytes());

        for (i, chunk) in chunks.iter().enumerate() {
            let more = if i + 1 < num_chunks { 1 } else { 0 };
            let body = format!("m={};{}", more, b64_encode(chunk));
            decoder.process(body.as_bytes());
        }

        let result = decoder.finish(1, false).expect("decode");
        match result {
            KittyDecodeResult::Image(img) => {
                assert_eq!(img.width, w);
                assert_eq!(img.height, h);
                assert_eq!(img.data.len(), rgba.len());
                assert_eq!(img.data, rgba);
            }
            _ => panic!("expected Image result"),
        }
    }

    /// Reproduces the mpv v0.40.0 bug from issue #7: RGB data is slightly
    /// short of expected w*h*3 (720 bytes / 240 pixels missing). bcon must
    /// pad with zeros instead of rejecting the frame.
    #[test]
    fn rgb_size_mismatch_tolerant() {
        let w = 100u32;
        let h = 100u32;
        let expected_rgb = (w * h * 3) as usize; // 30000
        let short_by = 720usize; // same magnitude as reported bug
        let actual_rgb = expected_rgb - short_by; // 29280

        let rgb_data: Vec<u8> = (0..actual_rgb).map(|i| (i & 0xff) as u8).collect();
        let b64 = b64_encode(&rgb_data);

        let mut decoder = KittyDecoder::new();
        let first = format!("a=T,f=24,s={},v={},m=0;{}", w, h, b64);
        decoder.process(first.as_bytes());

        let result = decoder.finish(1, false);
        assert!(result.is_ok(), "should not reject short RGB data");
        match result.unwrap() {
            KittyDecodeResult::Image(img) => {
                assert_eq!(img.width, w);
                assert_eq!(img.height, h);
                // RGBA output padded to full size
                assert_eq!(img.data.len(), (w * h * 4) as usize);
            }
            _ => panic!("expected Image result"),
        }
    }

    /// base64 with stray bytes should not abort decoding.
    #[test]
    fn base64_stray_bytes_tolerated() {
        let data = vec![0xFFu8; 12]; // 12 bytes = 16 base64 chars
        let mut b64 = b64_encode(&data);
        // Insert a stray byte (tab, null, or other non-base64)
        b64.insert(4, '\t');
        b64.insert(8, '\0' as char);

        let decoded = super::base64_decode(b64.as_bytes());
        assert!(decoded.is_some(), "should not abort on stray bytes");
        assert_eq!(decoded.unwrap(), data);
    }
}
