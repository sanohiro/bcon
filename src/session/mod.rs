//! Session management abstraction
//!
//! Provides two backends:
//! - Direct VT control (requires root, current default)
//! - libseat (requires seatd/logind, no root needed)

#[cfg(all(target_os = "linux", feature = "seatd"))]
mod seatd;

#[cfg(all(target_os = "linux", feature = "seatd"))]
#[allow(unused_imports)]
pub use seatd::{SeatDevice, SeatSession, SessionEvent};
