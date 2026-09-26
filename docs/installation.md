# Installation Guide

English | **[Japanese](installation.ja.md)**

## Basic Setup (Debian/Ubuntu)

For Japanese environment, see [Japanese Environment Setup](#japanese-environment-setup) below.

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

## Japanese Environment Setup

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

## User Login Session (no sudo required)

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

## Build from Source

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

## Manual Start

Run directly from TTY (virtual console):

```bash
# Switch to TTY
Ctrl+Alt+F2

# Run bcon
sudo ./target/release/bcon

# Return to graphical session
Ctrl+Alt+F1  # or F7
```

## Rootless Mode

bcon includes libseat support by default, enabling:
- Running without root privileges
- Proper session tracking (`loginctl list-sessions`)
- Integration with screen lock, power management
- Login session from GDM/SDDM

The same binary works both with `sudo` and as a user session.

For a rootless run, switch to an unused text VT (often `Ctrl+Alt+F3` on
GNOME), log in there, and run `bcon --backend=seatd` from that login shell.
You do not need to disable getty for this interactive setup: getty has already
handed the VT to your login session. If a system service starts bcon directly
on a VT instead, disable that VT's getty so the two services do not own it.


## For Arch Linux Users
There are some differences compared to Debian/Ubuntu.

1. Package manager(for official repository)

In Arch, you use pacman instead of apt.
```
sudo apt install [package-name]
```
```
sudo pacman -S [package-name]
```

2. AUR(Arch User Repository) & AUR helper

The bcon package is listed in AUR(community-maintained by @kay-ws).
You will need to use AUR helper(such as yay, paru) to retrieve the package.
```
yay -S [AUR-package-name]       # don't use sudo 
```

if you don't have the AUR helper, you can get it as follows.
(Using "yay" as an example.)
```bash
sudo pacman -S --needed git base-devel
git clone https://aur.archlinux.org/yay.git
cd yay && makepkg -si
```

3. Install bcon

Replace
```
curl -fsSL https://sanohiro.github.io/bcon/install.sh | sudo sh
sudo apt install bcon
```
with
```
yay -S bcon
```

4. User Login Session

Session files (`bcon-session`, `bcon.desktop`) are installed in their final locations by the AUR package, so no manual deployment is needed.
