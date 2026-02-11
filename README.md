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
- **Editors**: Neovim, Helix

Following the Unix philosophy, bcon does one thing well: beautiful, fast rendering.

**Enjoy your CLI life.**

## Features

### Rendering
- **GPU Rendering**: OpenGL ES via DRM/KMS + EGL + GBM
- **Sharp Text**: Pixel-aligned glyph rendering
- **True Color**: Full 24-bit color support
- **Ligatures**: Font ligature support (FiraCode, JetBrains Mono, etc.)
- **Emoji**: Color emoji rendering (Noto Color Emoji)

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
- **Visual Bell**: Screen flash on bell character
- **URL Detection**: Ctrl+Click to copy URLs

## Requirements

- Linux with DRM/KMS support
- GPU with OpenGL ES 2.0+
- Rust toolchain (1.82+)

### Privilege Options

| Mode | Requirement | Build Command |
|------|-------------|---------------|
| Root mode | Run as root (`sudo`) | `cargo build --release` |
| Rootless mode | systemd-logind or seatd | `cargo build --release --features seatd` |

**Rootless mode** uses [libseat](https://sr.ht/~kennylevinsen/seatd/) for proper session management. Benefits:
- No root required
- Proper session tracking (`loginctl list-sessions`)
- Integration with screen lock, power management
- Clean VT switching with Wayland/X11 sessions

### System Packages (Debian/Ubuntu)

```bash
# Build dependencies
sudo apt install \
    libdrm-dev libgbm-dev \
    libegl1-mesa-dev libgles2-mesa-dev \
    libxkbcommon-dev libinput-dev libudev-dev \
    libdbus-1-dev libwayland-dev \
    libfontconfig1-dev libfreetype-dev \
    pkg-config cmake clang

# Optional: for rootless build (--features seatd)
sudo apt install libseat-dev

# Runtime (fonts)
sudo apt install fonts-dejavu-core

# Optional: Japanese support
sudo apt install fonts-noto-cjk fcitx5 fcitx5-mozc

# Optional: Color emoji
sudo apt install fonts-noto-color-emoji
```

## Installation

### apt (Debian/Ubuntu)

```bash
# Add repository
curl -fsSL https://sanohiro.github.io/bcon/install.sh | sudo sh

# Install
sudo apt install bcon
```

### From source

```bash
# Standard build (requires root to run)
cargo build --release

# Rootless build (requires logind/seatd)
cargo build --release --features seatd

# Generate config file
./target/release/bcon --init-config

# For Japanese users
./target/release/bcon --init-config=vim,jp
```

## Usage

### Manual Start

Run from TTY (virtual console), not inside X11/Wayland:

```bash
# Switch to TTY
Ctrl+Alt+F2

# Run bcon (standard build)
sudo ./target/release/bcon

# Run bcon (rootless build with --features seatd)
./target/release/bcon

# Return to graphical session
Ctrl+Alt+F1  # or F7
```

### systemd Service (Recommended for Daily Use)

```bash
# Install binary and service
sudo cp target/release/bcon /usr/local/bin/
sudo cp bcon@.service /etc/systemd/system/

# Generate system config
sudo bcon --init-config=system,vim,jp

# Enable on tty2 (keeps tty1 as fallback)
sudo systemctl disable getty@tty2
sudo systemctl enable bcon@tty2
sudo systemctl start bcon@tty2

# Switch to bcon
Ctrl+Alt+F2
```

### Login Session (GDM/SDDM)

With rootless build, bcon can be selected as a session from the login screen:

```bash
# Install session file
sudo cp bcon.desktop /usr/share/wayland-sessions/

# Now "bcon" appears in GDM/SDDM session selector
```

This allows direct login to bcon without starting a desktop environment — saves memory and boot time.

### Rootless systemd Service

For rootless builds (`--features seatd`), create a user-specific service:

```ini
# /etc/systemd/system/bcon@.service
[Unit]
Description=bcon terminal on %I
After=systemd-logind.service

[Service]
Type=simple
ExecStart=/usr/local/bin/bcon
StandardInput=tty
StandardOutput=tty
TTYPath=/dev/%I
TTYReset=yes
TTYVHangup=yes

# Rootless: run as regular user
User=youruser
Group=youruser
SupplementaryGroups=video input

[Install]
WantedBy=multi-user.target
```

### With Japanese Input (IME)

```bash
# Start fcitx5 daemon
fcitx5 -d

# Run bcon (preserve environment for D-Bus)
sudo -E ./target/release/bcon

# Toggle IME: configured key (default: Ctrl+Shift+J)
```

## Configuration

Config file locations (in priority order):
1. `~/.config/bcon/config.toml` (user config)
2. `/etc/bcon/config.toml` (system config)
3. Built-in defaults

### Generate Config

```bash
# Default (international)
bcon --init-config

# With presets (combine with comma)
bcon --init-config=vim,jp
bcon --init-config=emacs,japanese
```

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
render_mode = "lcd"       # "grayscale" or "lcd"
lcd_filter = "light"      # "none" | "default" | "light" | "legacy"

[keybinds]
copy = ["ctrl+shift+c", "ctrl+insert"]
paste = ["ctrl+shift+v", "shift+insert"]
screenshot = ["printscreen", "ctrl+shift+s"]
ime_toggle = "ctrl+shift+j"

[terminal]
scrollback_lines = 10000
ime_disabled_apps = ["vim", "nvim", "emacs", "less", "man"]

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
| IME Toggle | `Ctrl+Shift+J` | Toggle IME on/off |

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

## Limitations

- **Multi-seat (DRM lease)**: Not supported. bcon uses exclusive access to the GPU. For multi-seat setups (multiple users on one PC with separate monitors/keyboards), use traditional X11/Wayland solutions.
- **Multiple monitors**: Currently outputs to one monitor only. If you have multiple monitors connected, bcon will use the first detected display.

## Architecture

```
User Application (shell, vim, etc.)
         ↓ PTY
        bcon
         ↓
┌────────────────────────────┐
│ VT Parser (escape codes)   │
│ Text Shaper (rustybuzz)    │
│ Glyph Rasterizer (fontdue) │
│ GPU Renderer (OpenGL ES)   │
│ DRM/KMS Output             │
└────────────────────────────┘
         ↓
       Display
```

## Target Users

- Developers using AI coding tools (Claude Code, aider, Codex)
- Server administrators who want rich terminal features
- Minimalists who don't need a full desktop environment
- Raspberry Pi / embedded Linux users

## License

MIT

## Acknowledgments

Inspired by:
- [Ghostty](https://github.com/ghostty-org/ghostty)
- [foot](https://codeberg.org/dnkl/foot)
- [yaft](https://github.com/uobikiemukot/yaft)
- [Alacritty](https://github.com/alacritty/alacritty)
