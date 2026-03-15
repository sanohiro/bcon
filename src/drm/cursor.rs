//! DRM hardware cursor support
//!
//! Uses the DRM cursor plane for zero-latency mouse cursor rendering.
//! The display controller composites the cursor at scanout time,
//! independent of the main framebuffer rendering pipeline.

#![allow(dead_code)]

use drm::control::crtc;
use drm::Device as DrmDevice;
use log::{info, warn};
use super::device::Device;

// ============================================================================
// DRM ioctl constants
// ============================================================================

const DRM_IOCTL_MODE_CURSOR: libc::c_ulong =
    nix::request_code_readwrite!(0x64, 0xA3, std::mem::size_of::<DrmModeCursor>())
        as libc::c_ulong;

const DRM_IOCTL_MODE_CURSOR2: libc::c_ulong =
    nix::request_code_readwrite!(0x64, 0xBB, std::mem::size_of::<DrmModeCursor2>())
        as libc::c_ulong;

const DRM_IOCTL_MODE_CREATE_DUMB: libc::c_ulong =
    nix::request_code_readwrite!(0x64, 0xB2, std::mem::size_of::<DrmModeCreateDumb>())
        as libc::c_ulong;

const DRM_IOCTL_MODE_MAP_DUMB: libc::c_ulong =
    nix::request_code_readwrite!(0x64, 0xB3, std::mem::size_of::<DrmModeMapDumb>())
        as libc::c_ulong;

const DRM_IOCTL_MODE_DESTROY_DUMB: libc::c_ulong =
    nix::request_code_readwrite!(0x64, 0xB4, std::mem::size_of::<DrmModeDestroyDumb>())
        as libc::c_ulong;

const DRM_MODE_CURSOR_BO: u32 = 0x01;
const DRM_MODE_CURSOR_MOVE: u32 = 0x02;

// ============================================================================
// DRM ioctl structs
// ============================================================================

#[repr(C)]
struct DrmModeCursor {
    flags: u32,
    crtc_id: u32,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    handle: u32,
}

#[repr(C)]
struct DrmModeCursor2 {
    flags: u32,
    crtc_id: u32,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    handle: u32,
    hot_x: i32,
    hot_y: i32,
}

#[repr(C)]
struct DrmModeCreateDumb {
    height: u32,
    width: u32,
    bpp: u32,
    flags: u32,
    handle: u32,
    pitch: u32,
    size: u64,
}

#[repr(C)]
struct DrmModeMapDumb {
    handle: u32,
    pad: u32,
    offset: u64,
}

#[repr(C)]
struct DrmModeDestroyDumb {
    handle: u32,
}

/// Default cursor size (most DRM drivers support 64x64)
const DEFAULT_CURSOR_SIZE: u32 = 64;

// ============================================================================
// HardwareCursor
// ============================================================================

/// Hardware cursor using DRM cursor plane.
///
/// The display controller composites this over the framebuffer at scanout,
/// so cursor movement is independent of the GPU rendering pipeline.
pub struct HardwareCursor {
    handle: u32,
    size: u32,
    device_fd: i32,
    crtc_id: u32,
}

