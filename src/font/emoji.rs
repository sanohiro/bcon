//! Emoji processing
//!
//! Determines if a character is an emoji and extracts glyphs from the following tables:
//! - CBDT/CBLC: Bitmap format (Noto Color Emoji, etc.)
//! - COLR/CPAL: Vector format (layer compositing)

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use log::{info, trace, warn};

/// Determines if a character is an emoji
pub fn is_emoji(c: char) -> bool {
    let cp = c as u32;
    matches!(cp,
        // Miscellaneous Symbols and Pictographs
        0x1F300..=0x1F5FF |
        // Emoticons
        0x1F600..=0x1F64F |
        // Transport and Map Symbols
        0x1F680..=0x1F6FF |
        // Supplemental Symbols and Pictographs
        0x1F900..=0x1F9FF |
        // Symbols and Pictographs Extended-A
        0x1FA00..=0x1FA6F |
        // Symbols and Pictographs Extended-B
        0x1FA70..=0x1FAFF |
        // Dingbats
        0x2700..=0x27BF |
        // Miscellaneous Symbols
        0x2600..=0x26FF |
        // Regional Indicator Symbols
        0x1F1E0..=0x1F1FF |
        // Various common emoji
        0x203C | 0x2049 | 0x2122 | 0x2139 |
        0x2194..=0x2199 |
        0x21A9..=0x21AA |
        0x231A..=0x231B |
        0x2328 | 0x23CF |
        0x23E9..=0x23F3 |
        0x23F8..=0x23FA |
        0x24C2 |
        0x25AA..=0x25AB |
        0x25B6 | 0x25C0 |
        0x25FB..=0x25FE |
        0x2934..=0x2935 |
        0x2B05..=0x2B07 |
        0x2B1B..=0x2B1C |
        0x2B50 | 0x2B55 |
        0x3030 | 0x303D | 0x3297 | 0x3299
    )
}

/// Checks if character is ZWJ (Zero Width Joiner)
#[allow(dead_code)]
pub fn is_zwj(c: char) -> bool {
    c == '\u{200D}'
}

/// Checks if character is Variation Selector-16 (emoji style selector)
#[allow(dead_code)]
pub fn is_emoji_variation_selector(c: char) -> bool {
    c == '\u{FE0F}'
}

/// Checks if character is a Regional Indicator Symbol (for flags)
#[allow(dead_code)]
pub fn is_regional_indicator(c: char) -> bool {
    let cp = c as u32;
    (0x1F1E6..=0x1F1FF).contains(&cp)
}

/// Color emoji bitmap
#[derive(Debug)]
pub struct EmojiGlyph {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>, // RGBA
}

/// COLR layer information
#[derive(Debug, Clone)]
struct ColrLayer {
    glyph_id: u16,
    palette_index: u16,
}

/// Loader for color emoji from CBDT/CBLC and COLR/CPAL tables
pub struct EmojiLoader {
    /// Glyph ID -> bitmap map (CBDT)
    glyphs: HashMap<u16, EmojiGlyph>,
    /// cmap table (codepoint -> glyph ID)
    cmap: HashMap<u32, u16>,
    /// Font size (for strike index selection)
    target_size: u32,
    /// rustybuzz Face (for GSUB support)
    face: Option<rustybuzz::Face<'static>>,
    /// COLR layer map (base_glyph_id -> layer list)
    colr_layers: HashMap<u16, Vec<ColrLayer>>,
    /// CPAL color palette (RGBA)
    cpal_colors: Vec<(u8, u8, u8, u8)>,
    /// fontdue Font (for COLR rasterization)
    fontdue_font: Option<fontdue::Font>,
}

impl EmojiLoader {
    /// Load emoji from font file
    pub fn load<P: AsRef<Path>>(path: P, target_size: u32) -> Option<Self> {
        info!("EmojiLoader: loading from {:?}", path.as_ref());
        let mut file = match File::open(path.as_ref()) {
            Ok(f) => f,
            Err(e) => {
                warn!("EmojiLoader: cannot open file: {}", e);
                return None;
            }
        };
        let mut data = Vec::new();
        if let Err(e) = file.read_to_end(&mut data) {
            warn!("EmojiLoader: failed to read file: {}", e);
            return None;
        }
        info!("EmojiLoader: {} bytes read", data.len());

        // Create 'static lifetime data for rustybuzz/fontdue
        // Note: This intentionally leaks memory because rustybuzz::Face requires 'static lifetime.
        // Emoji font is loaded once at startup, so this is acceptable.
        let static_data: &'static [u8] = Box::leak(data.into_boxed_slice());
        let face = rustybuzz::Face::from_slice(static_data, 0);
        if face.is_some() {
            info!("EmojiLoader: rustybuzz Face created successfully (GSUB support)");
        }

        // Create fontdue Font (for COLR rasterization)
        let fontdue_font =
            fontdue::Font::from_bytes(static_data, fontdue::FontSettings::default()).ok();
        if fontdue_font.is_some() {
            info!("EmojiLoader: fontdue Font created successfully (COLR support)");
        }

        Self::from_bytes_with_face(static_data, target_size, face, fontdue_font)
    }

