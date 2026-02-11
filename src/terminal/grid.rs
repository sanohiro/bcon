//! Character grid
//!
//! 2D cell array that manages terminal screen state.
//! Provides cursor position, character attributes, erase and scroll operations.

use std::collections::VecDeque;
use std::sync::{Arc, OnceLock};

use bitflags::bitflags;
use log::trace;
use smol_str::SmolStr;
use unicode_normalization::UnicodeNormalization;
use unicode_width::UnicodeWidthChar;

/// Convert char to SmolStr efficiently
/// For ASCII (1 byte), uses inline storage directly
#[inline]
fn char_to_smolstr(ch: char) -> SmolStr {
    let mut buf = [0u8; 4];
    let s = ch.encode_utf8(&mut buf);
    SmolStr::new(s)
}

/// Cursor style (DECSCUSR)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CursorStyle {
    /// Block cursor (default)
    #[default]
    Block,
    /// Underline cursor
    Underline,
    /// Bar (vertical line) cursor
    Bar,
}

/// Hyperlink information (OSC 8)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hyperlink {
    /// Link ID (optional)
    pub id: Option<String>,
    /// URL
    pub url: String,
}

/// Maximum scrollback lines
const MAX_SCROLLBACK: usize = 10000;

/// Check if codepoint is emoji (for ZWJ sequences)
/// Broadly covers characters used as components of ZWJ sequences
fn is_emoji_codepoint(cp: u32) -> bool {
    matches!(cp,
        // Main emoji blocks
        0x1F300..=0x1F5FF |  // Miscellaneous Symbols and Pictographs
        0x1F600..=0x1F64F |  // Emoticons
        0x1F680..=0x1F6FF |  // Transport and Map Symbols
        0x1F900..=0x1F9FF |  // Supplemental Symbols and Pictographs
        0x1FA00..=0x1FAFF |  // Symbols and Pictographs Extended
        0x1F1E0..=0x1F1FF |  // Regional Indicator Symbols (flags)

        // Symbol blocks (used in ZWJ sequences)
        0x2600..=0x26FF   |  // Miscellaneous Symbols (♀♂⚕⚖ etc)
        0x2700..=0x27BF   |  // Dingbats (✈✂ etc)
        0x2300..=0x23FF   |  // Miscellaneous Technical (⌚⏰⏩ etc)

        // Additional emoji-related
        0x203C | 0x2049 |    // ‼ ⁉
        0x2122 | 0x2139 |    // ™ ℹ
        0x2194..=0x2199 |    // Arrows
        0x21A9..=0x21AA |    // ↩ ↪
        0x24C2 |             // Ⓜ
        0x25AA..=0x25AB |    // ▪ ▫
        0x25B6 | 0x25C0 |    // ▶ ◀
        0x25FB..=0x25FE |    // Squares
        0x2934..=0x2935 |    // ⤴ ⤵
        0x2B05..=0x2B07 |    // ⬅⬆⬇
        0x2B1B..=0x2B1C |    // ⬛⬜
        0x2B50 | 0x2B55 |    // ⭐⭕
        0x3030 | 0x303D |    // 〰 〽
        0x3297 | 0x3299      // ㊗ ㊙
    )
}

/// Image placement information
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ImagePlacement {
    /// Image ID (corresponds to ImageRegistry ID)
    pub id: u32,
    /// Placement row (grid coordinates)
    pub row: usize,
    /// Placement column (grid coordinates)
    pub col: usize,
    /// Occupied cell width
    pub width_cells: usize,
    /// Occupied cell height
    pub height_cells: usize,
    /// Image pixel width
    pub pixel_width: u32,
    /// Image pixel height
    pub pixel_height: u32,
}

/// Text color
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Color {
    /// Default color (foreground: white, background: black)
    Default,
    /// 256-color palette index
    Indexed(u8),
    /// True Color (24bit RGB)
    Rgb(u8, u8, u8),
}

impl Color {
    /// Convert to RGBA float array (for shaders)
    pub fn to_rgba(&self, is_foreground: bool) -> [f32; 4] {
        match self {
            Color::Default => {
                if is_foreground {
                    [1.0, 1.0, 1.0, 1.0] // white
                } else {
                    [0.0, 0.0, 0.0, 0.0] // transparent (no background)
                }
            }
            Color::Indexed(idx) => index_to_rgba(*idx),
            Color::Rgb(r, g, b) => [*r as f32 / 255.0, *g as f32 / 255.0, *b as f32 / 255.0, 1.0],
        }
    }
}

/// Pre-computed 256-color palette as RGBA (compile-time generated)
/// This avoids runtime computation for every color lookup.
const fn generate_palette() -> [[f32; 4]; 256] {
    let mut palette = [[0.0f32; 4]; 256];

    // Helper to convert u8 RGB to normalized f32 RGBA
    const fn rgb(r: u8, g: u8, b: u8) -> [f32; 4] {
        [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0]
    }

    // Helper for 6x6x6 color cube value
    const fn cube_val(v: u8) -> u8 {
        if v == 0 { 0 } else { 55 + 40 * v }
    }

    // Standard 16 colors (ANSI)
    palette[0] = rgb(0, 0, 0);          // black
    palette[1] = rgb(205, 0, 0);        // red
    palette[2] = rgb(0, 205, 0);        // green
    palette[3] = rgb(205, 205, 0);      // yellow
    palette[4] = rgb(0, 0, 238);        // blue
    palette[5] = rgb(205, 0, 205);      // magenta
    palette[6] = rgb(0, 205, 205);      // cyan
    palette[7] = rgb(229, 229, 229);    // white
    palette[8] = rgb(127, 127, 127);    // bright black
    palette[9] = rgb(255, 0, 0);        // bright red
    palette[10] = rgb(0, 255, 0);       // bright green
    palette[11] = rgb(255, 255, 0);     // bright yellow
    palette[12] = rgb(92, 92, 255);     // bright blue
    palette[13] = rgb(255, 0, 255);     // bright magenta
    palette[14] = rgb(0, 255, 255);     // bright cyan
    palette[15] = rgb(255, 255, 255);   // bright white

    // 216-color cube (16-231): 6x6x6 RGB values
    let mut i = 16usize;
    while i < 232 {
        let n = (i - 16) as u8;
        let b_val = n % 6;
        let g_val = (n / 6) % 6;
        let r_val = n / 36;
        palette[i] = rgb(cube_val(r_val), cube_val(g_val), cube_val(b_val));
        i += 1;
    }

    // Grayscale (232-255): 24 shades from dark to light
    let mut i = 232usize;
    while i < 256 {
        let v = (8 + 10 * (i - 232)) as u8;
        palette[i] = rgb(v, v, v);
        i += 1;
    }

    palette
}

