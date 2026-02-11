//! Session management abstraction
//!
//! Provides two backends:
//! - Direct VT control (requires root, current default)
//! - libseat (requires seatd/logind, no root needed) - TODO
//!
//! Currently this module is infrastructure for future libseat integration.

// TODO: Implement libseat backend when API is verified
// #[cfg(feature = "seatd")]
// mod seatd;
// #[cfg(feature = "seatd")]
// pub use seatd::SeatdSession;
