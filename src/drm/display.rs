//! DRM display management
//!
//! Mode setting and page flip handling

#![allow(dead_code)]

use anyhow::{anyhow, bail, Context, Result};
use drm::control::{connector, crtc, framebuffer, Device as ControlDevice, Mode};
use log::{debug, info};

use super::device::Device;
use super::hdr::{self, HdrCapabilities};

/// Display configuration
pub struct DisplayConfig {
    pub connector_handle: connector::Handle,
    pub crtc_handle: crtc::Handle,
    pub mode: Mode,
    pub width: u32,
    pub height: u32,
    /// HDR capabilities detected from display EDID
    pub hdr: HdrCapabilities,
}

impl DisplayConfig {
    /// Auto-detect connected display and get configuration
    pub fn auto_detect(device: &Device) -> Result<Self> {
        Self::detect_with_preference(device, false)
    }

    /// Detect display with external monitor preference
    ///
    /// When prefer_external is true, external connectors (HDMI, DP, etc.)
    /// are prioritized over internal displays (eDP, LVDS).
    pub fn detect_with_preference(device: &Device, prefer_external: bool) -> Result<Self> {
        let (connector_handle, connector_info) =
            device.find_preferred_connector(prefer_external)?;

        info!(
            "Connector: {:?}, type: {:?}",
            connector_handle,
            connector_info.interface()
        );

        Self::from_connector(device, connector_handle, &connector_info)
    }

    /// Create DisplayConfig from a specific connector
    pub fn from_connector(
        device: &Device,
        connector_handle: connector::Handle,
        connector_info: &connector::Info,
    ) -> Result<Self> {
        let (crtc_handle, _crtc_info) = device.find_crtc_for_connector(connector_info)?;
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
        info!("Display mode: {}x{} @ {}Hz", width, height, mode.vrefresh());

        // Detect HDR capabilities from EDID
        let hdr = detect_hdr_capabilities(device, connector_handle);
        hdr.log();

        Ok(Self {
            connector_handle,
            crtc_handle,
            mode,
            width: width as u32,
            height: height as u32,
            hdr,
        })
    }

    /// Check if this config uses an external connector
    pub fn is_external(&self, device: &Device) -> bool {
        if let Ok(info) = device.get_connector(self.connector_handle) {
            super::device::is_external_connector(info.interface())
        } else {
            false
        }
    }
}

/// Detect HDR capabilities from connector EDID
fn detect_hdr_capabilities(device: &Device, connector: connector::Handle) -> HdrCapabilities {
    // Try to get EDID blob from connector properties
    match get_connector_edid(device, connector) {
        Ok(edid) => {
            debug!("EDID data: {} bytes", edid.len());
            hdr::parse_edid_hdr(&edid)
        }
        Err(e) => {
            debug!("Could not read EDID: {}", e);
            HdrCapabilities::default()
        }
    }
}

/// Get EDID blob from connector
fn get_connector_edid(device: &Device, connector: connector::Handle) -> Result<Vec<u8>> {
    #![allow(unused_imports)]
    use drm::control::property;

    // Get connector properties
    let props = device
        .get_properties(connector)
        .context("Failed to get connector properties")?;

    // Find EDID property
    for (&prop_handle, &value) in props.iter() {
        let prop_info = device
            .get_property(prop_handle)
            .context("Failed to get property info")?;

        if prop_info.name().to_str() == Ok("EDID") {
            // Value is a blob ID
            if value == 0 {
                bail!("EDID blob not available (value=0)");
            }

            // Get blob data
            let blob = device
                .get_property_blob(value)
                .context("Failed to get EDID blob")?;

            return Ok(blob);
        }
    }

    bail!("EDID property not found")
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

            // SAFETY: framebuffer::Handle from the drm crate is a newtype wrapper around u32.
            // Both types have identical memory layout, so transmute is safe.
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
        // SAFETY: framebuffer::Handle is a newtype wrapper around u32 (same layout).
        // We need the raw u32 value for the ioctl call.
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
