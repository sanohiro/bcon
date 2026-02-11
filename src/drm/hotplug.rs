//! DRM hotplug detection
//!
//! Monitors udev events for DRM connector changes (monitor plug/unplug).
//! Integrates with bcon's event loop for real-time hotplug handling.

#![allow(dead_code)]

use anyhow::{Context, Result};
use log::{debug, info, warn};
use std::os::unix::io::{AsRawFd, RawFd};

use drm::control::connector;

/// Hotplug event types
#[derive(Debug, Clone)]
pub enum HotplugEvent {
    /// A connector state changed (connect/disconnect/mode change)
    ConnectorChanged,
}

/// udev-based hotplug monitor for DRM devices
pub struct HotplugMonitor {
    socket: udev::MonitorSocket,
}

impl HotplugMonitor {
    /// Create a new hotplug monitor for DRM subsystem
    pub fn new() -> Result<Self> {
        let socket = udev::MonitorBuilder::new()
            .context("Failed to create udev monitor builder")?
            .match_subsystem("drm")
            .context("Failed to match drm subsystem")?
            .listen()
            .context("Failed to start udev monitor")?;

        info!("DRM hotplug monitor initialized");
        Ok(Self { socket })
    }

    /// Get the raw file descriptor for polling
    pub fn as_raw_fd(&self) -> RawFd {
        self.socket.as_raw_fd()
    }

    /// Check for hotplug events (non-blocking)
    ///
    /// Returns Some(HotplugEvent) if a hotplug event occurred.
    pub fn poll(&mut self) -> Option<HotplugEvent> {
        // Iterate over available events (non-blocking due to MonitorSocket)
        for event in self.socket.iter() {
            // Only process "change" actions with HOTPLUG=1
            if event.action().map(|a| a == "change").unwrap_or(false) {
                // Check for HOTPLUG property
                if let Some(hotplug) = event.property_value("HOTPLUG") {
                    if hotplug == "1" {
                        debug!("DRM hotplug event: {:?}", event.devpath().to_string_lossy());
                        return Some(HotplugEvent::ConnectorChanged);
                    }
                }
            }
        }
        None
    }
}

/// Snapshot of connector state for comparison
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectorSnapshot {
    pub handle: connector::Handle,
    pub connected: bool,
    pub mode_count: usize,
    pub interface: connector::Interface,
}

impl ConnectorSnapshot {
    pub fn from_info(handle: connector::Handle, info: &connector::Info) -> Self {
        Self {
            handle,
            connected: info.state() == connector::State::Connected,
            mode_count: info.modes().len(),
            interface: info.interface(),
        }
    }

    /// Check if this is an external connector (HDMI, DP, DVI, VGA)
    pub fn is_external(&self) -> bool {
        super::device::is_external_connector(self.interface)
    }

    /// Check if this is an internal connector (eDP, LVDS, DSI)
    pub fn is_internal(&self) -> bool {
        super::device::is_internal_connector(self.interface)
    }
}

/// Take a snapshot of all connector states
pub fn snapshot_connectors(device: &impl drm::control::Device) -> Result<Vec<ConnectorSnapshot>> {
    let resources = device
        .resource_handles()
        .context("Failed to get DRM resources")?;

    let mut snapshots = Vec::new();
    for &handle in resources.connectors() {
        if let Ok(info) = device.get_connector(handle, false) {
            snapshots.push(ConnectorSnapshot::from_info(handle, &info));
        }
    }

    Ok(snapshots)
}

/// Detect changes between two connector snapshots
pub fn detect_changes(old: &[ConnectorSnapshot], new: &[ConnectorSnapshot]) -> ConnectorChanges {
    let mut changes = ConnectorChanges::default();

    for new_conn in new {
        let old_conn = old.iter().find(|c| c.handle == new_conn.handle);

        match old_conn {
            Some(old) => {
                if !old.connected && new_conn.connected {
                    changes.connected.push(new_conn.clone());
                } else if old.connected && !new_conn.connected {
                    changes.disconnected.push(old.clone());
                } else if old.mode_count != new_conn.mode_count {
                    changes.mode_changed.push(new_conn.handle);
                }
            }
            None => {
                // New connector appeared (rare, but possible with USB DisplayLink)
                if new_conn.connected {
                    changes.connected.push(new_conn.clone());
                }
            }
        }
    }

    // Check for removed connectors
    for old_conn in old {
        if !new.iter().any(|c| c.handle == old_conn.handle) {
            if old_conn.connected {
                changes.disconnected.push(old_conn.clone());
            }
        }
    }

    changes
}

/// Summary of connector state changes
#[derive(Debug, Default)]
pub struct ConnectorChanges {
    /// Connectors that were connected (with snapshot info)
    pub connected: Vec<ConnectorSnapshot>,
    /// Connectors that were disconnected (with snapshot info)
    pub disconnected: Vec<ConnectorSnapshot>,
    /// Connectors whose mode list changed
    pub mode_changed: Vec<connector::Handle>,
}

impl ConnectorChanges {
    /// Returns true if any changes occurred
    pub fn has_changes(&self) -> bool {
        !self.connected.is_empty() || !self.disconnected.is_empty() || !self.mode_changed.is_empty()
    }

    /// Log the changes
    pub fn log(&self) {
        for snap in &self.connected {
            info!(
                "Monitor connected: {:?} ({:?})",
                snap.handle, snap.interface
            );
        }
        for snap in &self.disconnected {
            warn!(
                "Monitor disconnected: {:?} ({:?})",
                snap.handle, snap.interface
            );
        }
        for handle in &self.mode_changed {
            info!("Monitor mode changed: {:?}", handle);
        }
    }

    /// Check if an external monitor was connected
    pub fn external_connected(&self) -> bool {
        self.connected.iter().any(|s| s.is_external())
    }

    /// Check if an external monitor was disconnected
    pub fn external_disconnected(&self) -> bool {
        self.disconnected.iter().any(|s| s.is_external())
    }

    /// Get the newly connected external connector (if any)
    pub fn get_connected_external(&self) -> Option<&ConnectorSnapshot> {
        self.connected.iter().find(|s| s.is_external())
    }
}
