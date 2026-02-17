//! libseat session backend
//!
//! Provides rootless DRM/input access via seatd or logind.

use std::cell::RefCell;
use std::collections::HashMap;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::Path;
use std::rc::Rc;
use std::sync::mpsc;

use anyhow::{Context, Result};
use libseat::{Seat, SeatEvent, SeatRef};
use log::{debug, info, trace, warn};

/// Session event from libseat
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEvent {
    /// Session enabled (VT acquired)
    Enable,
    /// Session disabled (VT released)
    Disable,
}

/// Device opened via libseat
pub struct SeatDevice {
    /// libseat device ID
    #[allow(dead_code)]
    pub device_id: i32,
    /// File descriptor (owned)
    pub fd: OwnedFd,
}

impl SeatDevice {
    pub fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

/// Shared state for libseat callback
struct SeatState {
    /// Event sender
    event_tx: mpsc::Sender<SessionEvent>,
    /// Is session currently active?
    active: bool,
}

/// libseat session manager
pub struct SeatSession {
    /// libseat handle
    seat: Seat,
    /// Shared state (kept for callback lifetime)
    #[allow(dead_code)]
    state: Rc<RefCell<SeatState>>,
    /// Event receiver
    event_rx: mpsc::Receiver<SessionEvent>,
    /// Opened devices (path -> device_id)
    devices: HashMap<String, i32>,
    /// Device ID counter
    next_device_id: i32,
}

impl SeatSession {
    /// Open a new seat session
    pub fn open() -> Result<Self> {
        let (event_tx, event_rx) = mpsc::channel();

        let state = Rc::new(RefCell::new(SeatState {
            event_tx,
            active: false,
        }));

        let state_clone = state.clone();

        let mut seat = Seat::open(move |seat_ref: &mut SeatRef, event: SeatEvent| {
            let mut state = state_clone.borrow_mut();
            match event {
                SeatEvent::Enable => {
                    info!("libseat: session enabled");
                    state.active = true;
                    let _ = state.event_tx.send(SessionEvent::Enable);
                }
                SeatEvent::Disable => {
                    info!("libseat: session disabled");
                    state.active = false;
                    // Must call disable() to acknowledge
                    if let Err(e) = seat_ref.disable() {
                        warn!("libseat: failed to disable seat: {}", e);
                    }
                    let _ = state.event_tx.send(SessionEvent::Disable);
                }
            }
        })
        .context("Failed to open libseat session")?;

        info!("libseat: opened seat '{}'", seat.name());

        Ok(Self {
            seat,
            state,
            event_rx,
            devices: HashMap::new(),
            next_device_id: 1,
        })
    }

    /// Get the seat name
    #[allow(dead_code)]
    pub fn name(&mut self) -> &str {
        self.seat.name()
    }

    /// Check if session is currently active
    #[allow(dead_code)]
    pub fn is_active(&self) -> bool {
        self.state.borrow().active
    }

    /// Get pollable file descriptor for event loop integration
    #[allow(dead_code)]
    pub fn get_fd(&mut self) -> Result<RawFd> {
        let borrowed_fd = self.seat.get_fd().context("Failed to get seat fd")?;
        Ok(borrowed_fd.as_raw_fd())
    }

    /// Dispatch pending events (call when fd is readable)
    ///
    /// Returns true if events were processed
    pub fn dispatch(&mut self) -> Result<bool> {
        let count = self
            .seat
            .dispatch(0)
            .context("Failed to dispatch seat events")?;
        Ok(count > 0)
    }

    /// Try to receive a session event (non-blocking)
    pub fn try_recv_event(&self) -> Option<SessionEvent> {
        self.event_rx.try_recv().ok()
    }

    /// Open a device (DRM or evdev)
    ///
    /// Returns the device with its fd and internal device_id.
    /// The fd is valid only while the session is active.
    pub fn open_device<P: AsRef<Path>>(&mut self, path: P) -> Result<SeatDevice> {
        let path_str = path.as_ref().to_string_lossy().to_string();

        let device = self
            .seat
            .open_device(&path)
            .with_context(|| format!("Failed to open device: {}", path_str))?;

        let device_id = self.next_device_id;
        self.next_device_id += 1;
        let raw_fd = device.as_fd().as_raw_fd();

        debug!(
            "libseat: opened device {} (id={}, fd={})",
            path_str, device_id, raw_fd
        );

        // Store device id for later closing
        self.devices.insert(path_str, device_id);

        // Wrap fd as OwnedFd
        // Note: libseat manages the fd lifecycle, we dup it for safety
        let dup_fd = nix::unistd::dup(raw_fd).context("Failed to dup device fd")?;
        let owned_fd = unsafe { OwnedFd::from_raw_fd(dup_fd) };

        Ok(SeatDevice {
            device_id,
            fd: owned_fd,
        })
    }

    /// Close a device
    #[allow(dead_code)]
    pub fn close_device(&mut self, device: SeatDevice) -> Result<()> {
        // Find and remove the device from our map
        self.devices.retain(|_, &mut id| id != device.device_id);

        // Note: We don't call seat.close_device() because libseat handles this
        // when the device is dropped or session ends.
        // The OwnedFd in SeatDevice will be closed when dropped.

        trace!("libseat: closed device id={}", device.device_id);
        Ok(())
    }

    /// Request VT switch to another session
    #[allow(dead_code)]
    pub fn switch_session(&mut self, session: i32) -> Result<()> {
        self.seat
            .switch_session(session)
            .with_context(|| format!("Failed to switch to session {}", session))?;
        Ok(())
    }
}

impl Drop for SeatSession {
    fn drop(&mut self) {
        info!("libseat: closing session");
        // Devices will be closed automatically by libseat
        self.devices.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: These tests require seatd or logind to be running
    // and the user to have appropriate permissions.
    // Skip in CI environment.

    #[test]
    #[ignore]
    fn test_open_session() {
        let session = SeatSession::open();
        assert!(session.is_ok(), "Failed to open seat session");
    }
}
