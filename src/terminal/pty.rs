//! PTY (pseudo-terminal) management
//!
//! Creates PTY pair with forkpty and spawns shell in child process.
//! Provides master side read/write and terminal size setting.

#![allow(dead_code)]

use anyhow::{anyhow, Result};
use log::info;
use nix::pty::{forkpty, ForkptyResult, Winsize};
use nix::sys::wait::{waitpid, WaitPidFlag};
use nix::unistd::{ForkResult, Pid};
use std::io;
use std::os::fd::{AsRawFd, OwnedFd};

/// PTY management structure
pub struct Pty {
    /// Master side file descriptor
    master: OwnedFd,
    /// Child process PID
    child_pid: Pid,
}

impl Pty {
    /// Create PTY and spawn shell
    ///
    /// Specify initial terminal size with `cols`, `rows`.
    /// `term_env` sets the TERM environment variable.
    pub fn spawn(cols: u16, rows: u16, term_env: &str) -> Result<Self> {
        Self::spawn_with_pixels(cols, rows, 0, 0, term_env, &[])
    }

    /// Create PTY and spawn shell with extra environment variables
    pub fn spawn_with_env(
        cols: u16,
        rows: u16,
        term_env: &str,
        extra_env: &[(&str, &str)],
    ) -> Result<Self> {
        Self::spawn_with_pixels(cols, rows, 0, 0, term_env, extra_env)
    }

    /// Create PTY and spawn shell (with pixel size)
    ///
    /// Specify initial terminal size with `cols`, `rows`,
    /// and pixel size with `xpixel`, `ypixel`.
    /// `term_env` sets the TERM environment variable.
    /// `extra_env` sets additional environment variables for the child process.
    pub fn spawn_with_pixels(
        cols: u16,
        rows: u16,
        xpixel: u16,
        ypixel: u16,
        term_env: &str,
        extra_env: &[(&str, &str)],
    ) -> Result<Self> {
        let winsize = Winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: xpixel,
            ws_ypixel: ypixel,
        };

        let ForkptyResult {
            master,
            fork_result,
        } = unsafe { forkpty(Some(&winsize), None)? };

