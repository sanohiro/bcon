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
    /// Reusable buffer: result list
    result_buf: Vec<(usize, ShapedGlyph)>,
    /// Reusable buffer: segment characters
    segment_chars_buf: Vec<char>,
    /// Reusable buffer: segment column positions
    segment_cols_buf: Vec<usize>,
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
            result_buf: Vec::with_capacity(256),
            segment_chars_buf: Vec::with_capacity(128),
            segment_cols_buf: Vec::with_capacity(128),
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

        // Clear and reuse internal buffers
        self.result_buf.clear();
        self.segment_chars_buf.clear();
        self.segment_cols_buf.clear();

        let mut col = 0;
        while col < cols {
            let cell = grid.cell(row, col);

            // Skip continuation cells (width=0), space, NUL
            if cell.width == 0 {
                // Flush if segment exists
                self.flush_segment_internal();
                col += 1;
                continue;
            }

            let ch = cell.ch();

            if ch == ' ' || ch == '\0' {
                // Flush if segment exists
                self.flush_segment_internal();
                col += 1;
                continue;
            }

            // Wide character (width=2) -> pass through without shaping
            if cell.width == 2 {
                self.flush_segment_internal();
                self.result_buf.push((
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
                self.flush_segment_internal();
                self.result_buf.push((
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
            self.segment_chars_buf.push(ch);
            self.segment_cols_buf.push(col);
            col += 1;
        }

        // Flush remaining segment at end of line
        self.flush_segment_internal();

        // Return ownership of result (buffer will be reused on next call)
        std::mem::take(&mut self.result_buf)
    }

    /// Shape collected consecutive segment using internal buffers
    fn flush_segment_internal(&mut self) {
        if self.segment_chars_buf.is_empty() {
            return;
        }

        let mut buffer = rustybuzz::UnicodeBuffer::new();

        // Add each character with cluster=column number
        for (i, &ch) in self.segment_chars_buf.iter().enumerate() {
            buffer.add(ch, self.segment_cols_buf[i] as u32);
        }

        // Execute shaping
        let glyph_buffer = rustybuzz::shape(&self.face_main, &self.features, buffer);

        let infos = glyph_buffer.glyph_infos();

        for (i, info) in infos.iter().enumerate() {
            let glyph_id = info.glyph_id as u16;
            let cluster = info.cluster;

            // Calculate cell_span: difference from next glyph's cluster
            let next_cluster = if i + 1 < infos.len() {
                infos[i + 1].cluster
            } else {
                // Last glyph: columns to segment end
                (self.segment_cols_buf.last().unwrap() + 1) as u32
            };

            let cell_span = (next_cluster - cluster).max(1) as u8;

            // Identify original character (cluster is column number)
            let ch = self
                .segment_chars_buf
                .iter()
                .zip(self.segment_cols_buf.iter())
                .find(|(_, &c)| c as u32 == cluster)
                .map(|(&ch, _)| ch)
                .unwrap_or(' ');

            self.result_buf.push((
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

        self.segment_chars_buf.clear();
        self.segment_cols_buf.clear();
    }
}
