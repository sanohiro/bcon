# bcon

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

GPU-accelerated terminal emulator for Linux console (TTY) — no X11/Wayland required.

**[日本語版 README](README.ja.md)**

## Overview

**bcon** brings modern terminal features directly to the Linux console — GPU-accelerated, smooth, beautiful rendering without the desktop environment overhead.

Think of it as "Ghostty for the Linux console" — focusing on what matters: crisp text, smooth scrolling, true color, and responsive input.

## Why bcon?

AI coding tools (Claude Code, Codex, Gemini CLI) have transformed development workflows. We spend less time in VSCode and more time in the terminal.

Look around — the only thing running on your X11/Wayland session might be a terminal emulator. So why not skip the desktop entirely?

**bcon** is the answer. It brings the modern terminal experience of Ghostty or Alacritty directly to the Linux console. GPU acceleration, True Color, Sixel/Kitty graphics, Japanese input — no X11 required.

### What bcon does (and doesn't do)

bcon is the **foundation layer**. Leave session management and multiplexing to the tools that do it best:

- **Sessions**: tmux, zellij, screen
- **Files**: yazi, ranger
- **Editors**: Emacs, Neovim, Helix

Following the Unix philosophy, bcon does one thing well: beautiful, fast rendering.

**Enjoy your CLI life.**

## Features

### Rendering
- **GPU Rendering**: OpenGL ES via DRM/KMS + EGL + GBM
- **Sharp Text**: Pixel-aligned glyph rendering
- **True Color**: Full 24-bit color support
- **Ligatures**: Font ligature support (FiraCode, JetBrains Mono, etc.)
- **Emoji**: Color emoji rendering (Noto Color Emoji)
- **HiDPI Scaling**: Configurable display scale (1.0x - 2.0x)
- **HDR Detection**: Automatic HDR capability detection from EDID

### Graphics
- **Sixel Graphics**: Display images in terminal
- **Kitty Graphics Protocol**: Fast image transfer (direct, file, shared memory)

### Terminal
- **Scrollback**: Configurable scrollback buffer (default: 10,000 lines)
- **Mouse Support**: Selection, wheel scroll, button events (X10/SGR/URXVT protocols)
- **OSC 52 Clipboard**: Apps can read/write clipboard via escape sequences
- **Bracketed Paste**: Secure paste mode support

### Input
- **Keyboard**: Full keyboard support via evdev + xkbcommon
- **Japanese Input**: fcitx5 integration via D-Bus
- **IME Auto-disable**: Automatically disable IME for vim/emacs/etc.
- **Key Repeat**: Configurable key repeat delay/rate

### UX
- **Copy Mode**: Vim-like keyboard navigation for text selection
- **Text Search**: Incremental search in scrollback (Ctrl+Shift+F)
- **Screenshot**: Save terminal as PNG (PrintScreen or Ctrl+Shift+S)
- **Font Scaling**: Runtime font size adjustment (Ctrl+Plus/Minus)
- **Monitor Hotplug**: Automatic detection and switching of monitors
- **External Monitor Priority**: Auto-switch to HDMI/DP when connected (laptops)
- **Visual Bell**: Screen flash on bell character
- **URL Detection**: Ctrl+Click to copy URLs

## Requirements

- Linux with DRM/KMS support (Debian/Ubuntu recommended)
- GPU with OpenGL ES 2.0+

## Installation (Debian/Ubuntu)

### Basic Setup

```bash
# 1. Install bcon
curl -fsSL https://sanohiro.github.io/bcon/install.sh | sudo sh
sudo apt install bcon

# 2. Generate config file
sudo bcon --init-config=system           # Default
sudo bcon --init-config=system,vim       # Vim users
sudo bcon --init-config=system,emacs     # Emacs users

# 3. Enable systemd service (tty2)
sudo systemctl disable getty@tty2
sudo systemctl enable bcon@tty2
sudo systemctl start bcon@tty2

# 4. Switch to bcon
# Ctrl+Alt+F2
```

### Japanese Environment Setup

```bash
# 1. Install bcon and Japanese packages
curl -fsSL https://sanohiro.github.io/bcon/install.sh | sudo sh
sudo apt install bcon fonts-noto-cjk fonts-noto-color-emoji

# Minimal fcitx5 install (recommended)
# Standard fcitx5 pulls in many Qt/GTK GUI modules.
# bcon runs without X11/Wayland, so GUI is unnecessary.
# Use --no-install-recommends for minimal footprint.
sudo apt install --no-install-recommends fcitx5 fcitx5-mozc

# 2. Setup fcitx5 auto-start
echo 'fcitx5 -d &>/dev/null' >> ~/.bashrc
# or ~/.zshrc

# 3. Generate config file (Japanese preset)
sudo bcon --init-config=system,vim,jp    # Vim users
sudo bcon --init-config=system,emacs,jp  # Emacs users

# 4. Enable systemd service (tty2)
sudo systemctl disable getty@tty2
sudo systemctl enable bcon@tty2
sudo systemctl start bcon@tty2

# 5. Switch to bcon
# Ctrl+Alt+F2

# Toggle IME: Ctrl+Space (fcitx5 default)
```

