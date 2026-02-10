//! DRM/KMS display management

pub mod device;
pub mod display;

pub use device::{Device, VtEvent, VtSwitcher};
pub use display::{set_crtc, DisplayConfig, DrmFramebuffer, SavedCrtc};