    /// Load from bytes (with rustybuzz Face + fontdue Font)
    fn from_bytes_with_face(
        data: &[u8],
        target_size: u32,
        face: Option<rustybuzz::Face<'static>>,
        fontdue_font: Option<fontdue::Font>,
    ) -> Option<Self> {
        // Parse OpenType header
        if data.len() < 12 {
            warn!("EmojiLoader: data too short");
            return None;
        }

        let sfnt_version = read_u32(data, 0);
        info!("EmojiLoader: sfnt_version = 0x{:08X}", sfnt_version);

        // TrueType or OpenType
        if sfnt_version != 0x00010000 && sfnt_version != 0x4F54544F {
            // TTC (font collection) case
            if sfnt_version == 0x74746366 {
                info!("EmojiLoader: TTC font collection detected");
                // Use the first font
                let num_fonts = read_u32(data, 8);
                if num_fonts == 0 {
                    warn!("EmojiLoader: TTC has no fonts");
                    return None;
                }
                let font_offset = read_u32(data, 12) as usize;
                info!("EmojiLoader: TTC first font offset = {}", font_offset);
                return Self::parse_font_at_offset(
                    data,
                    font_offset,
                    target_size,
                    face,
                    fontdue_font,
                );
            }
            warn!("EmojiLoader: unknown font format: 0x{:08X}", sfnt_version);
            return None;
        }

        Self::parse_font_at_offset(data, 0, target_size, face, fontdue_font)
    }

    fn parse_font_at_offset(
        data: &[u8],
        font_offset: usize,
        target_size: u32,
        face: Option<rustybuzz::Face<'static>>,
        fontdue_font: Option<fontdue::Font>,
    ) -> Option<Self> {
        let mut loader = Self {
            glyphs: HashMap::new(),
            cmap: HashMap::new(),
            target_size,
            face,
            colr_layers: HashMap::new(),
            cpal_colors: Vec::new(),
            fontdue_font,
        };

        if font_offset + 12 > data.len() {
            warn!("EmojiLoader: font offset out of bounds");
            return None;
        }

        let num_tables = read_u16(data, font_offset + 4) as usize;
        info!("EmojiLoader: {} tables found", num_tables);

        let mut cmap_offset = 0usize;
        let mut cblc_offset = 0usize;
        let mut cbdt_offset = 0usize;
        let mut colr_offset = 0usize;
        let mut cpal_offset = 0usize;
        let mut sbix_offset = 0usize;
        let mut svg_offset = 0usize;

        // Parse table directory
        for i in 0..num_tables {
            let entry_offset = font_offset + 12 + i * 16;
            if entry_offset + 16 > data.len() {
                break;
            }

            let tag = &data[entry_offset..entry_offset + 4];
            let offset = read_u32(data, entry_offset + 8) as usize;

            let tag_str = std::str::from_utf8(tag).unwrap_or("????");
            trace!("EmojiLoader: table '{}' at offset {}", tag_str, offset);

            match tag {
                b"cmap" => cmap_offset = offset,
                b"CBLC" => cblc_offset = offset,
                b"CBDT" => cbdt_offset = offset,
                b"COLR" => colr_offset = offset,
                b"CPAL" => cpal_offset = offset,
                b"sbix" => sbix_offset = offset,
                b"SVG " => svg_offset = offset,
                _ => {}
            }
        }

        info!(
            "EmojiLoader: cmap={}, CBLC={}, CBDT={}, COLR={}, CPAL={}, sbix={}, SVG={}",
            cmap_offset,
            cblc_offset,
            cbdt_offset,
            colr_offset,
            cpal_offset,
            sbix_offset,
            svg_offset
        );

        // Parse cmap
        if cmap_offset > 0 {
            loader.parse_cmap(data, cmap_offset);
            info!("EmojiLoader: cmap parsed, {} entries", loader.cmap.len());
        } else {
            warn!("EmojiLoader: cmap table not found");
        }

        // Parse CBLC/CBDT (bitmap format)
        if cblc_offset > 0 && cbdt_offset > 0 {
            loader.parse_cblc_cbdt(data, cblc_offset, cbdt_offset);
            info!("EmojiLoader: CBDT parsed, {} glyphs", loader.glyphs.len());
        }

        // Parse COLR/CPAL (vector format)
        if colr_offset > 0 && cpal_offset > 0 {
            loader.parse_cpal(data, cpal_offset);
            loader.parse_colr(data, colr_offset);
            info!(
                "EmojiLoader: COLR/CPAL parsed, {} color glyphs, {} palette entries",
                loader.colr_layers.len(),
                loader.cpal_colors.len()
            );
        }

        // sbix/SVG not supported
        if sbix_offset > 0 && loader.glyphs.is_empty() && loader.colr_layers.is_empty() {
            warn!("EmojiLoader: this font uses sbix (Apple) format (not supported)");
        }
        if svg_offset > 0 && loader.glyphs.is_empty() && loader.colr_layers.is_empty() {
            warn!("EmojiLoader: this font uses SVG format (not supported)");
        }

        // Fail if both CBDT and COLR are empty
        if loader.glyphs.is_empty() && loader.colr_layers.is_empty() {
            warn!("EmojiLoader: no glyphs were loaded");
            return None;
        }

        info!(
            "EmojiLoader: {} glyphs loaded, {} cmap entries",
            loader.glyphs.len(),
            loader.cmap.len()
        );

        Some(loader)
    }

    /// Parse cmap table
    fn parse_cmap(&mut self, data: &[u8], offset: usize) {
        if offset + 4 > data.len() {
            return;
        }

        let num_tables = read_u16(data, offset + 2) as usize;

        // Look for Format 12 (Unicode full) or Format 4 (BMP)
        for i in 0..num_tables {
            let record_offset = offset + 4 + i * 8;
            if record_offset + 8 > data.len() {
                break;
            }

            let platform_id = read_u16(data, record_offset);
            let encoding_id = read_u16(data, record_offset + 2);
            let subtable_offset = offset + read_u32(data, record_offset + 4) as usize;

            // Unicode platform
            if platform_id == 0 || (platform_id == 3 && encoding_id == 10) {
                if subtable_offset + 2 > data.len() {
                    continue;
                }
                let format = read_u16(data, subtable_offset);

                if format == 12 {
                    self.parse_cmap_format12(data, subtable_offset);
                    return;
                }
            }
        }
    }

