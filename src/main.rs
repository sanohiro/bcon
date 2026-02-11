//! bcon - GPU-accelerated terminal for Linux console
//!
//! # Architecture
//!
//! ```text
//! ┌──────────────────────────────────────────┐
//! │              Event Loop                  │
//! ├──────────────────────────────────────────┤
//! │  Input (evdev)  →  Terminal (VT/PTY)    │
//! │                          ↓               │
//! │              GPU Renderer (OpenGL ES)    │
//! │                          ↓               │
//! │              DRM/KMS Output              │
//! └──────────────────────────────────────────┘
//! ```

mod config;
mod drm;
mod font;
mod gpu;
mod input;
mod session;
mod terminal;

use anyhow::{anyhow, Context, Result};
use log::{info, trace};
use std::time::Duration;

#[cfg(all(target_os = "linux", feature = "seatd"))]
use std::cell::RefCell;
#[cfg(all(target_os = "linux", feature = "seatd"))]
use std::rc::Rc;

/// sRGB to linear conversion (same calculation as shader)
fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Linear to sRGB conversion (same calculation as shader)
fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// Auto-detect DRM device
fn find_drm_device() -> Result<String> {
    for i in 0..8 {
        let path = format!("/dev/dri/card{}", i);
        if std::path::Path::new(&path).exists() {
            return Ok(path);
        }
    }
    Err(anyhow!("/dev/dri/card* not found"))
}

/// Load font for testing: BCON_FONT env var -> ligature font -> system font
fn load_test_font() -> Result<Vec<u8>> {
    // Use environment variable if specified
    if let Ok(path) = std::env::var("BCON_FONT") {
        let data = std::fs::read(&path)
            .with_context(|| format!("Cannot read font specified by BCON_FONT: {}", path))?;
        eprintln!("Font: {} (BCON_FONT)", path);
        return Ok(data);
    }

    // Search for ligature-capable fonts first
    let ligature_fonts = [
        "/usr/share/fonts/truetype/firacode/FiraCode-Regular.ttf",
        "/usr/share/fonts/truetype/firacode/FiraCode-Retina.ttf",
        "/usr/share/fonts/opentype/firacode/FiraCode-Regular.otf",
        "/Library/Fonts/FiraCode-Regular.ttf",
        "/usr/share/fonts/JetBrainsMono-Regular.ttf",
        "/usr/share/fonts/truetype/jetbrains-mono/JetBrainsMono-Regular.ttf",
    ];

    for path in &ligature_fonts {
        if let Ok(data) = std::fs::read(path) {
            eprintln!("Font: {} (ligature-capable)", path);
            return Ok(data);
        }
    }

    // Fallback: system font
    eprintln!("Ligature font not found - falling back to system font");
    font::atlas::load_system_font()
}

/// Print help message
fn print_help() {
    println!(
        r#"bcon {} - GPU-accelerated terminal emulator for Linux console

USAGE:
    bcon [OPTIONS]

OPTIONS:
    -h, --help              Print this help message
    -V, --version           Print version information
    -t, --test              Test mode (verify build without DRM)
    --init-config[=PRESET]  Generate config file with optional preset
    --test-shaper           Test font shaping (debug mode)

PRESETS (for --init-config):
    default    Standard keybinds (Ctrl+Shift+C/V, etc.)
    vim        Vim-like scroll (Ctrl+Shift+U/D)
    emacs      Emacs-like scroll (Alt+Shift+V/N)
    japanese   CJK fonts + IME auto-disable (alias: jp)

    Combine presets with comma: --init-config=vim,jp

EXAMPLES:
    bcon                          Run bcon (requires TTY, not X11/Wayland)
    bcon --init-config            Generate default config
    bcon --init-config=vim,jp     Generate config with vim and japanese presets
    sudo bcon                     Run with root privileges (required for DRM)

CONFIG FILE:
    ~/.config/bcon/config.toml

For more information, see: https://github.com/sanohiro/bcon
"#,
        env!("CARGO_PKG_VERSION")
    );
}

/// Shaper test mode: verify text shaping without GPU
fn test_shaper_mode() -> Result<()> {
    eprintln!("=== Text Shaper Test ===\n");

    // Use BCON_FONT env var or prioritize ligature fonts
    let font_data: &'static [u8] = Box::leak(
        load_test_font()
            .context("Failed to load font")?
            .into_boxed_slice(),
    );
    let cjk_font_data: Option<&'static [u8]> =
        font::atlas::load_cjk_font().map(|d| -> &'static [u8] { Box::leak(d.into_boxed_slice()) });

    // fontdue Font (for glyph existence check)
    let font_main = fontdue::Font::from_bytes(font_data, fontdue::FontSettings::default())
        .map_err(|e| anyhow!("Failed to load font: {}", e))?;

    eprintln!("Font loaded");

    // Create shaper
    let mut shaper = match font::shaper::TextShaper::new(font_data, cjk_font_data) {
        Some(s) => s,
        None => {
            eprintln!("Failed to create shaper");
            return Ok(());
        }
    };
    eprintln!("Shaper initialized\n");

    // Test strings
    let test_strings = [
        "val == 0",
        "a != b",
        "fn -> bool",
        "x <= y",
        "a => b",
        "<=>",
        "hello world",
        "abc 日本語 xyz",
        "// comment",
        "www",
        "|> >>= <|>",
    ];

    let cols = 80;
    let rows = 1;
    let mut grid = terminal::grid::Grid::new(cols, rows);

    for test in &test_strings {
        // Clear grid
        grid.erase_in_display(2);
        grid.move_cursor_to(1, 1);

        // Write characters to grid
        for ch in test.chars() {
            grid.put_char(ch);
        }

        // Execute shaping
        let shaped = shaper.shape_line(&grid, 0, &font_main);

        eprintln!("Input: \"{}\"", test);
        eprintln!("  Shaping result ({} glyphs):", shaped.len());

        let mut has_ligature = false;
        let mut has_calt = false;
        for (col, sg) in &shaped {
            // Compare with default glyph ID (without shaping)
            let default_gid = font_main.lookup_glyph_index(sg.ch);
            let is_substituted =
                sg.key.glyph_id != 0 && default_gid != 0 && sg.key.glyph_id != default_gid;
            let is_merged = sg.cell_span > 1;

            let marker = if is_merged {
                has_ligature = true;
                " <- ligature (merged)"
            } else if is_substituted {
                has_calt = true;
                " <- calt substitution!"
            } else {
                ""
            };
            eprintln!(
                "    col={:<3} glyph_id={:<5} (default={:<5}) span={} ch='{}'{}",
                col, sg.key.glyph_id, default_gid, sg.cell_span, sg.ch, marker
            );
        }
        if has_ligature || has_calt {
            let kind = if has_ligature && has_calt {
                "merged ligature + calt substitution"
            } else if has_ligature {
                "merged ligature"
            } else {
                "calt context substitution"
            };
            eprintln!("  => Shaping effect detected ({})", kind);
        } else {
            eprintln!("  (No shaping effect)");
        }
        eprintln!();
    }

    eprintln!("=== Test complete ===");
    Ok(())
}

/// Update text selection range with Shift+Arrow keys
fn handle_selection_key(term: &mut terminal::Terminal, keycode: u32, cols: usize) {
    let cur_row = term.grid.cursor_row;
    let cur_col = term.grid.cursor_col;
    let rows = term.grid.rows();

    // Start selection with cursor position as anchor if no selection exists
    let sel = term.selection.get_or_insert(terminal::Selection {
        anchor_row: cur_row,
        anchor_col: cur_col,
        end_row: cur_row,
        end_col: cur_col,
    });

    // evdev keycodes
    const KEY_LEFT: u32 = 105;
    const KEY_RIGHT: u32 = 106;
    const KEY_UP: u32 = 103;
    const KEY_DOWN: u32 = 108;
    const KEY_HOME: u32 = 102;
    const KEY_END: u32 = 107;

    match keycode {
        KEY_LEFT => {
            if sel.end_col > 0 {
                sel.end_col -= 1;
            } else if sel.end_row > 0 {
                sel.end_row -= 1;
                sel.end_col = cols.saturating_sub(1);
            }
        }
        KEY_RIGHT => {
            if sel.end_col < cols - 1 {
                sel.end_col += 1;
            } else if sel.end_row < rows - 1 {
                sel.end_row += 1;
                sel.end_col = 0;
            }
        }
        KEY_UP => {
            sel.end_row = sel.end_row.saturating_sub(1);
        }
        KEY_DOWN => {
            sel.end_row = (sel.end_row + 1).min(rows - 1);
        }
        KEY_HOME => {
            sel.end_col = 0;
        }
        KEY_END => {
            sel.end_col = cols.saturating_sub(1);
        }
        _ => {}
    }
}

/// Expand ~ in path to user's home directory
fn expand_path(path: &str, user_home: Option<&str>) -> String {
    if path.starts_with("~/") {
        if let Some(home) = user_home {
            return format!("{}{}", home, &path[1..]);
        }
    } else if path == "~" {
        if let Some(home) = user_home {
            return home.to_string();
        }
    }
    // Fallback: use dirs::home_dir() or keep as-is
    if path.starts_with("~") {
        if let Some(home) = dirs::home_dir() {
            if path == "~" {
                return home.to_string_lossy().to_string();
            } else {
                return format!("{}{}", home.to_string_lossy(), &path[1..]);
            }
        }
    }
    path.to_string()
}

