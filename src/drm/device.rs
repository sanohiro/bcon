//! DRM device management
//!
//! Opens DRM device (/dev/dri/card*) and
//! enumerates available connectors, CRTCs, and encoders

use anyhow::{anyhow, Context, Result};
use drm::control::{connector, crtc, encoder, Device as ControlDevice, ResourceHandles};
use drm::Device as BasicDevice;
use log::{debug, info, warn};
use std::fs::{File, OpenOptions};
use std::os::unix::io::{AsFd, AsRawFd, BorrowedFd, RawFd};
use std::path::Path;

/// DRM device wrapper
pub struct Device {
    file: File,
    resources: ResourceHandles,
}

// Trait implementations required by drm crate
impl AsFd for Device {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.file.as_fd()
    }
}

impl BasicDevice for Device {}
impl ControlDevice for Device {}

impl Device {
    /// Open DRM device
    ///
    /// # Arguments
    /// * `path` - Device path (e.g., "/dev/dri/card0")
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        info!("Opening DRM device: {}", path.display());

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .with_context(|| format!("Cannot open DRM device {}", path.display()))?;

        // Create temporary device wrapper to get resources
        struct TempDevice<'a>(&'a File);
        impl AsFd for TempDevice<'_> {
            fn as_fd(&self) -> BorrowedFd<'_> {
                self.0.as_fd()
            }
        }
        impl BasicDevice for TempDevice<'_> {}
        impl ControlDevice for TempDevice<'_> {}

        let temp = TempDevice(&file);

        // Acquire DRM master privileges
        unsafe {
            let ret = libc::ioctl(file.as_raw_fd(), drm_ioctl::DRM_IOCTL_SET_MASTER);
            if ret < 0 {
                let err = std::io::Error::last_os_error();
                if err.raw_os_error() != Some(libc::EACCES) {
                    warn!("DRM master set warning: {} (ignoring and continuing)", err);
                }
            }
        }

        // Get resources
        let resources = temp.resource_handles().context("Failed to get DRM resources")?;

        info!(
            "DRM resources: connectors={}, crtcs={}, encoders={}, framebuffers={}",
            resources.connectors().len(),
            resources.crtcs().len(),
            resources.encoders().len(),
            resources.framebuffers().len()
        );

        Ok(Self { file, resources })
    }

    /// Get resource handles
    #[allow(dead_code)]
    pub fn resources(&self) -> &ResourceHandles {
        &self.resources
    }

    /// Get connector info
    pub fn get_connector(&self, handle: connector::Handle) -> Result<connector::Info> {
        ControlDevice::get_connector(self, handle, false)
            .with_context(|| format!("Failed to get connector {:?} info", handle))
    }

    /// Get encoder info
    pub fn get_encoder(&self, handle: encoder::Handle) -> Result<encoder::Info> {
        ControlDevice::get_encoder(self, handle)
            .with_context(|| format!("Failed to get encoder {:?} info", handle))
    }

    /// Get CRTC info
    pub fn get_crtc(&self, handle: crtc::Handle) -> Result<crtc::Info> {
        ControlDevice::get_crtc(self, handle)
            .with_context(|| format!("Failed to get CRTC {:?} info", handle))
    }

    /// Get RawFd (needed for GBM/EGL)
    pub fn as_raw_fd(&self) -> RawFd {
        self.file.as_raw_fd()
    }

    /// Duplicate fd and return as File (for GBM device)
    pub fn dup_fd(&self) -> Result<std::fs::File> {
        use std::os::unix::io::FromRawFd;
        let fd = unsafe { libc::dup(self.file.as_raw_fd()) };
        if fd < 0 {
            return Err(anyhow!("fd dup failed: {}", std::io::Error::last_os_error()));
        }
        Ok(unsafe { std::fs::File::from_raw_fd(fd) })
    }

    /// Find connected connector
    pub fn find_connected_connector(&self) -> Result<(connector::Handle, connector::Info)> {
        for &handle in self.resources.connectors() {
            let info = self.get_connector(handle)?;
            if info.state() == connector::State::Connected {
                debug!("Found connected connector: {:?}", handle);
                return Ok((handle, info));
            }
        }
        Err(anyhow!("No connected connector found"))
    }

    /// Find CRTC for connector
    pub fn find_crtc_for_connector(
        &self,
        connector: &connector::Info,
    ) -> Result<(crtc::Handle, crtc::Info)> {
        // First check current encoder
        if let Some(encoder_handle) = connector.current_encoder() {
            let encoder = self.get_encoder(encoder_handle)?;
            if let Some(crtc_handle) = encoder.crtc() {
                let crtc = self.get_crtc(crtc_handle)?;
                return Ok((crtc_handle, crtc));
            }
        }

        // Find available encoder and CRTC
        for &encoder_handle in connector.encoders() {
            let encoder = self.get_encoder(encoder_handle)?;

            // Check CRTCs supported by encoder
            let possible = encoder.possible_crtcs();
            let filtered = self.resources.filter_crtcs(possible);

            for crtc_handle in filtered {
                let crtc = self.get_crtc(crtc_handle)?;
                return Ok((crtc_handle, crtc));
            }
        }

        Err(anyhow!("No CRTC found for connector"))
    }

    /// Drop DRM master privileges (allows VT switch)
    pub fn drop_master(&self) -> Result<()> {
        let ret = unsafe {
            libc::ioctl(self.file.as_raw_fd(), drm_ioctl::DRM_IOCTL_DROP_MASTER)
        };
        if ret < 0 {
            return Err(anyhow!("DROP_MASTER failed: {}", std::io::Error::last_os_error()));
        }
        info!("DRM master dropped");
        Ok(())
    }

    /// Acquire DRM master privileges (after VT switch back)
    pub fn set_master(&self) -> Result<()> {
        let ret = unsafe {
            libc::ioctl(self.file.as_raw_fd(), drm_ioctl::DRM_IOCTL_SET_MASTER)
        };
        if ret < 0 {
            return Err(anyhow!("SET_MASTER failed: {}", std::io::Error::last_os_error()));
        }
        info!("DRM master acquired");
        Ok(())
    }
}

