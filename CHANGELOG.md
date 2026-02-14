# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.17] - 2026-02-14

### Fixed
- GDM login session: keyboard initialization no longer requires TTY
- evdev keyboard via libseat now works standalone (no TTY needed)

## [0.2.16] - 2026-02-14

### Fixed
- Import AsFd trait for libseat Device fd access

## [0.2.15] - 2026-02-14

### Fixed
- libseat Device fd access: use as_fd().as_raw_fd()

## [0.2.14] - 2026-02-14

### Fixed
- Build errors with libseat 0.2 API changes
- Unused import warnings
- Unreachable pattern warning in emoji detection

## [0.2.13] - 2026-02-14

### Fixed
- CI build failure: add libseat-dev to build dependencies
- .deb package now includes libseat1 as runtime dependency
- .deb package now includes bcon.desktop and bcon-session

## [0.2.12] - 2026-02-14

### Changed
- Added explicit release instructions to CLAUDE.md

## [0.2.11] - 2026-02-14

### Changed
- libseat (seatd) now enabled by default
- Single binary works for both root and GDM/SDDM login sessions

### Added
- bcon-session wrapper script for login session support
- GDM/SDDM login session documentation

### Fixed
- bcon.desktop now correctly configured for xsessions

## [0.2.10] - 2026-02-14

### Added
- Programmatic Powerline triangle rendering (E0B0-E0B3)
- Smoothstep anti-aliasing for Powerline glyphs
- Powerline support documented in README

## [0.2.9] - 2026-02-13

### Fixed
- LCD subpixel rendering on colored backgrounds (yazi highlight text smearing)
- Box-drawing characters (─│┌┐└┘├┤┬┴┼) now pixel-perfect aligned
- Rounded corners (╭╮╯╰) properly connect with adjacent lines

### Added
- Per-instance LCD disable for grayscale AA on high-chroma backgrounds
- Anti-aliased programmatic rendering for box-drawing characters
- Terminal feature stubs: cursor style (DECSCUSR), focus events, OSC 7/10/11

## [0.1.0] - 2026-02-09

### Added

#### Rendering
- GPU-accelerated rendering via OpenGL ES (DRM/KMS + EGL + GBM)
- LCD subpixel rendering for crisp text
- Font ligature support (FiraCode, JetBrains Mono, etc.)
- Color emoji rendering (Noto Color Emoji)
- True Color (24-bit) support

#### Graphics Protocols
- Sixel graphics display
- Kitty graphics protocol (direct, file, shared memory, temp file transmission)

#### Terminal Emulation
- VT220/xterm compatible escape sequence handling
- Scrollback buffer (configurable, default 10,000 lines)
- Mouse support (X10, SGR, URXVT protocols)
- OSC 52 clipboard integration
- OSC 7 current directory tracking
- OSC 8 hyperlinks
- OSC 10/11 color queries
- Bracketed paste mode
- Synchronized output (CSI ? 2026)
- Focus events (CSI ? 1004)
- Cursor styles (Block, Underline, Bar)
- Kitty keyboard protocol

#### Input
- Full keyboard support via evdev + xkbcommon
- Japanese input via fcitx5 D-Bus integration
- Automatic IME disable for vim/emacs/etc.
- Configurable key repeat delay/rate

#### User Experience
- Vim-like copy mode for text selection
- Incremental search in scrollback
- Screenshot to PNG
- Runtime font size adjustment
- Visual bell (border flash)
- URL detection with Ctrl+Click
- TOML-based configuration
- Multiple keybind presets (default, vim, emacs, japanese)

#### Terminal Detection
- XTGETTCAP (DCS + q) support for capability queries
- Enhanced DA1/DA2 responses
- TIOCGWINSZ with pixel dimensions
- CSI 14/16/18 t window size queries

### Notes

- First public release
- Designed for Linux console (TTY) without X11/Wayland
- Requires root or logind session for DRM access
