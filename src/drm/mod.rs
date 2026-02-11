//! DRM/KMS display management

pub mod device;
pub mod display;

pub use device::{setup_sigterm_handler, sigterm_received, Device, VtEvent, VtSwitcher};
pub use display::{set_crtc, DisplayConfig, DrmFramebuffer, SavedCrtc};
