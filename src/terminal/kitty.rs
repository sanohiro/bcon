//! Kitty graphics protocol
//!
//! https://sw.kovidgoyal.net/kitty/graphics-protocol/

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
}

/// Kitty decoder
pub struct KittyDecoder {
    /// Accumulated data (multi-chunk support)
    data_buffer: Vec<u8>,
    /// Current parameters
    params: KittyParams,
    /// Whether this is the first chunk
    first_chunk: bool,
}

impl KittyDecoder {
    pub fn new() -> Self {
        Self {
            data_buffer: Vec::new(),
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

        // Decode and accumulate Base64 payload (with size limit)
        if !payload.is_empty() {
            if let Some(decoded) = base64_decode(payload.as_bytes()) {
                // Check size limit before extending
                if self.data_buffer.len() + decoded.len() <= MAX_IMAGE_DATA_SIZE {
                    self.data_buffer.extend(decoded);
                } else {
                    warn!(
                        "Kitty: image data exceeds {}MB limit, truncating",
                        MAX_IMAGE_DATA_SIZE / 1024 / 1024
                    );
                    // Truncate to limit
                    let remaining = MAX_IMAGE_DATA_SIZE.saturating_sub(self.data_buffer.len());
                    if remaining > 0 {
                        self.data_buffer.extend(&decoded[..remaining]);
                    }
                }
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
                        self.params.compose_mode = value.parse().unwrap_or(0);
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
    pub fn finish(self, next_id: u32) -> Result<KittyDecodeResult, String> {
        let params = self.params;
        let raw_data = self.data_buffer;
        let id = if params.id != 0 { params.id } else { next_id };

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
        let mut data = load_data_from_transmission(params.transmission, raw_data)?;

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

        info!("Kitty: decoded image {}x{} (id={})", width, height, id);

        Ok(KittyDecodeResult::Image(KittyImage {
            id,
            width,
            height,
            data: rgba,
        }))
    }

    /// Get parameters
    pub fn params(&self) -> &KittyParams {
        &self.params
    }
}

/// Load data from transmission medium
fn load_data_from_transmission(
    transmission: KittyTransmission,
    raw_data: Vec<u8>,
) -> Result<Vec<u8>, String> {
    match transmission {
        KittyTransmission::Direct => Ok(raw_data),
        KittyTransmission::File => {
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
            let path =
                String::from_utf8(raw_data).map_err(|_| "invalid temp file path encoding")?;
            let path = path.trim();
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
            // Use checked arithmetic to prevent overflow
            let expected_size = (w as usize)
                .checked_mul(h as usize)
                .and_then(|wh| wh.checked_mul(4))
                .ok_or_else(|| "RGBA dimensions too large".to_string())?;
            if data.len() != expected_size {
                return Err(format!(
                    "RGBA size mismatch: expected {}, got {}",
                    expected_size,
                    data.len()
                ));
            }
            Ok((w, h, data))
        }
    }
}

/// Base64 decode
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

/// PNG decode
fn decode_png(data: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    use image::io::Reader as ImageReader;
    use std::io::Cursor;

    let reader = ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .map_err(|e| format!("PNG format error: {}", e))?;

    let img = reader
        .decode()
        .map_err(|e| format!("PNG decode error: {}", e))?;

    let rgba = img.to_rgba8();
    let width = rgba.width();
    let height = rgba.height();

    Ok((width, height, rgba.into_raw()))
}

/// Convert RGB to RGBA
fn rgb_to_rgba(data: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let expected = (width * height * 3) as usize;
    if data.len() != expected {
        return Err(format!(
            "RGB size mismatch: expected {}, got {}",
            expected,
            data.len()
        ));
    }

    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for chunk in data.chunks(3) {
        rgba.push(chunk[0]);
        rgba.push(chunk[1]);
        rgba.push(chunk[2]);
        rgba.push(255);
    }
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
        unsafe { libc::close(fd) };
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