impl HardwareCursor {
    /// Create a hardware cursor with a crosshair image.
    ///
    /// Returns `None` if the driver doesn't support hardware cursors.
    pub fn new(device: &Device, crtc_handle: crtc::Handle) -> Option<Self> {
        // Query cursor size from driver (fallback to 64x64)
        let size = device
            .get_driver_capability(drm::DriverCapability::CursorWidth)
            .ok()
            .and_then(|v| if v > 0 { Some(v as u32) } else { None })
            .unwrap_or(DEFAULT_CURSOR_SIZE);

        let fd = device.as_raw_fd();
        let crtc_id: u32 = From::from(crtc_handle);

        // Create dumb buffer for cursor (ARGB8888)
        let mut create = DrmModeCreateDumb {
            height: size,
            width: size,
            bpp: 32,
            flags: 0,
            handle: 0,
            pitch: 0,
            size: 0,
        };

        if unsafe { libc::ioctl(fd, DRM_IOCTL_MODE_CREATE_DUMB, &mut create as *mut _) } < 0 {
            warn!(
                "Failed to create cursor dumb buffer: {}",
                std::io::Error::last_os_error()
            );
            return None;
        }

        // Map the buffer to write pixel data
        let mut map = DrmModeMapDumb {
            handle: create.handle,
            pad: 0,
            offset: 0,
        };

        if unsafe { libc::ioctl(fd, DRM_IOCTL_MODE_MAP_DUMB, &mut map as *mut _) } < 0 {
            warn!(
                "Failed to map cursor buffer: {}",
                std::io::Error::last_os_error()
            );
            destroy_dumb(fd, create.handle);
            return None;
        }

        // mmap the buffer
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                create.size as usize,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                map.offset as libc::off_t,
            )
        };

        if ptr == libc::MAP_FAILED {
            warn!(
                "Failed to mmap cursor buffer: {}",
                std::io::Error::last_os_error()
            );
            destroy_dumb(fd, create.handle);
            return None;
        }

        // Render crosshair cursor into the buffer
        let pixels =
            unsafe { std::slice::from_raw_parts_mut(ptr as *mut u32, (size * size) as usize) };
        render_crosshair(pixels, size);

        // Unmap (data is already in the dumb buffer)
        unsafe {
            libc::munmap(ptr, create.size as usize);
        }

        // Set cursor on CRTC via raw ioctl
        let hotspot = (size / 2) as i32;
        let mut cursor2 = DrmModeCursor2 {
            flags: DRM_MODE_CURSOR_BO,
            crtc_id,
            x: 0,
            y: 0,
            width: size,
            height: size,
            handle: create.handle,
            hot_x: hotspot,
            hot_y: hotspot,
        };

        if unsafe { libc::ioctl(fd, DRM_IOCTL_MODE_CURSOR2, &mut cursor2 as *mut _) } < 0 {
            warn!(
                "Failed to set hardware cursor: {}",
                std::io::Error::last_os_error()
            );
            destroy_dumb(fd, create.handle);
            return None;
        }

        info!(
            "Hardware cursor enabled ({}x{}, hotspot={})",
            size, size, hotspot
        );

        Some(Self {
            handle: create.handle,
            size,
            device_fd: fd,
            crtc_id,
        })
    }

    /// Move cursor to screen position.
    ///
    /// Called directly from the input handler — no frame render needed.
    /// The display controller updates the cursor position at scanout.
    #[inline]
    pub fn move_to(&self, x: f64, y: f64) {
        let mut cursor = DrmModeCursor {
            flags: DRM_MODE_CURSOR_MOVE,
            crtc_id: self.crtc_id,
            x: x.round() as i32,
            y: y.round() as i32,
            width: 0,
            height: 0,
            handle: 0,
        };
        unsafe {
            libc::ioctl(
                self.device_fd,
                DRM_IOCTL_MODE_CURSOR,
                &mut cursor as *mut _,
            );
        }
    }

    /// Hide the cursor
    pub fn hide(&self) {
        let mut cursor = DrmModeCursor {
            flags: DRM_MODE_CURSOR_BO,
            crtc_id: self.crtc_id,
            x: 0,
            y: 0,
            width: self.size,
            height: self.size,
            handle: 0, // handle=0 disables cursor
        };
        unsafe {
            libc::ioctl(
                self.device_fd,
                DRM_IOCTL_MODE_CURSOR,
                &mut cursor as *mut _,
            );
        }
    }

    /// Show the cursor (re-enable with the stored buffer)
    pub fn show(&self) {
        let hotspot = (self.size / 2) as i32;
        let mut cursor2 = DrmModeCursor2 {
            flags: DRM_MODE_CURSOR_BO,
            crtc_id: self.crtc_id,
            x: 0,
            y: 0,
            width: self.size,
            height: self.size,
            handle: self.handle,
            hot_x: hotspot,
            hot_y: hotspot,
        };
        unsafe {
            libc::ioctl(
                self.device_fd,
                DRM_IOCTL_MODE_CURSOR2,
                &mut cursor2 as *mut _,
            );
        }
    }
}

impl Drop for HardwareCursor {
    fn drop(&mut self) {
        // Hide cursor before destroying the buffer
        self.hide();
        destroy_dumb(self.device_fd, self.handle);
    }
}

/// Destroy a dumb buffer
fn destroy_dumb(fd: i32, handle: u32) {
    let mut destroy = DrmModeDestroyDumb { handle };
    unsafe {
        libc::ioctl(fd, DRM_IOCTL_MODE_DESTROY_DUMB, &mut destroy as *mut _);
    }
}

// ============================================================================
// Crosshair rendering
// ============================================================================

/// Render a crosshair cursor into an ARGB8888 pixel buffer.
///
/// The crosshair has:
/// - Black outline (1px border) for visibility on light backgrounds
/// - White fill for visibility on dark backgrounds
fn render_crosshair(pixels: &mut [u32], size: u32) {
    let s = size as i32;
    let center = s / 2;
    let arm_len = 5; // pixels from center
    let thickness = 1; // half-thickness of the arms

    // Clear to fully transparent
    for p in pixels.iter_mut() {
        *p = 0x00000000;
    }

    let set = |pixels: &mut [u32], x: i32, y: i32, argb: u32| {
        if x >= 0 && x < s && y >= 0 && y < s {
            pixels[(y * s + x) as usize] = argb;
        }
    };

    let white: u32 = 0xFFFFFFFF; // ARGB: fully opaque white
    let black: u32 = 0xE0000000; // ARGB: mostly opaque black

    // Draw horizontal arm with outline
    for dx in -arm_len..=arm_len {
        let x = center + dx;
        // Black outline (top and bottom of arm)
        set(pixels, x, center - thickness - 1, black);
        set(pixels, x, center + thickness + 1, black);
        // White fill
        for dy in -thickness..=thickness {
            set(pixels, x, center + dy, white);
        }
    }

    // Draw vertical arm with outline
    for dy in -arm_len..=arm_len {
        let y = center + dy;
        // Black outline (left and right of arm)
        set(pixels, center - thickness - 1, y, black);
        set(pixels, center + thickness + 1, y, black);
        // White fill
        for dx in -thickness..=thickness {
            set(pixels, center + dx, y, white);
        }
    }

    // Outline at arm tips
    set(pixels, center - arm_len - 1, center, black);
    set(pixels, center + arm_len + 1, center, black);
    set(pixels, center, center - arm_len - 1, black);
    set(pixels, center, center + arm_len + 1, black);
}