// DRM ioctl constants
mod drm_ioctl {
    // Linux: include/uapi/drm/drm.h
    // _IO('d', 0x1e) = SET_MASTER, _IO('d', 0x1f) = DROP_MASTER
    const DRM_IOCTL_BASE: u64 = 0x64;
    pub const DRM_IOCTL_SET_MASTER: libc::c_ulong =
        nix::request_code_none!(DRM_IOCTL_BASE, 0x1e) as libc::c_ulong;
    pub const DRM_IOCTL_DROP_MASTER: libc::c_ulong =
        nix::request_code_none!(DRM_IOCTL_BASE, 0x1f) as libc::c_ulong;
}

// VT ioctl constants
const VT_GETSTATE: libc::c_ulong = 0x5603;
const VT_ACTIVATE: libc::c_ulong = 0x5606;
const VT_WAITACTIVE: libc::c_ulong = 0x5607;

/// VT focus state tracking and switching helper
pub struct VtFocusTracker {
    /// fd for VT operations (usually /dev/tty0 or /dev/console)
    console_fd: Option<RawFd>,
    /// Target VT number (from stdin TTY)
    target_vt: Option<u16>,
    was_focused: bool,
}

impl VtFocusTracker {
    /// Create VT focus tracker and switch to target VT
    pub fn new() -> Self {
        // Get VT number from stdin (systemd sets TTYPath which connects stdin to the TTY)
        let target_vt = Self::get_vt_from_stdin();

        // Open /dev/tty0 (console master) for VT switching operations
        // This works regardless of which TTY we're attached to
        let console_fd = unsafe {
            let fd = libc::open(b"/dev/tty0\0".as_ptr() as *const libc::c_char, libc::O_RDWR);
            if fd >= 0 {
                Some(fd)
            } else {
                // Fallback to /dev/console
                let fd = libc::open(b"/dev/console\0".as_ptr() as *const libc::c_char, libc::O_RDWR);
                if fd >= 0 { Some(fd) } else { None }
            }
        };

        if console_fd.is_none() {
            warn!("Cannot open /dev/tty0 or /dev/console for VT switching");
        }

        let mut tracker = Self {
            console_fd,
            target_vt,
            was_focused: true,
        };

        // Switch to target VT if we have one
        if let Some(vt) = target_vt {
            info!("Target VT: {}", vt);
            if let Err(e) = tracker.activate_vt(vt) {
                warn!("Failed to activate VT{}: {}", vt, e);
            } else {
                info!("Activated VT{}", vt);
            }
        } else {
            warn!("Could not determine target VT from stdin");
        }

        tracker
    }

