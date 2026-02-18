//! Safe wrappers for ioctl system calls
//!
//! Provides error-handling wrappers around common ioctl operations
//! to reduce unsafe boilerplate in device.rs and display.rs.

#![allow(dead_code)]

use anyhow::{anyhow, Result};
use std::os::unix::io::RawFd;

/// Execute an ioctl command that takes no argument.
///
/// # Safety
/// The caller must ensure the fd is valid and the ioctl command
/// is appropriate for the device type.
///
/// # Arguments
/// * `fd` - File descriptor
/// * `cmd` - ioctl command number
/// * `cmd_name` - Human-readable name for error messages
pub fn ioctl_no_arg(fd: RawFd, cmd: libc::c_ulong, cmd_name: &str) -> Result<()> {
    let ret = unsafe { libc::ioctl(fd, cmd) };
    if ret < 0 {
        Err(anyhow!(
            "{} failed on fd {}: {}",
            cmd_name,
            fd,
            std::io::Error::last_os_error()
        ))
    } else {
        Ok(())
    }
}

/// Execute an ioctl command with a mutable argument.
///
/// # Safety
/// The caller must ensure:
/// - The fd is valid
/// - The ioctl command is appropriate for the device type
/// - The argument type matches what the ioctl expects
///
/// # Arguments
/// * `fd` - File descriptor
/// * `cmd` - ioctl command number
/// * `arg` - Mutable reference to the argument
/// * `cmd_name` - Human-readable name for error messages
pub fn ioctl_with_mut_arg<T>(
    fd: RawFd,
    cmd: libc::c_ulong,
    arg: &mut T,
    cmd_name: &str,
) -> Result<()> {
    let ret = unsafe { libc::ioctl(fd, cmd, arg as *mut T) };
    if ret < 0 {
        Err(anyhow!(
            "{} failed on fd {}: {}",
            cmd_name,
            fd,
            std::io::Error::last_os_error()
        ))
    } else {
        Ok(())
    }
}

/// Execute an ioctl command with a const argument (passed by reference).
///
/// # Safety
/// The caller must ensure:
/// - The fd is valid
/// - The ioctl command is appropriate for the device type
/// - The argument type matches what the ioctl expects
///
/// # Arguments
/// * `fd` - File descriptor
/// * `cmd` - ioctl command number
/// * `arg` - Reference to the argument
/// * `cmd_name` - Human-readable name for error messages
pub fn ioctl_with_ref_arg<T>(fd: RawFd, cmd: libc::c_ulong, arg: &T, cmd_name: &str) -> Result<()> {
    let ret = unsafe { libc::ioctl(fd, cmd, arg as *const T) };
    if ret < 0 {
        Err(anyhow!(
            "{} failed on fd {}: {}",
            cmd_name,
            fd,
            std::io::Error::last_os_error()
        ))
    } else {
        Ok(())
    }
}

/// Execute an ioctl command with an integer argument.
///
/// # Safety
/// The caller must ensure:
/// - The fd is valid
/// - The ioctl command is appropriate for the device type
///
/// # Arguments
/// * `fd` - File descriptor
/// * `cmd` - ioctl command number
/// * `arg` - Integer argument
/// * `cmd_name` - Human-readable name for error messages
pub fn ioctl_with_int_arg(
    fd: RawFd,
    cmd: libc::c_ulong,
    arg: libc::c_int,
    cmd_name: &str,
) -> Result<()> {
    let ret = unsafe { libc::ioctl(fd, cmd, arg) };
    if ret < 0 {
        Err(anyhow!(
            "{} failed on fd {}: {}",
            cmd_name,
            fd,
            std::io::Error::last_os_error()
        ))
    } else {
        Ok(())
    }
}

/// Execute an ioctl command that may be interrupted by a signal.
/// Retries on EINTR.
///
/// # Arguments
/// * `fd` - File descriptor
/// * `cmd` - ioctl command number
/// * `arg` - Integer argument
/// * `cmd_name` - Human-readable name for error messages
pub fn ioctl_with_int_arg_retry(
    fd: RawFd,
    cmd: libc::c_ulong,
    arg: libc::c_int,
    cmd_name: &str,
) -> Result<()> {
    loop {
        let ret = unsafe { libc::ioctl(fd, cmd, arg) };
        if ret >= 0 {
            return Ok(());
        }
        let err = std::io::Error::last_os_error();
        if err.kind() != std::io::ErrorKind::Interrupted {
            return Err(anyhow!("{} failed on fd {}: {}", cmd_name, fd, err));
        }
        // EINTR: retry the ioctl
    }
}