static PALETTE_256: [[f32; 4]; 256] = generate_palette();

/// Convert 256-color palette index to RGBA (O(1) table lookup)
#[inline]
fn index_to_rgba(idx: u8) -> [f32; 4] {
    PALETTE_256[idx as usize]
}

/// Underline style (CSI 4:x m)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UnderlineStyle {
    #[default]
    None,
    /// Single line (CSI 4 m or CSI 4:1 m)
    Single,
    /// Double line (CSI 4:2 m)
    Double,
    /// Wavy line (CSI 4:3 m) - curly/wavy
    Curly,
    /// Dotted line (CSI 4:4 m)
    Dotted,
    /// Dashed line (CSI 4:5 m)
    Dashed,
}

bitflags! {
    /// Cell character attributes
    #[derive(Debug, Clone, Copy, PartialEq, Default)]
    pub struct CellAttrs: u16 {
        const BOLD      = 0b0000_0000_0001;
        const DIM       = 0b0000_0000_0010;
        const ITALIC    = 0b0000_0000_0100;
        const UNDERLINE = 0b0000_0000_1000;  // Backward compatible (used with UnderlineStyle)
        const BLINK     = 0b0000_0001_0000;
        const INVERSE   = 0b0000_0010_0000;
        const HIDDEN    = 0b0000_0100_0000;
        const STRIKE    = 0b0000_1000_0000;
        const OVERLINE  = 0b0001_0000_0000;  // Overline (CSI 53 m)
    }
}

/// Data for one cell
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Cell {
    /// Grapheme cluster (supports multiple codepoints for emoji ligatures, flags, etc.)
    /// Uses SmolStr for inline storage (no heap allocation for short strings up to 22 bytes)
    pub grapheme: SmolStr,
    pub fg: Color,
    pub bg: Color,
    pub attrs: CellAttrs,
    /// Character width: 1=half-width, 2=full-width(head), 0=full-width(continuation)
    pub width: u8,
    /// Hyperlink (OSC 8)
    pub hyperlink: Option<Arc<Hyperlink>>,
    /// Underline style (CSI 4:x m)
    pub underline_style: UnderlineStyle,
    /// Underline color (CSI 58;2;r;g;b m) - Uses foreground color if None
    pub underline_color: Option<Color>,
}

/// Static space character for default cells
static SPACE: SmolStr = SmolStr::new_inline(" ");

/// Static reference for empty cell
static EMPTY_CELL: OnceLock<Cell> = OnceLock::new();

impl Cell {
    /// Create empty cell
    pub fn empty() -> Cell {
        Cell {
            grapheme: SPACE.clone(),
            fg: Color::Default,
            bg: Color::Default,
            attrs: CellAttrs::empty(),
            width: 1,
            hyperlink: None,
            underline_style: UnderlineStyle::None,
            underline_color: None,
        }
    }

    /// Static reference to empty cell
    pub fn empty_ref() -> &'static Cell {
        EMPTY_CELL.get_or_init(|| Cell::empty())
    }

    /// Backward compatible: returns first character
    pub fn ch(&self) -> char {
        self.grapheme.chars().next().unwrap_or(' ')
    }
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            grapheme: SPACE.clone(),
            fg: Color::Default,
            bg: Color::Default,
            attrs: CellAttrs::empty(),
            width: 1,
            hyperlink: None,
            underline_style: UnderlineStyle::None,
            underline_color: None,
        }
    }
}

/// Pen state (current drawing attributes)
#[derive(Debug, Clone)]
struct Pen {
    fg: Color,
    bg: Color,
    attrs: CellAttrs,
    underline_style: UnderlineStyle,
    underline_color: Option<Color>,
}

impl Default for Pen {
    fn default() -> Self {
        Self {
            fg: Color::Default,
            bg: Color::Default,
            attrs: CellAttrs::empty(),
            underline_style: UnderlineStyle::None,
            underline_color: None,
        }
    }
}

// ========== Sub-structures for Grid ==========

/// Terminal mode flags (DECSET/DECRST)
#[derive(Debug, Clone, Default)]
pub struct TerminalModes {
    /// Cursor visibility flag (DECTCEM, ?25)
    pub cursor_visible: bool,
    /// Auto-wrap mode (DECAWM, ?7)
    pub auto_wrap: bool,
    /// Application cursor keys mode (DECCKM, ?1)
    pub application_cursor_keys: bool,
    /// Bracketed paste mode (?2004)
    pub bracketed_paste: bool,
    /// Mouse mode (?1000=X10, ?1002=button, ?1003=all events)
    pub mouse_mode: MouseMode,
    /// SGR mouse mode (?1006) - extended coordinate format
    pub mouse_sgr: bool,
    /// Focus event reporting flag (?1004)
    pub send_focus_events: bool,
    /// Synchronized Update mode (?2026)
    pub synchronized_update: bool,
}

