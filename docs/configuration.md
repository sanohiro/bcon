# Configuration

English | **[Japanese](configuration.ja.md)**

## Config File Locations

- `/etc/bcon/config.toml` — system config (for systemd service)
- `~/.config/bcon/config.toml` — user config

## Available Presets

| Preset | Description |
|--------|-------------|
| `default` | Standard keybinds (Ctrl+Shift+C/V, etc.) |
| `vim` | Vim-like scroll (Ctrl+Shift+U/D) |
| `emacs` | Emacs-like scroll (Alt+Shift+V/N) |
| `japanese` / `jp` | CJK fonts + IME auto-disable |
| `system` | Write to /etc/bcon/config.toml instead of user config |

Generate a config file:

```bash
bcon --init-config=vim,jp           # user config
sudo bcon --init-config=system,vim  # system config
```

## Example Config

```toml
[font]
main = "JetBrains Mono"             # Ligature font recommended (see Recommended Fonts)
cjk = "Noto Sans CJK JP"           # or full path: "/usr/share/fonts/.../X.ttf"
emoji = "Noto Color Emoji"
symbols = "Hack Nerd Font Mono"
size = 16.0                          # Font size in px (default: 16.0)
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

[mouse]
speed = 1.0                  # Cursor speed multiplier (default: 1.0)
                             # Auto-scaled by resolution (e.g. 2.0x on 4K)

[display]
prefer_external = true       # Prefer external monitors (HDMI/DP) over internal
auto_switch = true           # Auto-switch on hotplug connect/disconnect

[drm]
device = "auto"              # "auto" probes each GPU and selects one with a connected display
                             # or explicit path: "/dev/dri/card1"

[notifications]
enabled = true               # Enable OSC 9/99 notifications (default: true)

[paths]
screenshot_dir = "~/Pictures"
```

### DRM Device Selection

`device = "auto"` (default) probes each `/dev/dri/card*` and selects the first GPU with a connected display. You can also specify a device path explicitly:

```toml
[drm]
device = "/dev/dri/card1"    # Use a specific GPU
```

To check which device bcon is using, look at the startup log:

```bash
journalctl -u bcon@tty2 -e | grep "DRM"
```

```
DRM auto-detect: /dev/dri/card0 has no connected connectors, skipping
DRM auto-detect: /dev/dri/card1 has 1 connected connector(s), selected
DRM device: /dev/dri/card1 (auto-detected)
```

### Optimus Laptops (Intel + NVIDIA)

On Optimus laptops, displays are typically routed through the Intel iGPU even when an NVIDIA GPU is present. `device = "auto"` handles this automatically — it skips GPUs with no connected displays.

If bcon fails to start with an error like `No connected connector found`, check which GPU has connected displays:

```bash
# Find available DRM devices
ls -l /dev/dri/card*

# Check which GPU each device uses
udevadm info -a /dev/dri/card0 | grep -i vendor
udevadm info -a /dev/dri/card1 | grep -i vendor
```

**If you want to use the NVIDIA GPU directly**, enable kernel modesetting:

```bash
echo 'options nvidia-drm modeset=1' | sudo tee /etc/modprobe.d/nvidia-drm.conf
sudo update-initramfs -u
sudo reboot
```

If this still doesn't work (common on Optimus), set the Intel iGPU explicitly:

```toml
[drm]
device = "/dev/dri/card1"    # Use Intel iGPU instead of NVIDIA
```

## Nerd Fonts (Icons)

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

## Recommended Fonts

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

## Font Size

The default font size is `16.0` (px). For high-resolution displays, increase the size for readability:

| Resolution | Recommended size |
|------------|-----------------|
| 1080p (FHD) | 16.0 (default) |
| 1440p (QHD) | 18.0 – 20.0 |
| 2160p (4K) | 22.0 – 28.0 |

```toml
[font]
size = 24.0
```

You can also adjust at runtime with `Ctrl+Plus` / `Ctrl+Minus` (`Ctrl+0` to reset).
