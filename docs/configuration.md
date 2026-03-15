# Configuration

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
