//! GPU context management
//!
//! GBM + EGL + OpenGL ES setup

use anyhow::{anyhow, Context, Result};
use gbm::AsRaw;
use glow::HasContext;
use khronos_egl as egl;
use log::info;
use std::ffi::c_void;

// EGL_PLATFORM_GBM_KHR (EGL extension)
const EGL_PLATFORM_GBM_KHR: egl::Enum = 0x31D7;

/// GBM device
pub struct GbmDevice {
    device: gbm::Device<std::fs::File>,
}

impl GbmDevice {
    /// Create GBM device from DRM file descriptor
    pub fn new(drm_file: std::fs::File) -> Result<Self> {
        let device = gbm::Device::new(drm_file)
            .map_err(|e| anyhow!("Failed to create GBM device: {:?}", e))?;
        info!("GBM device created");
        Ok(Self { device })
    }

    /// Reference to internal device
    pub fn device(&self) -> &gbm::Device<std::fs::File> {
        &self.device
    }
}

/// GBM surface
#[allow(dead_code)]
pub struct GbmSurface {
    surface: gbm::Surface<std::fs::File>,
    width: u32,
    height: u32,
}

impl GbmSurface {
    /// Create GBM surface
    pub fn new(device: &gbm::Device<std::fs::File>, width: u32, height: u32) -> Result<Self> {
        let surface = device
            .create_surface::<std::fs::File>(
                width,
                height,
                gbm::Format::Argb8888,
                gbm::BufferObjectFlags::SCANOUT | gbm::BufferObjectFlags::RENDERING,
            )
            .map_err(|e| anyhow!("Failed to create GBM surface: {:?}", e))?;

        info!("GBM surface created: {}x{}", width, height);
        Ok(Self {
            surface,
            width,
            height,
        })
    }

    /// Reference to internal surface
    pub fn surface(&self) -> &gbm::Surface<std::fs::File> {
        &self.surface
    }

    /// Lock front buffer and get buffer object
    pub fn lock_front_buffer(&self) -> Result<gbm::BufferObject<std::fs::File>> {
        unsafe {
            self.surface
                .lock_front_buffer()
                .map_err(|e| anyhow!("Failed to lock front buffer: {:?}", e))
        }
    }

    #[allow(dead_code)]
    pub fn width(&self) -> u32 {
        self.width
    }

    #[allow(dead_code)]
    pub fn height(&self) -> u32 {
        self.height
    }
}

/// EGL instance type (dynamic loading)
type EglInstance = egl::Instance<egl::Dynamic<libloading::Library, egl::EGL1_5>>;

/// EGL context
pub struct EglContext {
    instance: EglInstance,
    display: egl::Display,
    context: egl::Context,
    surface: egl::Surface,
    #[allow(dead_code)]
    config: egl::Config,
}

impl EglContext {
    /// Initialize EGL with GBM platform
    pub fn new(
        gbm_device: &gbm::Device<std::fs::File>,
        gbm_surface: &gbm::Surface<std::fs::File>,
    ) -> Result<Self> {
        // Load EGL library
        let lib = unsafe {
            libloading::Library::new("libEGL.so.1")
                .or_else(|_| libloading::Library::new("libEGL.so"))
                .context("Failed to load EGL library")?
        };

        let instance: EglInstance = unsafe {
            egl::DynamicInstance::<egl::EGL1_5>::load_required_from(lib)
                .context("Failed to create EGL instance")?
        };

        // Get display with GBM platform
        let display = unsafe {
            instance
                .get_platform_display(
                    EGL_PLATFORM_GBM_KHR,
                    gbm_device.as_raw() as *mut c_void,
                    &[egl::ATTRIB_NONE],
                )
                .context("Failed to get EGL display")?
        };

        // Initialize EGL
        instance
            .initialize(display)
            .context("Failed to initialize EGL")?;

        // Get version info
        if let Ok(version_str) = instance.query_string(Some(display), egl::VERSION) {
            info!("EGL version: {}", version_str.to_string_lossy());
        }

        // Bind OpenGL ES API
        instance
            .bind_api(egl::OPENGL_ES_API)
            .context("Failed to bind OpenGL ES API")?;

        // Choose config (try ES3, fallback to ES2)
        let config = Self::choose_config(&instance, display, egl::OPENGL_ES3_BIT)
            .or_else(|_| Self::choose_config(&instance, display, egl::OPENGL_ES2_BIT))
            .context("Failed to choose EGL config")?;

        // Create context (try ES3, fallback to ES2)
        let context_attribs_es3 = [egl::CONTEXT_CLIENT_VERSION, 3, egl::NONE];
        let context_attribs_es2 = [egl::CONTEXT_CLIENT_VERSION, 2, egl::NONE];
        let context = instance
            .create_context(display, config, None, &context_attribs_es3)
            .or_else(|_| instance.create_context(display, config, None, &context_attribs_es2))
            .context("Failed to create EGL context")?;

        // Create window surface (wrap GBM surface)
        // Try create_platform_window_surface, fallback to create_window_surface
        let surface = unsafe {
            instance
                .create_platform_window_surface(
                    display,
                    config,
                    gbm_surface.as_raw() as *mut c_void,
                    &[egl::ATTRIB_NONE],
                )
                .or_else(|_| {
                    instance.create_window_surface(
                        display,
                        config,
                        gbm_surface.as_raw() as egl::NativeWindowType,
                        None,
                    )
                })
                .context("Failed to create EGL surface")?
        };

        // Make context current
        instance
            .make_current(display, Some(surface), Some(surface), Some(context))
            .context("Failed to make EGL context current")?;

        info!("EGL context created");

        Ok(Self {
            instance,
            display,
            context,
            surface,
            config,
        })
    }

