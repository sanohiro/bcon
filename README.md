# bcon

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![CI](https://github.com/sanohiro/bcon/actions/workflows/ci.yml/badge.svg)](https://github.com/sanohiro/bcon/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/sanohiro/bcon)](https://github.com/sanohiro/bcon/releases/latest)

GPU-accelerated terminal emulator for Linux console (TTY) — no X11/Wayland required.

**[Documentation](docs/)** · **[Installation](docs/installation.md)** · **[Configuration](docs/configuration.md)** · **[Japanese](README.ja.md)**

- **GPU-rendered text** — OpenGL ES via DRM/KMS for sharp, smooth rendering
- **Sixel & Kitty graphics** — display images directly in your terminal
- **Built-in panes & tabs** — no tmux needed, no graphics passthrough issues
- **Japanese input** — fcitx5 integration via D-Bus, works on bare console

![bcon — Claude Code, yazi with image preview, and vim running in split panes on Linux TTY](demo/screenshot-split-panes.png)

## Why bcon?

AI coding tools (Claude Code, Codex, Gemini CLI) have transformed development workflows. We spend less time in VSCode and more time in the terminal.

Look around — the only thing running on your X11/Wayland session might be a terminal emulator. So why not skip the desktop entirely?

**bcon** is the answer. It brings the modern terminal experience of Ghostty or Alacritty directly to the Linux console. GPU acceleration, True Color, Sixel/Kitty graphics, Japanese input — no X11 required.

### What bcon does

bcon includes **built-in split panes and tabs** — no need for tmux or screen for basic multiplexing. This is important because terminal multiplexers often break Kitty graphics protocol passthrough, defeating bcon's image rendering capabilities.

**Enjoy your CLI life.**

| | Real TTY | GPU accel | Kitty graphics | IME | Split panes |
|---|:---:|:---:|:---:|:---:|:---:|
| **bcon** | Yes | Yes | Yes | Yes | Built-in |
| kitty / alacritty / ghostty | No (needs X11/Wayland) | Yes | Varies | Desktop IME | Varies |
| tmux / screen | Yes | No | Passthrough issues | N/A | Yes |

## Demo

### Script & Basic Features

https://github.com/user-attachments/assets/8576c907-1f7b-4582-8eb8-de04a853b604

### Real-World Usage (yazi, btop, Claude Code, etc.)

https://github.com/user-attachments/assets/ebff498c-25b7-4662-8750-8d6e35661963

## Features

### Rendering
- **GPU Rendering**: OpenGL ES via DRM/KMS + EGL + GBM
- **Sharp Text**: Pixel-aligned glyph rendering
- **True Color**: Full 24-bit color support
- **Ligatures**: Font ligature support (requires a ligature font — see [Recommended Fonts](docs/configuration.md#recommended-fonts))
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
- **Touchpad Support**: Tap-to-click, natural scroll, disable-while-typing (via libinput)
- **Touchpad Gestures**: Pinch-to-zoom font size, 3-finger swipe for tab switching
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

## Quick Start

For Japanese environment (CJK fonts + fcitx5 IME), see [Japanese Environment Setup](docs/installation.md#japanese-environment-setup) instead.

```bash
# 1. Install
curl -fsSL https://sanohiro.github.io/bcon/install.sh | sudo sh
sudo apt install bcon

# 2. (Optional) Install Nerd Font for icons (yazi, lsd, etc.)
#    See: docs/configuration.md#nerd-fonts-icons

# 3. Generate config & enable service
sudo bcon --init-config=system,vim    # or: system,emacs / system,jp / system,vim,jp
sudo systemctl disable getty@tty2
sudo systemctl enable --now bcon@tty2

# 4. Switch to bcon: Ctrl+Alt+F2
```

## Documentation

| Document | Description |
|----------|-------------|
| **[Installation Guide](docs/installation.md)** | All install methods: apt, Japanese setup, login session, build from source, rootless |
| **[Configuration](docs/configuration.md)** | Config file reference, presets, fonts, Nerd Fonts |
| **[Keybinds](docs/keybinds.md)** | Full keybind tables (Default / Vim / Emacs) + Copy Mode keys |

## Requirements

- Linux with DRM/KMS support (Debian/Ubuntu recommended)
- GPU with OpenGL ES 3.0+

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