    /// Get VT number from stdin (fd 0)
    fn get_vt_from_stdin() -> Option<u16> {
        // First try ttyname to get the TTY path
        let tty_path = unsafe {
            let ptr = libc::ttyname(0); // stdin
            if ptr.is_null() {
                None
            } else {
                Some(std::ffi::CStr::from_ptr(ptr).to_string_lossy().into_owned())
            }
        };

        if let Some(path) = &tty_path {
            info!("stdin TTY: {}", path);
            // Parse /dev/ttyN -> N
            if let Some(num_str) = path.strip_prefix("/dev/tty") {
                if let Ok(vt) = num_str.parse::<u16>() {
                    if vt >= 1 && vt <= 63 {
                        return Some(vt);
                    }
                }
            }
        }

        // Fallback: use fstat on stdin
        let mut stat_buf: libc::stat = unsafe { std::mem::zeroed() };
        let ret = unsafe { libc::fstat(0, &mut stat_buf) };
        if ret < 0 {
            return None;
        }

        let major = libc::major(stat_buf.st_rdev);
        let minor = libc::minor(stat_buf.st_rdev);

        info!("stdin device: major={}, minor={}", major, minor);

        // tty1-tty63 is major=4, minor=1-63
        if major == 4 && minor >= 1 && minor <= 63 {
            Some(minor as u16)
        } else {
            None
        }
    }

    /// Activate (switch to) specific VT
    fn activate_vt(&self, vt: u16) -> Result<()> {
        let Some(fd) = self.console_fd else {
            return Err(anyhow!("No console fd"));
        };

        info!("VT_ACTIVATE({}) on fd {}", vt, fd);

        // VT_ACTIVATE: Switch to VT
        let ret = unsafe { libc::ioctl(fd, VT_ACTIVATE, vt as libc::c_int) };
        if ret < 0 {
            return Err(anyhow!("VT_ACTIVATE failed: {}", std::io::Error::last_os_error()));
        }

        // VT_WAITACTIVE: Wait until VT switch is complete
        let ret = unsafe { libc::ioctl(fd, VT_WAITACTIVE, vt as libc::c_int) };
        if ret < 0 {
            return Err(anyhow!("VT_WAITACTIVE failed: {}", std::io::Error::last_os_error()));
        }

        Ok(())
    }

    /// Check if VT is currently active
    /// Uses VT_GETSTATE ioctl
    pub fn is_focused(&self) -> bool {
        let Some(fd) = self.console_fd else {
            return true; // Assume focused if no console
        };

        let Some(target) = self.target_vt else {
            return true; // Not a VT, always focused
        };

        // VT_GETSTATE: Get currently active VT
        #[repr(C)]
        struct VtStat {
            v_active: libc::c_ushort,
            v_signal: libc::c_ushort,
            v_state: libc::c_ushort,
        }

        let mut stat = VtStat {
            v_active: 0,
            v_signal: 0,
            v_state: 0,
        };

        let ret = unsafe { libc::ioctl(fd, VT_GETSTATE, &mut stat) };
        if ret < 0 {
            return true; // Assume focused on error
        }

        stat.v_active == target
    }

    /// Get target VT number
    pub fn target_vt(&self) -> Option<u16> {
        self.target_vt
    }

    /// Switch to a specific VT (public interface)
    pub fn switch_to(&self, vt: u16) -> Result<()> {
        self.activate_vt(vt)
    }

    /// Check if focus state changed, return new state if changed
    pub fn check_focus_change(&mut self) -> Option<bool> {
        let focused = self.is_focused();
        if focused != self.was_focused {
            self.was_focused = focused;
            Some(focused)
        } else {
            None
        }
    }
}

impl Drop for VtFocusTracker {
    fn drop(&mut self) {
        if let Some(fd) = self.console_fd {
            unsafe { libc::close(fd); }
        }
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        // Release DRM master privileges
        unsafe {
            libc::ioctl(self.file.as_raw_fd(), drm_ioctl::DRM_IOCTL_DROP_MASTER);
        }
    }
}
