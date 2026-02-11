//! HDR (High Dynamic Range) detection and metadata
//!
//! Parses EDID for HDR capabilities and provides HDR metadata structures
//! for DRM HDR_OUTPUT_METADATA property.

#![allow(dead_code)]

use log::{debug, info, trace};

/// HDR capabilities detected from display EDID
#[derive(Debug, Clone, Default)]
pub struct HdrCapabilities {
    /// Display supports HDR10 (SMPTE ST.2084 PQ)
    pub hdr10: bool,
    /// Display supports HLG (Hybrid Log-Gamma)
    pub hlg: bool,
    /// Display supports HDR10+ (dynamic metadata)
    pub hdr10_plus: bool,
    /// Display supports Rec. 2020 wide color gamut
    pub rec2020: bool,
    /// Display supports DCI-P3 color space
    pub dci_p3: bool,
    /// Maximum content light level (nits), if reported
    pub max_cll: Option<u16>,
    /// Maximum frame-average light level (nits), if reported
    pub max_fall: Option<u16>,
    /// Maximum luminance (cd/m2), if reported
    pub max_luminance: Option<u16>,
    /// Minimum luminance (0.0001 cd/m2 units), if reported
    pub min_luminance: Option<u16>,
}

impl HdrCapabilities {
    /// Returns true if any HDR format is supported
    pub fn supports_hdr(&self) -> bool {
        self.hdr10 || self.hlg || self.hdr10_plus
    }

    /// Returns true if wide color gamut is supported
    pub fn supports_wide_gamut(&self) -> bool {
        self.rec2020 || self.dci_p3
    }

    /// Log detected capabilities
    pub fn log(&self) {
        if self.supports_hdr() {
            info!(
                "HDR supported: HDR10={}, HLG={}, HDR10+={}, Rec2020={}, DCI-P3={}",
                self.hdr10, self.hlg, self.hdr10_plus, self.rec2020, self.dci_p3
            );
            if let Some(cll) = self.max_cll {
                info!("  Max content light level: {} nits", cll);
            }
            if let Some(fall) = self.max_fall {
                info!("  Max frame-average light level: {} nits", fall);
            }
        } else {
            debug!("HDR not supported by display");
        }
    }
}

/// Parse HDR capabilities from EDID data
///
/// EDID structure:
/// - Base EDID: 128 bytes
/// - Extension blocks: 128 bytes each (count at offset 126)
/// - CEA-861 extension (tag 0x02 or 0x03) contains HDR metadata
pub fn parse_edid_hdr(edid: &[u8]) -> HdrCapabilities {
    let mut caps = HdrCapabilities::default();

    // Minimum EDID size check
    if edid.len() < 128 {
        debug!("EDID too short: {} bytes", edid.len());
        return caps;
    }

    // Verify EDID header (bytes 0-7)
    const EDID_HEADER: [u8; 8] = [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];
    if edid[0..8] != EDID_HEADER {
        debug!("Invalid EDID header");
        return caps;
    }

    // Extension block count at offset 126
    let extension_count = edid[126] as usize;
    trace!("EDID extension blocks: {}", extension_count);

    // Parse each extension block
    for ext_idx in 0..extension_count {
        let offset = 128 + ext_idx * 128;
        if offset + 128 > edid.len() {
            break;
        }

        let block = &edid[offset..offset + 128];

        // CEA-861 extension tag
        if block[0] == 0x02 || block[0] == 0x03 {
            parse_cea_extension(block, &mut caps);
        }
    }

    caps
}

/// Parse CEA-861 extension block for HDR data
fn parse_cea_extension(block: &[u8], caps: &mut HdrCapabilities) {
    // Byte 2: DTD begin offset (data blocks end before this)
    let dtd_offset = block[2] as usize;
    if dtd_offset < 4 || dtd_offset > 127 {
        return;
    }

    // Parse data block collection (starts at byte 4)
    let mut offset = 4;
    while offset < dtd_offset {
        let header = block[offset];
        let tag = (header >> 5) & 0x07;
        let length = (header & 0x1F) as usize;
        offset += 1;

        if offset + length > dtd_offset {
            break;
        }

        let data = &block[offset..offset + length];

        match tag {
            0x07 => {
                // Extended tag (first byte is extended tag code)
                if !data.is_empty() {
                    parse_extended_data_block(data, caps);
                }
            }
            _ => {
                // Other tags: Video, Audio, Speaker, Vendor-Specific, etc.
                trace!("CEA data block: tag={}, len={}", tag, length);
            }
        }

        offset += length;
    }
}

/// Parse extended data block (tag 0x07)
fn parse_extended_data_block(data: &[u8], caps: &mut HdrCapabilities) {
    if data.is_empty() {
        return;
    }

    let extended_tag = data[0];
    let payload = &data[1..];

    match extended_tag {
        0x06 => {
            // HDR Static Metadata Data Block
            parse_hdr_static_metadata(payload, caps);
        }
        0x05 => {
            // Colorimetry Data Block
            parse_colorimetry(payload, caps);
        }
        _ => {
            trace!("Extended data block: tag={:#04x}, len={}", extended_tag, payload.len());
        }
    }
}