### User Login Session (no sudo required)

Start directly from GDM/SDDM login screen:

```bash
# 1. Install bcon
curl -fsSL https://sanohiro.github.io/bcon/install.sh | sudo sh
sudo apt install bcon

# 2. Generate user config
bcon --init-config=vim,jp    # saves to ~/.config/bcon/config.toml

# 3. Select "bcon" session from login screen
```

Log in directly to bcon without starting a desktop environment. Saves memory and boot time.

## Configuration

Config file locations:
- `/etc/bcon/config.toml` (system config, for systemd service)
- `~/.config/bcon/config.toml` (user config)

### Available Presets

| Preset | Description |
|--------|-------------|
| `default` | Standard keybinds (Ctrl+Shift+C/V, etc.) |
| `vim` | Vim-like scroll (Ctrl+Shift+U/D) |
| `emacs` | Emacs-like scroll (Alt+Shift+V/N) |
| `japanese` / `jp` | CJK fonts + IME auto-disable |
| `system` | Write to /etc/bcon/config.toml instead of user config |

### Example Config

```toml
[font]
main = "/usr/share/fonts/truetype/firacode/FiraCode-Regular.ttf"
cjk = "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc"
emoji = "/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf"
size = 16.0
render_mode = "lcd"
lcd_filter = "light"

[keybinds]
copy = ["ctrl+shift+c", "ctrl+insert"]
paste = ["ctrl+shift+v", "shift+insert"]
screenshot = ["printscreen", "ctrl+shift+s"]

[terminal]
scrollback_lines = 10000
ime_disabled_apps = ["vim", "nvim", "emacs", "less", "man"]

[keyboard]
repeat_delay = 400           # Key repeat delay (ms)
repeat_rate = 30             # Key repeat rate (ms)
xkb_layout = "us"            # XKB keyboard layout
xkb_options = "ctrl:nocaps"  # XKB options (Caps Lock as Ctrl)

[display]
prefer_external = true       # Prefer external monitors (HDMI/DP) over internal
auto_switch = true           # Auto-switch on hotplug connect/disconnect

[paths]
screenshot_dir = "~/Pictures"
```

### Keybinds

| Action | Default | Description |
|--------|---------|-------------|
| Copy | `Ctrl+Shift+C` | Copy selection to clipboard |
| Paste | `Ctrl+Shift+V` | Paste from clipboard |
| Screenshot | `PrintScreen` | Save screenshot as PNG |
| Search | `Ctrl+Shift+F` | Search in scrollback |
| Copy Mode | `Ctrl+Shift+Space` | Enter vim-like copy mode |
| Font + | `Ctrl+Plus` | Increase font size |
| Font - | `Ctrl+Minus` | Decrease font size |
| Font Reset | `Ctrl+0` | Reset font size |
| Scroll Up | `Shift+PageUp` | Scroll back |
| Scroll Down | `Shift+PageDown` | Scroll forward |

### Copy Mode Keys (Vim-like)

| Key | Action |
|-----|--------|
| `h/j/k/l` | Move cursor |
| `w/b` | Word forward/backward |
| `0/$` | Line start/end |
| `g/G` | Top/bottom of buffer |
| `v` | Start/toggle selection |
| `y` | Yank (copy) and exit |
| `/` | Search |
| `Esc` | Exit copy mode |

## Other Installation Methods

### Build from Source

```bash
# Build dependencies
sudo apt install \
    libdrm-dev libgbm-dev \
    libegl1-mesa-dev libgles2-mesa-dev \
    libxkbcommon-dev libinput-dev libudev-dev \
    libdbus-1-dev libwayland-dev \
    libfontconfig1-dev libfreetype-dev \
    pkg-config cmake clang

# Rust toolchain (1.82+) required
cargo build --release

# Generate config
./target/release/bcon --init-config=vim,jp
```

### Manual Start

Run directly from TTY (virtual console):

```bash
# Switch to TTY
Ctrl+Alt+F2

# Run bcon
sudo ./target/release/bcon

# Return to graphical session
Ctrl+Alt+F1  # or F7
```

### Rootless Mode

Run without root privileges using libseat:

```bash
# Additional package
sudo apt install libseat-dev

# Rootless build
cargo build --release --features seatd

# Run without root
./target/release/bcon
```

Benefits:
- No root required
- Proper session tracking (`loginctl list-sessions`)
- Integration with screen lock, power management

## Limitations

- **Multi-seat (DRM lease)**: Not supported. bcon uses exclusive access to the GPU.
- **Multiple monitors**: Currently outputs to one monitor only.

## License

MIT

## Acknowledgments

Inspired by:
- [Ghostty](https://github.com/ghostty-org/ghostty)
- [foot](https://codeberg.org/dnkl/foot)
- [yaft](https://github.com/uobikiemukot/yaft)
- [Alacritty](https://github.com/alacritty/alacritty)
