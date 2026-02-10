//! Keyboard input
//!
//! Set console TTY stdin to raw mode and
//! read keystrokes non-blocking.
//! Kernel VT layer handles scancode to character conversion,
//! so we get normal ASCII keys and escape sequences (arrow keys, etc.).

use anyhow::{anyhow, Result};
use log::info;
use nix::fcntl::{fcntl, FcntlArg, OFlag};
use nix::sys::termios::{self, Termios};
use std::os::fd::{AsRawFd, BorrowedFd};

/// Keyboard input management
pub struct Keyboard {
    /// stdin file descriptor
    fd: i32,
    /// Original termios settings (for restoration)
    orig_termios: Termios,
}

impl Keyboard {
    /// Initialize keyboard input by setting TTY to raw mode
    pub fn new() -> Result<Self> {
        let fd = std::io::stdin().as_raw_fd();
        let borrowed = unsafe { BorrowedFd::borrow_raw(fd) };

        // Save original settings
        let orig_termios =
            termios::tcgetattr(borrowed).map_err(|e| anyhow!("tcgetattr failed: {}", e))?;

        // Set to raw mode
        let mut raw = orig_termios.clone();
        termios::cfmakeraw(&mut raw);
        termios::tcsetattr(borrowed, termios::SetArg::TCSAFLUSH, &raw)
            .map_err(|e| anyhow!("tcsetattr failed: {}", e))?;

        // Set non-blocking
        let flags = fcntl(fd, FcntlArg::F_GETFL).map_err(|e| anyhow!("F_GETFL failed: {}", e))?;
        let mut flags = OFlag::from_bits_truncate(flags);
        flags.insert(OFlag::O_NONBLOCK);
        fcntl(fd, FcntlArg::F_SETFL(flags)).map_err(|e| anyhow!("F_SETFL failed: {}", e))?;

        info!("Keyboard initialized (raw mode)");

        Ok(Self { fd, orig_termios })
    }

    /// Read key input non-blocking
    ///
    /// Returns number of bytes read if data available.
    /// Returns Ok(0) if no data available.
    pub fn read(&self, buf: &mut [u8]) -> Result<usize> {
        match nix::unistd::read(self.fd, buf) {
            Ok(n) => Ok(n),
            Err(nix::errno::Errno::EAGAIN) => Ok(0),
            Err(e) => Err(anyhow!("Keyboard read error: {}", e)),
        }
    }
}

impl Drop for Keyboard {
    fn drop(&mut self) {
        // Restore original termios settings
        let borrowed = unsafe { BorrowedFd::borrow_raw(self.fd) };
        let _ = termios::tcsetattr(borrowed, termios::SetArg::TCSAFLUSH, &self.orig_termios);
        info!("Keyboard settings restored");
    }
}