    /// Parse cmap Format 12
    fn parse_cmap_format12(&mut self, data: &[u8], offset: usize) {
        if offset + 16 > data.len() {
            return;
        }

        let num_groups = read_u32(data, offset + 12) as usize;

        for i in 0..num_groups {
            let group_offset = offset + 16 + i * 12;
            if group_offset + 12 > data.len() {
                break;
            }

            let start_char = read_u32(data, group_offset);
            let end_char = read_u32(data, group_offset + 4);
            let start_glyph = read_u32(data, group_offset + 8);

            for (j, cp) in (start_char..=end_char).enumerate() {
                let glyph_id = (start_glyph + j as u32) as u16;
                self.cmap.insert(cp, glyph_id);
            }
        }
    }

    /// Parse CBLC/CBDT tables
    fn parse_cblc_cbdt(&mut self, data: &[u8], cblc_offset: usize, cbdt_offset: usize) {
        if cblc_offset + 8 > data.len() {
            return;
        }

        let major_version = read_u16(data, cblc_offset);
        if major_version != 2 && major_version != 3 {
            warn!("CBLC: unsupported version {}", major_version);
            return;
        }

        let num_sizes = read_u32(data, cblc_offset + 4) as usize;
        info!("CBLC: version={}, num_sizes={}", major_version, num_sizes);

        // Select the optimal size strike
        // Choose the smallest one >= target size. If none, choose the largest
        let mut best_strike_offset = 0usize;
        let mut best_ppem = 0u8;
        let mut smallest_ppem = 255u8;
        let mut smallest_strike_offset = 0usize;

        for i in 0..num_sizes {
            let strike_offset = cblc_offset + 8 + i * 48;
            if strike_offset + 48 > data.len() {
                break;
            }

            let ppem_x = data[strike_offset + 44];
            let ppem_y = data[strike_offset + 45];
            info!("CBLC: strike {} ppem={}x{}", i, ppem_x, ppem_y);

            // Record the smallest strike
            if ppem_y < smallest_ppem {
                smallest_ppem = ppem_y;
                smallest_strike_offset = strike_offset;
            }

            // Choose the smallest one >= target size
            if ppem_y as u32 >= self.target_size {
                if best_ppem == 0 || ppem_y < best_ppem {
                    best_ppem = ppem_y;
                    best_strike_offset = strike_offset;
                }
            }
        }

        // If none >= target size, use the smallest one
        if best_strike_offset == 0 {
            best_strike_offset = smallest_strike_offset;
            best_ppem = smallest_ppem;
        }

        if best_strike_offset == 0 {
            warn!("CBLC: no suitable strike found");
            return;
        }

        info!("CBLC: using strike with ppem={}", best_ppem);

        // Parse BitmapSize table
        let index_subtable_array_offset_rel = read_u32(data, best_strike_offset) as usize;
        let index_subtable_array_offset = cblc_offset + index_subtable_array_offset_rel;
        let _index_tables_size = read_u32(data, best_strike_offset + 4) as usize;
        let num_index_subtables = read_u32(data, best_strike_offset + 8) as usize;

        info!(
            "CBLC: indexSubTableArrayOffset={} (rel {}), numIndexSubTables={}",
            index_subtable_array_offset, index_subtable_array_offset_rel, num_index_subtables
        );

        for i in 0..num_index_subtables {
            let subtable_offset = index_subtable_array_offset + i * 8;
            if subtable_offset + 8 > data.len() {
                warn!(
                    "CBLC: subtable {} offset {} out of bounds",
                    i, subtable_offset
                );
                break;
            }

            let first_glyph = read_u16(data, subtable_offset);
            let last_glyph = read_u16(data, subtable_offset + 2);
            let additional_offset = read_u32(data, subtable_offset + 4) as usize;

            let header_offset = index_subtable_array_offset + additional_offset;
            if header_offset + 8 > data.len() {
                warn!(
                    "CBLC: subtable {} header offset {} out of bounds",
                    i, header_offset
                );
                continue;
            }

            let index_format = read_u16(data, header_offset);
            let image_format = read_u16(data, header_offset + 2);
            let image_data_offset = cbdt_offset + read_u32(data, header_offset + 4) as usize;

            info!(
                "CBLC: subtable {} glyphs {}-{}, indexFormat={}, imageFormat={}, imageDataOffset={}",
                i, first_glyph, last_glyph, index_format, image_format, image_data_offset
            );

            self.parse_index_subtable(
                data,
                header_offset,
                index_format,
                image_format,
                image_data_offset,
                first_glyph,
                last_glyph,
                cbdt_offset,
            );
        }

        info!("CBLC: parsed {} glyphs total", self.glyphs.len());
    }

