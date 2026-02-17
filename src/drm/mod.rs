//! DRM/KMS display management

pub mod device;
pub mod display;
pub mod hdr;
#[cfg(target_os = "linux")]
pub mod hotplug;

#[allow(unused_imports)]
pub use device::{
    get_active_vt, get_target_vt, is_vt_active, setup_sigterm_handler, sigterm_received,
    wait_for_vt, Device, VtEvent, VtSwitcher,
};
pub use display::{set_crtc, DisplayConfig, DrmFramebuffer, SavedCrtc};
// HDR types exported for public API (future use)
#[allow(unused_imports)]
pub use hdr::{HdrCapabilities, HdrOutputMetadata};
#[cfg(target_os = "linux")]
pub use hotplug::HotplugMonitor;
// Hotplug types exported for public API
#[cfg(target_os = "linux")]
#[allow(unused_imports)]
pub use hotplug::{ConnectorChanges, ConnectorSnapshot, HotplugEvent};