impl TerminalModes {
    pub fn new() -> Self {
        Self {
            cursor_visible: true,
            auto_wrap: true,
            ..Default::default()
        }
    }
}

/// Cursor appearance state
#[derive(Debug, Clone, Default)]
pub struct CursorAppearance {
    /// Cursor style (DECSCUSR)
    pub style: CursorStyle,
    /// Cursor blink flag
    pub blink: bool,
}

/// Shell integration state (OSC 133)
#[derive(Debug, Clone, Default)]
pub struct ShellState {
    /// Prompt start row
    pub prompt_row: Option<usize>,
    /// Command execution start row
    pub command_row: Option<usize>,
    /// Last command exit code
    pub last_exit_code: Option<i32>,
}

/// Keyboard protocol state
#[derive(Debug, Clone, Default)]
pub struct KeyboardState {
    /// modifyOtherKeys level (0=disabled, 1=partial, 2=full)
    pub modify_other_keys: u8,
    /// Kitty keyboard protocol flags
    /// Bit 0: Report ambiguous keys in CSI u format
    /// Bit 1: Report event type (press/repeat/release)
    /// Bit 2: Report alternate keys
    /// Bit 3: Report all keys in CSI u format
    /// Bit 4: Report associated text
    pub kitty_flags: u32,
}

/// Dynamic colors (OSC 10/11)
#[derive(Debug, Clone, Default)]
pub struct DynamicColors {
    /// OSC 10 foreground color (RGB, None = use default)
    pub fg: Option<(u8, u8, u8)>,
    /// OSC 11 background color (RGB, None = use default)
    pub bg: Option<(u8, u8, u8)>,
}

/// Character grid
pub struct Grid {
    // ===== Core display state =====
    /// Cell array (row-major)
    cells: Vec<Cell>,
    /// Number of columns
    cols: usize,
    /// Number of rows
    rows: usize,
    /// Cursor row (0-indexed)
    pub cursor_row: usize,
    /// Cursor column (0-indexed)
    pub cursor_col: usize,
    /// Current pen state
    pen: Pen,
    /// Scrollback history (oldest at front, newest at back)
    scrollback: VecDeque<Vec<Cell>>,
    /// Maximum scrollback lines
    pub max_scrollback: usize,
    /// Saved cursor position
    saved_cursor: Option<(usize, usize)>,
    /// Last printed character (for REP)
    last_char: char,
    /// Top of scroll region (0-indexed)
    scroll_top: usize,
    /// Bottom of scroll region (0-indexed, inclusive)
    scroll_bottom: usize,
    /// In ZWJ sequence flag
    in_zwj_sequence: bool,
    /// Alternate screen buffer (?1049)
    alternate_screen: Option<AlternateScreen>,
    /// Bell notification flag (reset after drawing)
    pub bell_triggered: bool,
    /// Current hyperlink (OSC 8)
    pub current_hyperlink: Option<Arc<Hyperlink>>,
    /// Image placement list
    pub image_placements: Vec<ImagePlacement>,

    // ===== Grouped state =====
    /// Terminal mode flags
    pub modes: TerminalModes,
    /// Cursor appearance
    pub cursor: CursorAppearance,
    /// Shell integration
    pub shell: ShellState,
    /// Keyboard protocol
    pub keyboard: KeyboardState,
    /// Dynamic colors
    pub colors: DynamicColors,
    /// Row buffer pool for scrollback reuse (reduces allocations)
    row_pool: Vec<Vec<Cell>>,
}

/// Mouse tracking mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MouseMode {
    /// Mouse tracking disabled
    #[default]
    None,
    /// X10 mode (?1000) - button press only
    X10,
    /// Button event mode (?1002) - press/release/drag
    ButtonEvent,
    /// Any event mode (?1003) - includes movement
    AnyEvent,
}

/// Alternate screen buffer
struct AlternateScreen {
    cells: Vec<Cell>,
    cursor_row: usize,
    cursor_col: usize,
}

impl Grid {
    /// Create grid with specified size
    pub fn new(cols: usize, rows: usize) -> Self {
        Self::with_scrollback(cols, rows, MAX_SCROLLBACK)
    }

