//! Text shaping
//!
//! Uses rustybuzz for text shaping to detect
//! programming font ligatures (`==`, `->`, `!=`, etc.).
//!
//! Since terminals use fixed-width grids, shaping only affects
//! **glyph selection** (positions are fixed to cell grid).

use crate::font::atlas::GlyphKey;
use crate::terminal::grid::Grid;
use fontdue::Font;
use log::debug;
use std::str::FromStr;

/// Shaped glyph
#[allow(dead_code)]
pub struct ShapedGlyph {
    /// Atlas lookup key (glyph_id=0 means char-based fallback)
    pub key: GlyphKey,
    /// Column position in original text
    pub cluster: u32,
    /// Occupied cell count (ligature: 2+)
    pub cell_span: u8,
    /// Original character for fallback
    pub ch: char,
}

/// Text shaper
pub struct TextShaper {
    face_main: rustybuzz::Face<'static>,
    #[allow(dead_code)]
    face_cjk: Option<rustybuzz::Face<'static>>,
    features: Vec<rustybuzz::Feature>,
}

impl TextShaper {
    /// Create shaper from font data
    ///
    /// Font data requires `'static` lifetime (allocated with `Box::leak`)
    pub fn new(font_data: &'static [u8], cjk_font_data: Option<&'static [u8]>) -> Option<Self> {
        let face_main = rustybuzz::Face::from_slice(font_data, 0)?;

        let face_cjk = cjk_font_data.and_then(|data| rustybuzz::Face::from_slice(data, 0));

        // Enable OTF features: calt, liga, clig
        let features = ["calt", "liga", "clig"]
            .iter()
            .filter_map(|s| rustybuzz::Feature::from_str(s).ok())
            .collect();

        debug!("TextShaper initialized");

        Some(Self {
            face_main,
            face_cjk,
            features,
        })
    }

    /// Shape one line
    ///
    /// Scans cells in line and shapes consecutive half-width character segments
    /// that exist in main font using rustybuzz. CJK/wide characters pass through without shaping.
    ///
    /// Returns: List of (column position, ShapedGlyph)
    pub fn shape_line(
        &mut self,
        grid: &Grid,
        row: usize,
        font_main: &Font,
    ) -> Vec<(usize, ShapedGlyph)> {
        let cols = grid.cols();
        let mut result = Vec::new();

        // For collecting consecutive segments
        let mut segment_chars: Vec<char> = Vec::new();
        let mut segment_cols: Vec<usize> = Vec::new();

        let mut col = 0;
        while col < cols {
            let cell = grid.cell(row, col);

            // Skip continuation cells (width=0), space, NUL
            if cell.width == 0 {
                // Flush if segment exists
                self.flush_segment(&mut segment_chars, &mut segment_cols, &mut result);
                col += 1;
                continue;
            }

            let ch = cell.ch();

            if ch == ' ' || ch == '\0' {
                // Flush if segment exists
                self.flush_segment(&mut segment_chars, &mut segment_cols, &mut result);
                col += 1;
                continue;
            }

            // Wide character (width=2) -> pass through without shaping
            if cell.width == 2 {
                self.flush_segment(&mut segment_chars, &mut segment_cols, &mut result);
                result.push((
                    col,
                    ShapedGlyph {
                        key: GlyphKey {
                            font_idx: 0,
                            glyph_id: 0,
                        },
                        cluster: col as u32,
                        cell_span: 2,
                        ch,
                    },
                ));
                col += 2; // Wide character occupies 2 cells
                continue;
            }

            // No glyph in main font -> pass through
            if font_main.lookup_glyph_index(ch) == 0 {
                self.flush_segment(&mut segment_chars, &mut segment_cols, &mut result);
                result.push((
                    col,
                    ShapedGlyph {
                        key: GlyphKey {
                            font_idx: 0,
                            glyph_id: 0,
                        },
                        cluster: col as u32,
                        cell_span: 1,
                        ch,
                    },
                ));
                col += 1;
                continue;
            }

            // Half-width character in main font -> add to segment
            segment_chars.push(ch);
            segment_cols.push(col);
            col += 1;
        }

        // Flush remaining segment at end of line
        self.flush_segment(&mut segment_chars, &mut segment_cols, &mut result);

        result
    }

    /// Shape collected consecutive segment and add to result
    fn flush_segment(
        &mut self,
        chars: &mut Vec<char>,
        cols: &mut Vec<usize>,
        result: &mut Vec<(usize, ShapedGlyph)>,
    ) {
        if chars.is_empty() {
            return;
        }

        let shaped = self.shape_segment(chars, cols);
        result.extend(shaped);

        chars.clear();
        cols.clear();
    }

    /// Shape consecutive half-width character segment
    fn shape_segment(&mut self, chars: &[char], cols: &[usize]) -> Vec<(usize, ShapedGlyph)> {
        let mut buffer = rustybuzz::UnicodeBuffer::new();

        // Add each character with cluster=column number
        for (i, &ch) in chars.iter().enumerate() {
            buffer.add(ch, cols[i] as u32);
        }

        // Execute shaping
        let glyph_buffer = rustybuzz::shape(&self.face_main, &self.features, buffer);

        let infos = glyph_buffer.glyph_infos();
        let mut result = Vec::with_capacity(infos.len());

        for (i, info) in infos.iter().enumerate() {
            let glyph_id = info.glyph_id as u16;
            let cluster = info.cluster;

            // Calculate cell_span: difference from next glyph's cluster
            let next_cluster = if i + 1 < infos.len() {
                infos[i + 1].cluster
            } else {
                // Last glyph: columns to segment end
                // Last element of cols + 1 is segment end
                (cols.last().unwrap() + 1) as u32
            };

            let cell_span = (next_cluster - cluster).max(1) as u8;

            // Identify original character (cluster is column number)
            let ch = chars
                .iter()
                .zip(cols.iter())
                .find(|(_, &c)| c as u32 == cluster)
                .map(|(&ch, _)| ch)
                .unwrap_or(' ');

            result.push((
                cluster as usize,
                ShapedGlyph {
                    key: GlyphKey {
                        font_idx: 0,
                        glyph_id,
                    },
                    cluster,
                    cell_span,
                    ch,
                },
            ));
        }

        result
    }
}