    fn parse_index_subtable(
        &mut self,
        data: &[u8],
        header_offset: usize,
        index_format: u16,
        image_format: u16,
        image_data_offset: usize,
        first_glyph: u16,
        last_glyph: u16,
        _cbdt_offset: usize,
    ) {
        let num_glyphs = (last_glyph - first_glyph + 1) as usize;
        let mut parsed_count = 0usize;

        match index_format {
            1 => {
                // Format 1: variable size images (4-byte offsets)
                for i in 0..num_glyphs {
                    let offset_entry = header_offset + 8 + i * 4;
                    if offset_entry + 4 > data.len() {
                        break;
                    }

                    let rel_offset = read_u32(data, offset_entry) as usize;
                    let sbit_offset = image_data_offset + rel_offset;
                    let glyph_id = first_glyph + i as u16;

                    if let Some(glyph) = self.parse_glyph(data, sbit_offset, image_format) {
                        self.glyphs.insert(glyph_id, glyph);
                        parsed_count += 1;
                    }
                }
                trace!(
                    "IndexSubtable format 1: parsed {}/{} glyphs",
                    parsed_count,
                    num_glyphs
                );
            }
            2 => {
                // Format 2: fixed size images
                let image_size = read_u32(data, header_offset + 8) as usize;
                let num_glyphs = (last_glyph - first_glyph + 1) as usize;

                for i in 0..num_glyphs {
                    let sbit_offset = image_data_offset + i * image_size;
                    let glyph_id = first_glyph + i as u16;

                    if let Some(glyph) = self.parse_glyph(data, sbit_offset, image_format) {
                        self.glyphs.insert(glyph_id, glyph);
                    }
                }
            }
            3 => {
                // Format 3: variable size images (16-bit offsets)
                let num_glyphs = (last_glyph - first_glyph + 1) as usize;
                for i in 0..num_glyphs {
                    let offset_entry = header_offset + 8 + i * 2;
                    if offset_entry + 2 > data.len() {
                        break;
                    }

                    let sbit_offset = image_data_offset + read_u16(data, offset_entry) as usize;
                    let glyph_id = first_glyph + i as u16;

                    if let Some(glyph) = self.parse_glyph(data, sbit_offset, image_format) {
                        self.glyphs.insert(glyph_id, glyph);
                    }
                }
            }
            _ => {
                trace!("CBLC: unsupported index format {}", index_format);
            }
        }
    }

    fn parse_glyph(&self, data: &[u8], offset: usize, image_format: u16) -> Option<EmojiGlyph> {
        match image_format {
            17 => {
                // Format 17: small metrics, PNG data
                if offset + 9 > data.len() {
                    trace!("parse_glyph: format 17 offset {} out of bounds", offset);
                    return None;
                }

                let height = data[offset] as u32;
                let width = data[offset + 1] as u32;
                let data_len = read_u32(data, offset + 5) as usize;

                let png_offset = offset + 9;
                if png_offset + data_len > data.len() {
                    trace!("parse_glyph: format 17 PNG data out of bounds");
                    return None;
                }

                let png_data = &data[png_offset..png_offset + data_len];
                self.decode_png(png_data, width, height)
            }
            18 => {
                // Format 18: big metrics, PNG data
                if offset + 12 > data.len() {
                    trace!("parse_glyph: format 18 offset {} out of bounds", offset);
                    return None;
                }

                let height = data[offset] as u32;
                let width = data[offset + 1] as u32;
                let data_len = read_u32(data, offset + 8) as usize;

                let png_offset = offset + 12;
                if png_offset + data_len > data.len() {
                    trace!("parse_glyph: format 18 PNG data out of bounds");
                    return None;
                }

                let png_data = &data[png_offset..png_offset + data_len];
                self.decode_png(png_data, width, height)
            }
            19 => {
                // Format 19: metrics in CBLC, PNG data only
                if offset + 4 > data.len() {
                    trace!("parse_glyph: format 19 offset {} out of bounds", offset);
                    return None;
                }

                let data_len = read_u32(data, offset) as usize;
                let png_offset = offset + 4;
                if png_offset + data_len > data.len() {
                    trace!(
                        "parse_glyph: format 19 PNG data out of bounds (offset={}, len={})",
                        png_offset,
                        data_len
                    );
                    return None;
                }

                let png_data = &data[png_offset..png_offset + data_len];
                // Get size from PNG
                self.decode_png_auto_size(png_data)
            }
            _ => {
                warn!("CBDT: unsupported image format {}", image_format);
                None
            }
        }
    }

    fn decode_png(&self, png_data: &[u8], _width: u32, _height: u32) -> Option<EmojiGlyph> {
        self.decode_png_auto_size(png_data)
    }

    fn decode_png_auto_size(&self, png_data: &[u8]) -> Option<EmojiGlyph> {
        use image::io::Reader as ImageReader;
        use std::io::Cursor;

        let reader = match ImageReader::new(Cursor::new(png_data)).with_guessed_format() {
            Ok(r) => r,
            Err(e) => {
                trace!("decode_png: format guess failed: {}", e);
                return None;
            }
        };

        let img = match reader.decode() {
            Ok(i) => i,
            Err(e) => {
                trace!("decode_png: decode failed: {}", e);
                return None;
            }
        };

        let rgba = img.to_rgba8();
        let width = rgba.width();
        let height = rgba.height();

        Some(EmojiGlyph {
            width,
            height,
            data: rgba.into_raw(),
        })
    }

    /// Parse CPAL table (color palette)
    fn parse_cpal(&mut self, data: &[u8], offset: usize) {
        if offset + 12 > data.len() {
            return;
        }

        let version = read_u16(data, offset);
        let num_palette_entries = read_u16(data, offset + 2) as usize;
        let num_palettes = read_u16(data, offset + 4) as usize;
        let num_color_records = read_u16(data, offset + 6) as usize;
        let color_records_offset = offset + read_u32(data, offset + 8) as usize;

        info!(
            "CPAL: version={}, {} palette entries, {} palettes, {} colors",
            version, num_palette_entries, num_palettes, num_color_records
        );

        // Read colors from the first palette
        // Each color record is 4 bytes BGRA
        for i in 0..num_palette_entries.min(num_color_records) {
            let color_offset = color_records_offset + i * 4;
            if color_offset + 4 > data.len() {
                break;
            }
            let b = data[color_offset];
            let g = data[color_offset + 1];
            let r = data[color_offset + 2];
            let a = data[color_offset + 3];
            self.cpal_colors.push((r, g, b, a));
        }

        trace!("CPAL: loaded {} colors", self.cpal_colors.len());
    }

