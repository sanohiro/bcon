//! DRM/KMS display management

pub mod device;
pub mod display;

#[allow(deprecated)]
pub use device::VtFocusTracker;
pub use device::{Device, VtEvent, VtSwitcher};
pub use display::{set_crtc, DisplayConfig, DrmFramebuffer, SavedCrtc};
