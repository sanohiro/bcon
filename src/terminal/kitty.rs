//! Kitty graphics protocol
//!
//! https://sw.kovidgoyal.net/kitty/graphics-protocol/

use log::{info, trace, warn};

/// Maximum accumulated image data size (256MB)
/// Allows 8K RGBA images (7680x4320x4 = 132MB raw)
const MAX_IMAGE_DATA_SIZE: usize = 256 * 1024 * 1024;

/// Kitty graphics image
#[derive(Debug)]
pub struct KittyImage {
    pub id: u32,
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>, // RGBA
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
                        self.params.width = value.parse().unwrap_or(0);
                    }
                    "v" => {
                        self.params.height = value.parse().unwrap_or(0);
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
                        self.params.rows = value.parse().unwrap_or(0);
                    }
                    "z" => {
                        self.params.z_index = value.parse().unwrap_or(0);
                    }
                    "d" => {
                        self.params.delete_target = value.chars().next();
                    }
                    _ => {
                        trace!("Kitty: unknown param {}={}", key, value);
                    }
                }
            }
        }
    }

    /// Decode complete, generate image
    pub fn finish(self, next_id: u32) -> Result<KittyImage, String> {
        let params = self.params;
        let raw_data = self.data_buffer;

        // No image for delete action
        if params.action == KittyAction::Delete {
            return Err("delete action".to_string());
        }

        // Get data according to transmission medium
        let mut data = match params.transmission {
            KittyTransmission::Direct => {
                // Direct transmission: data_buffer is the payload
                raw_data
            }
            KittyTransmission::File => {
                // File path: data_buffer is path string
                let path = String::from_utf8(raw_data)
                    .map_err(|_| "invalid file path encoding")?;
                let path = path.trim();
                info!("Kitty: reading image from file: {}", path);
                std::fs::read(path)
                    .map_err(|e| format!("failed to read file '{}': {}", path, e))?
            }
            KittyTransmission::TempFile => {
                // Temporary file: data_buffer is path string, delete after reading
                let path = String::from_utf8(raw_data)
                    .map_err(|_| "invalid temp file path encoding")?;
                let path = path.trim();
                info!("Kitty: reading image from temp file: {}", path);
                let data = std::fs::read(path)
                    .map_err(|e| format!("failed to read temp file '{}': {}", path, e))?;
                // Delete temporary file
                if let Err(e) = std::fs::remove_file(path) {
                    warn!("Kitty: failed to remove temp file '{}': {}", path, e);
                }
                data
            }
            KittyTransmission::SharedMemory => {
                // Shared memory: POSIX shm_open
                let name = String::from_utf8(raw_data)
                    .map_err(|_| "invalid shm name encoding")?;
                let name = name.trim();
                info!("Kitty: reading image from shared memory: {}", name);
                read_shared_memory(name)?
            }
        };

        // zlib decompression
        if params.compression == Some('z') {
            data = decompress_zlib(&data)?;
        }

        // Decode according to format
        let (width, height, rgba) = match params.format {
            KittyFormat::Png => decode_png(&data)?,
            KittyFormat::Rgb => {
                let w = params.width;
                let h = params.height;
                if w == 0 || h == 0 {
                    return Err("missing width/height for RGB".to_string());
                }
                let rgba = rgb_to_rgba(&data, w, h)?;
                (w, h, rgba)
            }
            KittyFormat::Rgba => {
                let w = params.width;
                let h = params.height;
                if w == 0 || h == 0 {
                    return Err("missing width/height for RGBA".to_string());
                }
                if data.len() != (w * h * 4) as usize {
                    return Err(format!(
                        "RGBA size mismatch: expected {}, got {}",
                        w * h * 4,
                        data.len()
                    ));
                }
                (w, h, data)
            }
        };

        let id = if params.id != 0 { params.id } else { next_id };

        info!("Kitty: decoded image {}x{} (id={})", width, height, id);

        Ok(KittyImage {
            id,
            width,
            height,
            data: rgba,
        })
    }

    /// Get parameters
    pub fn params(&self) -> &KittyParams {
        &self.params
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

/// zlib decompression
fn decompress_zlib(data: &[u8]) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut decoder = flate2::read::ZlibDecoder::new(data);
    let mut output = Vec::new();
    decoder
        .read_to_end(&mut output)
        .map_err(|e| format!("zlib decompress error: {}", e))?;
    Ok(output)
}

/// Read data from shared memory (POSIX shm)
fn read_shared_memory(name: &str) -> Result<Vec<u8>, String> {
    use std::os::unix::io::FromRawFd;
    use std::io::Read;

    // Get file descriptor with shm_open
    let c_name = std::ffi::CString::new(name)
        .map_err(|_| "invalid shm name")?;

    let fd = unsafe {
        libc::shm_open(c_name.as_ptr(), libc::O_RDONLY, 0)
    };

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
        return Err(format!(
            "fstat failed: {}",
            std::io::Error::last_os_error()
        ));
    }

    let size = stat.st_size as usize;

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