    /// Parse COLR table (color layers)
    fn parse_colr(&mut self, data: &[u8], offset: usize) {
        if offset + 14 > data.len() {
            return;
        }

        let version = read_u16(data, offset);
        let num_base_glyphs = read_u16(data, offset + 2) as usize;
        let base_glyph_offset = offset + read_u32(data, offset + 4) as usize;
        let layer_records_offset = offset + read_u32(data, offset + 8) as usize;
        let num_layer_records = read_u16(data, offset + 12) as usize;

        info!(
            "COLR: version={}, {} base glyphs, {} layer records",
            version, num_base_glyphs, num_layer_records
        );

        // COLRv1 is more complex, only v0 is supported
        if version > 0 {
            warn!("COLR: version {} not supported (v0 only)", version);
        }

        // BaseGlyphRecord: glyph_id (u16), first_layer_idx (u16), num_layers (u16)
        for i in 0..num_base_glyphs {
            let record_offset = base_glyph_offset + i * 6;
            if record_offset + 6 > data.len() {
                break;
            }

            let glyph_id = read_u16(data, record_offset);
            let first_layer_idx = read_u16(data, record_offset + 2) as usize;
            let num_layers = read_u16(data, record_offset + 4) as usize;

            let mut layers = Vec::with_capacity(num_layers);

            // LayerRecord: glyph_id (u16), palette_index (u16)
            for j in 0..num_layers {
                let layer_offset = layer_records_offset + (first_layer_idx + j) * 4;
                if layer_offset + 4 > data.len() {
                    break;
                }

                let layer_glyph_id = read_u16(data, layer_offset);
                let palette_index = read_u16(data, layer_offset + 2);

                layers.push(ColrLayer {
                    glyph_id: layer_glyph_id,
                    palette_index,
                });
            }

            if !layers.is_empty() {
                self.colr_layers.insert(glyph_id, layers);
            }
        }

        trace!(
            "COLR: loaded {} base glyphs with layers",
            self.colr_layers.len()
        );
    }

    /// Rasterize COLR glyph (composite multiple layers)
    fn render_colr_glyph(&self, glyph_id: u16, size: f32) -> Option<EmojiGlyph> {
        let layers = self.colr_layers.get(&glyph_id)?;
        let font = self.fontdue_font.as_ref()?;

        if layers.is_empty() {
            return None;
        }

        // Determine size from the first layer
        let first_metrics = font.metrics_indexed(layers[0].glyph_id as u16, size);
        if first_metrics.width == 0 || first_metrics.height == 0 {
            return None;
        }

        // Calculate bounding box for all layers
        let mut min_x = i32::MAX;
        let mut min_y = i32::MAX;
        let mut max_x = i32::MIN;
        let mut max_y = i32::MIN;

        for layer in layers {
            let metrics = font.metrics_indexed(layer.glyph_id as u16, size);
            let left = metrics.xmin;
            let top = -(metrics.ymin + metrics.height as i32);
            let right = left + metrics.width as i32;
            let bottom = top + metrics.height as i32;

            min_x = min_x.min(left);
            min_y = min_y.min(top);
            max_x = max_x.max(right);
            max_y = max_y.max(bottom);
        }

        let width = (max_x - min_x).max(1) as u32;
        let height = (max_y - min_y).max(1) as u32;

        // Initialize RGBA buffer (transparent)
        let mut rgba = vec![0u8; (width * height * 4) as usize];

        // Draw each layer from bottom to top
        for layer in layers {
            // rasterize_indexed returns (Metrics, Vec<u8>)
            let (metrics, coverage) = font.rasterize_indexed(layer.glyph_id as u16, size);

            // Get color from palette
            let (r, g, b, a) = if (layer.palette_index as usize) < self.cpal_colors.len() {
                self.cpal_colors[layer.palette_index as usize]
            } else {
                (0, 0, 0, 255) // Default: black
            };

            // Calculate layer offset
            let layer_x = metrics.xmin - min_x;
            let layer_y = -(metrics.ymin + metrics.height as i32) - min_y;

            // Composite with alpha blending
            for py in 0..metrics.height {
                for px in 0..metrics.width {
                    let cov_idx = py * metrics.width + px;
                    let cov = coverage.get(cov_idx).copied().unwrap_or(0);
                    if cov == 0 {
                        continue;
                    }

                    let dst_x = layer_x + px as i32;
                    let dst_y = layer_y + py as i32;

                    if dst_x < 0 || dst_y < 0 || dst_x >= width as i32 || dst_y >= height as i32 {
                        continue;
                    }

                    let dst_idx = ((dst_y as u32 * width + dst_x as u32) * 4) as usize;
                    if dst_idx + 3 >= rgba.len() {
                        continue;
                    }

                    // Source alpha = coverage * palette_alpha / 255
                    let src_alpha = (cov as u32 * a as u32 / 255) as u8;
                    if src_alpha == 0 {
                        continue;
                    }

                    // Alpha blend: dst = src * src_a + dst * (1 - src_a)
                    let dst_a = rgba[dst_idx + 3];
                    let inv_src_a = 255 - src_alpha;

                    rgba[dst_idx] = ((r as u32 * src_alpha as u32
                        + rgba[dst_idx] as u32 * inv_src_a as u32)
                        / 255) as u8;
                    rgba[dst_idx + 1] = ((g as u32 * src_alpha as u32
                        + rgba[dst_idx + 1] as u32 * inv_src_a as u32)
                        / 255) as u8;
                    rgba[dst_idx + 2] = ((b as u32 * src_alpha as u32
                        + rgba[dst_idx + 2] as u32 * inv_src_a as u32)
                        / 255) as u8;
                    rgba[dst_idx + 3] =
                        (src_alpha as u32 + dst_a as u32 * inv_src_a as u32 / 255) as u8;
                }
            }
        }

        Some(EmojiGlyph {
            width,
            height,
            data: rgba,
        })
    }