/// Parse HDR Static Metadata Data Block
fn parse_hdr_static_metadata(data: &[u8], caps: &mut HdrCapabilities) {
    if data.is_empty() {
        return;
    }

    // Byte 0: Supported EOTF (Electro-Optical Transfer Function)
    let eotf = data[0];
    // Bit 0: Traditional gamma - SDR luminance range
    // Bit 1: Traditional gamma - HDR luminance range
    // Bit 2: SMPTE ST 2084 (PQ) - HDR10
    // Bit 3: Hybrid Log-Gamma (HLG)
    caps.hdr10 = (eotf & 0x04) != 0;
    caps.hlg = (eotf & 0x08) != 0;

    debug!(
        "HDR EOTF support: SDR={}, HDR_gamma={}, PQ/HDR10={}, HLG={}",
        (eotf & 0x01) != 0,
        (eotf & 0x02) != 0,
        caps.hdr10,
        caps.hlg
    );

    // Byte 1: Static metadata descriptor support
    if data.len() > 1 {
        let descriptors = data[1];
        // Bit 0: Static Metadata Type 1
        caps.hdr10_plus = (descriptors & 0x02) != 0 || (descriptors & 0x04) != 0;
    }

    // Bytes 2+: Luminance data (if present)
    if data.len() > 2 {
        // Max luminance (cv = 50 * 2^(cv/32))
        let max_lum_cv = data[2];
        if max_lum_cv > 0 {
            // Approximate calculation
            let max_lum = 50.0 * 2.0_f32.powf(max_lum_cv as f32 / 32.0);
            caps.max_luminance = Some(max_lum as u16);
        }
    }

    if data.len() > 3 {
        // Max frame-average luminance
        let max_fall_cv = data[3];
        if max_fall_cv > 0 {
            let max_fall = 50.0 * 2.0_f32.powf(max_fall_cv as f32 / 32.0);
            caps.max_fall = Some(max_fall as u16);
        }
    }

    if data.len() > 4 {
        // Min luminance (in 0.0001 cd/m2 units)
        caps.min_luminance = Some(data[4] as u16);
    }
}

/// Parse Colorimetry Data Block
fn parse_colorimetry(data: &[u8], caps: &mut HdrCapabilities) {
    if data.is_empty() {
        return;
    }

    // Byte 0: Colorimetry support flags
    let flags = data[0];
    // Bit 5: BT2020_cYCC
    // Bit 6: BT2020_YCC
    // Bit 7: BT2020_RGB
    caps.rec2020 = (flags & 0x80) != 0 || (flags & 0x40) != 0 || (flags & 0x20) != 0;

    // Byte 1 (if present): Extended colorimetry
    if data.len() > 1 {
        let ext_flags = data[1];
        // Bit 7: DCI-P3
        caps.dci_p3 = (ext_flags & 0x80) != 0;
    }

    debug!(
        "Colorimetry: Rec2020={}, DCI-P3={}",
        caps.rec2020, caps.dci_p3
    );
}

/// HDR output metadata for DRM (matches kernel struct hdr_output_metadata)
#[repr(C)]
#[derive(Debug, Clone, Default)]
pub struct HdrOutputMetadata {
    /// Metadata type (0 = HDMI_STATIC_METADATA_TYPE1)
    pub metadata_type: u32,
    /// HDMI metadata type 1
    pub hdmi_type1: HdmiMetadataType1,
}

/// HDMI Static Metadata Type 1 (for HDR10)
#[repr(C)]
#[derive(Debug, Clone, Default)]
pub struct HdmiMetadataType1 {
    /// EOTF (0=SDR, 2=PQ, 3=HLG)
    pub eotf: u8,
    /// Static Metadata Descriptor ID
    pub metadata_type: u8,
    /// Display primaries (x0, y0, x1, y1, x2, y2) in 0.00002 units
    pub display_primaries: [[u16; 2]; 3],
    /// White point (x, y) in 0.00002 units
    pub white_point: [u16; 2],
    /// Max display mastering luminance (cd/m2)
    pub max_display_mastering_luminance: u16,
    /// Min display mastering luminance (0.0001 cd/m2)
    pub min_display_mastering_luminance: u16,
    /// Max content light level (nits)
    pub max_cll: u16,
    /// Max frame-average light level (nits)
    pub max_fall: u16,
}

impl HdrOutputMetadata {
    /// Create HDR10 metadata for typical content
    pub fn hdr10_default() -> Self {
        Self {
            metadata_type: 0, // HDMI_STATIC_METADATA_TYPE1
            hdmi_type1: HdmiMetadataType1 {
                eotf: 2, // SMPTE ST 2084 (PQ)
                metadata_type: 0,
                // Rec. 2020 primaries (in 0.00002 units)
                display_primaries: [
                    [35400, 14600], // Red
                    [8500, 39850],  // Green
                    [6550, 2300],   // Blue
                ],
                white_point: [15635, 16450], // D65
                max_display_mastering_luminance: 1000,
                min_display_mastering_luminance: 1,
                max_cll: 1000,
                max_fall: 400,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_edid() {
        let caps = parse_edid_hdr(&[]);
        assert!(!caps.supports_hdr());
    }

    #[test]
    fn test_short_edid() {
        let caps = parse_edid_hdr(&[0u8; 64]);
        assert!(!caps.supports_hdr());
    }

    #[test]
    fn test_basic_edid_no_extensions() {
        // Valid EDID header, no extensions
        let mut edid = vec![0u8; 128];
        edid[0..8].copy_from_slice(&[0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00]);
        edid[126] = 0; // No extensions

        let caps = parse_edid_hdr(&edid);
        assert!(!caps.supports_hdr());
    }
}