        match fork_result {
            ForkResult::Child => {
                // Child process: set environment variables and spawn shell
                std::env::set_var("TERM", term_env);
                std::env::set_var("COLORTERM", "truecolor");

                // Set extra environment variables (e.g., DBUS_SESSION_BUS_ADDRESS for IME)
                for (key, value) in extra_env {
                    std::env::set_var(key, value);
                }

                // If running as root (uid=0), use /bin/login for authentication
                // Otherwise, spawn user's shell directly
                if unsafe { libc::getuid() } == 0 {
                    // Running as root (e.g., systemd service) - require login
                    let login = std::ffi::CString::new("/bin/login").unwrap();
                    let argv0 = std::ffi::CString::new("login").unwrap();
                    match nix::unistd::execvp(&login, &[&argv0]) {
                        Ok(infallible) => match infallible {},
                        Err(e) => panic!("Failed to exec /bin/login: {}", e),
                    }
                } else {
                    // Running as normal user - spawn shell directly
                    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
                    let shell_cstr =
                        std::ffi::CString::new(shell.as_str()).expect("NUL byte in shell path");

                    // Launch as login shell (prefix with '-')
                    let shell_name = std::path::Path::new(&shell)
                        .file_name()
                        .map(|n| format!("-{}", n.to_string_lossy()))
                        .unwrap_or_else(|| "-sh".to_string());
                    let argv0 = std::ffi::CString::new(shell_name).expect("NUL byte in argv0");

                    match nix::unistd::execvp(&shell_cstr, &[&argv0]) {
                        Ok(infallible) => match infallible {},
                        Err(e) => panic!("Failed to spawn shell: {}", e),
                    }
                }
            }
            ForkResult::Parent { child } => {
                info!(
                    "PTY spawned: pid={}, master_fd={}",
                    child,
                    master.as_raw_fd()
                );

                // Set master fd to non-blocking
                let flags = nix::fcntl::fcntl(master.as_raw_fd(), nix::fcntl::FcntlArg::F_GETFL)?;
                let mut flags = nix::fcntl::OFlag::from_bits_truncate(flags);
                flags.insert(nix::fcntl::OFlag::O_NONBLOCK);
                nix::fcntl::fcntl(master.as_raw_fd(), nix::fcntl::FcntlArg::F_SETFL(flags))?;

                Ok(Self {
                    master,
                    child_pid: child,
                })
            }
        }
    }

    /// Non-blocking read from PTY
    ///
    /// Returns number of bytes read if data available.
    /// Returns Ok(0) if no data.
    pub fn read(&self, buf: &mut [u8]) -> Result<usize> {
        match nix::unistd::read(self.master.as_raw_fd(), buf) {
            Ok(n) => Ok(n),
            Err(nix::errno::Errno::EAGAIN) => Ok(0),
            Err(e) => Err(anyhow!("PTY read error: {}", e)),
        }
    }

    /// Write data to PTY
    pub fn write(&self, data: &[u8]) -> Result<usize> {
        match nix::unistd::write(self.master.as_raw_fd(), data) {
            Ok(n) => Ok(n),
            Err(e) => Err(anyhow!("PTY write error: {}", e)),
        }
    }

    /// Change terminal size (TIOCSWINSZ) - without pixel size
    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        self.set_size_with_pixels(cols, rows, 0, 0)
    }

    /// Change terminal size (TIOCSWINSZ) - with pixel size
    pub fn resize_with_pixels(&self, cols: u16, rows: u16, xpixel: u16, ypixel: u16) -> Result<()> {
        self.set_size_with_pixels(cols, rows, xpixel, ypixel)
    }

    /// Change terminal size (TIOCSWINSZ)
    pub fn set_size(&self, cols: u16, rows: u16) -> Result<()> {
        self.set_size_with_pixels(cols, rows, 0, 0)
    }

    /// Change terminal size (TIOCSWINSZ) - with pixel size
    pub fn set_size_with_pixels(
        &self,
        cols: u16,
        rows: u16,
        xpixel: u16,
        ypixel: u16,
    ) -> Result<()> {
        let winsize = Winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: xpixel,
            ws_ypixel: ypixel,
        };

        unsafe {
            let ret = libc::ioctl(
                self.master.as_raw_fd(),
                libc::TIOCSWINSZ,
                &winsize as *const Winsize,
            );
            if ret < 0 {
                return Err(anyhow!("TIOCSWINSZ failed: {}", io::Error::last_os_error()));
            }
        }

        // Send SIGWINCH to child process
        let _ = nix::sys::signal::kill(self.child_pid, nix::sys::signal::Signal::SIGWINCH);

        Ok(())
    }

    /// Check if child process is alive
    pub fn is_alive(&self) -> bool {
        match waitpid(self.child_pid, Some(WaitPidFlag::WNOHANG)) {
            Ok(nix::sys::wait::WaitStatus::StillAlive) => true,
            Ok(_) => false, // Exited
            Err(_) => false,
        }
    }

    /// Get foreground process name
    ///
    /// Gets the PTY's foreground process group
    /// and returns the leader process name.
    /// Can also detect processes inside tmux/screen/zellij.
    /// Returns None if unavailable.
    pub fn foreground_process_name(&self) -> Option<String> {
        // Get foreground process group with tcgetpgrp
        let pgid = unsafe { libc::tcgetpgrp(self.master.as_raw_fd()) };
        if pgid <= 0 {
            return None;
        }

        // Get process name
        let proc_name = Self::get_process_name(pgid)?;

        // For terminal multiplexers, find the foreground process inside
        match proc_name.as_str() {
            // tmux: get current command via tmux display-message
            name if name == "tmux: client" || name.starts_with("tmux") => {
                Self::get_tmux_foreground_command().or_else(|| Self::find_leaf_process(pgid))
            }
            // screen: screen -Q title or process tree
            "screen" | "SCREEN" => {
                Self::get_screen_foreground_command().or_else(|| Self::find_leaf_process(pgid))
            }
            // zellij: traverse process tree
            "zellij" => {
                Self::find_zellij_foreground(pgid).or_else(|| Self::find_leaf_process(pgid))
            }
            // Normal process
            _ => Some(proc_name),
        }
    }

    /// Get process name from PID
    fn get_process_name(pid: i32) -> Option<String> {
        let comm_path = format!("/proc/{}/comm", pid);
        std::fs::read_to_string(&comm_path)
            .ok()
            .map(|s| s.trim().to_string())
    }

    /// Get currently running command in tmux's current pane
    fn get_tmux_foreground_command() -> Option<String> {
        let output = std::process::Command::new("tmux")
            .args(["display-message", "-p", "#{pane_current_command}"])
            .output()
            .ok()?;

        if output.status.success() {
            let cmd = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !cmd.is_empty() {
                return Some(cmd);
            }
        }
        None
    }

    /// Get currently running command in screen's current window
    fn get_screen_foreground_command() -> Option<String> {
        // Get current window title with screen -Q title
        // (In many cases, the running command name becomes the title)
        let output = std::process::Command::new("screen")
            .args(["-Q", "title"])
            .output()
            .ok()?;

        if output.status.success() {
            let title = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !title.is_empty() && title != "bash" && title != "zsh" && title != "sh" {
                return Some(title);
            }
        }

        // Fallback: try to get from hardstatus
        None
    }

    /// Find currently running process in zellij's current pane
    fn find_zellij_foreground(_client_pid: i32) -> Option<String> {
        // zellij server process manages child processes
        // Find pane process via server from client PID

        // First find zellij-server process
        if let Ok(entries) = std::fs::read_dir("/proc") {
            for entry in entries.flatten() {
                if let Ok(pid) = entry.file_name().to_string_lossy().parse::<i32>() {
                    if let Some(name) = Self::get_process_name(pid) {
                        if name == "zellij" {
                            // Find deepest process from zellij server's child processes
                            if let Some(leaf) = Self::find_leaf_process(pid) {
                                // Prefer non-shell processes
                                if !matches!(leaf.as_str(), "bash" | "zsh" | "sh" | "fish") {
                                    return Some(leaf);
                                }
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// Traverse process tree to find deepest leaf process
    /// Excludes multiplexers and shells to find actual command
    fn find_leaf_process(pid: i32) -> Option<String> {
        let mut current_pid = pid;
        let mut last_meaningful_name: Option<String> = None;
        let mut visited = std::collections::HashSet::new();

        loop {
            if visited.contains(&current_pid) {
                break;
            }
            visited.insert(current_pid);

            // Get child processes
            let children = Self::get_children(current_pid);

            if children.is_empty() {
                // Leaf process
                if let Some(name) = Self::get_process_name(current_pid) {
                    return Some(name);
                }
                break;
            }

            // Save current process name (excluding multiplexers/shells)
            if let Some(name) = Self::get_process_name(current_pid) {
                if !Self::is_wrapper_process(&name) {
                    last_meaningful_name = Some(name);
                }
            }

            // Follow first child process (assumed to be active pane)
            current_pid = children[0];
        }

        last_meaningful_name
    }

    /// Get list of child process IDs
    fn get_children(pid: i32) -> Vec<i32> {
        let children_path = format!("/proc/{}/task/{}/children", pid, pid);
        std::fs::read_to_string(&children_path)
            .unwrap_or_default()
            .split_whitespace()
            .filter_map(|s| s.parse().ok())
            .collect()
    }

    /// Check if process is a wrapper (multiplexer, shell, etc.)
    fn is_wrapper_process(name: &str) -> bool {
        matches!(
            name,
            "tmux"
                | "tmux: client"
                | "tmux: server"
                | "screen"
                | "SCREEN"
                | "zellij"
                | "bash"
                | "zsh"
                | "sh"
                | "fish"
                | "dash"
                | "ksh"
                | "tcsh"
                | "csh"
        ) || name.starts_with("tmux:")
    }

    /// Get the UID of the child process
    pub fn child_uid(&self) -> Option<u32> {
        let status_path = format!("/proc/{}/status", self.child_pid);
        let content = std::fs::read_to_string(&status_path).ok()?;

        for line in content.lines() {
            if line.starts_with("Uid:") {
                // Format: "Uid:\treal\teffective\tsaved\tfs"
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    return parts[1].parse().ok();
                }
            }
        }
        None
    }

    /// Get the home directory of the child process's owner
    pub fn child_home_dir(&self) -> Option<String> {
        let uid = self.child_uid()?;

        // Use getpwuid to get user info
        unsafe {
            let pwd = libc::getpwuid(uid);
            if pwd.is_null() {
                return None;
            }
            let home = std::ffi::CStr::from_ptr((*pwd).pw_dir);
            home.to_str().ok().map(|s| s.to_string())
        }
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        // Send SIGHUP and wait for child process to exit
        let _ = nix::sys::signal::kill(self.child_pid, nix::sys::signal::Signal::SIGHUP);
        let _ = waitpid(self.child_pid, None);
    }
}

/// Read and expand /etc/issue file (like getty does)
///
/// Expands the following escape sequences:
/// - \d  Current date
/// - \l  TTY name (e.g., tty2)
/// - \m  Machine architecture
/// - \n  Hostname (nodename)
/// - \o  Domain name
/// - \r  Kernel release
/// - \s  Kernel name (e.g., Linux)
/// - \t  Current time
/// - \v  Kernel version
/// - \\  Literal backslash
///
/// Returns None if /etc/issue doesn't exist or can't be read.
pub fn read_issue(tty_name: &str) -> Option<String> {
    let content = std::fs::read_to_string("/etc/issue").ok()?;
    Some(expand_issue(&content, tty_name))
}

/// Expand /etc/issue escape sequences
fn expand_issue(content: &str, tty_name: &str) -> String {
    let mut result = String::with_capacity(content.len() * 2);
    let mut chars = content.chars().peekable();

    // Get system info (cached)
    let uname = get_uname();
    let hostname = uname.nodename.clone();
    let machine = uname.machine.clone();
    let release = uname.release.clone();
    let sysname = uname.sysname.clone();
    let version = uname.version.clone();

    // Get domain name
    let domainname = get_domainname().unwrap_or_default();

    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('d') => {
                    // Current date
                    let now = chrono::Local::now();
                    result.push_str(&now.format("%a %b %d %Y").to_string());
                }
                Some('l') => {
                    // TTY name
                    result.push_str(tty_name);
                }
                Some('m') => {
                    // Machine architecture
                    result.push_str(&machine);
                }
                Some('n') => {
                    // Hostname
                    result.push_str(&hostname);
                }
                Some('o') => {
                    // Domain name
                    result.push_str(&domainname);
                }
                Some('r') => {
                    // Kernel release
                    result.push_str(&release);
                }
                Some('s') => {
                    // Kernel name
                    result.push_str(&sysname);
                }
                Some('t') => {
                    // Current time
                    let now = chrono::Local::now();
                    result.push_str(&now.format("%H:%M:%S").to_string());
                }
                Some('v') => {
                    // Kernel version
                    result.push_str(&version);
                }
                Some('\\') => {
                    result.push('\\');
                }
                Some(other) => {
                    // Unknown escape, keep as-is
                    result.push('\\');
                    result.push(other);
                }
                None => {
                    result.push('\\');
                }
            }
        } else {
            result.push(c);
        }
    }

    result
}

/// System info from uname()
struct UnameInfo {
    sysname: String,
    nodename: String,
    release: String,
    version: String,
    machine: String,
}

fn get_uname() -> UnameInfo {
    let mut utsname: libc::utsname = unsafe { std::mem::zeroed() };
    unsafe { libc::uname(&mut utsname) };

    // Use CStr::from_ptr for portable handling of utsname fields
    // (i8 on x86_64, u8 on aarch64)
    unsafe fn field_to_string(ptr: *const libc::c_char) -> String {
        std::ffi::CStr::from_ptr(ptr).to_string_lossy().into_owned()
    }

    unsafe {
        UnameInfo {
            sysname: field_to_string(utsname.sysname.as_ptr() as *const libc::c_char),
            nodename: field_to_string(utsname.nodename.as_ptr() as *const libc::c_char),
            release: field_to_string(utsname.release.as_ptr() as *const libc::c_char),
            version: field_to_string(utsname.version.as_ptr() as *const libc::c_char),
            machine: field_to_string(utsname.machine.as_ptr() as *const libc::c_char),
        }
    }
}

fn get_domainname() -> Option<String> {
    std::fs::read_to_string("/proc/sys/kernel/domainname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s != "(none)")
}