    /// Get emoji bitmap by codepoint
    pub fn get_glyph(&self, codepoint: u32) -> Option<&EmojiGlyph> {
        let glyph_id = self.cmap.get(&codepoint)?;
        self.glyphs.get(glyph_id)
    }

    /// Get emoji bitmap by glyph ID (from CBDT)
    pub fn get_glyph_by_id(&self, glyph_id: u16) -> Option<&EmojiGlyph> {
        self.glyphs.get(&glyph_id)
    }

    /// Check if COLR glyph exists
    pub fn has_colr_glyph(&self, glyph_id: u16) -> bool {
        self.colr_layers.contains_key(&glyph_id)
    }

    /// Dynamically render and get COLR glyph
    pub fn get_colr_glyph(&self, glyph_id: u16, size: f32) -> Option<EmojiGlyph> {
        self.render_colr_glyph(glyph_id, size)
    }

    /// Shape grapheme and get glyph ID (flag/ZWJ support)
    pub fn shape_grapheme(&self, grapheme: &str) -> Option<u16> {
        let face = self.face.as_ref()?;

        let mut buffer = rustybuzz::UnicodeBuffer::new();
        buffer.push_str(grapheme);

        let glyph_buffer = rustybuzz::shape(face, &[], buffer);
        let infos = glyph_buffer.glyph_infos();

        // Debug: output shaping result
        info!(
            "shape_grapheme: {:?} -> {} glyphs: {:?}",
            grapheme,
            infos.len(),
            infos.iter().map(|i| i.glyph_id).collect::<Vec<_>>()
        );

        // When ligature is applied, it becomes one glyph
        if infos.len() == 1 {
            let glyph_id = infos[0].glyph_id as u16;
            let in_cbdt = self.glyphs.contains_key(&glyph_id);
            let in_colr = self.colr_layers.contains_key(&glyph_id);
            info!(
                "  glyph_id={}, in_cbdt={}, in_colr={}",
                glyph_id, in_cbdt, in_colr
            );
            // Return if exists in CBDT or COLR
            if in_cbdt || in_colr {
                return Some(glyph_id);
            }
        }

        None
    }

    /// Number of loaded glyphs
    #[allow(dead_code)]
    pub fn glyph_count(&self) -> usize {
        self.glyphs.len()
    }

    /// Get glyph ID from codepoint
    pub fn codepoint_to_glyph_id(&self, codepoint: u32) -> Option<u16> {
        self.cmap.get(&codepoint).copied()
    }
}

// Helper functions
fn read_u16(data: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes([data[offset], data[offset + 1]])
}

fn read_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

/// Emoji atlas glyph information
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct EmojiGlyphInfo {
    /// Top-left U coordinate on texture (0.0-1.0)
    pub uv_x: f32,
    /// Top-left V coordinate on texture (0.0-1.0)
    pub uv_y: f32,
    /// Width on texture (0.0-1.0)
    pub uv_w: f32,
    /// Height on texture (0.0-1.0)
    pub uv_h: f32,
    /// Glyph pixel width
    pub width: u32,
    /// Glyph pixel height
    pub height: u32,
}

/// RGBA texture atlas for emoji
pub struct EmojiAtlas {
    /// OpenGL texture
    texture: Option<glow::Texture>,
    /// Codepoint -> glyph info map
    glyphs: HashMap<u32, EmojiGlyphInfo>,
    /// Texture width
    pub width: u32,
    /// Texture height
    pub height: u32,
    /// CPU-side texture data (RGBA)
    data: Vec<u8>,
    /// Shelf packing: current X position
    cursor_x: u32,
    /// Shelf packing: current Y position
    cursor_y: u32,
    /// Shelf packing: max height of current row
    row_height: u32,
    /// Emoji loader
    loader: Option<EmojiLoader>,
    /// GPU re-upload flag
    dirty: bool,
}

impl EmojiAtlas {
    /// Create a new emoji atlas
    pub fn new(emoji_font_path: Option<&str>, target_size: u32) -> Self {
        let width = 2048u32;
        let height = 2048u32;
        let data = vec![0u8; (width * height * 4) as usize];

        // Candidate paths for emoji fonts
        let emoji_font_paths = [
            emoji_font_path.unwrap_or(""),
            "/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf",
            "/usr/share/fonts/noto-emoji/NotoColorEmoji.ttf",
            "/usr/share/fonts/google-noto-emoji/NotoColorEmoji.ttf",
            "/usr/share/fonts/truetype/ancient-scripts/Symbola_hint.ttf",
            "/usr/share/fonts/TTF/NotoColorEmoji.ttf",
        ];

        let mut loader = None;
        for path in &emoji_font_paths {
            if path.is_empty() {
                continue;
            }
            if !Path::new(path).exists() {
                trace!("EmojiAtlas: font not found: {}", path);
                continue;
            }
            info!("EmojiAtlas: trying font: {}", path);
            if let Some(l) = EmojiLoader::load(path, target_size) {
                info!("EmojiAtlas: emoji font loaded: {}", path);
                loader = Some(l);
                break;
            }
        }

        if loader.is_none() {
            warn!("EmojiAtlas: no available emoji font found");
        }

        Self {
            texture: None,
            glyphs: HashMap::new(),
            width,
            height,
            data,
            cursor_x: 0,
            cursor_y: 0,
            row_height: 0,
            loader,
            dirty: false,
        }
    }