/// Save screenshot
fn save_screenshot(
    gl: &glow::Context,
    width: u32,
    height: u32,
    screenshot_dir: &str,
    user_home: Option<&str>,
) -> Result<()> {
    use glow::HasContext;

    // Expand ~ to user's home directory
    let screenshot_dir = expand_path(screenshot_dir, user_home);

    // Read pixel data from framebuffer
    let mut pixels = vec![0u8; (width * height * 4) as usize];
    unsafe {
        gl.read_pixels(
            0,
            0,
            width as i32,
            height as i32,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelPackData::Slice(&mut pixels),
        );
    }

    // Flip vertically since OpenGL origin is bottom-left
    let row_size = (width * 4) as usize;
    let mut flipped = vec![0u8; pixels.len()];
    for y in 0..height as usize {
        let src_row = (height as usize - 1 - y) * row_size;
        let dst_row = y * row_size;
        flipped[dst_row..dst_row + row_size].copy_from_slice(&pixels[src_row..src_row + row_size]);
    }

    // Generate filename
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let path = format!("{}/bcon_screenshot_{}.png", screenshot_dir, timestamp);

    // Save as PNG
    let file = std::fs::File::create(&path)?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&flipped)?;

    info!("Screenshot saved: {}", path);
    Ok(())
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    // Check command line arguments
    let args: Vec<String> = std::env::args().collect();

    // --help
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return Ok(());
    }

    // --version
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("bcon {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    info!("bcon starting...");

    let test_mode = args.iter().any(|a| a == "--test" || a == "-t");

    if test_mode {
        info!("Test mode: skipping DRM initialization");
        eprintln!("[OK] bcon build verification complete");
        return Ok(());
    }

    // Config file generation mode
    // --init-config or --init-config=PRESET (default, emacs-like, vim-like)
    let init_config_arg = args.iter().find(|a| a.starts_with("--init-config"));
    if let Some(arg) = init_config_arg {
        let preset = if arg.contains('=') {
            arg.split('=').nth(1).unwrap_or("default")
        } else {
            "default"
        };

        match config::Config::write_config_with_preset(preset) {
            Ok(path) => {
                println!("Config file generated:");
                println!("  Preset: {}", preset);
                println!("  Path:   {}", path.display());
                println!();
                println!("Available presets (combine with comma):");
                println!("  default  - Standard keybinds (Ctrl+Shift+C/V, etc.)");
                println!("  vim      - Vim-like scroll (Ctrl+Shift+U/D)");
                println!("  emacs    - Emacs-like scroll (Alt+Shift+V/N)");
                println!("  japanese - CJK fonts + IME auto-disable (alias: jp)");
                println!();
                println!("Examples:");
                println!("  bcon --init-config=vim,jp");
                println!("  bcon --init-config=emacs,japanese");
                return Ok(());
            }
            Err(e) => {
                eprintln!("Failed to generate config: {}", e);
                return Err(e);
            }
        }
    }

    // Load config file
    let mut cfg = config::Config::load();

    // Parse keybinds (multiple keys per action supported)
    let mut kb_copy = config::ParsedKeybinds::parse(&cfg.keybinds.copy);
    let mut kb_paste = config::ParsedKeybinds::parse(&cfg.keybinds.paste);
    let mut kb_screenshot = config::ParsedKeybinds::parse(&cfg.keybinds.screenshot);
    let mut kb_search = config::ParsedKeybinds::parse(&cfg.keybinds.search);
    let mut kb_copy_mode = config::ParsedKeybinds::parse(&cfg.keybinds.copy_mode);
    let mut kb_font_increase = config::ParsedKeybinds::parse(&cfg.keybinds.font_increase);
    let mut kb_font_decrease = config::ParsedKeybinds::parse(&cfg.keybinds.font_decrease);
    let mut kb_font_reset = config::ParsedKeybinds::parse(&cfg.keybinds.font_reset);
    let mut kb_scroll_up = config::ParsedKeybinds::parse(&cfg.keybinds.scroll_up);
    let mut kb_scroll_down = config::ParsedKeybinds::parse(&cfg.keybinds.scroll_down);
    let mut kb_ime_toggle = config::ParsedKeybinds::parse(&cfg.keybinds.ime_toggle);

    // Config file change watcher (Linux only)
    #[cfg(target_os = "linux")]
    let config_watcher = config::default_config_path().and_then(|path| {
        if path.exists() {
            config::ConfigWatcher::new(&path).ok()
        } else {
            None
        }
    });
    #[cfg(target_os = "linux")]
    if config_watcher.is_some() {
        info!("Config hot-reload enabled");
    }

    // Shaper test mode: log font shaping results
    let test_shaper = args.iter().any(|a| a == "--test-shaper");
    if test_shaper {
        return test_shaper_mode();
    }

    // Set up SIGTERM handler for graceful shutdown (systemd stop)
    drm::setup_sigterm_handler();

    // Phase 0: Session management
    // - With seatd feature: Use libseat for rootless operation
    // - Without seatd: Use VtSwitcher (requires root)

    #[cfg(all(target_os = "linux", feature = "seatd"))]
    let seat_session = {
        info!("Opening libseat session...");
        Rc::new(RefCell::new(
            session::SeatSession::open().context("Failed to open libseat session")?
        ))
    };

    #[cfg(not(all(target_os = "linux", feature = "seatd")))]
    let mut vt_switcher = {
        // VtSwitcher sets up process-controlled VT switching via VT_SETMODE
        // The kernel sends SIGUSR1/SIGUSR2 signals for VT switch requests
        drm::VtSwitcher::new().context("Failed to initialize VT switcher")?
    };

    // Phase 1: DRM/KMS initialization
    let drm_path = find_drm_device()?;
    info!("DRM device: {}", drm_path);

    #[cfg(all(target_os = "linux", feature = "seatd"))]
    let drm_device = {
        let seat_drm_device = seat_session
            .borrow_mut()
            .open_device(&drm_path)
            .context("Cannot open DRM device via libseat")?;
        drm::Device::from_fd(seat_drm_device.as_raw_fd())
            .context("Cannot create DRM device from libseat fd")?
    };

    #[cfg(not(all(target_os = "linux", feature = "seatd")))]
    let drm_device = drm::Device::open(&drm_path)
        .context("Cannot open DRM device. Root privileges may be required.")?;

    // Detect display configuration (prefer external monitors if configured)
    let mut display_config =
        drm::DisplayConfig::detect_with_preference(&drm_device, cfg.display.prefer_external)
            .context("Failed to detect display configuration")?;

    info!(
        "Display: {}x{} (external: {})",
        display_config.width, display_config.height,
        display_config.is_external(&drm_device)
    );

    // Initialize DRM hotplug monitor (Linux only)
    #[cfg(target_os = "linux")]
    let mut hotplug_monitor = match drm::HotplugMonitor::new() {
        Ok(m) => Some(m),
        Err(e) => {
            info!("Hotplug monitor unavailable: {}", e);
            None
        }
    };
    #[cfg(target_os = "linux")]
    let mut last_connector_snapshot = drm::hotplug::snapshot_connectors(&drm_device)
        .unwrap_or_default();

    // Create GBM device (shares fd with DRM)
    let gbm_file = drm_device.dup_fd()?;
    let gbm_device = gpu::GbmDevice::new(gbm_file)?;

    // Create GBM surface
    let gbm_surface = gpu::GbmSurface::new(
        gbm_device.device(),
        display_config.width,
        display_config.height,
    )?;

    // Create EGL context
    let egl_context = gpu::EglContext::new(gbm_device.device(), gbm_surface.surface())?;

    // Create OpenGL ES renderer
    let renderer = gpu::GlRenderer::new(&egl_context)?;
    renderer.set_viewport(
        0,
        0,
        display_config.width as i32,
        display_config.height as i32,
    );

    // Save original CRTC settings (restore on exit)
    let saved_crtc = drm::SavedCrtc::save(&drm_device, &display_config)?;

    // Render first frame and set mode
    let init_bg = cfg.appearance.background_rgb();
    renderer.clear(init_bg.0, init_bg.1, init_bg.2, 1.0);
    egl_context.swap_buffers()?;

    let bo = gbm_surface.lock_front_buffer()?;
    let fb = drm::DrmFramebuffer::from_bo(&drm_device, &bo)?;
    drm::set_crtc(&drm_device, &display_config, &fb)?;

    info!("Phase 1 initialization complete");

    // Phase 2: Text rendering initialization (FreeType + LCD subpixel)
    let gl = renderer.gl();

    // Load font
    let font_data: &'static [u8] = if !cfg.font.main.is_empty() {
        Box::leak(
            std::fs::read(&cfg.font.main)
                .with_context(|| format!("Failed to read font: {}", cfg.font.main))?
                .into_boxed_slice(),
        )
    } else {
        Box::leak(
            font::atlas::load_system_font()
                .context("Failed to load font")?
                .into_boxed_slice(),
        )
    };

    // Apply display scale factor to font size
    let scale_factor = cfg.appearance.scale.max(0.5).min(4.0);
    let font_size = (cfg.font.size * scale_factor) as u32;
    if (scale_factor - 1.0).abs() > 0.01 {
        info!("Display scale: {}x (font size: {}pt → {}px)", scale_factor, cfg.font.size, font_size);
    }

    // Load CJK font (continue on failure)
    let cjk_font_data: Option<&[u8]> = if !cfg.font.cjk.is_empty() {
        std::fs::read(&cfg.font.cjk)
            .ok()
            .map(|d| -> &'static [u8] { Box::leak(d.into_boxed_slice()) })
    } else {
        font::atlas::load_cjk_font().map(|d| -> &'static [u8] { Box::leak(d.into_boxed_slice()) })
    };

    // LCD filter settings (from config)
    let lcd_filter = font::freetype::LcdFilterMode::from_str(&cfg.font.lcd_filter);
    let lcd_subpixel = font::freetype::LcdSubpixel::from_str(&cfg.font.lcd_subpixel);
    let lcd_mode = lcd_subpixel.to_lcd_mode();
    let subpixel_bgr = lcd_subpixel.is_bgr();
    let hinting_mode = font::freetype::HintingMode::from_str(&cfg.font.lcd_hinting);

    // Create FreeType + LCD subpixel rendering atlas
    let mut glyph_atlas = font::lcd_atlas::LcdGlyphAtlas::new(
        gl,
        font_data,
        font_size,
        cjk_font_data,
        lcd_mode,
        lcd_filter,
        cfg.font.lcd_weights,
        cfg.font.lcd_subpixel_positioning,
        hinting_mode,
    )
    .context("Failed to create LCD glyph atlas")?;

    info!(
        "FreeType LCD atlas initialized: cell={}x{:.0}, subpixel_pos={}, hinting={:?}",
        glyph_atlas.cell_width, glyph_atlas.cell_height,
        glyph_atlas.is_subpixel_positioning_enabled(),
        hinting_mode
    );

    // Create emoji atlas
    let emoji_font_path = if !cfg.font.emoji.is_empty() {
        Some(cfg.font.emoji.as_str())
    } else {
        Some("/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf")
    };
    let mut emoji_atlas =
        font::emoji::EmojiAtlas::new(emoji_font_path, glyph_atlas.cell_height as u32);
    if emoji_atlas.is_available() {
        info!("Emoji font loaded: {:?}", emoji_font_path);
    } else {
        info!("No emoji font (monochrome fallback)");
    }

    // Create LCD text renderer (FreeType + linear color space compositing)
    // No shaping: FreeType provides hinted glyphs
    let mut text_renderer =
        gpu::LcdTextRenderer::new(gl).context("Failed to initialize LCD text renderer")?;
    text_renderer.set_subpixel_bgr(subpixel_bgr);
    text_renderer.set_gamma(cfg.font.lcd_gamma);
    text_renderer.set_stem_darkening(cfg.font.lcd_stem_darkening);
    text_renderer.set_contrast(cfg.font.lcd_contrast);
    text_renderer.set_fringe_reduction(cfg.font.lcd_fringe_reduction);

    info!(
        "LCD subpixel settings: {:?} (BGR={}, gamma={:.2}, contrast={:.2}, fringe={:.2})",
        lcd_subpixel, subpixel_bgr, cfg.font.lcd_gamma, cfg.font.lcd_contrast, cfg.font.lcd_fringe_reduction
    );

    // Create UI renderer (rounded rectangles for candidate window, etc.)
    let mut ui_renderer = gpu::UiRenderer::new(gl).context("Failed to initialize UI renderer")?;

    // Create image renderer (Sixel image rendering, etc.)
    let mut image_renderer = gpu::ImageRenderer::new(gl).context("Failed to initialize image renderer")?;

    // Create emoji renderer (color emoji rendering)
    let mut emoji_renderer =
        gpu::EmojiRenderer::new(gl).context("Failed to initialize emoji renderer")?;

    // Create curly underline renderer (SDF + smoothstep anti-aliasing)
    let mut curly_renderer =
        gpu::CurlyRenderer::new(gl).context("Failed to initialize curly renderer")?;

    // Create FBO for cached rendering (enables partial updates)
    let fbo = gpu::Fbo::new(gl, display_config.width, display_config.height)
        .context("Failed to initialize FBO")?;

    info!("Phase 2 initialization complete");

    // Phase 3: Terminal initialization
    let screen_w = display_config.width;
    let screen_h = display_config.height;
    let mut cell_w = glyph_atlas.cell_width;
    let mut cell_h = glyph_atlas.cell_height;

    // Terminal margin (padding from screen edges)
    let margin_x = 8.0_f32;
    let margin_y = 8.0_f32;

    // Calculate grid size (accounting for margins)
    let mut grid_cols = ((screen_w as f32 - margin_x * 2.0) / cell_w) as usize;
    let mut grid_rows = ((screen_h as f32 - margin_y * 2.0) / cell_h) as usize;
    info!(
        "Grid size: {}x{} (cell: {:.0}x{:.0}px)",
        grid_cols, grid_rows, cell_w, cell_h
    );

    // Base font size (for reset)
    let base_font_size = font_size;

    let mut term = terminal::Terminal::with_scrollback(
        grid_cols,
        grid_rows,
        cfg.terminal.scrollback_lines,
        &cfg.terminal.term_env,
    )
    .context("Failed to initialize terminal")?;

    // Set cell size for Sixel image placement
    term.set_cell_size(cell_w as u32, cell_h as u32);

    // Set clipboard path
    term.set_clipboard_path(&cfg.paths.clipboard_file);

    // Display /etc/issue (like getty does) if running as root
    // Note: When using libseat (seatd feature), we don't run as root
    #[cfg(not(all(target_os = "linux", feature = "seatd")))]
    if unsafe { libc::getuid() } == 0 {
        let tty_name = format!("tty{}", vt_switcher.target_vt());
        if let Some(issue) = terminal::pty::read_issue(&tty_name) {
            // Process issue text through terminal to display it
            term.process_output(issue.as_bytes());
            // Ensure cursor is at column 0 for login prompt
            term.process_output(b"\r\n");
            info!("Displayed /etc/issue for {}", tty_name);
        }
    }

    // Phase 4: Keyboard input initialization
    let keyboard = input::Keyboard::new().context("Failed to initialize keyboard")?;

    // evdev input (keyboard + mouse, continue with SSH stdin if unavailable)
    #[cfg(all(target_os = "linux", feature = "seatd"))]
    let mut evdev_keyboard =
        match input::EvdevKeyboard::new_with_seat(
            display_config.width,
            display_config.height,
            seat_session.clone(),
            &cfg.keyboard,
        ) {
            Ok(kb) => {
                info!("evdev input initialized via libseat (keyboard + mouse)");
                Some(kb)
            }
            Err(e) => {
                info!("evdev input unavailable (continuing with SSH stdin): {}", e);
                None
            }
        };

    #[cfg(not(all(target_os = "linux", feature = "seatd")))]
    let mut evdev_keyboard =
        match input::EvdevKeyboard::new(display_config.width, display_config.height, &cfg.keyboard) {
            Ok(kb) => {
                info!("evdev input initialized (keyboard + mouse)");
                Some(kb)
            }
            Err(e) => {
                info!("evdev input unavailable (continuing with SSH stdin): {}", e);
                None
            }
        };

    info!("Phase 4 initialization complete");

    // Phase 5d: fcitx5 IME initialization (optional)
    let ime_client = match input::ime::ImeClient::try_new() {
        Ok(c) => {
            info!("fcitx5 IME connected");
            Some(c)
        }
        Err(e) => {
            info!("fcitx5 IME unavailable (continuing with direct input): {}", e);
            None
        }
    };
    let mut preedit = input::ime::PreeditState::new();
    let mut candidate_state: Option<input::ime::CandidateState> = None;

    info!("Terminal loop started");

    // Notify systemd that we're ready
    let _ = sd_notify::notify(true, &[sd_notify::NotifyState::Ready]);

    // Keep previous frame's BO/FB
    let mut prev_bo = Some(bo);
    let mut prev_fb = Some(fb);
    let mut key_buf = [0u8; 256];
    let mut needs_redraw = true;

    // Mouse state
    let mut mouse_selecting = false;
    let mut mouse_x: f64 = (screen_w / 2) as f64;
    let mut mouse_y: f64 = (screen_h / 2) as f64;
    let mut scroll_accum: f64 = 0.0;
    let mut mouse_button_held: Option<u8> = None;

    // Double/triple click detection
    let mut last_click_time = std::time::Instant::now();
    let mut click_count = 0u8;
    let mut last_click_col = 0usize;
    let mut last_click_row = 0usize;
    const DOUBLE_CLICK_MS: u128 = 300;

    // Bell flash
    let mut bell_flash_until: Option<std::time::Instant> = None;
    const BELL_FLASH_DURATION_MS: u64 = 100;

    // Cursor blink
    let mut cursor_blink_visible = true;
    let mut last_blink_toggle = std::time::Instant::now();
    const CURSOR_BLINK_INTERVAL_MS: u64 = 530; // ~530ms blink interval

    // Font size change request (currently log output only)
    let mut font_size_delta: i32 = 0;

    // Ctrl key state tracking (for URL click)
    let mut ctrl_pressed = false;

    // Search mode
    let mut search_mode = false;

    // Screenshot flag
    let mut take_screenshot = false;

    // IME enabled state (bcon-level toggle)
    let mut ime_enabled = true;
    // IME manual override (disable auto-switch when user explicitly toggles)
    let mut ime_manual_override = false;
    // Previous foreground process name (for change detection)
    let mut last_fg_process: Option<String> = None;
    // Last foreground process check time (rate limiting)
    let mut last_fg_check = std::time::Instant::now();
    const FG_CHECK_INTERVAL_MS: u64 = 200; // Check every 200ms

    // DRM master state (for VT switching)
    let mut drm_master_held = true;

    // Appearance colors (from config, may be overridden by OSC 10/11)
    let config_bg = cfg.appearance.background_rgb();
    let config_fg = cfg.appearance.foreground_rgb();
    let config_cursor = cfg.appearance.cursor_rgb();
    let config_selection = cfg.appearance.selection_rgb();
    let config_cursor_opacity = cfg.appearance.cursor_opacity;

    loop {
        // Poll for session events (VT switching)
        #[cfg(all(target_os = "linux", feature = "seatd"))]
        {
            // Dispatch libseat events
            if let Ok(true) = seat_session.borrow_mut().dispatch() {
                // Events dispatched, check for session state changes
            }

            // Process session events
            while let Some(event) = seat_session.borrow().try_recv_event() {
                match event {
                    session::SessionEvent::Disable => {
                        // Session disabled (VT switched away)
                        info!("libseat: session disabled");

                        // Send focus out event to terminal applications
                        if let Err(e) = term.send_focus_event(false) {
                            log::debug!("Failed to send FocusOut event: {}", e);
                        }

                        drm_master_held = false;
                        // Note: libseat handles DRM master automatically
                    }
                    session::SessionEvent::Enable => {
                        // Session enabled (VT acquired, or resume from suspend)
                        info!("libseat: session enabled");

                        drm_master_held = true;
                        needs_redraw = true;

                        // Invalidate GPU textures (may have been lost during suspend)
                        glyph_atlas.invalidate();
                        emoji_atlas.invalidate();
                        image_renderer.invalidate_all(gl);
                        info!("GPU textures invalidated for re-upload");

                        // Send focus in event to terminal applications
                        if let Err(e) = term.send_focus_event(true) {
                            log::debug!("Failed to send FocusIn event: {}", e);
                        }
                    }
                }
            }
        }

        #[cfg(not(all(target_os = "linux", feature = "seatd")))]
        {
            // Poll for VT switch signals (SIGUSR1/SIGUSR2 via signalfd)
            while let Some(event) = vt_switcher.poll() {
                match event {
                    drm::VtEvent::Release => {
                        // Kernel requests us to release the VT
                        info!("VT release requested");

                        // Send focus out event to terminal applications
                        if let Err(e) = term.send_focus_event(false) {
                            log::debug!("Failed to send FocusOut event: {}", e);
                        }

                        if drm_master_held {
                            if let Err(e) = drm_device.drop_master() {
                                log::warn!("Failed to drop DRM master: {}", e);
                            } else {
                                drm_master_held = false;
                            }
                        }
                        // Acknowledge release - this allows the VT switch to proceed
                        if let Err(e) = vt_switcher.ack_release() {
                            log::warn!("Failed to acknowledge VT release: {}", e);
                        }
                    }
                    drm::VtEvent::Acquire => {
                        // Kernel grants us the VT (or resume from suspend)
                        info!("VT acquire");
                        if !drm_master_held {
                            if let Err(e) = drm_device.set_master() {
                                log::warn!("Failed to acquire DRM master: {}", e);
                            } else {
                                drm_master_held = true;
                                needs_redraw = true;
                            }
                        }
                        // Acknowledge acquire
                        if let Err(e) = vt_switcher.ack_acquire() {
                            log::warn!("Failed to acknowledge VT acquire: {}", e);
                        }

                        // Invalidate GPU textures (may have been lost during suspend)
                        glyph_atlas.invalidate();
                        emoji_atlas.invalidate();
                        image_renderer.invalidate_all(gl);
                        info!("GPU textures invalidated for re-upload");

                        // Send focus in event to terminal applications
                        if let Err(e) = term.send_focus_event(true) {
                            log::debug!("Failed to send FocusIn event: {}", e);
                        }
                    }
                }
            }
        }

        // Check for SIGTERM (graceful shutdown from systemd)
        if drm::sigterm_received() {
            info!("Received SIGTERM, shutting down gracefully...");
            let _ = sd_notify::notify(true, &[sd_notify::NotifyState::Stopping]);
            break;
        }

        // Skip most processing if we don't have DRM master (VT switched away)
        if !drm_master_held {
            std::thread::sleep(Duration::from_millis(100));
            continue;
        }
        // Cursor blink update
        let now = std::time::Instant::now();
        if now.duration_since(last_blink_toggle).as_millis() >= CURSOR_BLINK_INTERVAL_MS as u128 {
            cursor_blink_visible = !cursor_blink_visible;
            last_blink_toggle = now;
            // Trigger redraw in blink mode
            if term.grid.cursor.blink {
                needs_redraw = true;
            }
        }

        // Check child process alive
        if !term.is_alive() {
            info!("Child process terminated");
            let _ = sd_notify::notify(true, &[sd_notify::NotifyState::Stopping]);
            break;
        }

        // Config hot-reload (Linux only)
        #[cfg(target_os = "linux")]
        if let Some(ref watcher) = config_watcher {
            if watcher.check_reload() {
                info!("Config file change detected, reloading...");
                let new_cfg = config::Config::load();

                // Re-parse keybinds
                kb_copy = config::ParsedKeybinds::parse(&new_cfg.keybinds.copy);
                kb_paste = config::ParsedKeybinds::parse(&new_cfg.keybinds.paste);
                kb_screenshot = config::ParsedKeybinds::parse(&new_cfg.keybinds.screenshot);
                kb_search = config::ParsedKeybinds::parse(&new_cfg.keybinds.search);
                kb_copy_mode = config::ParsedKeybinds::parse(&new_cfg.keybinds.copy_mode);
                kb_font_increase = config::ParsedKeybinds::parse(&new_cfg.keybinds.font_increase);
                kb_font_decrease = config::ParsedKeybinds::parse(&new_cfg.keybinds.font_decrease);
                kb_font_reset = config::ParsedKeybinds::parse(&new_cfg.keybinds.font_reset);
                kb_scroll_up = config::ParsedKeybinds::parse(&new_cfg.keybinds.scroll_up);
                kb_scroll_down = config::ParsedKeybinds::parse(&new_cfg.keybinds.scroll_down);
                kb_ime_toggle = config::ParsedKeybinds::parse(&new_cfg.keybinds.ime_toggle);

                // Update IME disable app list
                cfg = new_cfg;

                info!("Config reload complete");
                needs_redraw = true;
            }
        }

        // DRM hotplug detection (Linux only)
        #[cfg(target_os = "linux")]
        if let Some(ref mut monitor) = hotplug_monitor {
            if monitor.poll().is_some() {
                // Hotplug event detected - re-enumerate connectors
                if let Ok(new_snapshot) = drm::hotplug::snapshot_connectors(&drm_device) {
                    let changes = drm::hotplug::detect_changes(
                        &last_connector_snapshot,
                        &new_snapshot,
                    );

                    if changes.has_changes() {
                        changes.log();

                        // Check if we should switch displays
                        let should_switch = cfg.display.auto_switch && (
                            // External monitor connected
                            (cfg.display.prefer_external && changes.external_connected()) ||
                            // Current display disconnected
                            changes.disconnected.iter().any(|s| s.handle == display_config.connector_handle)
                        );

                        if should_switch {
                            // Try to switch to preferred display
                            match drm::DisplayConfig::detect_with_preference(&drm_device, cfg.display.prefer_external) {
                                Ok(new_config) => {
                                    if new_config.connector_handle != display_config.connector_handle {
                                        // Check if resolution changed
                                        if new_config.width == display_config.width
                                            && new_config.height == display_config.height
                                        {
                                            // Same resolution - update config, next frame will apply
                                            info!(
                                                "Switching display to {:?} (same resolution)",
                                                new_config.connector_handle
                                            );
                                            display_config = new_config;
                                            info!("Display switch scheduled (will apply on next frame)");
                                        } else {
                                            // Resolution changed - need restart
                                            log::warn!(
                                                "Display resolution changed ({}x{} -> {}x{}). Restart bcon to apply.",
                                                display_config.width, display_config.height,
                                                new_config.width, new_config.height
                                            );
                                        }
                                    }
                                }
                                Err(e) => {
                                    log::warn!("Failed to detect new display: {}", e);
                                }
                            }
                        }

                        last_connector_snapshot = new_snapshot;
                        needs_redraw = true;
                    }
                }
            }
        }

        // Read TTY stdin keyboard input and forward to PTY
        // Skip if evdev keyboard is available (prevent double input)
        if evdev_keyboard.is_none() {
            match keyboard.read(&mut key_buf) {
                Ok(0) => {}
                Ok(n) => {
                    let _ = term.write_to_pty(&key_buf[..n]);
                }
                Err(e) => {
                    log::warn!("Keyboard read error: {}", e);
                }
            }
        }

        // Process evdev input (keyboard + mouse)
        if let Some(ref mut evdev_kb) = evdev_keyboard {
            let (key_events, mouse_events) = evdev_kb.process_raw_events();

            // Keyboard event processing
            for raw in &key_events {
                // VT switching is now handled by VtSwitcher via kernel signals
                // (Ctrl+Alt+Fn triggers SIGUSR1/SIGUSR2 via VT_SETMODE)

                // Update Ctrl state (for URL click detection)
                ctrl_pressed = raw.mods_ctrl;

                if !raw.is_press {
                    // Only send release events when IME is enabled
                    if ime_enabled {
                        if let Some(ref ime) = ime_client {
                            ime.send_key(input::ime::ImeKeyEvent {
                                keysym: raw.keysym,
                                keycode: raw.keycode,
                                state: raw.xkb_state,
                                is_release: true,
                            });
                        }
                    }
                    continue;
                }

                let shift = raw.mods_shift;

                let ctrl = raw.mods_ctrl;
                let alt = raw.mods_alt;
                let keysym = raw.keysym;

                // Scroll up (configurable)
                if kb_scroll_up.matches(ctrl, shift, alt, raw.keycode, keysym) {
                    term.scroll_back(grid_rows / 2);
                    needs_redraw = true;
                    continue;
                }
                // Scroll down (configurable)
                if kb_scroll_down.matches(ctrl, shift, alt, raw.keycode, keysym) {
                    term.scroll_forward(grid_rows / 2);
                    needs_redraw = true;
                    continue;
                }

                // Copy (configurable)
                if kb_copy.matches(ctrl, shift, alt, raw.keycode, keysym) {
                    term.copy_selection();
                    needs_redraw = true;
                    continue;
                }

                // Paste (configurable)
                if kb_paste.matches(ctrl, shift, alt, raw.keycode, keysym) {
                    let _ = term.paste_clipboard();
                    needs_redraw = true;
                    continue;
                }

                // Screenshot (configurable)
                if kb_screenshot.matches(ctrl, shift, alt, raw.keycode, keysym) {
                    // Set screenshot flag (execute after rendering)
                    take_screenshot = true;
                    needs_redraw = true;
                    continue;
                }

                // Copy mode start (configurable)
                if kb_copy_mode.matches(ctrl, shift, alt, raw.keycode, keysym) {
                    if term.copy_mode.is_none() {
                        term.enter_copy_mode();
                    }
                    needs_redraw = true;
                    continue;
                }

                // Key input processing in copy mode
                if term.copy_mode.is_some() {
                    match raw.keysym {
                        // Escape: Exit copy mode
                        xkbcommon::xkb::keysyms::KEY_Escape => {
                            term.exit_copy_mode();
                        }
                        // h / Left arrow: Move left
                        xkbcommon::xkb::keysyms::KEY_h | xkbcommon::xkb::keysyms::KEY_Left => {
                            term.copy_mode_move(0, -1);
                        }
                        // j / Down arrow: Move down
                        xkbcommon::xkb::keysyms::KEY_j | xkbcommon::xkb::keysyms::KEY_Down => {
                            term.copy_mode_move(1, 0);
                        }
                        // k / Up arrow: Move up
                        xkbcommon::xkb::keysyms::KEY_k | xkbcommon::xkb::keysyms::KEY_Up => {
                            term.copy_mode_move(-1, 0);
                        }
                        // l / Right arrow: Move right
                        xkbcommon::xkb::keysyms::KEY_l | xkbcommon::xkb::keysyms::KEY_Right => {
                            term.copy_mode_move(0, 1);
                        }
                        // v: Toggle selection
                        xkbcommon::xkb::keysyms::KEY_v => {
                            term.copy_mode_toggle_selection();
                        }
                        // y: Yank (copy and exit)
                        xkbcommon::xkb::keysyms::KEY_y => {
                            term.copy_mode_yank();
                        }
                        // g: Go to top
                        xkbcommon::xkb::keysyms::KEY_g => {
                            term.copy_mode_goto_top();
                        }
                        // G: Go to bottom
                        xkbcommon::xkb::keysyms::KEY_G => {
                            term.copy_mode_goto_bottom();
                        }
                        // 0 / Home: Line start
                        xkbcommon::xkb::keysyms::KEY_0 | xkbcommon::xkb::keysyms::KEY_Home => {
                            term.copy_mode_goto_line_start();
                        }
                        // $ / End: Line end
                        xkbcommon::xkb::keysyms::KEY_dollar | xkbcommon::xkb::keysyms::KEY_End => {
                            term.copy_mode_goto_line_end();
                        }
                        // w: Word forward
                        xkbcommon::xkb::keysyms::KEY_w => {
                            term.copy_mode_word_forward();
                        }
                        // b: Word backward
                        xkbcommon::xkb::keysyms::KEY_b => {
                            term.copy_mode_word_backward();
                        }
                        // Ctrl+u: Half page up
                        xkbcommon::xkb::keysyms::KEY_u if ctrl => {
                            term.copy_mode_page_up();
                        }
                        // Ctrl+d: Half page down
                        xkbcommon::xkb::keysyms::KEY_d if ctrl => {
                            term.copy_mode_page_down();
                        }
                        // /: Enter search mode
                        xkbcommon::xkb::keysyms::KEY_slash => {
                            search_mode = true;
                            term.start_search();
                        }
                        _ => {}
                    }
                    needs_redraw = true;
                    continue;
                }

                // Search mode start (configurable)
                if kb_search.matches(ctrl, shift, alt, raw.keycode, keysym) {
                    if !search_mode {
                        search_mode = true;
                        term.start_search();
                        info!("Search mode started");
                    }
                    needs_redraw = true;
                    continue;
                }

                // Key input processing in search mode
                if search_mode {
                    if let Some(ref mut search) = term.search {
                        let has_matches = !search.matches.is_empty();

                        match raw.keysym {
                            // Escape: End search
                            xkbcommon::xkb::keysyms::KEY_Escape => {
                                search_mode = false;
                                term.end_search();
                                info!("Search mode ended");
                            }
                            // Enter: Execute search
                            xkbcommon::xkb::keysyms::KEY_Return
                            | xkbcommon::xkb::keysyms::KEY_KP_Enter => {
                                term.execute_search();
                                term.scroll_to_current_match();
                                // If in copy mode, end search and move cursor to match position
                                if term.copy_mode.is_some() {
                                    if let Some(ref s) = term.search {
                                        if !s.matches.is_empty() {
                                            let (abs_row, start_col, _) =
                                                s.matches[s.current_match];
                                            let scrollback_len = term.grid.scrollback_len();
                                            // Convert to display coordinates
                                            if abs_row >= scrollback_len {
                                                let display_row = abs_row - scrollback_len;
                                                if let Some(ref mut cm) = term.copy_mode {
                                                    cm.cursor_row = display_row;
                                                    cm.cursor_col = start_col;
                                                }
                                            }
                                        }
                                    }
                                    search_mode = false;
                                    term.end_search();
                                }
                            }
                            // Backspace: Delete one character
                            xkbcommon::xkb::keysyms::KEY_BackSpace => {
                                search.query.pop();
                                // Clear matches when query changes
                                search.matches.clear();
                            }
                            // n / N: Move between matches (only after search executed)
                            xkbcommon::xkb::keysyms::KEY_n | xkbcommon::xkb::keysyms::KEY_N
                                if has_matches =>
                            {
                                if shift {
                                    search.prev_match();
                                } else {
                                    search.next_match();
                                }
                                term.scroll_to_current_match();
                            }
                            // Normal character: Add to query
                            _ => {
                                if !raw.utf8.is_empty() && !ctrl {
                                    search.query.push_str(&raw.utf8);
                                    // Clear matches when query changes
                                    search.matches.clear();
                                }
                            }
                        }
                    }
                    needs_redraw = true;
                    continue;
                }

                // Font size increase (configurable)
                if kb_font_increase.matches(ctrl, shift, alt, raw.keycode, keysym) {
                    font_size_delta += 2;
                    let new_size = (base_font_size as f32 + font_size_delta as f32).max(8.0).min(72.0);
                    let (new_cell_w, new_cell_h) = glyph_atlas.resize(new_size);
                    cell_w = new_cell_w;
                    cell_h = new_cell_h;
                    grid_cols = ((screen_w as f32 - margin_x * 2.0) / cell_w) as usize;
                    grid_rows = ((screen_h as f32 - margin_y * 2.0) / cell_h) as usize;
                    term.resize(grid_cols, grid_rows);
                    term.set_cell_size(cell_w as u32, cell_h as u32);
                    emoji_atlas.resize(cell_h as u32);
                    needs_redraw = true;
                    continue;
                }

                // Font size decrease (configurable)
                if kb_font_decrease.matches(ctrl, shift, alt, raw.keycode, keysym) {
                    font_size_delta -= 2;
                    let new_size = (base_font_size as f32 + font_size_delta as f32).max(8.0).min(72.0);
                    let (new_cell_w, new_cell_h) = glyph_atlas.resize(new_size);
                    cell_w = new_cell_w;
                    cell_h = new_cell_h;
                    grid_cols = ((screen_w as f32 - margin_x * 2.0) / cell_w) as usize;
                    grid_rows = ((screen_h as f32 - margin_y * 2.0) / cell_h) as usize;
                    term.resize(grid_cols, grid_rows);
                    term.set_cell_size(cell_w as u32, cell_h as u32);
                    emoji_atlas.resize(cell_h as u32);
                    needs_redraw = true;
                    continue;
                }

                // Font size reset (configurable)
                if kb_font_reset.matches(ctrl, shift, alt, raw.keycode, keysym) {
                    font_size_delta = 0;
                    let (new_cell_w, new_cell_h) = glyph_atlas.resize(base_font_size as f32);
                    cell_w = new_cell_w;
                    cell_h = new_cell_h;
                    grid_cols = ((screen_w as f32 - margin_x * 2.0) / cell_w) as usize;
                    grid_rows = ((screen_h as f32 - margin_y * 2.0) / cell_h) as usize;
                    term.resize(grid_cols, grid_rows);
                    term.set_cell_size(cell_w as u32, cell_h as u32);
                    emoji_atlas.resize(cell_h as u32);
                    needs_redraw = true;
                    continue;
                }

                // Shift+Arrow: Text selection
                if shift && matches!(raw.keycode, 105 | 106 | 103 | 108 | 102 | 107) {
                    handle_selection_key(&mut term, raw.keycode, grid_cols);
                    needs_redraw = true;
                    continue;
                }

                // Clear selection on non-modified key input
                if term.selection.is_some() && !shift {
                    term.selection = None;
                    needs_redraw = true;
                }

                // IME toggle (configurable)
                if kb_ime_toggle.matches(ctrl, shift, alt, raw.keycode, keysym) {
                    ime_enabled = !ime_enabled;
                    ime_manual_override = true; // Manual override: disable auto-switch
                    info!("IME {} (manual)", if ime_enabled { "enabled" } else { "disabled" });
                    // Visual feedback with bell flash
                    bell_flash_until = Some(
                        std::time::Instant::now() + Duration::from_millis(BELL_FLASH_DURATION_MS),
                    );
                    needs_redraw = true;
                    continue;
                }

                // Process other keys normally
                // When IME enabled, send to fcitx5; when disabled, send directly to PTY
                if ime_enabled {
                    if let Some(ref ime) = ime_client {
                        ime.send_key(input::ime::ImeKeyEvent {
                            keysym: raw.keysym,
                            keycode: raw.keycode,
                            state: raw.xkb_state,
                            is_release: false,
                        });
                    } else {
                        // Send directly to PTY when IME client is unavailable
                        let sym = xkbcommon::xkb::Keysym::new(raw.keysym);
                        let kb_config = input::KeyboardConfig {
                            application_cursor_keys: term.grid.modes.application_cursor_keys,
                            modify_other_keys: term.grid.keyboard.modify_other_keys,
                            kitty_flags: term.grid.keyboard.kitty_flags,
                        };
                        let bytes = input::keysym_to_bytes_with_mods(
                            sym,
                            &raw.utf8,
                            raw.mods_ctrl,
                            raw.mods_alt,
                            raw.mods_shift,
                            &kb_config,
                        );
                        if !bytes.is_empty() {
                            let _ = term.write_to_pty(&bytes);
                        }
                    }
                } else {
                    // Send directly to PTY when IME is disabled
                    let sym = xkbcommon::xkb::Keysym::new(raw.keysym);
                    let kb_config = input::KeyboardConfig {
                        application_cursor_keys: term.grid.modes.application_cursor_keys,
                        modify_other_keys: term.grid.keyboard.modify_other_keys,
                        kitty_flags: term.grid.keyboard.kitty_flags,
                    };
                    let bytes = input::keysym_to_bytes_with_mods(
                        sym,
                        &raw.utf8,
                        raw.mods_ctrl,
                        raw.mods_alt,
                        raw.mods_shift,
                        &kb_config,
                    );
                    if !bytes.is_empty() {
                        let _ = term.write_to_pty(&bytes);
                    }
                }
            }

            // Mouse event processing
            for mouse in &mouse_events {
                match mouse {
                    input::MouseEvent::ButtonPress { button, x, y } => {
                        let col = (*x / cell_w as f64) as usize;
                        let row = (*y / cell_h as f64) as usize;

                        // Ctrl+Left click: Copy URL to clipboard
                        if ctrl_pressed && *button == input::BTN_LEFT {
                            let clamped_row = row.min(grid_rows - 1);
                            let clamped_col = col.min(grid_cols - 1);
                            if let Some(url) = term.detect_url_at(clamped_row, clamped_col) {
                                term.copy_url_to_clipboard(&url);
                                // Visual feedback with bell flash
                                bell_flash_until = Some(
                                    std::time::Instant::now()
                                        + Duration::from_millis(BELL_FLASH_DURATION_MS),
                                );
                            }
                            needs_redraw = true;
                            continue;
                        }

                        // Send to PTY if mouse mode is enabled
                        if term.mouse_mode_enabled() {
                            let btn = match *button {
                                input::BTN_LEFT => 0,
                                input::BTN_MIDDLE => 1,
                                input::BTN_RIGHT => 2,
                                _ => 0,
                            };
                            let _ = term.send_mouse_press(
                                btn,
                                col.min(grid_cols - 1),
                                row.min(grid_rows - 1),
                            );
                            mouse_button_held = Some(btn);
                        } else if *button == input::BTN_LEFT {
                            let now = std::time::Instant::now();
                            let elapsed = now.duration_since(last_click_time).as_millis();
                            let same_pos = col == last_click_col && row == last_click_row;

                            // Double/triple click detection
                            if elapsed < DOUBLE_CLICK_MS && same_pos {
                                click_count = (click_count + 1).min(3);
                            } else {
                                click_count = 1;
                            }
                            last_click_time = now;
                            last_click_col = col;
                            last_click_row = row;

                            let clamped_row = row.min(grid_rows - 1);
                            let clamped_col = col.min(grid_cols - 1);

                            match click_count {
                                2 => {
                                    // Double click: Word selection
                                    term.select_word(clamped_row, clamped_col);
                                    mouse_selecting = false;
                                }
                                3 => {
                                    // Triple click: Line selection
                                    term.select_line(clamped_row);
                                    mouse_selecting = false;
                                }
                                _ => {
                                    // Single click: Start normal selection
                                    term.selection = Some(terminal::Selection {
                                        anchor_row: clamped_row,
                                        anchor_col: clamped_col,
                                        end_row: clamped_row,
                                        end_col: clamped_col,
                                    });
                                    mouse_selecting = true;
                                }
                            }
                        }

                        if *button == input::BTN_MIDDLE && !term.mouse_mode_enabled() {
                            // Middle button: Paste (only when mouse mode is disabled)
                            let _ = term.paste_clipboard();
                        }
                        needs_redraw = true;
                    }
                    input::MouseEvent::Move { x, y } => {
                        let col = (*x / cell_w as f64) as usize;
                        let row = (*y / cell_h as f64) as usize;

                        // Send move event if mouse mode is enabled
                        if term.mouse_mode_enabled() {
                            let _ = term.send_mouse_move(
                                col.min(grid_cols - 1),
                                row.min(grid_rows - 1),
                                mouse_button_held,
                            );
                        } else if mouse_selecting {
                            // Dragging: Update selection range
                            if let Some(ref mut sel) = term.selection {
                                sel.end_row = row.min(grid_rows - 1);
                                sel.end_col = col.min(grid_cols - 1);
                            }
                            needs_redraw = true;
                        }
                        mouse_x = *x;
                        mouse_y = *y;
                    }
                    input::MouseEvent::ButtonRelease { button, x, y } => {
                        let col = (*x / cell_w as f64) as usize;
                        let row = (*y / cell_h as f64) as usize;

                        // Send to PTY if mouse mode is enabled
                        if term.mouse_mode_enabled() {
                            let btn = match *button {
                                input::BTN_LEFT => 0,
                                input::BTN_MIDDLE => 1,
                                input::BTN_RIGHT => 2,
                                _ => 0,
                            };
                            let _ = term.send_mouse_release(
                                btn,
                                col.min(grid_cols - 1),
                                row.min(grid_rows - 1),
                            );
                        } else if *button == input::BTN_LEFT {
                            // Left button release: Confirm selection + auto copy
                            mouse_selecting = false;
                            if term.selection.is_some() {
                                term.copy_selection();
                            }
                        }
                        // Always reset button state
                        mouse_button_held = None;
                    }
                    input::MouseEvent::Scroll { delta, x, y } => {
                        let col = (*x / cell_w as f64) as usize;
                        let row = (*y / cell_h as f64) as usize;

                        // Send to PTY if ButtonEvent/AnyEvent + SGR is enabled
                        // Don't send wheel events in X10 mode
                        let use_mouse_wheel = term.mouse_mode_enabled()
                            && term.grid.modes.mouse_mode != terminal::grid::MouseMode::X10;

                        if use_mouse_wheel {
                            let d = if *delta < 0.0 { -1i8 } else { 1i8 };
                            let _ = term.send_mouse_wheel(
                                d,
                                col.min(grid_cols - 1),
                                row.min(grid_rows - 1),
                            );
                        } else {
                            // Scroll accumulation (negative=up/history, positive=down/live)
                            scroll_accum += delta;
                            // Execute scroll when accumulated to 1 line
                            while scroll_accum >= 1.0 {
                                term.scroll_forward(1);
                                scroll_accum -= 1.0;
                                needs_redraw = true;
                            }
                            while scroll_accum <= -1.0 {
                                term.scroll_back(1);
                                scroll_accum += 1.0;
                                needs_redraw = true;
                            }
                        }
                    }
                }
            }

            if !key_events.is_empty() || !mouse_events.is_empty() {
                needs_redraw = true;
            }
        }

        // Process IME events
        if let Some(ref ime) = ime_client {
            for event in ime.poll_events() {
                match event {
                    input::ime::ImeEvent::Commit(text) => {
                        info!("IME Commit: {:?}", text);
                        let _ = term.write_to_pty(text.as_bytes());
                        preedit.clear();
                        candidate_state = None;
                        needs_redraw = true;
                    }
                    input::ime::ImeEvent::Preedit { segments, cursor } => {
                        let pe_text: String = segments.iter().map(|s| s.text.as_str()).collect();
                        let formats: Vec<i32> = segments.iter().map(|s| s.format).collect();
                        trace!(
                            "IME Preedit: {:?} formats={:?} cursor={}",
                            pe_text, formats, cursor
                        );
                        preedit.segments = segments;
                        preedit.cursor = cursor;
                        needs_redraw = true;
                    }
                    input::ime::ImeEvent::PreeditClear => {
                        trace!("IME PreeditClear");
                        preedit.clear();
                        needs_redraw = true;
                    }
                    input::ime::ImeEvent::ForwardKey {
                        keysym, state, is_release,
                    } => {
                        if !is_release {
                            // Extract modifiers from xkb state
                            let mods_ctrl = (state & 0x4) != 0;  // Control
                            let mods_alt = (state & 0x8) != 0;   // Mod1 (Alt)
                            let mods_shift = (state & 0x1) != 0; // Shift

                            let sym = xkbcommon::xkb::Keysym::new(keysym);
                            let utf8 = xkbcommon::xkb::keysym_to_utf8(sym);
                            let kb_config = input::KeyboardConfig {
                                application_cursor_keys: term.grid.modes.application_cursor_keys,
                                modify_other_keys: term.grid.keyboard.modify_other_keys,
                                kitty_flags: term.grid.keyboard.kitty_flags,
                            };
                            let bytes = input::keysym_to_bytes_with_mods(
                                sym,
                                &utf8,
                                mods_ctrl,
                                mods_alt,
                                mods_shift,
                                &kb_config,
                            );
                            if !bytes.is_empty() {
                                let _ = term.write_to_pty(&bytes);
                                needs_redraw = true;
                            }
                        }
                    }
                    input::ime::ImeEvent::UpdateCandidates(state) => {
                        trace!(
                            "IME Candidates: {} items, selected={}",
                            state.candidates.len(),
                            state.selected_index
                        );
                        candidate_state = Some(state);
                        needs_redraw = true;
                    }
                    input::ime::ImeEvent::ClearCandidates => {
                        trace!("IME ClearCandidates");
                        candidate_state = None;
                        needs_redraw = true;
                    }
                }
            }
        }

        // Process PTY output
        let mut total_read = 0;
        loop {
            match term.process_pty_output() {
                Ok(0) => break,
                Ok(n) => total_read += n,
                Err(e) => {
                    log::warn!("PTY read error: {}", e);
                    break;
                }
            }
        }

        if total_read > 0 {
            needs_redraw = true;
            // Return to live position on new output
            term.scroll_to_bottom();
            // Clear selection on new output
            term.selection = None;

            // Start flash if bell notification
            if term.grid.bell_triggered {
                term.grid.bell_triggered = false;
                bell_flash_until =
                    Some(std::time::Instant::now() + Duration::from_millis(BELL_FLASH_DURATION_MS));
            }
        }

        // Continue redraw if bell flash is active
        if let Some(until) = bell_flash_until {
            if std::time::Instant::now() < until {
                needs_redraw = true;
            } else {
                bell_flash_until = None;
            }
        }

        // Focus events are sent when VT switch signals are processed above
        // (VtSwitcher handles SIGUSR1/SIGUSR2 for acquire/release)

        // IME auto-switch based on foreground process
        // Rate limited (considering command execution cost like tmux display-message)
        let now = std::time::Instant::now();
        if !cfg.terminal.ime_disabled_apps.is_empty()
            && now.duration_since(last_fg_check).as_millis() >= FG_CHECK_INTERVAL_MS as u128
        {
            last_fg_check = now;
            let current_fg = term.foreground_process_name();
            if current_fg != last_fg_process {
                // Reset manual override when process changes
                if ime_manual_override {
                    ime_manual_override = false;
                    info!("IME returned to auto mode");
                }

                if let Some(ref proc_name) = current_fg {
                    // Check if process name is in the list
                    let should_disable =
                        cfg.terminal.ime_disabled_apps.iter().any(|app| {
                            proc_name == app || proc_name.ends_with(&format!("/{}", app))
                        });

                    let new_ime_state = !should_disable;
                    if ime_enabled != new_ime_state {
                        ime_enabled = new_ime_state;
                        info!(
                            "IME auto-{}: {} ({})",
                            if ime_enabled { "enabled" } else { "disabled" },
                            proc_name,
                            if should_disable {
                                "disabled app"
                            } else {
                                "normal"
                            }
                        );
                    }
                }
                last_fg_process = current_fg;
            }
        }

        // Sleep briefly if no changes (reduce CPU load)
        if !needs_redraw {
            std::thread::sleep(Duration::from_millis(8));
            continue;
        }
        needs_redraw = false;

        // Render screen to FBO
        // Use OSC 11 dynamic background if set, otherwise config background
        let bg_color = if let Some((r, g, b)) = term.grid.colors.bg {
            (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0)
        } else {
            config_bg
        };

        // Bind FBO for rendering
        fbo.bind(gl);

        // Clear FBO: if all dirty or overlays active, clear everything; otherwise clear only dirty rows
        // Selection/search overlays require full clear because they change independently of content
        let has_overlays = term.selection.is_some() || search_mode || term.copy_mode.is_some();
        if term.grid.is_all_dirty() || has_overlays {
            fbo.clear(gl, bg_color.0, bg_color.1, bg_color.2, 1.0);
        } else if term.has_dirty_rows() {
            // Clear only dirty rows using scissor test
            for row in 0..term.grid.rows() {
                if term.grid.is_row_dirty(row) {
                    fbo.clear_rows(gl, row, row, cell_h, margin_y, bg_color.0, bg_color.1, bg_color.2);
                }
            }
        }
        // If no rows are dirty and no overlays, FBO content is preserved from previous frame

        text_renderer.begin();
        text_renderer.set_bg_color(bg_color.0, bg_color.1, bg_color.2);
        emoji_renderer.begin();
        curly_renderer.begin();

        // Effective foreground color: OSC 10 > config > hardcoded white
        let default_fg = if let Some((r, g, b)) = term.grid.colors.fg {
            [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0]
        } else {
            [config_fg.0, config_fg.1, config_fg.2, 1.0]
        };

        // Helper to get foreground color, using default_fg for Color::Default
        let effective_fg = |color: &terminal::grid::Color| -> [f32; 4] {
            if matches!(color, terminal::grid::Color::Default) {
                default_fg
            } else {
                color.to_rgba(true)
            }
        };

        let grid = &term.grid;
        let ascent = glyph_atlas.ascent;

        // Determine if partial rendering is possible (no overlays active)
        // When overlays are active, we must render all rows for correctness
        let partial_render = !has_overlays && !grid.is_all_dirty();

        // === Pass 1: Background color (run-length encoded) ===
        // Combine consecutive cells with same background into single rectangles
        for row in 0..grid.rows() {
            // Skip non-dirty rows when partial rendering is enabled
            if partial_render && !grid.is_row_dirty(row) {
                continue;
            }
            let y = margin_y + row as f32 * cell_h;
            let mut run_start: Option<(usize, [f32; 4])> = None;

            for col in 0..grid.cols() {
                let cell = term.display_cell(row, col);

                // Skip continuation cells (width==0)
                if cell.width == 0 {
                    continue;
                }

                let bg = cell.bg.to_rgba(false);

                if let Some((start, run_color)) = run_start {
                    if bg == run_color {
                        // Continue the run
                        continue;
                    } else {
                        // Flush previous run
                        if run_color[3] > 0.0 {
                            let x = margin_x + start as f32 * cell_w;
                            let w = (col - start) as f32 * cell_w;
                            text_renderer.push_rect(x, y, w, cell_h, run_color, &glyph_atlas);
                        }
                        // Start new run
                        run_start = Some((col, bg));
                    }
                } else {
                    // Start first run
                    run_start = Some((col, bg));
                }
            }

            // Flush final run
            if let Some((start, run_color)) = run_start {
                if run_color[3] > 0.0 {
                    let x = margin_x + start as f32 * cell_w;
                    let w = (grid.cols() - start) as f32 * cell_w;
                    text_renderer.push_rect(x, y, w, cell_h, run_color, &glyph_atlas);
                }
            }
        }

        // === Pass 1.5: Selection highlight (one rect per row) ===
        let selection_color = [config_selection.0, config_selection.1, config_selection.2, 0.35];
        if let Some(ref sel) = term.selection {
            let max_cols = grid.cols();
            for row in 0..grid.rows() {
                // Get column range for this row
                if let Some((start_col, end_col)) = sel.cols_for_row(row, max_cols) {
                    let x = margin_x + start_col as f32 * cell_w;
                    let y = margin_y + row as f32 * cell_h;
                    let w = (end_col - start_col) as f32 * cell_w;
                    text_renderer.push_rect(x, y, w, cell_h, selection_color, &glyph_atlas);
                }
            }
        }

        // === Pass 1.6: Search match highlight (one rect per match) ===
        if search_mode {
            let current_match = term.current_search_match();
            for row in 0..grid.rows() {
                let matches = term.get_search_matches_for_display_row(row);
                for &(start_col, end_col, match_idx) in matches {
                    let clamped_end = end_col.min(grid.cols());
                    if start_col >= clamped_end {
                        continue;
                    }
                    let x = margin_x + start_col as f32 * cell_w;
                    let y = margin_y + row as f32 * cell_h;
                    let w = (clamped_end - start_col) as f32 * cell_w;
                    let is_current = current_match == Some(match_idx);
                    let color = if is_current {
                        [1.0, 0.6, 0.0, 0.5] // Current match: orange
                    } else {
                        [1.0, 1.0, 0.0, 0.3] // Others: yellow
                    };
                    text_renderer.push_rect(x, y, w, cell_h, color, &glyph_atlas);
                }
            }
        }

        // === Pass 2: Text rendering (FreeType LCD mode) ===
        let max_cols = grid.cols();

        for row in 0..grid.rows() {
            // Skip non-dirty rows when partial rendering is enabled
            if partial_render && !grid.is_row_dirty(row) {
                continue;
            }

            // Pre-compute selection range for this row (avoids per-cell normalized() call)
            let row_selection = term.selection.as_ref().and_then(|s| s.cols_for_row(row, max_cols));

            // Pre-compute search matches for this row (avoids per-cell function call)
            // Returns slice reference - no allocation needed
            let row_search_matches = if search_mode {
                term.get_search_matches_for_display_row(row)
            } else {
                &[][..]
            };
            let current_match_idx = term.current_search_match();

            // Underline rendering: batch consecutive cells with same style into runs
            let mut col = 0;
            while col < grid.cols() {
                let cell = if term.scroll_offset > 0 {
                    term.display_cell(row, col)
                } else {
                    grid.cell(row, col)
                };

                // Skip width=0 (continuation cells)
                if cell.width == 0 {
                    col += 1;
                    continue;
                }

                // Skip if no underline
                let has_underline = cell.underline_style != terminal::grid::UnderlineStyle::None
                    || cell.hyperlink.is_some();
                if !has_underline {
                    col += 1;
                    continue;
                }

                // Run start: group consecutive cells with same style and color
                let run_start = col;
                let run_style = cell.underline_style;
                let run_color = cell
                    .underline_color
                    .map(|c| c.to_rgba(true))
                    .unwrap_or_else(|| effective_fg(&cell.fg));
                let run_has_hyperlink = cell.hyperlink.is_some();

                // Find run end
                col += 1;
                while col < grid.cols() {
                    let next_cell = if term.scroll_offset > 0 {
                        term.display_cell(row, col)
                    } else {
                        grid.cell(row, col)
                    };
                    if next_cell.width == 0 {
                        col += 1;
                        continue;
                    }
                    let next_style = next_cell.underline_style;
                    let next_color = next_cell
                        .underline_color
                        .map(|c| c.to_rgba(true))
                        .unwrap_or_else(|| effective_fg(&next_cell.fg));
                    let next_has_hyperlink = next_cell.hyperlink.is_some();

                    // Continue run while style, color, and hyperlink match
                    if next_style == run_style
                        && next_color == run_color
                        && next_has_hyperlink == run_has_hyperlink
                    {
                        col += 1;
                    } else {
                        break;
                    }
                }
                let run_end = col;
                let run_len = run_end - run_start;

                // Run coordinates (including margins)
                let run_x = margin_x + run_start as f32 * cell_w;
                let run_w = run_len as f32 * cell_w;
                let run_y = margin_y + row as f32 * cell_h;
                let underline_y = run_y + cell_h - 2.0;

                // Draw according to underline style
                match run_style {
                    terminal::grid::UnderlineStyle::None => {
                        // Single line for hyperlinks
                        if run_has_hyperlink {
                            text_renderer.push_rect(
                                run_x, underline_y, run_w, 1.0, run_color, &glyph_atlas,
                            );
                        }
                    }
                    terminal::grid::UnderlineStyle::Single => {
                        text_renderer.push_rect(
                            run_x, underline_y, run_w, 1.0, run_color, &glyph_atlas,
                        );
                    }
                    terminal::grid::UnderlineStyle::Double => {
                        text_renderer.push_rect(
                            run_x, underline_y - 2.0, run_w, 1.0, run_color, &glyph_atlas,
                        );
                        text_renderer.push_rect(
                            run_x, underline_y, run_w, 1.0, run_color, &glyph_atlas,
                        );
                    }
                    terminal::grid::UnderlineStyle::Curly => {
                        // Curly line: draw with SDF shader (anti-aliased)
                        let baseline_y = (run_y + ascent).round();
                        let desc = (cell_h - ascent).max(2.0);

                        // Allowed line: below the upper half of descender
                        let allowed_top = baseline_y + desc * 0.5;

                        let mut amplitude = (desc * 0.40).clamp(1.2, 2.0);
                        let mut thickness = (desc * 0.12).clamp(0.6, 0.9);
                        if thickness > amplitude * 0.5 {
                            thickness = amplitude * 0.5;
                        }

                        let wavelength = (cell_w * 1.3).clamp(6.0, 9.0);

                        // Place base position near cell bottom
                        let mut base_y = run_y + cell_h - 1.0;
                        let mut wave_top = base_y - amplitude - thickness;

                        // If wave top is above allowed line, push down base_y
                        if wave_top < allowed_top {
                            let delta = allowed_top - wave_top;
                            base_y += delta;
                            wave_top += delta;
                        }

                        // Reduce amplitude if overflowing bottom
                        let max_base = run_y + cell_h - 1.0;
                        if base_y > max_base {
                            let overshoot = base_y - max_base;
                            amplitude = (amplitude - overshoot).max(1.0);
                            base_y = max_base;
                            wave_top = base_y - amplitude - thickness;
                        }

                        base_y = base_y.floor() + 0.5;

                        let wave_height = (amplitude + thickness) * 2.0 + 2.0;

                        curly_renderer.push_curly(
                            run_x,
                            wave_top,
                            run_w,
                            wave_height,
                            run_color,
                            amplitude,
                            wavelength,
                            thickness,
                            base_y,
                        );
                    }
                    terminal::grid::UnderlineStyle::Dotted => {
                        let dot_size = 1.0;
                        let gap = 2.0;
                        let mut dx = run_x;
                        while dx < run_x + run_w {
                            text_renderer.push_rect(
                                dx, underline_y, dot_size, 1.0, run_color, &glyph_atlas,
                            );
                            dx += dot_size + gap;
                        }
                    }
                    terminal::grid::UnderlineStyle::Dashed => {
                        let dash_len = 4.0_f32;
                        let gap = 2.0_f32;
                        let mut dx = run_x;
                        while dx < run_x + run_w {
                            text_renderer.push_rect(
                                dx,
                                underline_y,
                                dash_len.min(run_x + run_w - dx),
                                1.0,
                                run_color,
                                &glyph_atlas,
                            );
                            dx += dash_len + gap;
                        }
                    }
                }
            }

            // Character-based rendering (FreeType LCD mode) + overline/strikethrough
            // Combined into single loop to reduce grid traversals
            for col in 0..grid.cols() {
                let cell = term.display_cell(row, col);
                if cell.width == 0 {
                    continue;
                }

                let x = margin_x + col as f32 * cell_w;
                let y = margin_y + row as f32 * cell_h;

                // Overline rendering (CSI 53 m)
                if cell.attrs.contains(terminal::grid::CellAttrs::OVERLINE) {
                    let fg = effective_fg(&cell.fg);
                    text_renderer.push_rect(x, y, cell_w, 1.0, fg, &glyph_atlas);
                }

                // Strikethrough rendering (CSI 9 m)
                if cell.attrs.contains(terminal::grid::CellAttrs::STRIKE) {
                    let fg = effective_fg(&cell.fg);
                    let strike_y = y + cell_h / 2.0;
                    text_renderer.push_rect(x, strike_y, cell_w, 1.0, fg, &glyph_atlas);
                }

                let grapheme = &cell.grapheme;
                if !grapheme.is_empty() && grapheme != " " {
                    let fg = effective_fg(&cell.fg);

                    // Calculate final background color (needed for LCD subpixel compositing)
                    // Composite in order: cell BG -> selection highlight -> search highlight
                    // Blend in linear space to match shader
                    let mut bg_rgba = cell.bg.to_rgba(false);

                    // Selection highlight - blend in linear space
                    if let Some((sel_start, sel_end)) = row_selection {
                        if col >= sel_start && col < sel_end {
                            let a = selection_color[3];
                            // sRGB -> linear
                            let bg_lin = [
                                srgb_to_linear(bg_rgba[0]),
                                srgb_to_linear(bg_rgba[1]),
                                srgb_to_linear(bg_rgba[2]),
                            ];
                            let sel_lin = [
                                srgb_to_linear(selection_color[0]),
                                srgb_to_linear(selection_color[1]),
                                srgb_to_linear(selection_color[2]),
                            ];
                            // Blend in linear space
                            let blended_lin = [
                                sel_lin[0] * a + bg_lin[0] * (1.0 - a),
                                sel_lin[1] * a + bg_lin[1] * (1.0 - a),
                                sel_lin[2] * a + bg_lin[2] * (1.0 - a),
                            ];
                            // Linear -> sRGB
                            bg_rgba[0] = linear_to_srgb(blended_lin[0]);
                            bg_rgba[1] = linear_to_srgb(blended_lin[1]);
                            bg_rgba[2] = linear_to_srgb(blended_lin[2]);
                        }
                    }

                    // Search highlight - blend in linear space (uses pre-computed matches)
                    if search_mode {
                        for &(start_col, end_col, match_idx) in row_search_matches {
                            if col >= start_col && col < end_col.min(grid.cols()) {
                                let is_current = current_match_idx == Some(match_idx);
                                let hl_color = if is_current {
                                    [1.0, 0.6, 0.0, 0.5] // Orange
                                } else {
                                    [1.0, 1.0, 0.0, 0.3] // Yellow
                                };
                                let a = hl_color[3];
                                // sRGB -> linear
                                let bg_lin = [
                                    srgb_to_linear(bg_rgba[0]),
                                    srgb_to_linear(bg_rgba[1]),
                                    srgb_to_linear(bg_rgba[2]),
                                ];
                                let hl_lin = [
                                    srgb_to_linear(hl_color[0]),
                                    srgb_to_linear(hl_color[1]),
                                    srgb_to_linear(hl_color[2]),
                                ];
                                // Blend in linear space
                                let blended_lin = [
                                    hl_lin[0] * a + bg_lin[0] * (1.0 - a),
                                    hl_lin[1] * a + bg_lin[1] * (1.0 - a),
                                    hl_lin[2] * a + bg_lin[2] * (1.0 - a),
                                ];
                                // Linear -> sRGB
                                bg_rgba[0] = linear_to_srgb(blended_lin[0]);
                                bg_rgba[1] = linear_to_srgb(blended_lin[1]);
                                bg_rgba[2] = linear_to_srgb(blended_lin[2]);
                                break;
                            }
                        }
                    }

                    let bg = [bg_rgba[0], bg_rgba[1], bg_rgba[2]];
                    let first_char = cell.ch();

                    // Emoji check (judge by entire grapheme)
                    let is_emoji_grapheme = grapheme.chars().any(|c| font::emoji::is_emoji(c));
                    let mut emoji_drawn = false;
                    if is_emoji_grapheme && emoji_atlas.is_available() {
                        // Search by entire grapheme (supports flags and ZWJ sequences)
                        if let Some(info) =
                            emoji_atlas.ensure_grapheme(gl, grapheme, cell_h as u32)
                        {
                            let emoji_size = cell_h;
                            let cell_span_w = cell_w * 2.0;
                            let emoji_x = x + (cell_span_w - emoji_size) / 2.0;
                            emoji_renderer.push_emoji(
                                emoji_x, y, emoji_size, emoji_size, info.uv_x, info.uv_y,
                                info.uv_w, info.uv_h,
                            );
                            emoji_drawn = true;
                        } else if let Some(first_emoji) =
                            grapheme.chars().find(|c| font::emoji::is_emoji(*c))
                        {
                            // Retry with first emoji if entire ZWJ sequence not found
                            if let Some(info) = emoji_atlas.ensure_glyph(first_emoji, cell_h as u32)
                            {
                                let emoji_size = cell_h;
                                let cell_span_w = cell_w * 2.0;
                                let emoji_x = x + (cell_span_w - emoji_size) / 2.0;
                                emoji_renderer.push_emoji(
                                    emoji_x, y, emoji_size, emoji_size, info.uv_x, info.uv_y,
                                    info.uv_w, info.uv_h,
                                );
                                emoji_drawn = true;
                            }
                        }
                    }
                    if !emoji_drawn {
                        glyph_atlas.ensure_glyph(first_char);
                        let baseline_y = (y + ascent).round();
                        text_renderer.push_char_with_bg(first_char, x, baseline_y, fg, bg, &glyph_atlas);
                    }
                }
            }
        }

        // === Pass 3: Sixel image rendering ===
        image_renderer.begin();
        for placement in &term.grid.image_placements {
            if let Some(image) = term.images.get(placement.id) {
                // Upload texture if not already uploaded
                if !image_renderer.has_texture(placement.id) {
                    image_renderer.upload_image(gl, image);
                }
                // Drawing coordinates (accounting for scroll offset)
                let img_row = placement.row as isize - term.scroll_offset as isize;
                if img_row + placement.height_cells as isize <= 0 {
                    continue; // Off-screen (above)
                }
                if img_row >= grid_rows as isize {
                    continue; // Off-screen (below)
                }
                let x = margin_x + placement.col as f32 * cell_w;
                let y = margin_y + img_row as f32 * cell_h;
                image_renderer.draw(placement.id, x, y, image.width as f32, image.height as f32);
            }
        }
        image_renderer.flush(gl, screen_w, screen_h);

        // === Pass 3.5: Emoji rendering ===
        emoji_atlas.upload_if_dirty(gl);
        emoji_renderer.flush(gl, &emoji_atlas, screen_w, screen_h);

        // === Pass 3.6: Curly line rendering (SDF + smoothstep) ===
        curly_renderer.flush(gl, screen_w, screen_h);

        // === Preedit rendering (IME composition text) ===
        // Hide preedit and cursor during scrollback display
        let preedit_total_cols = if term.scroll_offset > 0 {
            0
        } else if !preedit.is_empty() {
            let pe_col = grid.cursor_col;
            let pe_row = grid.cursor_row;
            let pe_y = margin_y + pe_row as f32 * cell_h;

            // Draw in order: background -> text per segment
            // fcitx5 format flags: 8=Underline(composing), 16=Highlight(conversion target)

            // Pass A: Background rectangles for conversion target segments
            let mut offset_col = 0usize;
            for seg in &preedit.segments {
                let is_highlight = (seg.format & 16) != 0;
                let seg_cols: usize = seg
                    .text
                    .chars()
                    .map(|ch| unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1))
                    .sum();

                if is_highlight {
                    let seg_x = margin_x + (pe_col + offset_col) as f32 * cell_w;
                    let seg_w = seg_cols as f32 * cell_w;
                    text_renderer.push_rect(
                        seg_x,
                        pe_y,
                        seg_w,
                        cell_h,
                        [0.7, 0.7, 0.7, 1.0],
                        &glyph_atlas,
                    );
                }
                offset_col += seg_cols;
            }
            let total_cols = offset_col;

            // Pass B: Text
            offset_col = 0;
            for seg in &preedit.segments {
                let is_highlight = (seg.format & 16) != 0;
                let fg = if is_highlight {
                    [0.0, 0.0, 0.0, 1.0] // Black (on white background)
                } else {
                    [1.0, 1.0, 1.0, 1.0] // White
                };

                let seg_start = offset_col;
                for ch in seg.text.chars() {
                    let char_x = margin_x + (pe_col + offset_col) as f32 * cell_w;
                    let baseline_y = (pe_y + ascent).round();
                    let ch_w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1);

                    glyph_atlas.ensure_glyph(ch);
                    text_renderer.push_char(ch, char_x, baseline_y, fg, &glyph_atlas);
                    offset_col += ch_w;
                }

                // Underline for composing segment
                if !is_highlight {
                    let ul_x = margin_x + (pe_col + seg_start) as f32 * cell_w;
                    let ul_w = (offset_col - seg_start) as f32 * cell_w;
                    let ul_y = pe_y + cell_h - 2.0;
                    text_renderer.push_rect(
                        ul_x,
                        ul_y,
                        ul_w,
                        2.0,
                        [1.0, 1.0, 1.0, 1.0],
                        &glyph_atlas,
                    );
                }
            }

            total_cols
        } else {
            0
        };

        // Draw cursor (white rectangular block)
        // Only show copy mode cursor during copy mode
        if let Some(ref cm) = term.copy_mode {
            // Copy mode cursor (yellow outline)
            let cursor_x = margin_x + cm.cursor_col as f32 * cell_w;
            let cursor_y = margin_y + cm.cursor_row as f32 * cell_h;
            let border = 2.0_f32;
            // Top edge
            text_renderer.push_rect(
                cursor_x,
                cursor_y,
                cell_w,
                border,
                [1.0, 0.8, 0.0, 0.9],
                &glyph_atlas,
            );
            // Bottom edge
            text_renderer.push_rect(
                cursor_x,
                cursor_y + cell_h - border,
                cell_w,
                border,
                [1.0, 0.8, 0.0, 0.9],
                &glyph_atlas,
            );
            // Left edge
            text_renderer.push_rect(
                cursor_x,
                cursor_y,
                border,
                cell_h,
                [1.0, 0.8, 0.0, 0.9],
                &glyph_atlas,
            );
            // Right edge
            text_renderer.push_rect(
                cursor_x + cell_w - border,
                cursor_y,
                border,
                cell_h,
                [1.0, 0.8, 0.0, 0.9],
                &glyph_atlas,
            );
        } else if term.scroll_offset == 0 && grid.modes.cursor_visible {
            // Normal cursor (hidden during scrollback display)
            // In blink mode, toggle visibility with cursor_blink_visible
            let should_draw = !grid.cursor.blink || cursor_blink_visible;

            if should_draw {
                // Display at end of preedit when composing
                let cursor_x = margin_x + (grid.cursor_col + preedit_total_cols) as f32 * cell_w;
                let cursor_y = margin_y + grid.cursor_row as f32 * cell_h;
                let cursor_color = [config_cursor.0, config_cursor.1, config_cursor.2, config_cursor_opacity];

                // Draw according to cursor style
                match grid.cursor.style {
                terminal::grid::CursorStyle::Block => {
                    text_renderer.push_rect(
                        cursor_x,
                        cursor_y,
                        cell_w,
                        cell_h,
                        cursor_color,
                        &glyph_atlas,
                    );
                }
                terminal::grid::CursorStyle::Underline => {
                    let underline_h = 2.0;
                    text_renderer.push_rect(
                        cursor_x,
                        cursor_y + cell_h - underline_h,
                        cell_w,
                        underline_h,
                        cursor_color,
                        &glyph_atlas,
                    );
                }
                terminal::grid::CursorStyle::Bar => {
                    let bar_w = 2.0;
                    text_renderer.push_rect(
                        cursor_x,
                        cursor_y,
                        bar_w,
                        cell_h,
                        cursor_color,
                        &glyph_atlas,
                    );
                }
                }
            }
        }

        // Draw mouse cursor (small white rectangle)
        {
            let cursor_size = 8.0_f32;
            let mx = mouse_x as f32;
            let my = mouse_y as f32;
            // Crosshair style
            text_renderer.push_rect(
                mx - cursor_size / 2.0,
                my - 1.0,
                cursor_size,
                2.0,
                [1.0, 1.0, 1.0, 0.9],
                &glyph_atlas,
            );
            text_renderer.push_rect(
                mx - 1.0,
                my - cursor_size / 2.0,
                2.0,
                cursor_size,
                [1.0, 1.0, 1.0, 0.9],
                &glyph_atlas,
            );
        }

        // === Pass 1: Text rendering (grid + preedit + cursor) ===
        glyph_atlas.upload_if_dirty(gl);
        text_renderer.flush(gl, &glyph_atlas, screen_w, screen_h);

        // === Candidate window rendering (3 passes: UI background -> candidate text) ===
        if let Some(ref cands) = candidate_state {
            if !cands.candidates.is_empty() {
                // Layout constants
                let padding = 8.0_f32;
                let item_gap = 4.0_f32;
                let corner_radius = 6.0_f32;
                let highlight_radius = 4.0_f32;

                // Calculate candidate text width
                let mut max_label_w = 0.0_f32;
                let mut max_text_w = 0.0_f32;
                for (label, text) in &cands.candidates {
                    let label_cols: usize = label
                        .chars()
                        .map(|ch| unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1))
                        .sum();
                    let text_cols: usize = text
                        .chars()
                        .map(|ch| unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1))
                        .sum();
                    max_label_w = max_label_w.max(label_cols as f32 * cell_w);
                    max_text_w = max_text_w.max(text_cols as f32 * cell_w);
                }
                let label_text_gap = cell_w; // Gap between label and text
                let win_w = padding * 2.0 + max_label_w + label_text_gap + max_text_w;
                let item_h = cell_h + item_gap;
                let num_cands = cands.candidates.len() as f32;
                // Page indicator row height (if has_prev/has_next)
                let indicator_h = if cands.has_prev || cands.has_next {
                    cell_h
                } else {
                    0.0
                };
                let win_h = padding * 2.0 + num_cands * item_h - item_gap + indicator_h;

                // Window position: directly below preedit
                let pe_col = grid.cursor_col;
                let pe_row = grid.cursor_row;
                let mut win_x = pe_col as f32 * cell_w;
                let mut win_y = (pe_row + 1) as f32 * cell_h;

                // Screen edge overflow correction
                if win_x + win_w > screen_w as f32 {
                    win_x = (screen_w as f32 - win_w).max(0.0);
                }
                if win_y + win_h > screen_h as f32 {
                    // Display above
                    win_y = (pe_row as f32 * cell_h - win_h).max(0.0);
                }

                // Pass 2: UI background rendering (rounded rectangle)
                ui_renderer.begin();

                // Drop shadow + window background
                ui_renderer.push_shadow_rounded_rect(
                    win_x,
                    win_y,
                    win_w,
                    win_h,
                    corner_radius,
                    [0.15, 0.15, 0.18, 0.95], // Window background
                    3.0,                      // Shadow offset
                    [0.0, 0.0, 0.0, 0.4],     // Shadow color
                );

                // Selection highlight
                if cands.selected_index >= 0
                    && (cands.selected_index as usize) < cands.candidates.len()
                {
                    let sel_y = win_y + padding + cands.selected_index as f32 * item_h;
                    ui_renderer.push_rounded_rect(
                        win_x + padding * 0.5,
                        sel_y,
                        win_w - padding,
                        cell_h,
                        highlight_radius,
                        [0.3, 0.45, 0.7, 0.9], // Selection highlight color
                    );
                }

                ui_renderer.flush(gl, screen_w, screen_h);

                // Pass 3: Candidate text rendering
                text_renderer.begin();

                for (i, (label, text)) in cands.candidates.iter().enumerate() {
                    let is_selected = i as i32 == cands.selected_index;
                    let item_y = win_y + padding + i as f32 * item_h;
                    let baseline_y = (item_y + ascent).round();

                    // Label color
                    let label_color = if is_selected {
                        [0.8, 0.85, 0.9, 1.0]
                    } else {
                        [0.5, 0.55, 0.6, 1.0]
                    };
                    // Text color
                    let text_color = if is_selected {
                        [1.0, 1.0, 1.0, 1.0]
                    } else {
                        [0.92, 0.92, 0.95, 1.0]
                    };

                    // Label rendering
                    let label_x = win_x + padding;
                    for ch in label.chars() {
                        glyph_atlas.ensure_glyph(ch);
                    }
                    text_renderer.push_text(label, label_x, baseline_y, label_color, &glyph_atlas);

                    // Text rendering
                    let text_x = win_x + padding + max_label_w + label_text_gap;
                    for ch in text.chars() {
                        glyph_atlas.ensure_glyph(ch);
                    }
                    text_renderer.push_text(text, text_x, baseline_y, text_color, &glyph_atlas);
                }

                // Page indicator
                if cands.has_prev || cands.has_next {
                    let ind_y = win_y + padding + num_cands * item_h;
                    let baseline_y = (ind_y + ascent).round();
                    let indicator = match (cands.has_prev, cands.has_next) {
                        (true, true) => "< >",
                        (true, false) => "<",
                        (false, true) => ">",
                        _ => "",
                    };
                    if !indicator.is_empty() {
                        for ch in indicator.chars() {
                            glyph_atlas.ensure_glyph(ch);
                        }
                        // Right-align
                        let ind_cols: usize = indicator
                            .chars()
                            .map(|ch| unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1))
                            .sum();
                        let ind_x = win_x + win_w - padding - ind_cols as f32 * cell_w;
                        text_renderer.push_text(
                            indicator,
                            ind_x,
                            baseline_y,
                            [0.5, 0.55, 0.6, 1.0],
                            &glyph_atlas,
                        );
                    }
                }

                glyph_atlas.upload_if_dirty(gl);
                text_renderer.flush(gl, &glyph_atlas, screen_w, screen_h);
            }
        }

        // === Copy mode status (screen bottom) ===
        if term.copy_mode.is_some() && !search_mode {
            let bar_h = cell_h + 8.0;
            let bar_y = screen_h as f32 - bar_h;
            let padding = 8.0_f32;

            // Background
            ui_renderer.begin();
            ui_renderer.push_rounded_rect(
                0.0,
                bar_y,
                screen_w as f32,
                bar_h,
                0.0,
                [0.2, 0.15, 0.1, 0.95], // Warm color
            );
            ui_renderer.flush(gl, screen_w, screen_h);

            // Text
            text_renderer.begin();

            let status = if term
                .copy_mode
                .as_ref()
                .map(|cm| cm.selecting)
                .unwrap_or(false)
            {
                "[VISUAL] v:toggle  y:yank  hjkl:move  /:search  Esc:exit"
            } else {
                "[COPY] v:select  y:yank  hjkl:move  /:search  Esc:exit"
            };

            for ch in status.chars() {
                glyph_atlas.ensure_glyph(ch);
            }
            text_renderer.push_text(
                status,
                padding,
                bar_y + 4.0 + ascent,
                [1.0, 0.9, 0.7, 1.0],
                &glyph_atlas,
            );

            glyph_atlas.upload_if_dirty(gl);
            text_renderer.flush(gl, &glyph_atlas, screen_w, screen_h);
        }

        // === Search bar (screen bottom) ===
        if search_mode {
            if let Some(ref search) = term.search {
                let bar_h = cell_h + 8.0;
                let bar_y = screen_h as f32 - bar_h;
                let padding = 8.0_f32;

                // Background
                ui_renderer.begin();
                ui_renderer.push_rounded_rect(
                    0.0,
                    bar_y,
                    screen_w as f32,
                    bar_h,
                    0.0,
                    [0.15, 0.15, 0.2, 0.95],
                );
                ui_renderer.flush(gl, screen_w, screen_h);

                // Text
                text_renderer.begin();

                // Prompt "/"
                let prompt = "/";
                for ch in prompt.chars() {
                    glyph_atlas.ensure_glyph(ch);
                }
                text_renderer.push_text(
                    prompt,
                    padding,
                    bar_y + 4.0 + ascent,
                    [0.6, 0.6, 0.6, 1.0],
                    &glyph_atlas,
                );

                // Search query
                let query_x = padding + cell_w;
                for ch in search.query.chars() {
                    glyph_atlas.ensure_glyph(ch);
                }
                text_renderer.push_text(
                    &search.query,
                    query_x,
                    bar_y + 4.0 + ascent,
                    [1.0, 1.0, 1.0, 1.0],
                    &glyph_atlas,
                );

                // Cursor
                let query_cols: usize = search
                    .query
                    .chars()
                    .map(|ch| unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1))
                    .sum();
                let cursor_x = query_x + query_cols as f32 * cell_w;
                text_renderer.push_rect(
                    cursor_x,
                    bar_y + 4.0,
                    2.0,
                    cell_h,
                    [1.0, 1.0, 1.0, 0.8],
                    &glyph_atlas,
                );

                // Match count display
                if !search.matches.is_empty() {
                    let match_info =
                        format!("{}/{}", search.current_match + 1, search.matches.len());
                    for ch in match_info.chars() {
                        glyph_atlas.ensure_glyph(ch);
                    }
                    let info_cols: usize = match_info
                        .chars()
                        .map(|ch| unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1))
                        .sum();
                    let info_x = screen_w as f32 - padding - info_cols as f32 * cell_w;
                    text_renderer.push_text(
                        &match_info,
                        info_x,
                        bar_y + 4.0 + ascent,
                        [0.7, 0.7, 0.7, 1.0],
                        &glyph_atlas,
                    );
                }

                glyph_atlas.upload_if_dirty(gl);
                text_renderer.flush(gl, &glyph_atlas, screen_w, screen_h);
            }
        }

        // === Bell flash (visual bell) ===
        if bell_flash_until.is_some() {
            // Light up screen edges (border only)
            let border = 4.0;
            let color = [1.0, 0.6, 0.2, 0.8]; // Orange
            text_renderer.begin();
            // Top
            text_renderer.push_rect(0.0, 0.0, screen_w as f32, border, color, &glyph_atlas);
            // Bottom
            text_renderer.push_rect(0.0, screen_h as f32 - border, screen_w as f32, border, color, &glyph_atlas);
            // Left
            text_renderer.push_rect(0.0, border, border, screen_h as f32 - border * 2.0, color, &glyph_atlas);
            // Right
            text_renderer.push_rect(screen_w as f32 - border, border, border, screen_h as f32 - border * 2.0, color, &glyph_atlas);
            glyph_atlas.upload_if_dirty(gl);
            text_renderer.flush(gl, &glyph_atlas, screen_w, screen_h);
        }

        // === Screenshot ===
        if take_screenshot {
            take_screenshot = false;
            let user_home = term.user_home_dir();
            if let Err(e) = save_screenshot(gl, screen_w, screen_h, &cfg.paths.screenshot_dir, user_home.as_deref()) {
                log::warn!("Screenshot save failed: {}", e);
            } else {
                // Flash on success
                bell_flash_until =
                    Some(std::time::Instant::now() + Duration::from_millis(BELL_FLASH_DURATION_MS));
            }
        }

        // Unbind FBO and blit to screen
        fbo.unbind(gl);
        fbo.blit_to_screen(gl, screen_w, screen_h);

        // Buffer swap (skip during Synchronized Update mode or when VT switched away)
        // CSI ? 2026 h starts buffering, CSI ? 2026 l displays all at once
        if !term.is_synchronized_update() && drm_master_held {
            egl_context.swap_buffers()?;

            // Get front buffer and display to DRM
            let new_bo = gbm_surface.lock_front_buffer()?;
            let new_fb = drm::DrmFramebuffer::from_bo(&drm_device, &new_bo)?;
            drm::set_crtc(&drm_device, &display_config, &new_fb)?;

            // Release previous frame
            prev_bo = Some(new_bo);
            prev_fb = Some(new_fb);

            // Clear dirty flags after successful render
            term.clear_dirty();
        }
    }

    // Keyboard restoration (automatic on drop)
    drop(keyboard);

    // Resource cleanup
    fbo.destroy(gl);
    emoji_renderer.destroy(gl);
    image_renderer.destroy(gl);
    ui_renderer.destroy(gl);
    curly_renderer.destroy(gl);
    text_renderer.destroy(gl);
    glyph_atlas.destroy(gl);
    emoji_atlas.destroy(gl);

    // Restore previous mode
    drop(prev_fb);
    drop(prev_bo);
    saved_crtc.restore(&drm_device, display_config.crtc_handle);

    // Delete clipboard file
    let _ = std::fs::remove_file(&cfg.paths.clipboard_file);

    info!("bcon terminated");
    Ok(())
}
