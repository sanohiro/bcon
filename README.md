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

### What bcon does

bcon includes **built-in split panes and tabs** — no need for tmux or screen for basic multiplexing. This is important because terminal multiplexers often break Kitty graphics protocol passthrough, defeating bcon's image rendering capabilities.

Of course, you can still use external tools alongside bcon:

- **Sessions**: tmux, screen (for persistent sessions / SSH)
- **Files**: yazi, ranger
- **Editors**: Emacs, Neovim, Helix

**Enjoy your CLI life.**

## Demo

### Script & Basic Features

https://github.com/user-attachments/assets/8576c907-1f7b-4582-8eb8-de04a853b604

### Real-World Usage (yazi, btop, Claude Code, etc.)

https://github.com/user-attachments/assets/083f80f6-17ec-4026-ab2f-52ba151631e5

## Features

### Rendering
- **GPU Rendering**: OpenGL ES via DRM/KMS + EGL + GBM
- **Sharp Text**: Pixel-aligned glyph rendering
- **True Color**: Full 24-bit color support
- **Ligatures**: Font ligature support (requires a ligature font — see [Recommended Fonts](#recommended-fonts))
- **Emoji**: Color emoji rendering (Noto Color Emoji)
- **Powerline**: Pixel-perfect Powerline/Nerd Font glyphs
- **HiDPI Scaling**: Configurable display scale (1.0x - 2.0x)
- **HDR Detection**: Automatic HDR capability detection from EDID

### Graphics
- **Sixel Graphics**: Display images in terminal
- **Kitty Graphics Protocol**: Fast image transfer (direct, file, shared memory)

### Terminal
- **Scrollback**: Configurable scrollback buffer (default: 10,000 lines)
- **Mouse Support**: Selection, wheel scroll, button events (X10/SGR/URXVT/SGR-Pixels protocols)
- **OSC 52 Clipboard**: Apps can read/write clipboard via escape sequences
- **Bracketed Paste**: Secure paste mode support
- **Colored Underlines**: SGR 58/59 colored underline with 5 styles (single, double, curly, dotted, dashed)
- **Synchronized Output**: Mode 2026 flicker-free rendering for fast-updating applications
- **OSC 4/10/11/12**: Dynamic palette, foreground, background, and cursor color changes
- **Notifications**: OSC 9 (iTerm2) and OSC 99 (Kitty) desktop notification protocols with toast overlay and progress bar

### Input
- **Keyboard**: Full keyboard support via evdev + xkbcommon
- **Kitty Keyboard Protocol**: Progressive enhancement for Neovim, Helix, and other modern TUI apps
- **Japanese Input**: fcitx5 integration via D-Bus
- **IME Auto-disable**: Automatically disable IME for vim/emacs/etc.
- **Key Repeat**: Configurable key repeat delay/rate

### Split Panes & Tabs
- **Split Panes**: Horizontal and vertical split with binary tree layout
- **Pane Navigation**: Move focus between panes (arrow keys or hjkl in vim preset)
- **Pane Resize**: Adjust split ratios with keyboard shortcuts
- **Pane Zoom**: Toggle zoom to expand active pane to full screen
- **Tabs**: Multiple tabs with tab bar display
- **Auto Close**: Dead panes are automatically cleaned up
- **Mouse Focus**: Click to switch pane focus

### UX
- **Copy Mode**: Vim-like keyboard navigation for text selection
- **Text Search**: Incremental search in scrollback (Ctrl+Shift+F)
- **Screenshot**: Save terminal as PNG (PrintScreen or Ctrl+Shift+S)
- **Font Scaling**: Runtime font size adjustment (Ctrl+Plus/Minus)
- **Notification Panel**: Browse notification history (Ctrl+Shift+N), mute toggle (Ctrl+Shift+M)
- **Monitor Hotplug**: Automatic detection and switching of monitors
- **External Monitor Priority**: Auto-switch to HDMI/DP when connected (laptops)
- **Visual Bell**: Screen flash on bell character
- **URL Detection**: Ctrl+Click to copy URLs

## Requirements

- Linux with DRM/KMS support (Debian/Ubuntu recommended)
- GPU with OpenGL ES 2.0+

## Installation (Debian/Ubuntu)

### Basic Setup

For Japanese environment, see [Japanese Environment Setup](#japanese-environment-setup) instead.

```bash
# 1. Install bcon
curl -fsSL https://sanohiro.github.io/bcon/install.sh | sudo sh
sudo apt install bcon

# 2. (Optional) Install Nerd Font for icons (yazi, lsd, etc.)
sudo apt install fontconfig curl  # if not already installed
sudo mkdir -p /usr/local/share/fonts
cd /usr/local/share/fonts
sudo curl -OL https://github.com/ryanoasis/nerd-fonts/releases/latest/download/Hack.tar.xz
sudo tar xf Hack.tar.xz && sudo rm Hack.tar.xz
sudo fc-cache -fv

# 3. Generate config file (auto-detects Nerd Font if installed)
sudo bcon --init-config=system           # Default keybinds
sudo bcon --init-config=system,vim       # Vim-like keybinds
sudo bcon --init-config=system,emacs     # Emacs-like keybinds

# 4. Enable systemd service (tty2)
sudo systemctl disable getty@tty2
sudo systemctl enable bcon@tty2
sudo systemctl start bcon@tty2

# 5. Switch to bcon
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

# 2. (Optional) Install Nerd Font for icons (yazi, lsd, etc.)
sudo apt install fontconfig curl  # if not already installed
sudo mkdir -p /usr/local/share/fonts
cd /usr/local/share/fonts
sudo curl -OL https://github.com/ryanoasis/nerd-fonts/releases/latest/download/Hack.tar.xz
sudo tar xf Hack.tar.xz && sudo rm Hack.tar.xz
sudo fc-cache -fv

# 3. Generate config file (auto-detects Nerd Font if installed)
sudo bcon --init-config=system,jp        # Default keybinds
sudo bcon --init-config=system,vim,jp    # Vim-like keybinds
sudo bcon --init-config=system,emacs,jp  # Emacs-like keybinds

# 4. Enable systemd service (tty2)
sudo systemctl disable getty@tty2
sudo systemctl enable bcon@tty2
sudo systemctl start bcon@tty2

# 5. Switch to bcon
# Ctrl+Alt+F2

# Toggle IME: Ctrl+Space (fcitx5 default)
```

**Emacs users:** Ctrl+Space conflicts with `set-mark-command` (C-SPC). To use Super(Win)+Space instead:

```bash
mkdir -p ~/.config/fcitx5
cat >> ~/.config/fcitx5/config << 'EOF'
[Hotkey/TriggerKeys]
0=Super+space
EOF
```

### User Login Session (no sudo required)

Start directly from GDM/SDDM login screen:

```bash
# 1. Install bcon
curl -fsSL https://sanohiro.github.io/bcon/install.sh | sudo sh
sudo apt install bcon

# 2. Install session files
sudo cp /usr/share/bcon/bcon-session /usr/local/bin/
sudo chmod +x /usr/local/bin/bcon-session
sudo cp /usr/share/bcon/bcon.desktop /usr/share/xsessions/

# 3. Generate user config
bcon --init-config=vim,jp    # saves to ~/.config/bcon/config.toml

# 4. Select "bcon" session from login screen
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
main = "JetBrains Mono"             # Ligature font recommended (see Recommended Fonts)
cjk = "Noto Sans CJK JP"           # or full path: "/usr/share/fonts/.../X.ttf"
emoji = "Noto Color Emoji"
symbols = "Hack Nerd Font Mono"
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

[notifications]
enabled = true               # Enable OSC 9/99 notifications (default: true)

[paths]
screenshot_dir = "~/Pictures"
```

### Nerd Fonts (Icons)

For icon display in **yazi**, **ranger**, **lsd**, **eza**, **fish**, and Powerline prompts:

```bash
# Download and install Hack Nerd Font
sudo mkdir -p /usr/local/share/fonts
cd /usr/local/share/fonts
sudo curl -OL https://github.com/ryanoasis/nerd-fonts/releases/latest/download/Hack.tar.xz
sudo tar xf Hack.tar.xz && sudo rm Hack.tar.xz
sudo fc-cache -fv
```

Configure in `config.toml` — font name or file path both work:

```toml
[font]
symbols = "Hack Nerd Font Mono"    # by name (recommended)
# symbols = "/usr/local/share/fonts/HackNerdFontMono-Regular.ttf"  # by path
```

The `symbols` font is used as fallback for Powerline glyphs (U+E000-U+F8FF) and Nerd Font icons. If not specified, bcon auto-detects installed Nerd Fonts via fontconfig.

Note: Powerline arrow glyphs (E0B0-E0B7) are drawn programmatically for pixel-perfect rendering regardless of font.

### Recommended Fonts

The default monospace font (DejaVu Sans Mono) does **not** support ligatures. To enable ligatures like `=>` `->` `!=` `===`, install a ligature-capable font:

| Font | Ligatures | Install (Debian/Ubuntu) | Notes |
|------|-----------|------------------------|-------|
| **JetBrains Mono** | `=>` `->` `!=` `<=` | `sudo apt install fonts-jetbrains-mono` | Recommended for daily use — balanced readability |
| **FiraCode** | `=>` `->` `!=` `===` `>=` `|>` ... | `sudo apt install fonts-firacode` | Most ligature variants — great for demos |
| **Cascadia Code** | `=>` `->` `!=` `<=` | [GitHub releases](https://github.com/microsoft/cascadia-code/releases) | Microsoft's coding font |

```toml
[font]
main = "JetBrains Mono"    # or "Fira Code"
```

### Keybinds

| Action | Default | Vim | Emacs | Description |
|--------|---------|-----|-------|-------------|
| Copy | `Ctrl+Shift+C` | same | `Ctrl+Shift+W` | Copy selection to clipboard |
| Paste | `Ctrl+Shift+V` | same | `Ctrl+Shift+Y` | Paste from clipboard |
| Screenshot | `PrintScreen` | same | same | Save screenshot as PNG |
| Search | `Ctrl+Shift+F` | same | `Ctrl+Shift+S` | Search in scrollback |
| Copy Mode | `Ctrl+Shift+Space` | same | `Ctrl+Shift+M` | Enter vim-like copy mode |
| Font + | `Ctrl+Plus` | same | same | Increase font size |
| Font - | `Ctrl+Minus` | same | same | Decrease font size |
| Font Reset | `Ctrl+0` | same | same | Reset font size |
| Scroll Up | `Shift+PageUp` | `Ctrl+Shift+U` | `Alt+Shift+V` | Scroll back |
| Scroll Down | `Shift+PageDown` | `Ctrl+Shift+D` | `Alt+Shift+N` | Scroll forward |
| Notifications | `Ctrl+Shift+N` | same | same | Toggle notification panel |
| Mute | `Ctrl+Shift+M` | same | `Alt+Shift+M` | Toggle notification mute |
| Split Right | `Ctrl+Shift+Enter` | same | same | Split pane horizontally |
| Split Down | `Ctrl+Shift+D` | `Ctrl+Shift+\` | `Ctrl+Shift+D` | Split pane vertically |
| Close Pane | `Ctrl+Shift+W` | same | same | Close active pane |
| Pane Navigate | `Ctrl+Shift+Arrow` | `Ctrl+Shift+H/J/K/L` | `Ctrl+Shift+Arrow` | Move focus between panes |
| Pane Resize | `Ctrl+Shift+Alt+Arrow` | `Ctrl+Shift+Alt+H/J/K/L` | `Ctrl+Shift+Alt+Arrow` | Resize active pane |
| Zoom Pane | `Ctrl+Shift+Z` | same | same | Toggle pane zoom |
| New Tab | `Ctrl+Shift+T` | same | same | Open new tab |
| Close Tab | `Ctrl+Shift+Q` | same | same | Close active tab |
| Next Tab | `Ctrl+Shift+PageDown` | same | same | Switch to next tab |
| Prev Tab | `Ctrl+Shift+PageUp` | same | same | Switch to previous tab |

Multiple keys can be assigned to a single action in config:

```toml
[keybinds]
copy = ["ctrl+shift+c", "ctrl+insert"]
paste = ["ctrl+shift+v", "shift+insert"]
```

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
    libseat-dev \
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

bcon includes libseat support by default, enabling:
- Running without root privileges
- Proper session tracking (`loginctl list-sessions`)
- Integration with screen lock, power management
- Login session from GDM/SDDM

The same binary works both with `sudo` and as a user session.

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