    /// Create grid with specified size and scrollback limit
    pub fn with_scrollback(cols: usize, rows: usize, max_scrollback: usize) -> Self {
        Self {
            cells: vec![Cell::default(); cols * rows],
            cols,
            rows,
            cursor_row: 0,
            cursor_col: 0,
            pen: Pen::default(),
            scrollback: VecDeque::new(),
            max_scrollback,
            saved_cursor: None,
            last_char: ' ',
            scroll_top: 0,
            scroll_bottom: rows - 1,
            in_zwj_sequence: false,
            alternate_screen: None,
            bell_triggered: false,
            current_hyperlink: None,
            image_placements: Vec::new(),
            modes: TerminalModes::new(),
            cursor: CursorAppearance::default(),
            shell: ShellState::default(),
            keyboard: KeyboardState::default(),
            colors: DynamicColors::default(),
            row_pool: Vec::new(),
        }
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    // ========== Compatibility accessors (delegate to sub-structs) ==========
    // These provide backward compatibility during migration

    // TerminalModes
    #[inline] pub fn cursor_visible(&self) -> bool { self.modes.cursor_visible }
    #[inline] pub fn auto_wrap(&self) -> bool { self.modes.auto_wrap }
    #[inline] pub fn application_cursor_keys(&self) -> bool { self.modes.application_cursor_keys }
    #[inline] pub fn bracketed_paste(&self) -> bool { self.modes.bracketed_paste }
    #[inline] pub fn mouse_mode(&self) -> MouseMode { self.modes.mouse_mode }
    #[inline] pub fn mouse_sgr(&self) -> bool { self.modes.mouse_sgr }
    #[inline] pub fn send_focus_events(&self) -> bool { self.modes.send_focus_events }
    #[inline] pub fn synchronized_update(&self) -> bool { self.modes.synchronized_update }

    // CursorAppearance
    #[inline] pub fn cursor_style(&self) -> CursorStyle { self.cursor.style }
    #[inline] pub fn cursor_blink(&self) -> bool { self.cursor.blink }

    // ShellState
    #[inline] pub fn shell_prompt_row(&self) -> Option<usize> { self.shell.prompt_row }
    #[inline] pub fn shell_command_row(&self) -> Option<usize> { self.shell.command_row }
    #[inline] pub fn shell_last_exit_code(&self) -> Option<i32> { self.shell.last_exit_code }

    // KeyboardState
    #[inline] pub fn modify_other_keys(&self) -> u8 { self.keyboard.modify_other_keys }
    #[inline] pub fn kitty_keyboard_flags(&self) -> u32 { self.keyboard.kitty_flags }

    // DynamicColors
    #[inline] pub fn osc_fg_color(&self) -> Option<(u8, u8, u8)> { self.colors.fg }
    #[inline] pub fn osc_bg_color(&self) -> Option<(u8, u8, u8)> { self.colors.bg }

    /// Get reference to cell
    pub fn cell(&self, row: usize, col: usize) -> &Cell {
        &self.cells[row * self.cols + col]
    }

    /// Get mutable reference to cell
    fn cell_mut(&mut self, row: usize, col: usize) -> &mut Cell {
        &mut self.cells[row * self.cols + col]
    }

    // ========== Wide character helpers ==========

    /// Find head cell of wide character (skip continuation cells with width=0)
    fn find_wide_char_head(&self, row: usize, mut col: usize) -> usize {
        while col > 0 && self.cell(row, col).width == 0 {
            col -= 1;
        }
        col
    }

    /// Clear paired cell when overwriting partial cell of wide character
    ///
    /// - Overwriting width=2 cell (head) -> clear right neighbor continuation cell (width=0)
    /// - Overwriting width=0 cell (continuation) -> clear left neighbor head cell (width=2)
    fn clear_wide_char_at(&mut self, row: usize, col: usize) {
        let w = self.cell(row, col).width;
        if w == 2 {
            // Overwriting head cell -> clear right neighbor continuation cell
            if col + 1 < self.cols {
                *self.cell_mut(row, col + 1) = Cell::default();
            }
        } else if w == 0 {
            // Overwriting continuation cell -> clear left neighbor head cell
            if col > 0 {
                *self.cell_mut(row, col - 1) = Cell::default();
            }
        }
    }

    /// Combine combining character (dakuten, handakuten, etc.) with previous cell using NFC normalization
    ///
    /// macOS saves filenames in NFD (decomposed form), so
    /// "da" may be sent as "ta" + U+3099 (combining dakuten)
    fn combine_with_previous(&mut self, combining: char) {
        // Find previous cell
        let (row, col) = if self.cursor_col > 0 {
            (self.cursor_row, self.cursor_col - 1)
        } else if self.cursor_row > 0 {
            (self.cursor_row - 1, self.cols - 1)
        } else {
            return;
        };

        // Go back to head cell if continuation cell
        let col = if self.cell(row, col).width == 0 && col > 0 {
            col - 1
        } else {
            col
        };

        let base_grapheme = &self.cell(row, col).grapheme;
        if base_grapheme.is_empty() || *base_grapheme == " " {
            return;
        }

        // Add combining character to grapheme
        let mut combined = base_grapheme.to_string();
        combined.push(combining);

        // NFC normalization
        let normalized: String = combined.nfc().collect();
        self.cell_mut(row, col).grapheme = SmolStr::new(&normalized);
    }

    /// Add ZWJ or Variation Selector to previous cell's grapheme
    fn combine_grapheme(&mut self, ch: char) {
        let (row, col) = if self.cursor_col > 0 {
            (self.cursor_row, self.cursor_col - 1)
        } else if self.cursor_row > 0 {
            (self.cursor_row - 1, self.cols - 1)
        } else {
            return;
        };

        // Go back to head cell if continuation cell
        let col = if self.cell(row, col).width == 0 && col > 0 {
            col - 1
        } else {
            col
        };

        let base_grapheme = &self.cell(row, col).grapheme;
        if base_grapheme.is_empty() || base_grapheme == " " {
            return;
        }

        // Add to grapheme
        let mut combined = base_grapheme.to_string();
        combined.push(ch);
        self.cell_mut(row, col).grapheme = SmolStr::new(&combined);
    }

    /// Merge Regional Indicator with previous RI to form flag
    fn try_merge_regional_indicator(&mut self, ch: char) -> bool {
        // Find previous cell (skip continuation cells with width=0)
        let (row, col) = if self.cursor_col > 0 {
            (self.cursor_row, self.cursor_col - 1)
        } else if self.cursor_row > 0 {
            (self.cursor_row - 1, self.cols - 1)
        } else {
            return false;
        };

        // Skip continuation cells (width=0) to find actual character cell
        let col = self.find_wide_char_head(row, col);
        let prev_grapheme = &self.cell(row, col).grapheme;

        // Check if previous cell is a single RI
        if prev_grapheme.chars().count() != 1 {
            return false;
        }

        // Safe: we just verified chars().count() == 1 above
        let Some(prev_char) = prev_grapheme.chars().next() else {
            return false;
        };
        let prev_cp = prev_char as u32;

        if !(0x1F1E6..=0x1F1FF).contains(&prev_cp) {
            return false;
        }

        // Combine two RIs into flag grapheme
        let mut flag = prev_grapheme.to_string();
        flag.push(ch);

        // Update previous cell (width remains 2)
        self.cell_mut(row, col).grapheme = SmolStr::new(&flag);

        // Don't advance cursor (merged with previous cell)
        true
    }

    // ========== Character writing ==========

    /// Write character at cursor position and advance cursor
    pub fn put_char(&mut self, ch: char) {
        // ZWJ (Zero Width Joiner) combines with previous cell
        if ch == '\u{200D}' {
            self.combine_grapheme(ch);
            self.in_zwj_sequence = true;
            return;
        }

        // Variation Selector-16 (emoji style) combines with previous cell
        if ch == '\u{FE0F}' {
            self.combine_grapheme(ch);
            return;
        }

        // During ZWJ sequence, next emoji also combines with previous cell
        if self.in_zwj_sequence {
            self.in_zwj_sequence = false;
            // Combine if emoji, otherwise normal processing
            let cp = ch as u32;
            if is_emoji_codepoint(cp) {
                self.combine_grapheme(ch);
                return;
            }
        }

        // Regional Indicator: merge as flag if previous cell is also RI
        let cp = ch as u32;
        if (0x1F1E6..=0x1F1FF).contains(&cp) {
            if self.try_merge_regional_indicator(ch) {
                return;
            }
        }

        // Determine character width
        // Emoji may return 1 from unicode-width but terminals need 2 cells
        let char_width = if is_emoji_codepoint(cp) {
            2 // Force emoji to width=2
        } else {
            match ch.width() {
                None => return, // Control character -> skip
                Some(0) => {
                    // Combining character (dakuten, etc.) -> combine with previous cell
                    self.combine_with_previous(ch);
                    return;
                }
                Some(w) => w,
            }
        };

        // Wrap at right edge (only if auto_wrap is enabled)
        if self.cursor_col >= self.cols {
            if self.modes.auto_wrap {
                self.cursor_col = 0;
                self.cursor_row += 1;
                if self.cursor_row >= self.rows {
                    self.scroll_up(1);
                    self.cursor_row = self.rows - 1;
                }
            } else {
                // Stay at last column if auto_wrap is disabled
                self.cursor_col = self.cols - 1;
            }
        }

        // Wide character doesn't fit at right edge -> fill current cell with space and move to next line
        if char_width == 2 && self.cursor_col + 1 >= self.cols {
            // Fill rightmost cell with space
            self.clear_wide_char_at(self.cursor_row, self.cursor_col);
            *self.cell_mut(self.cursor_row, self.cursor_col) = Cell::default();
            self.cursor_col = 0;
            self.cursor_row += 1;
            if self.cursor_row >= self.rows {
                self.scroll_up(1);
                self.cursor_row = self.rows - 1;
            }
        }

        let fg = self.pen.fg;
        let bg = self.pen.bg;
        let attrs = self.pen.attrs;
        let underline_style = self.pen.underline_style;
        let underline_color = self.pen.underline_color;

        // Clear paired cell when overwriting existing wide character
        self.clear_wide_char_at(self.cursor_row, self.cursor_col);

        // Write primary cell
        let idx = self.cursor_row * self.cols + self.cursor_col;
        self.cells[idx] = Cell {
            grapheme: char_to_smolstr(ch),
            fg,
            bg,
            attrs,
            width: char_width as u8,
            hyperlink: self.current_hyperlink.clone(),
            underline_style,
            underline_color,
        };

        // Write continuation cell for wide characters
        if char_width == 2 {
            let next_col = self.cursor_col + 1;
            if next_col < self.cols {
                // Also clear existing wide character at continuation cell
                self.clear_wide_char_at(self.cursor_row, next_col);
                // Use static empty string for continuation cell
                static EMPTY: SmolStr = SmolStr::new_inline("");
                *self.cell_mut(self.cursor_row, next_col) = Cell {
                    grapheme: EMPTY.clone(),
                    fg,
                    bg,
                    attrs,
                    width: 0,
                    hyperlink: self.current_hyperlink.clone(),
                    underline_style,
                    underline_color,
                };
            }
        }

        self.cursor_col += char_width;
        self.last_char = ch;
    }

    // ========== Cursor movement ==========

    /// Move cursor to absolute position (1-indexed -> 0-indexed)
    pub fn move_cursor_to(&mut self, row: usize, col: usize) {
        self.cursor_row = row.saturating_sub(1).min(self.rows - 1);
        self.cursor_col = col.saturating_sub(1).min(self.cols - 1);
    }

    /// Move cursor up (CSI A)
    pub fn move_cursor_up(&mut self, n: usize) {
        self.cursor_row = self.cursor_row.saturating_sub(n);
    }

    /// Move cursor down (CSI B)
    pub fn move_cursor_down(&mut self, n: usize) {
        self.cursor_row = (self.cursor_row + n).min(self.rows - 1);
    }

    /// Move cursor right (CSI C)
    pub fn move_cursor_forward(&mut self, n: usize) {
        self.cursor_col = (self.cursor_col + n).min(self.cols - 1);
    }

    /// Move cursor left (CSI D)
    pub fn move_cursor_backward(&mut self, n: usize) {
        self.cursor_col = self.cursor_col.saturating_sub(n);
    }

    // ========== Erase ==========

    /// Erase display (CSI J)
    /// mode: 0=from cursor, 1=to cursor, 2=entire screen
    pub fn erase_in_display(&mut self, mode: u16) {
        match mode {
            0 => {
                // Erase from cursor to end
                self.erase_in_line(0);
                for row in (self.cursor_row + 1)..self.rows {
                    self.clear_row(row);
                    self.remove_images_at_row(row);
                }
            }
            1 => {
                // Erase from start to cursor
                for row in 0..self.cursor_row {
                    self.clear_row(row);
                    self.remove_images_at_row(row);
                }
                self.erase_in_line(1);
            }
            2 | 3 => {
                // Erase entire screen
                for cell in &mut self.cells {
                    *cell = Cell::default();
                }
                // Also clear image placements
                self.image_placements.clear();
            }
            _ => {}
        }
    }

    /// Erase line (CSI K)
    /// mode: 0=from cursor, 1=to cursor, 2=entire line
    pub fn erase_in_line(&mut self, mode: u16) {
        let row = self.cursor_row;
        match mode {
            0 => {
                // Also clear left neighbor head cell if start position is continuation cell (width=0)
                if self.cursor_col < self.cols && self.cell(row, self.cursor_col).width == 0 {
                    if self.cursor_col > 0 {
                        *self.cell_mut(row, self.cursor_col - 1) = Cell::default();
                    }
                }
                for col in self.cursor_col..self.cols {
                    *self.cell_mut(row, col) = Cell::default();
                }
                // Delete images overlapping this row
                self.remove_images_at_row(row);
            }
            1 => {
                let end = self.cursor_col.min(self.cols - 1);
                // Also clear right neighbor continuation cell if end position is head cell (width=2)
                if self.cell(row, end).width == 2 && end + 1 < self.cols {
                    *self.cell_mut(row, end + 1) = Cell::default();
                }
                for col in 0..=end {
                    *self.cell_mut(row, col) = Cell::default();
                }
                // Delete images overlapping this row
                self.remove_images_at_row(row);
            }
            2 => {
                self.clear_row(row);
                // Delete images overlapping this row
                self.remove_images_at_row(row);
            }
            _ => {}
        }
    }

    /// Clear row (optimized with fill)
    fn clear_row(&mut self, row: usize) {
        let start = row * self.cols;
        let end = start + self.cols;
        self.cells[start..end].fill(Cell::default());
    }

    /// Delete images overlapping specified row
    fn remove_images_at_row(&mut self, row: usize) {
        self.image_placements.retain(|p| {
            // Image range: p.row to p.row + p.height_cells - 1
            let img_end = p.row + p.height_cells.saturating_sub(1);
            // Keep if row is outside this range
            row < p.row || row > img_end
        });
    }

    // ========== Scroll ==========

    /// Scroll up (n lines)
    /// Scrolls within scroll region if set
    pub fn scroll_up(&mut self, n: usize) {
        let top = self.scroll_top;
        let bottom = self.scroll_bottom;
        let region_height = bottom - top + 1;
        let n = n.min(region_height);

        // Save to scrollback only for full-screen scroll
        if top == 0 && bottom == self.rows - 1 {
            for i in 0..n {
                let start = i * self.cols;
                // Reuse row buffer from pool if available
                let mut row_cells = self.row_pool.pop().unwrap_or_else(|| Vec::with_capacity(self.cols));
                row_cells.clear();
                row_cells.extend_from_slice(&self.cells[start..start + self.cols]);
                self.scrollback.push_back(row_cells);
            }
            // Return evicted rows to pool for reuse
            while self.scrollback.len() > self.max_scrollback {
                if let Some(old_row) = self.scrollback.pop_front() {
                    // Keep pool size bounded (max 32 rows)
                    if self.row_pool.len() < 32 {
                        self.row_pool.push(old_row);
                    }
                }
            }
        }

        // Shift rows up within scroll region
        // Use clone_from_slice for better performance (avoids per-element clone overhead)
        for row in top..(bottom + 1 - n) {
            let src_start = (row + n) * self.cols;
            let dst_start = row * self.cols;
            // split_at_mut allows us to have mutable refs to non-overlapping slices
            let (left, right) = self.cells.split_at_mut(src_start);
            left[dst_start..dst_start + self.cols].clone_from_slice(&right[..self.cols]);
        }

        // Clear bottom
        for row in (bottom + 1 - n)..=bottom {
            self.clear_row(row);
        }

        // Adjust image placement rows (delete scrolled out ones)
        for p in &mut self.image_placements {
            if p.row >= top && p.row <= bottom {
                p.row = p.row.saturating_sub(n);
            }
        }
        self.image_placements.retain(|p| p.row + p.height_cells > 0);
    }

    // ========== Scrollback ==========

    /// Number of scrollback lines
    pub fn scrollback_len(&self) -> usize {
        self.scrollback.len()
    }

    /// Get scrollback line (0 = oldest line)
    pub fn scrollback_row(&self, idx: usize) -> Option<&[Cell]> {
        self.scrollback.get(idx).map(|v| v.as_slice())
    }

    // ========== Control characters ==========

    /// Line feed (LF)
    pub fn linefeed(&mut self) {
        if self.cursor_row == self.scroll_bottom {
            // Scroll if at bottom of scroll region
            self.scroll_up(1);
        } else if self.cursor_row < self.rows - 1 {
            self.cursor_row += 1;
        }
    }

    /// Reverse index (RI / ESC M)
    pub fn reverse_index(&mut self) {
        if self.cursor_row == self.scroll_top {
            // Scroll down if at top of scroll region
            self.scroll_down(1);
        } else if self.cursor_row > 0 {
            self.cursor_row -= 1;
        }
    }

    /// Carriage return (CR)
    pub fn carriage_return(&mut self) {
        self.cursor_col = 0;
    }

    /// Tab (HT)
    pub fn tab(&mut self) {
        // Move to next multiple of 8 position
        let next_tab = (self.cursor_col / 8 + 1) * 8;
        self.cursor_col = next_tab.min(self.cols - 1);
    }

    /// Backspace (BS)
    pub fn backspace(&mut self) {
        if self.cursor_col > 0 {
            self.cursor_col -= 1;
            // If moved to continuation cell (width=0), go back to head cell
            self.cursor_col = self.find_wide_char_head(self.cursor_row, self.cursor_col);
        }
    }

    // ========== SGR (attribute setting) ==========

    /// SGR reset
    pub fn reset_attrs(&mut self) {
        self.pen = Pen::default();
    }

    /// Set foreground color
    pub fn set_fg(&mut self, color: Color) {
        self.pen.fg = color;
    }

    /// Set background color
    pub fn set_bg(&mut self, color: Color) {
        self.pen.bg = color;
    }

    /// Set attribute
    pub fn set_attr(&mut self, attr: CellAttrs) {
        self.pen.attrs.insert(attr);
    }

    /// Clear attribute
    pub fn clear_attr(&mut self, attr: CellAttrs) {
        self.pen.attrs.remove(attr);
    }

    /// Set underline style (CSI 4:x m)
    pub fn set_underline_style(&mut self, style: UnderlineStyle) {
        self.pen.underline_style = style;
        // Backward compatible: also set UNDERLINE flag
        if style != UnderlineStyle::None {
            self.pen.attrs.insert(CellAttrs::UNDERLINE);
        } else {
            self.pen.attrs.remove(CellAttrs::UNDERLINE);
        }
    }

    /// Set underline color (CSI 58;2;r;g;b m)
    pub fn set_underline_color(&mut self, color: Color) {
        self.pen.underline_color = Some(color);
    }

    /// Reset underline color (CSI 59 m)
    pub fn reset_underline_color(&mut self) {
        self.pen.underline_color = None;
    }

    /// Delete lines and scroll (CSI M)
    /// Operates within scroll region
    pub fn delete_lines(&mut self, n: usize) {
        let bottom = self.scroll_bottom;
        // Do nothing if cursor is outside scroll region
        if self.cursor_row < self.scroll_top || self.cursor_row > bottom {
            return;
        }
        let n = n.min(bottom - self.cursor_row + 1);
        let start = self.cursor_row;

        // Move rows from start+n to bottom to start
        for row in start..(bottom + 1 - n) {
            let src_start = (row + n) * self.cols;
            let dst_start = row * self.cols;
            let (left, right) = self.cells.split_at_mut(src_start);
            left[dst_start..dst_start + self.cols].clone_from_slice(&right[..self.cols]);
        }

        // Clear bottom
        for row in (bottom + 1 - n)..=bottom {
            self.clear_row(row);
        }
    }

    /// Delete characters (CSI P / DCH)
    /// Delete n characters from cursor position and shift right characters left
    pub fn delete_chars(&mut self, n: usize) {
        let row = self.cursor_row;
        let col = self.cursor_col;
        let n = n.min(self.cols - col);

        // Shift from col+n to end of line to col
        let row_start = row * self.cols;
        let src_start = col + n;
        let dst_start = col;

        // Left shift (copy from front)
        for i in 0..(self.cols - col - n) {
            self.cells[row_start + dst_start + i] = self.cells[row_start + src_start + i].clone();
        }

        // Fill right end with spaces
        for c in (self.cols - n)..self.cols {
            *self.cell_mut(row, c) = Cell::default();
        }
    }

    /// Insert characters (CSI @ / ICH)
    /// Insert n spaces at cursor position and shift right characters right
    pub fn insert_chars(&mut self, n: usize) {
        let row = self.cursor_row;
        let col = self.cursor_col;
        let n = n.min(self.cols - col);

        // Shift from col to end-n to col+n (copy right to left)
        let row_start = row * self.cols;
        for i in (col..(self.cols - n)).rev() {
            self.cells[row_start + i + n] = self.cells[row_start + i].clone();
        }

        // Fill insertion position with spaces
        for c in col..(col + n) {
            *self.cell_mut(row, c) = Cell::default();
        }
    }

    /// Erase characters (CSI X / ECH)
    /// Overwrite n characters from cursor position with spaces (no shift)
    pub fn erase_chars(&mut self, n: usize) {
        let row = self.cursor_row;
        let col = self.cursor_col;
        let n = n.min(self.cols - col);

        for c in col..(col + n) {
            *self.cell_mut(row, c) = Cell::default();
        }
    }

    /// Scroll down (CSI T / SD)
    /// Scroll down n lines (blank lines enter from top)
    /// Scrolls within scroll region if set
    pub fn scroll_down(&mut self, n: usize) {
        let top = self.scroll_top;
        let bottom = self.scroll_bottom;
        let region_height = bottom - top + 1;
        let n = n.min(region_height);

        // Shift rows down within scroll region (copy bottom to top)
        // Iterate in reverse to avoid overwriting source data
        for row in ((top + n)..=bottom).rev() {
            let src_start = (row - n) * self.cols;
            let dst_start = row * self.cols;
            // split_at_mut: left contains source, right contains destination
            let (left, right) = self.cells.split_at_mut(dst_start);
            right[..self.cols].clone_from_slice(&left[src_start..src_start + self.cols]);
        }

        // Fill top with spaces
        for row in top..(top + n) {
            self.clear_row(row);
        }

        // Adjust image placement rows
        for p in &mut self.image_placements {
            if p.row >= top && p.row <= bottom {
                p.row += n;
            }
        }
        // Delete images that scrolled out of screen
        self.image_placements.retain(|p| p.row < self.rows);
    }

    /// Save cursor position (CSI s / SCOSC)
    pub fn save_cursor(&mut self) {
        self.saved_cursor = Some((self.cursor_row, self.cursor_col));
    }

    /// Restore cursor position (CSI u / SCORC)
    pub fn restore_cursor(&mut self) {
        if let Some((row, col)) = self.saved_cursor {
            self.cursor_row = row.min(self.rows - 1);
            self.cursor_col = col.min(self.cols - 1);
        }
    }

    /// Repeat last character (CSI b / REP)
    pub fn repeat_char(&mut self, n: usize) {
        let ch = self.last_char;
        for _ in 0..n {
            self.put_char(ch);
        }
    }

    /// Set scroll region (CSI r / DECSTBM)
    /// top, bottom are 1-indexed. 0 is treated as default value.
    pub fn set_scroll_region(&mut self, top: usize, bottom: usize) {
        let top = if top == 0 { 1 } else { top };
        let bottom = if bottom == 0 { self.rows } else { bottom };

        // Convert to 0-indexed
        let top = (top - 1).min(self.rows - 1);
        let bottom = (bottom - 1).min(self.rows - 1);

        if top < bottom {
            self.scroll_top = top;
            self.scroll_bottom = bottom;
        }
        // Move cursor to top-left
        self.cursor_row = 0;
        self.cursor_col = 0;
    }

    /// Get scroll region
    #[allow(dead_code)]
    pub fn scroll_region(&self) -> (usize, usize) {
        (self.scroll_top, self.scroll_bottom)
    }

    /// Switch to alternate screen buffer (?1049 set)
    pub fn enter_alternate_screen(&mut self) {
        if self.alternate_screen.is_some() {
            return; // Already in alternate screen
        }
        // Save current state
        let saved = AlternateScreen {
            cells: self.cells.clone(),
            cursor_row: self.cursor_row,
            cursor_col: self.cursor_col,
        };
        self.alternate_screen = Some(saved);
        // Clear screen
        self.cells = vec![Cell::default(); self.cols * self.rows];
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.image_placements.clear();
    }

    /// Return to main screen buffer (?1049 reset)
    pub fn leave_alternate_screen(&mut self) {
        if let Some(saved) = self.alternate_screen.take() {
            self.cells = saved.cells;
            self.cursor_row = saved.cursor_row;
            self.cursor_col = saved.cursor_col;
            self.image_placements.clear();
        }
    }

    /// Check if in alternate screen
    #[allow(dead_code)]
    pub fn is_alternate_screen(&self) -> bool {
        self.alternate_screen.is_some()
    }

    /// Insert lines (CSI L)
    /// Operates within scroll region
    pub fn insert_lines(&mut self, n: usize) {
        let bottom = self.scroll_bottom;
        // Do nothing if cursor is outside scroll region
        if self.cursor_row < self.scroll_top || self.cursor_row > bottom {
            return;
        }
        let n = n.min(bottom - self.cursor_row + 1);

        // Shift down (copy bottom to top, iterate in reverse)
        for row in ((self.cursor_row + n)..=bottom).rev() {
            let src_start = (row - n) * self.cols;
            let dst_start = row * self.cols;
            let (left, right) = self.cells.split_at_mut(dst_start);
            right[..self.cols].clone_from_slice(&left[src_start..src_start + self.cols]);
        }

        // Clear inserted rows
        for row in self.cursor_row..(self.cursor_row + n) {
            self.clear_row(row);
        }
    }

    // ========== Image placement ==========

    /// Place image at current cursor position
    ///
    /// Calculates occupied cell count and moves cursor below image.
    pub fn place_image(
        &mut self,
        id: u32,
        pixel_width: u32,
        pixel_height: u32,
        cell_width: u32,
        cell_height: u32,
    ) {
        if cell_width == 0 || cell_height == 0 {
            trace!("place_image: cell size not set, skipping placement");
            return;
        }

        // Calculate occupied cells (round up)
        let width_cells = ((pixel_width + cell_width - 1) / cell_width) as usize;
        let height_cells = ((pixel_height + cell_height - 1) / cell_height) as usize;

        let placement = ImagePlacement {
            id,
            row: self.cursor_row,
            col: self.cursor_col,
            width_cells,
            height_cells,
            pixel_width,
            pixel_height,
        };

        trace!(
            "place_image: id={} at ({},{}) {}x{} cells",
            id,
            self.cursor_row,
            self.cursor_col,
            width_cells,
            height_cells
        );

        self.image_placements.push(placement);

        // Move cursor below image
        // May need to scroll for height_cells rows
        for _ in 0..height_cells {
            self.cursor_row += 1;
            if self.cursor_row >= self.rows {
                self.scroll_up(1); // image_placements rows also adjusted in scroll_up
                self.cursor_row = self.rows - 1;
            }
        }
        self.cursor_col = 0;
    }

    /// Delete image placements that scrolled out of screen
    #[allow(dead_code)]
    pub fn cleanup_image_placements(&mut self) {
        // Delete placements where row became negative (scrolled out)
        self.image_placements.retain(|p| {
            // Check if image bottom is within screen
            p.row + p.height_cells > 0
        });
    }

    /// Resize grid
    ///
    /// Preserves existing content as much as possible
    pub fn resize(&mut self, new_cols: usize, new_rows: usize) {
        if new_cols == self.cols && new_rows == self.rows {
            return;
        }

        let old_cols = self.cols;

        // Create new cell array
        let mut new_cells = vec![Cell::default(); new_cols * new_rows];

        // Copy existing cells (only common area)
        let copy_rows = self.rows.min(new_rows);
        let copy_cols = old_cols.min(new_cols);

        for row in 0..copy_rows {
            let src_start = row * old_cols;
            let dst_start = row * new_cols;
            new_cells[dst_start..dst_start + copy_cols]
                .clone_from_slice(&self.cells[src_start..src_start + copy_cols]);
        }

        self.cells = new_cells;
        self.cols = new_cols;
        self.rows = new_rows;

        // Keep cursor position within new size
        self.cursor_row = self.cursor_row.min(new_rows.saturating_sub(1));
        self.cursor_col = self.cursor_col.min(new_cols.saturating_sub(1));

        // Update scroll region
        self.scroll_top = 0;
        self.scroll_bottom = new_rows.saturating_sub(1);

        // Clear image placements
        self.image_placements.clear();

        // Clear row pool if column count changed (old rows have wrong size)
        if new_cols != old_cols {
            self.row_pool.clear();
        }
    }
}