    /// Get emoji glyph info for a character (rasterize and add to atlas if not present)
    pub fn ensure_glyph(&mut self, c: char, cell_height: u32) -> Option<EmojiGlyphInfo> {
        let codepoint = c as u32;

        // Return if already in atlas
        if let Some(info) = self.glyphs.get(&codepoint) {
            return Some(*info);
        }

        let loader = self.loader.as_ref()?;

        // First try CBDT (bitmap)
        if let Some(glyph) = loader.get_glyph(codepoint) {
            let glyph_data = EmojiGlyph {
                width: glyph.width,
                height: glyph.height,
                data: glyph.data.clone(),
            };
            return self.add_glyph_data(codepoint, &glyph_data, cell_height);
        }

        // If not in CBDT, try COLR (vector)
        if let Some(glyph_id) = loader.codepoint_to_glyph_id(codepoint) {
            if loader.has_colr_glyph(glyph_id) {
                let size = cell_height as f32 * 2.0; // 2x supersampling
                if let Some(glyph_data) = loader.get_colr_glyph(glyph_id, size) {
                    return self.add_glyph_data(codepoint, &glyph_data, cell_height);
                }
            }
        }

        None
    }

    /// Get emoji for grapheme cluster (flags, ZWJ sequences, etc.)
    pub fn ensure_grapheme(
        &mut self,
        _gl: &glow::Context,
        grapheme: &str,
        cell_height: u32,
    ) -> Option<EmojiGlyphInfo> {
        // Debug: log call
        trace!(
            "ensure_grapheme called: {:?} (len={})",
            grapheme,
            grapheme.chars().count()
        );

        // Use grapheme hash as key
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        grapheme.hash(&mut hasher);
        let grapheme_key = hasher.finish() as u32;

        // Return if already in atlas
        if let Some(info) = self.glyphs.get(&grapheme_key) {
            trace!("  -> cached");
            return Some(*info);
        }

        // Try to get from loader
        let loader = self.loader.as_ref()?;

        // If single character, get directly
        let chars: Vec<char> = grapheme.chars().collect();
        if chars.len() == 1 {
            return self.ensure_glyph(chars[0], cell_height);
        }

        // Try GSUB shaping (flag/ZWJ sequence support)
        trace!("  trying GSUB shaping...");
        if let Some(glyph_id) = loader.shape_grapheme(grapheme) {
            // First try CBDT (bitmap)
            if let Some(glyph) = loader.get_glyph_by_id(glyph_id) {
                let glyph_data = EmojiGlyph {
                    width: glyph.width,
                    height: glyph.height,
                    data: glyph.data.clone(),
                };
                trace!(
                    "GSUB shaping succeeded (CBDT): {:?} -> glyph_id={}",
                    grapheme,
                    glyph_id
                );
                return self.add_glyph_data(grapheme_key, &glyph_data, cell_height);
            }
            // If not in CBDT, try COLR (vector)
            if loader.has_colr_glyph(glyph_id) {
                let size = cell_height as f32 * 2.0; // 2x supersampling
                if let Some(glyph_data) = loader.get_colr_glyph(glyph_id, size) {
                    trace!(
                        "GSUB shaping succeeded (COLR): {:?} -> glyph_id={}",
                        grapheme,
                        glyph_id
                    );
                    return self.add_glyph_data(grapheme_key, &glyph_data, cell_height);
                }
            }
        }

        // Fallback when GSUB shaping fails
        trace!("GSUB shaping failed, falling back: {:?}", grapheme);

        // Handle ZWJ sequence
        if grapheme.contains('\u{200D}') {
            // Sequence containing ZWJ (Zero Width Joiner)
            trace!(
                "ZWJ sequence detected: {:?}, falling back without GSUB",
                grapheme
            );
        }

        // Fallback: use the first emoji character
        for c in &chars {
            if is_emoji(*c) {
                let cp = *c as u32;
                // First try CBDT
                if let Some(glyph) = loader.get_glyph(cp) {
                    let glyph_data = EmojiGlyph {
                        width: glyph.width,
                        height: glyph.height,
                        data: glyph.data.clone(),
                    };
                    return self.add_glyph_data(grapheme_key, &glyph_data, cell_height);
                }
                // Try COLR
                if let Some(glyph_id) = loader.codepoint_to_glyph_id(cp) {
                    if loader.has_colr_glyph(glyph_id) {
                        let size = cell_height as f32 * 2.0;
                        if let Some(glyph_data) = loader.get_colr_glyph(glyph_id, size) {
                            return self.add_glyph_data(grapheme_key, &glyph_data, cell_height);
                        }
                    }
                }
            }
        }

        None
    }

    /// Add glyph data to atlas
    fn add_glyph_data(
        &mut self,
        codepoint: u32,
        glyph: &EmojiGlyph,
        cell_height: u32,
    ) -> Option<EmojiGlyphInfo> {
        self.add_glyph(codepoint, glyph, cell_height)
    }

