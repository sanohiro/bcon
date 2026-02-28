//! DRM device management
//!
//! Opens DRM device (/dev/dri/card*) and
//! enumerates available connectors, CRTCs, and encoders
//!

#![allow(dead_code)]
//! VT switching is handled via VT_SETMODE with VT_PROCESS mode.
//! The kernel sends SIGUSR1/SIGUSR2 signals for VT switch requests.

use anyhow::{anyhow, Context, Result};
use drm::control::{connector, crtc, encoder, Device as ControlDevice, ResourceHandles};
use drm::Device as BasicDevice;
use log::{debug, info, warn};
use nix::sys::signal::{SigSet, Signal};
use nix::sys::signalfd::{SfdFlags, SignalFd};
use std::fs::{File, OpenOptions};
use std::os::unix::io::{AsFd, AsRawFd, BorrowedFd, FromRawFd, RawFd};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU8, Ordering};
use std::time::Duration;

/// Global flag for shutdown requested via signal (SIGTERM/SIGINT/SIGHUP)
static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Global TTY fd for panic hook recovery.
/// When set (>= 0), the panic hook will restore KD_TEXT and VT_AUTO
/// to prevent the console from being stuck in KD_GRAPHICS mode.
static PANIC_RECOVERY_TTY_FD: AtomicI32 = AtomicI32::new(-1);

/// Fallback flags for VT switch signals (SIGUSR1/SIGUSR2)
///
/// Normally we receive VT switch events via signalfd with SIGUSR1/2 blocked.
/// If the signals ever leak past the mask (e.g., an unexpected thread mask),
/// a default SIGUSR2 would terminate the process. This handler keeps the
/// process alive and lets us recover in the main loop.
static VT_SIG_PENDING: AtomicU8 = AtomicU8::new(0);
const VT_SIG_ACQUIRE: u8 = 0b01;
const VT_SIG_RELEASE: u8 = 0b10;

/// Check if shutdown was requested (SIGTERM, SIGINT, or SIGHUP)
pub fn shutdown_requested() -> bool {
    SHUTDOWN_REQUESTED.load(Ordering::Relaxed)
}

/// Set up signal handlers for graceful shutdown (call once at startup)
///
/// Handles SIGTERM (systemd stop), SIGINT (Ctrl+C), and SIGHUP (terminal hangup).
pub fn setup_signal_handlers() {
    unsafe {
        libc::signal(
            libc::SIGTERM,
            shutdown_signal_handler as *const () as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGINT,
            shutdown_signal_handler as *const () as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGHUP,
            shutdown_signal_handler as *const () as libc::sighandler_t,
        );
    }
}

extern "C" fn shutdown_signal_handler(_signo: libc::c_int) {
    SHUTDOWN_REQUESTED.store(true, Ordering::Relaxed);
}

/// Install a panic hook that restores the console to text mode.
///
/// Even with `panic = "abort"`, `std::panic::set_hook` runs before the abort.
/// This ensures KD_TEXT and VT_AUTO are restored so the console is usable.
pub fn setup_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let fd = PANIC_RECOVERY_TTY_FD.load(Ordering::Relaxed);
        if fd >= 0 {
            // Restore KD_TEXT
            unsafe { libc::ioctl(fd, KDSETMODE, KD_TEXT) };
            // Restore VT_AUTO
            let mode = VtMode {
                mode: VT_AUTO,
                waitv: 0,
                relsig: 0,
                acqsig: 0,
                frsig: 0,
            };
            unsafe { libc::ioctl(fd, VT_SETMODE, &mode) };
        }
        // Print panic info to stderr so it's visible on the restored console
        eprintln!("[bcon] PANIC: {}", info);
        prev(info);
    }));
}