    /// Swap buffers
    pub fn swap_buffers(&self) -> Result<()> {
        self.instance
            .swap_buffers(self.display, self.surface)
            .context("Failed to swap buffers")?;
        Ok(())
    }

    /// Choose EGL config
    fn choose_config(
        instance: &EglInstance,
        display: egl::Display,
        renderable_type: egl::Int,
    ) -> Result<egl::Config> {
        let config_attribs = [
            egl::SURFACE_TYPE,
            egl::WINDOW_BIT,
            egl::RED_SIZE,
            8,
            egl::GREEN_SIZE,
            8,
            egl::BLUE_SIZE,
            8,
            egl::ALPHA_SIZE,
            8,
            egl::DEPTH_SIZE,
            0,
            egl::RENDERABLE_TYPE,
            renderable_type,
            egl::NONE,
        ];

        instance
            .choose_first_config(display, &config_attribs)
            .context("choose_first_config failed")?
            .ok_or_else(|| anyhow!("No suitable EGL config found"))
    }

    /// Load GL function pointers
    pub fn get_proc_address(&self, name: &str) -> *const c_void {
        self.instance
            .get_proc_address(name)
            .map(|f| f as *const c_void)
            .unwrap_or(std::ptr::null())
    }
}

impl Drop for EglContext {
    fn drop(&mut self) {
        let _ = self.instance.make_current(self.display, None, None, None);
        let _ = self.instance.destroy_surface(self.display, self.surface);
        let _ = self.instance.destroy_context(self.display, self.context);
        let _ = self.instance.terminate(self.display);
    }
}

/// OpenGL ES version
#[derive(Clone, Copy, Debug)]
pub struct GlEsVersion {
    pub major: u32,
    pub minor: u32,
}

impl GlEsVersion {
    /// Parse version from GL_VERSION string (e.g., "OpenGL ES 3.1 Mesa 23.0.0")
    fn parse(version_str: &str) -> Self {
        // Look for "ES X.Y" pattern
        let default = Self { major: 3, minor: 0 };

        let es_pos = version_str.find("ES ");
        if let Some(pos) = es_pos {
            let after_es = &version_str[pos + 3..];
            let version_part: String = after_es
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();

            let parts: Vec<&str> = version_part.split('.').collect();
            if parts.len() >= 2 {
                if let (Ok(major), Ok(minor)) = (parts[0].parse(), parts[1].parse()) {
                    return Self { major, minor };
                }
            }
        }

        default
    }

    /// Check if ES 3.1+ (supports buffer_storage)
    pub fn supports_buffer_storage(&self) -> bool {
        self.major > 3 || (self.major == 3 && self.minor >= 1)
    }
}

/// OpenGL ES renderer
pub struct GlRenderer {
    gl: glow::Context,
    es_version: GlEsVersion,
}

impl GlRenderer {
    /// Initialize OpenGL ES from EGL context
    pub fn new(egl: &EglContext) -> Result<Self> {
        let gl = unsafe { glow::Context::from_loader_function(|name| egl.get_proc_address(name)) };

        // Display OpenGL ES info and parse version
        let es_version = unsafe {
            let version = gl.get_parameter_string(glow::VERSION);
            let renderer = gl.get_parameter_string(glow::RENDERER);
            let vendor = gl.get_parameter_string(glow::VENDOR);
            info!("OpenGL ES: {}", version);
            info!("Renderer: {}", renderer);
            info!("Vendor: {}", vendor);

            let es_ver = GlEsVersion::parse(&version);
            info!(
                "Detected ES {}.{} (buffer_storage: {})",
                es_ver.major,
                es_ver.minor,
                es_ver.supports_buffer_storage()
            );
            es_ver
        };

        Ok(Self { gl, es_version })
    }

    /// Clear screen (fill with solid color)
    pub fn clear(&self, r: f32, g: f32, b: f32, a: f32) {
        unsafe {
            self.gl.clear_color(r, g, b, a);
            self.gl.clear(glow::COLOR_BUFFER_BIT);
        }
    }

    /// Set viewport
    pub fn set_viewport(&self, x: i32, y: i32, width: i32, height: i32) {
        unsafe {
            self.gl.viewport(x, y, width, height);
        }
    }

    /// Reference to glow context
    #[allow(dead_code)]
    pub fn gl(&self) -> &glow::Context {
        &self.gl
    }

    /// Get ES version
    pub fn es_version(&self) -> GlEsVersion {
        self.es_version
    }
}
