//! DRM display management
//!
//! Mode setting and page flip handling

use anyhow::{anyhow, bail, Context, Result};
use drm::control::{connector, crtc, framebuffer, Device as ControlDevice, Mode};
use log::{debug, info};

use super::device::Device;

/// Display configuration
pub struct DisplayConfig {
    pub connector_handle: connector::Handle,
    pub crtc_handle: crtc::Handle,
    pub mode: Mode,
    pub width: u32,
    pub height: u32,
}

impl DisplayConfig {
    /// Auto-detect connected display and get configuration
    pub fn auto_detect(device: &Device) -> Result<Self> {
        let (connector_handle, connector_info) = device.find_connected_connector()?;

        info!(
            "Connector: {:?}, type: {:?}",
            connector_handle,
            connector_info.interface()
        );

        let (crtc_handle, _crtc_info) = device.find_crtc_for_connector(&connector_info)?;
        info!("CRTC: {:?}", crtc_handle);

        let modes = connector_info.modes();
        if modes.is_empty() {
            bail!("No available display modes");
        }

        let mode = modes
            .iter()
            .find(|m| {
                m.mode_type()
                    .contains(drm::control::ModeTypeFlags::PREFERRED)
            })
            .or_else(|| modes.first())
            .cloned()
            .ok_or_else(|| anyhow!("Failed to select display mode"))?;

        let (width, height) = mode.size();
        info!(
            "Display mode: {}x{} @ {}Hz",
            width,
            height,
            mode.vrefresh()
        );

        Ok(Self {
            connector_handle,
            crtc_handle,
            mode,
            width: width as u32,
            height: height as u32,
        })
    }
}

/// DRM framebuffer management
pub struct DrmFramebuffer {
    device_fd: std::os::unix::io::RawFd,
    fb: framebuffer::Handle,
}

impl DrmFramebuffer {
    /// Create framebuffer from GBM BO
    pub fn from_bo<T>(device: &Device, bo: &gbm::BufferObject<T>) -> Result<Self> {
        let raw_handle = bo
            .handle()
            .map_err(|e| anyhow!("Failed to get BO handle: {:?}", e))?;
        let handle = unsafe { raw_handle.s32 } as u32;
        let width = bo.width().map_err(|e| anyhow!("{:?}", e))?;
        let height = bo.height().map_err(|e| anyhow!("{:?}", e))?;
        let stride = bo.stride().map_err(|e| anyhow!("{:?}", e))?;

        // Add framebuffer directly via DRM_IOCTL_MODE_ADDFB
        let fb = unsafe {
            let mut fb_cmd = drm_mode_fb_cmd {
                fb_id: 0,
                width,
                height,
                pitch: stride,
                bpp: 32,
                depth: 24,
                handle,
            };

            let ret = libc::ioctl(
                device.as_raw_fd(),
                DRM_IOCTL_MODE_ADDFB,
                &mut fb_cmd as *mut _,
            );
            if ret < 0 {
                return Err(anyhow!(
                    "Failed to add framebuffer: {}",
                    std::io::Error::last_os_error()
                ));
            }

            debug!(
                "Framebuffer created: id={}, {}x{}, stride={}",
                fb_cmd.fb_id, width, height, stride
            );

            // framebuffer::Handle is internally u32
            std::mem::transmute::<u32, framebuffer::Handle>(fb_cmd.fb_id)
        };

        Ok(Self {
            device_fd: device.as_raw_fd(),
            fb,
        })
    }

    pub fn handle(&self) -> framebuffer::Handle {
        self.fb
    }
}

impl Drop for DrmFramebuffer {
    fn drop(&mut self) {
        unsafe {
            let mut fb_id = std::mem::transmute::<framebuffer::Handle, u32>(self.fb);
            libc::ioctl(self.device_fd, DRM_IOCTL_MODE_RMFB, &mut fb_id as *mut _);
        }
    }
}

/// Set display mode
pub fn set_crtc(device: &Device, config: &DisplayConfig, fb: &DrmFramebuffer) -> Result<()> {
    device
        .set_crtc(
            config.crtc_handle,
            Some(fb.handle()),
            (0, 0),
            &[config.connector_handle],
            Some(config.mode),
        )
        .context("Failed to set display mode")?;
    Ok(())
}

/// Save and restore original CRTC configuration
pub struct SavedCrtc {
    info: crtc::Info,
    connector: connector::Handle,
}

impl SavedCrtc {
    pub fn save(device: &Device, config: &DisplayConfig) -> Result<Self> {
        let info = device.get_crtc(config.crtc_handle)?;
        Ok(Self {
            info,
            connector: config.connector_handle,
        })
    }

    pub fn restore(&self, device: &Device, crtc_handle: crtc::Handle) {
        if let Some(fb) = self.info.framebuffer() {
            let _ = device.set_crtc(
                crtc_handle,
                Some(fb),
                self.info.position(),
                &[self.connector],
                self.info.mode(),
            );
        }
    }
}

// DRM ioctl constants
#[repr(C)]
struct drm_mode_fb_cmd {
    fb_id: u32,
    width: u32,
    height: u32,
    pitch: u32,
    bpp: u32,
    depth: u32,
    handle: u32,
}

const DRM_IOCTL_MODE_ADDFB: libc::c_ulong =
    nix::request_code_readwrite!(0x64, 0xAE, std::mem::size_of::<drm_mode_fb_cmd>())
        as libc::c_ulong;

const DRM_IOCTL_MODE_RMFB: libc::c_ulong =
    nix::request_code_readwrite!(0x64, 0xAF, std::mem::size_of::<u32>()) as libc::c_ulong;