    /// Add glyph to atlas (high-quality pre-scaling + 2x supersampling)
    fn add_glyph(
        &mut self,
        codepoint: u32,
        glyph: &EmojiGlyph,
        cell_height: u32,
    ) -> Option<EmojiGlyphInfo> {
        use image::{ImageBuffer, Rgba};

        // 2x supersampling: rasterize at 2x logical size
        const EMOJI_RENDER_SCALE: u32 = 2;
        let render_h = cell_height * EMOJI_RENDER_SCALE;
        let render_w = cell_height * EMOJI_RENDER_SCALE; // Keep square aspect ratio

        // Create ImageBuffer from source image
        let src_img: ImageBuffer<Rgba<u8>, Vec<u8>> =
            ImageBuffer::from_raw(glyph.width, glyph.height, glyph.data.clone())?;

        // High-quality resize (Lanczos3 - sharp and high quality)
        let resized = image::imageops::resize(
            &src_img,
            render_w,
            render_h,
            image::imageops::FilterType::Lanczos3,
        );

        let w = render_w;
        let h = render_h;
        let scaled_data = resized.into_raw();

        // Check if atlas has space
        if self.cursor_x + w > self.width {
            // Move to next row
            self.cursor_x = 0;
            self.cursor_y += self.row_height;
            self.row_height = 0;
        }

        if self.cursor_y + h > self.height {
            warn!("EmojiAtlas: no space available");
            return None;
        }

        // Copy bitmap to atlas
        let x = self.cursor_x;
        let y = self.cursor_y;

        for row in 0..h {
            for col in 0..w {
                let src_idx = ((row * w + col) * 4) as usize;
                let dst_idx = (((y + row) * self.width + (x + col)) * 4) as usize;

                if src_idx + 3 < scaled_data.len() && dst_idx + 3 < self.data.len() {
                    self.data[dst_idx] = scaled_data[src_idx]; // R
                    self.data[dst_idx + 1] = scaled_data[src_idx + 1]; // G
                    self.data[dst_idx + 2] = scaled_data[src_idx + 2]; // B
                    self.data[dst_idx + 3] = scaled_data[src_idx + 3]; // A
                }
            }
        }

        // Register glyph info (width/height is logical size = drawing size)
        let info = EmojiGlyphInfo {
            uv_x: x as f32 / self.width as f32,
            uv_y: y as f32 / self.height as f32,
            uv_w: w as f32 / self.width as f32,
            uv_h: h as f32 / self.height as f32,
            width: cell_height,  // Logical size (rendering size)
            height: cell_height, // Logical size (rendering size)
        };

        self.glyphs.insert(codepoint, info);

        // Advance cursor
        self.cursor_x += w;
        self.row_height = self.row_height.max(h);
        self.dirty = true;

        Some(info)
    }

    /// Whether emoji is available
    pub fn is_available(&self) -> bool {
        self.loader.is_some()
    }

    /// Get emoji glyph info
    #[allow(dead_code)]
    pub fn get_glyph(&self, c: char) -> Option<&EmojiGlyphInfo> {
        self.glyphs.get(&(c as u32))
    }

    /// Upload texture to GPU
    pub fn upload(&mut self, gl: &glow::Context) {
        use glow::HasContext;

        unsafe {
            // Create texture if not exists
            if self.texture.is_none() {
                if let Ok(tex) = gl.create_texture() {
                    gl.bind_texture(glow::TEXTURE_2D, Some(tex));

                    // LINEAR filter for sharp emoji
                    // (MIPMAP is too soft)
                    gl.tex_parameter_i32(
                        glow::TEXTURE_2D,
                        glow::TEXTURE_MIN_FILTER,
                        glow::LINEAR as i32,
                    );
                    gl.tex_parameter_i32(
                        glow::TEXTURE_2D,
                        glow::TEXTURE_MAG_FILTER,
                        glow::LINEAR as i32,
                    );
                    gl.tex_parameter_i32(
                        glow::TEXTURE_2D,
                        glow::TEXTURE_WRAP_S,
                        glow::CLAMP_TO_EDGE as i32,
                    );
                    gl.tex_parameter_i32(
                        glow::TEXTURE_2D,
                        glow::TEXTURE_WRAP_T,
                        glow::CLAMP_TO_EDGE as i32,
                    );

                    self.texture = Some(tex);
                }
            }

            if let Some(tex) = self.texture {
                gl.bind_texture(glow::TEXTURE_2D, Some(tex));

                // SRGB8_ALPHA8: GPU automatically converts sRGB -> linear
                // (no need for manual conversion in shader)
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::SRGB8_ALPHA8 as i32,
                    self.width as i32,
                    self.height as i32,
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    Some(&self.data),
                );

                gl.bind_texture(glow::TEXTURE_2D, None);
            }
        }

        self.dirty = false;
    }

    /// Upload to GPU if dirty
    pub fn upload_if_dirty(&mut self, gl: &glow::Context) {
        if self.dirty {
            self.upload(gl);
        }
    }

    /// Get texture
    pub fn texture(&self) -> Option<glow::Texture> {
        self.texture
    }

    /// Release resources
    pub fn destroy(&self, gl: &glow::Context) {
        use glow::HasContext;

        if let Some(tex) = self.texture {
            unsafe {
                gl.delete_texture(tex);
            }
        }
    }

    /// Mark texture as needing re-upload (call after GPU state loss, e.g., suspend/resume)
    pub fn invalidate(&mut self) {
        self.dirty = true;
    }

    /// Clear cache on font size change
    pub fn resize(&mut self, _new_cell_height: u32) {
        // Clear cached glyphs
        self.glyphs.clear();
        // Clear atlas data
        self.data.fill(0);
        // Reset packing cursor
        self.cursor_x = 0;
        self.cursor_y = 0;
        self.row_height = 0;
        self.dirty = true;
        info!("EmojiAtlas: cache cleared due to font size change");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_emoji() {
        assert!(is_emoji('😀'));
        assert!(is_emoji('🎉'));
        assert!(is_emoji('❤'));
        assert!(!is_emoji('A'));
        assert!(!is_emoji('あ'));
    }
}
