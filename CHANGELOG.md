# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.2.0] - 2026-03-29

### Added
- Kitty Graphics scroll tracking — images scroll with text, visible in scrollback
- Kitty Graphics Z-order — `z<0` draws below text, `z>=0` draws above text (2-pass rendering)
- Kitty Graphics delete targets: `d=z/Z` (by z-index), `d=q/Q` (by coordinate + z-index)
- Kitty Graphics test suite (21 tests covering display, deletion, scroll, z-order)
- Kitty Graphics Protocol implementation status doc (`docs/kitty-graphics-status.md`)

### Fixed
- Kitty Graphics image deletion (`a=d`) — default target, redraw trigger, all coordinate-based targets
- Kitty Graphics `z<0` images preserved from text overwrite (matches kitty spec)
- Kitty Graphics LRU cache: O(n) Vec replaced with O(1) generation counter

## [1.1.1] - 2026-03-29

### Fixed
- Kitty Graphics image deletion (`a=d`) now works — default target `d=a` was broken, screen wasn't redrawn after deletion
- Kitty Graphics texture cache LRU eviction replaced O(n) Vec with O(1) generation counter — fixes freeze with rapid image updates (e.g., Xkitty)

### Added
- Kitty Graphics delete targets: `d=a/A`, `d=i/I`, `d=n/N`, `d=c/C`, `d=p/P`, `d=x/X`, `d=y/Y` (uppercase frees image data, lowercase removes placements only)
- Kitty Graphics Protocol implementation status doc (`docs/kitty-graphics-status.md`)
- Kitty Graphics test suite (`tests/kitty-graphics-test.sh`)

## [1.1.0] - 2026-03-18

### Added
- Touchpad gesture support: 3-finger swipe for tab switching, pinch to zoom font size
- Touchpad settings: `tap_to_click`, `natural_scroll`, `disable_while_typing` in `[mouse]`
- `[mouse] speed` config option with resolution auto-scaling for HiDPI displays
- `[drm] device` config option (`"auto"` or explicit path like `"/dev/dri/card1"`)

### Fixed
- Hardware cursor hotspot offset — click position now matches crosshair center
- DRM auto-detect probes each GPU for connected displays (fixes Optimus laptops)
- Login screen idle timeout no longer triggers systemd restart loop
- Mouse speed: explicitly configured values used as-is, auto-scale only at default
- Emacs preset: close_pane keybind conflict resolved

## [1.0.4] - 2026-03-18

### Fixed
- Login screen idle timeout causing screen flicker — `login(1)` default 60s timeout triggered a systemd restart loop; now disabled via `LOGIN_TIMEOUT=0`

## [1.0.3] - 2026-03-18

### Fixed
- Hardware cursor hotspot offset — click position now matches crosshair center (was offset by cursor image size on some GPUs)
- Mouse speed auto-scaling: explicitly configured `speed` is used as-is; auto-scale only applies at default (1.0)

### Added
- `[mouse] speed` config option — cursor speed multiplier, auto-scaled by resolution when not set
- Font size documentation with resolution-based recommendations

## [1.0.2] - 2026-03-18

### Fixed
- DRM auto-detect now probes each GPU for connected displays instead of always using card0
  - Fixes Optimus laptops (Intel + NVIDIA) where card0 (NVIDIA) has no connected connectors

### Added
- `[drm] device` config option ("auto" default, or explicit path like "/dev/dri/card1")

## [1.0.1] - 2026-03-16

### Fixed
- Emacs preset: close_pane keybind (Ctrl+Shift+W) conflicted with copy — changed to Alt+Shift+W

## [1.0.0] - 2026-03-15

### Added
- Documentation site: `docs/` with installation, configuration, keybinds guides (EN/JA)
- CI workflow: `cargo build` on push/PR with Rust cache
- LICENSE file, social preview image
- Release badge and CI badge on README

### Changed
- README slimmed down (~150 lines), detailed docs moved to `docs/`
- OpenGL ES requirement corrected to 3.0+ (matches actual shader version)

## [0.6.1] - 2026-03-09

### Fixed
- Nerd Font auto-detection now scans `/usr/local/share/fonts/` (system-wide install path)
- README: Nerd Font install instructions now use `/usr/local/share/fonts/` for systemd compatibility

## [0.6.0] - 2026-03-08

### Added
- Font ligature support (FiraCode, JetBrains Mono, etc.)
- Split panes with binary tree layout (horizontal/vertical)
- Multiple tabs with tab bar
- Pane navigation, resize, zoom
- Mouse click to switch pane focus
- Dead pane auto-close

### Improved
- Input responsiveness

## [0.5.1] - 2026-03-01

### Fixed
- Allow Kitty graphics remote transfer (`t=f`/`t=t`/`t=s`) by default

## [0.5.0] - 2026-02-28

### Added
- Security hardening
- Performance improvements
- Code quality improvements

## [0.4.0] - 2026-02-24

### Added
- Notification system: OSC 9 (iTerm2) and OSC 99 (Kitty) protocols
- Toast overlay with progress bar
- Notification panel (Ctrl+Shift+N) and mute toggle (Ctrl+Shift+M)
- Control sequence improvements

## [0.3.0] - 2026-02-18

### Added
- Font name resolution via fontconfig
- Mouse protocol improvements (SGR-Pixels)
- Rendering quality improvements
- IME preedit display fix
- Headless IME auto-setup (fcitx5 via D-Bus)

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