/// Set up fallback handlers for VT switch signals.
///
/// This should not replace signalfd-based handling. It only prevents the
/// default action (terminate) if SIGUSR1/2 are delivered unexpectedly.
fn setup_vt_signal_handlers() {
    unsafe {
        libc::signal(
            libc::SIGUSR1,
            vt_signal_handler as *const () as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGUSR2,
            vt_signal_handler as *const () as libc::sighandler_t,
        );
    }
}

extern "C" fn vt_signal_handler(signo: libc::c_int) {
    match signo {
        libc::SIGUSR1 => {
            VT_SIG_PENDING.fetch_or(VT_SIG_ACQUIRE, Ordering::Relaxed);
        }
        libc::SIGUSR2 => {
            VT_SIG_PENDING.fetch_or(VT_SIG_RELEASE, Ordering::Relaxed);
        }
        _ => {}
    }
}

fn take_pending_vt_event() -> Option<VtEvent> {
    // Prefer Release over Acquire if both are pending.
    if (VT_SIG_PENDING.fetch_and(!VT_SIG_RELEASE, Ordering::Relaxed) & VT_SIG_RELEASE) != 0 {
        return Some(VtEvent::Release);
    }
    if (VT_SIG_PENDING.fetch_and(!VT_SIG_ACQUIRE, Ordering::Relaxed) & VT_SIG_ACQUIRE) != 0 {
        return Some(VtEvent::Acquire);
    }
    None
}

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

        // NOTE: DRM master is NOT acquired here.
        // Caller should acquire master only when VT is active using set_master().
        // This allows proper VT switching without "stealing" the display.

        // Get resources
        let resources = temp
            .resource_handles()
            .context("Failed to get DRM resources")?;

        info!(
            "DRM resources: connectors={}, crtcs={}, encoders={}, framebuffers={}",
            resources.connectors().len(),
            resources.crtcs().len(),
            resources.encoders().len(),
            resources.framebuffers().len()
        );

        Ok(Self { file, resources })
    }

    /// Create Device from a pre-opened file descriptor
    ///
    /// Used when libseat provides the DRM device fd.
    /// The fd is duplicated, so the original can be closed.
    #[cfg(all(target_os = "linux", feature = "seatd"))]
    pub fn from_fd(fd: RawFd) -> Result<Self> {
        info!("Creating DRM device from fd {}", fd);

        // Duplicate the fd so we own it
        let dup_fd = nix::unistd::dup(fd).context("Failed to dup DRM fd")?;
        let file = unsafe { File::from_raw_fd(dup_fd) };

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

        // Note: When using libseat, we don't need to call SET_MASTER
        // libseat handles DRM master privileges automatically

        // Get resources
        let resources = temp
            .resource_handles()
            .context("Failed to get DRM resources")?;

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
            return Err(anyhow!(
                "fd dup failed: {}",
                std::io::Error::last_os_error()
            ));
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

    /// Find preferred connected connector based on priority
    ///
    /// When prefer_external is true, external connectors (HDMI, DP, DVI, VGA)
    /// are prioritized over internal (eDP, LVDS).
    ///
    /// Priority order: HDMI > DisplayPort > DVI > VGA > eDP > LVDS > others
    pub fn find_preferred_connector(
        &self,
        prefer_external: bool,
    ) -> Result<(connector::Handle, connector::Info)> {
        let mut connectors: Vec<(connector::Handle, connector::Info, i32)> = Vec::new();

        for &handle in self.resources.connectors() {
            let info = self.get_connector(handle)?;
            if info.state() == connector::State::Connected {
                let priority = connector_priority(info.interface(), prefer_external);
                connectors.push((handle, info, priority));
            }
        }

        if connectors.is_empty() {
            return Err(anyhow!("No connected connector found"));
        }

        // Sort by priority (lower is better)
        connectors.sort_by_key(|(_, _, p)| *p);

        let (handle, info, _) = connectors
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("Internal error: no connectors after filtering"))?;
        info!("Selected connector: {:?} ({:?})", handle, info.interface());
        Ok((handle, info))
    }

    /// Get all connected connectors with their info
    pub fn get_connected_connectors(&self) -> Vec<(connector::Handle, connector::Info)> {
        let mut result = Vec::new();
        for &handle in self.resources.connectors() {
            if let Ok(info) = self.get_connector(handle) {
                if info.state() == connector::State::Connected {
                    result.push((handle, info));
                }
            }
        }
        result
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
        let ret = unsafe { libc::ioctl(self.file.as_raw_fd(), drm_ioctl::DRM_IOCTL_DROP_MASTER) };
        if ret < 0 {
            return Err(anyhow!(
                "DROP_MASTER failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        info!("DRM master dropped");
        Ok(())
    }

    /// Acquire DRM master privileges (after VT switch back)
    pub fn set_master(&self) -> Result<()> {
        let ret = unsafe { libc::ioctl(self.file.as_raw_fd(), drm_ioctl::DRM_IOCTL_SET_MASTER) };
        if ret < 0 {
            return Err(anyhow!(
                "SET_MASTER failed: {}",
                std::io::Error::last_os_error()
            ));
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

// VT ioctl constants (from linux/vt.h)
const VT_GETSTATE: libc::c_ulong = 0x5603;
const VT_SETMODE: libc::c_ulong = 0x5602;
const VT_RELDISP: libc::c_ulong = 0x5605;
const VT_ACTIVATE: libc::c_ulong = 0x5606;
const VT_WAITACTIVE: libc::c_ulong = 0x5607;

// VT_SETMODE constants
const VT_AUTO: libc::c_char = 0;
const VT_PROCESS: libc::c_char = 1;
const VT_ACKACQ: libc::c_int = 2;

// KDSETMODE constants (from linux/kd.h)
const KDSETMODE: libc::c_ulong = 0x4B3A;
const KD_TEXT: libc::c_long = 0x00;
const KD_GRAPHICS: libc::c_long = 0x01;

/// vt_mode structure for VT_SETMODE ioctl
#[repr(C)]
struct VtMode {
    mode: libc::c_char,    // VT_AUTO or VT_PROCESS
    waitv: libc::c_char,   // unused
    relsig: libc::c_short, // signal to send on release
    acqsig: libc::c_short, // signal to send on acquire
    frsig: libc::c_short,  // unused
}

/// VT switch event from signalfd
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VtEvent {
    /// Kernel requests us to release the VT (SIGUSR2)
    Release,
    /// Kernel grants us the VT (SIGUSR1)
    Acquire,
}

/// VT switching manager using VT_SETMODE with VT_PROCESS mode
///
/// This is the correct way to handle VT switching for DRM applications.
/// The kernel sends SIGUSR1 (acquire) and SIGUSR2 (release) signals
/// when VT switch is requested (e.g., Ctrl+Alt+Fn).
///
/// Also handles:
/// - KDSETMODE (KD_GRAPHICS/KD_TEXT) to prevent kernel console rendering
/// - TTY settings save/restore
/// - Input buffer flushing on VT switch
///
/// Reference: kmscon's src/uterm_vt.c
pub struct VtSwitcher {
    /// fd for the TTY we're running on
    tty_fd: RawFd,
    /// Target VT number
    target_vt: u16,
    /// signalfd for receiving SIGUSR1/SIGUSR2
    signal_fd: SignalFd,
    /// Whether we currently have focus (are the active VT)
    active: bool,
    /// Original signal mask to restore on drop
    old_sigmask: SigSet,
    /// Original keyboard mode (KD_TEXT/KD_GRAPHICS)
    original_kd_mode: libc::c_long,
}

impl VtSwitcher {
    /// Create a VT switcher and set up process-controlled VT mode
    ///
    /// This blocks SIGUSR1/SIGUSR2 and sets up signalfd to receive them.
    /// VT_SETMODE is called to enable process-controlled switching.
    pub fn new() -> Result<Self> {
        // Get VT number from systemd instance or stdin (TTYPath)
        let target_vt = get_target_vt()
            .ok_or_else(|| anyhow!("Cannot determine VT from stdin - not running on a VT?"))?;

        info!("Target VT: {}", target_vt);

        // Open the TTY for ioctls
        let tty_path = format!("/dev/tty{}", target_vt);
        let tty_path_cstr = std::ffi::CString::new(tty_path.clone()).context("Invalid TTY path")?;
        let tty_fd = unsafe { libc::open(tty_path_cstr.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
        if tty_fd < 0 {
            return Err(anyhow!(
                "Cannot open {}: {}",
                tty_path,
                std::io::Error::last_os_error()
            ));
        }

        // Block signals so we can receive them via signalfd
        // SIGUSR1/SIGUSR2: VT switching
        // Note: SIGTERM/SIGHUP are NOT blocked - they use default termination behavior
        // which allows systemd to stop the service properly
        let mut mask = SigSet::empty();
        mask.add(Signal::SIGUSR1);
        mask.add(Signal::SIGUSR2);

        // Save old mask and block signals
        let old_sigmask = mask
            .thread_swap_mask(nix::sys::signal::SigmaskHow::SIG_BLOCK)
            .context("Failed to block signals")?;

        // Create signalfd
        let signal_fd = SignalFd::with_flags(&mask, SfdFlags::SFD_NONBLOCK | SfdFlags::SFD_CLOEXEC)
            .context("Failed to create signalfd")?;

        // Install a fallback handler to avoid process termination if SIGUSR1/2
        // are delivered outside signalfd (should be rare, but it's safer).
        setup_vt_signal_handlers();

        // Wait for VT to become active.
        // Use a polling loop instead of VT_WAITACTIVE so signals can stop the service.
        // VT_WAITACTIVE can be uninterruptible when signals are handled with SA_RESTART.
        // Timeout after 10 seconds to prevent indefinite blocking.
        info!("Waiting for VT{} to become active...", target_vt);
        let vt_wait_start = std::time::Instant::now();
        const VT_WAIT_TIMEOUT: Duration = Duration::from_secs(10);
        let vt_wait_success = loop {
            if shutdown_requested() {
                info!(
                    "Shutdown signal received while waiting for VT{}, exiting",
                    target_vt
                );
                break false;
            }
            if vt_wait_start.elapsed() >= VT_WAIT_TIMEOUT {
                warn!(
                    "Timed out waiting for VT{} to become active ({}s)",
                    target_vt,
                    VT_WAIT_TIMEOUT.as_secs()
                );
                break false;
            }
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
            let ret = unsafe { libc::ioctl(tty_fd, VT_GETSTATE, &mut stat) };
            if ret >= 0 && stat.v_active == target_vt {
                break true;
            }
            std::thread::sleep(Duration::from_millis(100));
        };

        if !vt_wait_success {
            unsafe { libc::close(tty_fd) };
            old_sigmask.thread_set_mask().ok();
            return Err(anyhow!("VT_WAITACTIVE interrupted or failed"));
        }
        info!("VT{} is now active", target_vt);

        // Set up process-controlled VT switching
        // Same as kmscon: acqsig = SIGUSR1, relsig = SIGUSR2
        let mode = VtMode {
            mode: VT_PROCESS,
            waitv: 0,
            relsig: Signal::SIGUSR2 as libc::c_short, // release signal (switching away)
            acqsig: Signal::SIGUSR1 as libc::c_short, // acquire signal (switching to us)
            frsig: 0,
        };

        let ret = unsafe { libc::ioctl(tty_fd, VT_SETMODE, &mode) };
        if ret < 0 {
            // Cleanup
            unsafe { libc::close(tty_fd) };
            old_sigmask.thread_set_mask().ok();
            return Err(anyhow!(
                "VT_SETMODE failed: {}",
                std::io::Error::last_os_error()
            ));
        }

        // Save original keyboard mode and set KD_GRAPHICS
        // This prevents the kernel from rendering to the framebuffer
        let mut original_kd_mode: libc::c_long = KD_TEXT;
        unsafe {
            // KDGETMODE = 0x4B3B
            libc::ioctl(tty_fd, 0x4B3B as libc::c_ulong, &mut original_kd_mode);
        }

        let ret = unsafe { libc::ioctl(tty_fd, KDSETMODE, KD_GRAPHICS) };
        if ret < 0 {
            warn!(
                "KDSETMODE(KD_GRAPHICS) failed: {} (continuing anyway)",
                std::io::Error::last_os_error()
            );
        } else {
            info!("Set KD_GRAPHICS mode");
            // Register tty_fd for panic hook recovery.
            // From this point, any panic will restore KD_TEXT before aborting.
            PANIC_RECOVERY_TTY_FD.store(tty_fd, Ordering::Relaxed);
        }

        // Flush input buffer to clear any stale events
        unsafe { libc::tcflush(tty_fd, libc::TCIFLUSH) };

        info!(
            "VT{} process-controlled mode enabled (SIGUSR1=acquire, SIGUSR2=release)",
            target_vt
        );

        // Check if we're actually the active VT right now
        // VT_WAITACTIVE might have returned because VT was briefly active,
        // but another VT (e.g., tty1 for getty) could have become active since then.
        let is_active = {
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
            let ret = unsafe { libc::ioctl(tty_fd, VT_GETSTATE, &mut stat) };
            if ret >= 0 {
                stat.v_active == target_vt
            } else {
                // VT_GETSTATE failed, assume active since VT_WAITACTIVE returned
                true
            }
        };

        if is_active {
            info!("VT{} is active, keeping KD_GRAPHICS", target_vt);
        } else {
            // We're not the active VT - restore KD_TEXT so the active VT can display
            info!(
                "VT{} is not active (active={}), restoring KD_TEXT",
                target_vt, is_active
            );
            unsafe { libc::ioctl(tty_fd, KDSETMODE, KD_TEXT) };
        }

        Ok(Self {
            tty_fd,
            target_vt,
            signal_fd,
            active: is_active,
            old_sigmask,
            original_kd_mode,
        })
    }

    /// Get VT number from stdin
    fn get_vt_from_stdin() -> Option<u16> {
        // Try ttyname first
        let tty_path = unsafe {
            let ptr = libc::ttyname(0);
            if ptr.is_null() {
                None
            } else {
                Some(std::ffi::CStr::from_ptr(ptr).to_string_lossy().into_owned())
            }
        };

        if let Some(path) = &tty_path {
            debug!("stdin TTY: {}", path);
            if let Some(num_str) = path.strip_prefix("/dev/tty") {
                if let Ok(vt) = num_str.parse::<u16>() {
                    if vt >= 1 && vt <= 63 {
                        return Some(vt);
                    }
                }
            }
        }

        // Fallback: fstat on stdin
        let mut stat_buf: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstat(0, &mut stat_buf) } < 0 {
            return None;
        }

        let major = libc::major(stat_buf.st_rdev);
        let minor = libc::minor(stat_buf.st_rdev);

        debug!("stdin device: major={}, minor={}", major, minor);

        // tty1-tty63 is major=4, minor=1-63
        if major == 4 && minor >= 1 && minor <= 63 {
            Some(minor as u16)
        } else {
            None
        }
    }

    /// Get signalfd's raw fd for polling
    pub fn as_raw_fd(&self) -> RawFd {
        self.signal_fd.as_raw_fd()
    }

    /// Get target VT number
    pub fn target_vt(&self) -> u16 {
        self.target_vt
    }

    /// Check if we're currently the active VT
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Poll for VT switch events (non-blocking)
    ///
    /// Call this in the event loop to check for VT switch requests.
    /// Returns Some(VtEvent) if a switch event occurred.
    pub fn poll(&mut self) -> Option<VtEvent> {
        if let Some(event) = take_pending_vt_event() {
            return Some(event);
        }
        match self.signal_fd.read_signal() {
            Ok(Some(siginfo)) => {
                let signo = siginfo.ssi_signo as i32;
                if signo == Signal::SIGUSR2 as i32 {
                    // Release request - we should give up the VT
                    debug!("SIGUSR2 received: VT release requested");
                    Some(VtEvent::Release)
                } else if signo == Signal::SIGUSR1 as i32 {
                    // Acquire - we're getting the VT back
                    debug!("SIGUSR1 received: VT acquire");
                    Some(VtEvent::Acquire)
                } else {
                    None
                }
            }
            Ok(None) => None, // No signal pending
            Err(e) => {
                warn!("signalfd read error: {}", e);
                None
            }
        }
    }

    /// Acknowledge VT release - call this after dropping DRM master
    ///
    /// This tells the kernel we're done cleaning up and the VT switch can proceed.
    pub fn ack_release(&mut self) -> Result<()> {
        info!("Acknowledging VT release");

        // Flush input buffer before releasing
        unsafe { libc::tcflush(self.tty_fd, libc::TCIFLUSH) };

        // Restore text mode while VT is inactive (allows kernel console to work)
        unsafe { libc::ioctl(self.tty_fd, KDSETMODE, KD_TEXT) };

        let ret = unsafe { libc::ioctl(self.tty_fd, VT_RELDISP, 1 as libc::c_int) };
        if ret < 0 {
            return Err(anyhow!(
                "VT_RELDISP(1) failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        self.active = false;
        info!("VT released");
        Ok(())
    }

    /// Acknowledge VT acquire - call this after acquiring DRM master
    ///
    /// This tells the kernel we've finished acquiring resources.
    pub fn ack_acquire(&mut self) -> Result<()> {
        info!("Acknowledging VT acquire");

        // Restore graphics mode
        unsafe { libc::ioctl(self.tty_fd, KDSETMODE, KD_GRAPHICS) };

        // Flush any stale input events
        unsafe { libc::tcflush(self.tty_fd, libc::TCIFLUSH) };

        let ret = unsafe { libc::ioctl(self.tty_fd, VT_RELDISP, VT_ACKACQ) };
        if ret < 0 {
            return Err(anyhow!(
                "VT_RELDISP(VT_ACKACQ) failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        self.active = true;
        info!("VT acquired");
        Ok(())
    }

    /// Request switch to another VT
    ///
    /// This triggers a VT switch by calling VT_ACTIVATE.
    /// The kernel will send SIGUSR2 to us, and we should handle it
    /// in the event loop by calling ack_release().
    pub fn switch_to(&self, target: u16) -> Result<()> {
        info!("Requesting switch to VT{}", target);
        let ret = unsafe { libc::ioctl(self.tty_fd, VT_ACTIVATE, target as libc::c_int) };
        if ret < 0 {
            return Err(anyhow!(
                "VT_ACTIVATE({}) failed: {}",
                target,
                std::io::Error::last_os_error()
            ));
        }
        Ok(())
    }

    /// Check if VT is currently active using VT_GETSTATE
    pub fn is_focused(&self) -> bool {
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

        let ret = unsafe { libc::ioctl(self.tty_fd, VT_GETSTATE, &mut stat) };
        if ret < 0 {
            return self.active; // Fall back to tracked state
        }

        stat.v_active == self.target_vt
    }
}

impl Drop for VtSwitcher {
    fn drop(&mut self) {
        // Clear panic recovery fd (we're cleaning up properly)
        PANIC_RECOVERY_TTY_FD.store(-1, Ordering::Relaxed);

        // Restore original keyboard mode (KD_TEXT)
        let ret = unsafe { libc::ioctl(self.tty_fd, KDSETMODE, self.original_kd_mode) };
        if ret < 0 {
            warn!(
                "Failed to restore KD mode: {}",
                std::io::Error::last_os_error()
            );
        } else {
            info!("Restored KD_TEXT mode");
        }

        // Reset to VT_AUTO mode
        let mode = VtMode {
            mode: VT_AUTO,
            waitv: 0,
            relsig: 0,
            acqsig: 0,
            frsig: 0,
        };

        let ret = unsafe { libc::ioctl(self.tty_fd, VT_SETMODE, &mode) };
        if ret < 0 {
            warn!(
                "Failed to reset VT to VT_AUTO: {}",
                std::io::Error::last_os_error()
            );
        }

        // Restore original signal mask
        if let Err(e) = self.old_sigmask.thread_set_mask() {
            warn!("Failed to restore signal mask: {}", e);
        }

        unsafe { libc::close(self.tty_fd) };
        info!("VT switcher cleaned up");
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

// ============================================================================
// Public VT utility functions (used by both seatd and non-seatd code paths)
// ============================================================================

/// Get VT number from stdin (for systemd TTYPath integration)
///
/// This reads the VT number from stdin's tty device.
/// When systemd starts a service with TTYPath=/dev/ttyN, stdin will be
/// connected to that tty, allowing us to determine the target VT.
pub fn get_target_vt() -> Option<u16> {
    // Prefer systemd instance if present (e.g., bcon@tty2.service -> "tty2")
    if let Ok(instance) = std::env::var("SYSTEMD_INSTANCE") {
        if let Some(num_str) = instance.strip_prefix("tty") {
            if let Ok(vt) = num_str.parse::<u16>() {
                if vt >= 1 && vt <= 63 {
                    return Some(vt);
                }
            }
        }
    }

    // Fallback: parse SYSTEMD_UNIT if instance isn't available
    if let Ok(unit) = std::env::var("SYSTEMD_UNIT") {
        // Example: "bcon@tty2.service"
        if let Some(at) = unit.find('@') {
            let rest = &unit[at + 1..];
            if let Some(dot) = rest.find('.') {
                let inst = &rest[..dot];
                if let Some(num_str) = inst.strip_prefix("tty") {
                    if let Ok(vt) = num_str.parse::<u16>() {
                        if vt >= 1 && vt <= 63 {
                            return Some(vt);
                        }
                    }
                }
            }
        }
    }

    // Try ttyname first
    let tty_path = unsafe {
        let ptr = libc::ttyname(0);
        if ptr.is_null() {
            None
        } else {
            Some(std::ffi::CStr::from_ptr(ptr).to_string_lossy().into_owned())
        }
    };

    if let Some(path) = &tty_path {
        debug!("stdin TTY: {}", path);
        if let Some(num_str) = path.strip_prefix("/dev/tty") {
            if let Ok(vt) = num_str.parse::<u16>() {
                if vt >= 1 && vt <= 63 {
                    return Some(vt);
                }
            }
        }
    }

    // Fallback: fstat on stdin
    let mut stat_buf: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(0, &mut stat_buf) } < 0 {
        return None;
    }

    let major = libc::major(stat_buf.st_rdev);
    let minor = libc::minor(stat_buf.st_rdev);

    debug!("stdin device: major={}, minor={}", major, minor);

    // tty1-tty63 is major=4, minor=1-63
    if major == 4 && minor >= 1 && minor <= 63 {
        Some(minor as u16)
    } else {
        None
    }
}

/// Get currently active VT number
pub fn get_active_vt() -> Option<u16> {
    // First try sysfs (no special permissions required)
    if let Ok(content) = std::fs::read_to_string("/sys/class/tty/tty0/active") {
        // Content is like "tty2\n"
        if let Some(num_str) = content.trim().strip_prefix("tty") {
            if let Ok(vt) = num_str.parse::<u16>() {
                return Some(vt);
            }
        }
    }

    // Fallback to ioctl (requires /dev/tty0 access)
    #[repr(C)]
    struct VtStat {
        v_active: libc::c_ushort,
        v_signal: libc::c_ushort,
        v_state: libc::c_ushort,
    }

    // Open /dev/tty0 (console device) for ioctl
    let tty0 = std::ffi::CString::new("/dev/tty0").ok()?;
    let fd = unsafe { libc::open(tty0.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
    if fd < 0 {
        return None;
    }

    let mut stat = VtStat {
        v_active: 0,
        v_signal: 0,
        v_state: 0,
    };

    let ret = unsafe { libc::ioctl(fd, VT_GETSTATE, &mut stat) };
    unsafe { libc::close(fd) };

    if ret >= 0 {
        Some(stat.v_active)
    } else {
        None
    }
}

/// Check if specified VT is currently active
pub fn is_vt_active(target_vt: u16) -> bool {
    get_active_vt()
        .map(|active| active == target_vt)
        .unwrap_or(false)
}

/// Wait for specified VT to become active (blocking)
///
/// This blocks until the target VT becomes the active console.
/// Returns Ok(()) when the VT becomes active, or Err if waiting fails.
pub fn wait_for_vt(target_vt: u16) -> anyhow::Result<()> {
    // Open /dev/tty0 for ioctl
    let tty0 =
        std::ffi::CString::new("/dev/tty0").map_err(|_| anyhow::anyhow!("Invalid tty path"))?;
    let fd = unsafe { libc::open(tty0.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(anyhow::anyhow!(
            "Cannot open /dev/tty0: {}",
            std::io::Error::last_os_error()
        ));
    }

    info!("Waiting for VT{} to become active...", target_vt);

    loop {
        let ret = unsafe { libc::ioctl(fd, VT_WAITACTIVE, target_vt as libc::c_int) };
        if ret >= 0 {
            break;
        }
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::EINTR) {
            // Interrupted by signal - check if we should exit
            if shutdown_requested() {
                info!("Shutdown signal received during VT_WAITACTIVE, exiting");
                unsafe { libc::close(fd) };
                return Err(anyhow::anyhow!(
                    "VT_WAITACTIVE interrupted by shutdown signal"
                ));
            }
            continue;
        }
        unsafe { libc::close(fd) };
        return Err(anyhow::anyhow!("VT_WAITACTIVE failed: {}", err));
    }

    unsafe { libc::close(fd) };
    info!("VT{} is now active", target_vt);
    Ok(())
}

/// Get connector priority for display selection
///
/// When prefer_external is true, external connectors are prioritized.
/// Lower number = higher priority.
fn connector_priority(interface: connector::Interface, prefer_external: bool) -> i32 {
    use connector::Interface;

    if prefer_external {
        // External monitors first
        match interface {
            Interface::HDMIA | Interface::HDMIB => 10,
            Interface::DisplayPort => 20,
            Interface::DVID | Interface::DVII | Interface::DVIA => 30,
            Interface::VGA => 40,
            // Internal displays last
            Interface::EmbeddedDisplayPort => 100, // eDP (laptop internal)
            Interface::LVDS => 110,
            Interface::DSI => 120,
            // Other/unknown
            _ => 50,
        }
    } else {
        // First connected wins (no preference)
        0
    }
}

/// Check if connector is external (not internal laptop display)
pub fn is_external_connector(interface: connector::Interface) -> bool {
    use connector::Interface;
    matches!(
        interface,
        Interface::HDMIA
            | Interface::HDMIB
            | Interface::DisplayPort
            | Interface::DVID
            | Interface::DVII
            | Interface::DVIA
            | Interface::VGA
    )
}

/// Check if connector is internal (laptop built-in display)
pub fn is_internal_connector(interface: connector::Interface) -> bool {
    use connector::Interface;
    matches!(
        interface,
        Interface::EmbeddedDisplayPort | Interface::LVDS | Interface::DSI
    )
}
