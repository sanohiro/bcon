# 設定

**[English](configuration.md)** | Japanese

## 設定ファイルの場所

- `/etc/bcon/config.toml` — システム設定 (systemd サービス用)
- `~/.config/bcon/config.toml` — ユーザー設定

## 利用可能なプリセット

| プリセット | 説明 |
|-----------|------|
| `default` | 標準キーバインド (Ctrl+Shift+C/V など) |
| `vim` | Vim ライクスクロール (Ctrl+Shift+U/D) |
| `emacs` | Emacs ライクスクロール (Alt+Shift+V/N) |
| `japanese` / `jp` | CJK フォント + IME 自動無効化 |
| `system` | /etc/bcon/config.toml に出力 |

設定ファイルの生成:

```bash
bcon --init-config=vim,jp           # ユーザー設定
sudo bcon --init-config=system,vim  # システム設定
```

## 設定例

```toml
[font]
main = "JetBrains Mono"             # リガチャフォント推奨 (推奨フォント参照)
cjk = "Noto Sans CJK JP"           # またはフルパス: "/usr/share/fonts/.../X.ttf"
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
repeat_delay = 400           # キーリピート遅延 (ms)
repeat_rate = 30             # キーリピートレート (ms)
xkb_layout = "jp"            # XKB キーボードレイアウト
xkb_options = "ctrl:nocaps"  # XKB オプション (Caps Lock を Ctrl に)

[display]
prefer_external = true       # 外部モニター優先 (HDMI/DP > 内蔵)
auto_switch = true           # ホットプラグ時に自動切り替え

[drm]
device = "auto"              # "auto" または明示的パス: "/dev/dri/card1"

[notifications]
enabled = true               # OSC 9/99 通知を有効化 (デフォルト: true)

[paths]
screenshot_dir = "~/Pictures"
```

### Optimus ラップトップ (Intel + NVIDIA)

Optimus 環境では NVIDIA の DRM/KMS が EGL/GBM で動作しない場合があります。Intel iGPU のデバイスを指定してください:

```bash
# 利用可能な DRM デバイスを確認
ls -l /dev/dri/card*

# 各デバイスの GPU を確認
udevadm info -a /dev/dri/card0 | grep -i vendor
udevadm info -a /dev/dri/card1 | grep -i vendor
```

```toml
[drm]
device = "/dev/dri/card1"    # NVIDIA ではなく Intel iGPU を使用
```

## Nerd Fonts (アイコン表示)

**yazi**, **ranger**, **lsd**, **eza**, **fish** などでアイコンを表示するには Nerd Font が必要:

```bash
# Hack Nerd Font をダウンロード・インストール
sudo mkdir -p /usr/local/share/fonts
cd /usr/local/share/fonts
sudo curl -OL https://github.com/ryanoasis/nerd-fonts/releases/latest/download/Hack.tar.xz
sudo tar xf Hack.tar.xz && sudo rm Hack.tar.xz
sudo fc-cache -fv
```

`config.toml` で設定 — フォント名でもファイルパスでも指定可能:

```toml
[font]
symbols = "Hack Nerd Font Mono"    # フォント名で指定 (推奨)
# symbols = "/usr/local/share/fonts/HackNerdFontMono-Regular.ttf"  # パスでも可
```

`symbols` フォントは Powerline グリフ (U+E000-U+F8FF) や Nerd Font アイコンのフォールバックとして使用されます。指定しない場合、bcon は fontconfig 経由でインストール済みの Nerd Font を自動検出します。

注: Powerline 矢印グリフ (E0B0-E0B7) はフォントに関係なくプログラムでピクセルパーフェクトに描画されます。

## 推奨フォント

デフォルトの等幅フォント (DejaVu Sans Mono) は**リガチャ非対応**です。`=>` `->` `!=` `===` などのリガチャを有効にするには、リガチャ対応フォントをインストールしてください:

| フォント | リガチャ | インストール (Debian/Ubuntu) | 備考 |
|---------|---------|---------------------------|------|
| **JetBrains Mono** | `=>` `->` `!=` `<=` | `sudo apt install fonts-jetbrains-mono` | 普段使いにおすすめ — 可読性のバランスが良い |
| **FiraCode** | `=>` `->` `!=` `===` `>=` `\|>` ... | `sudo apt install fonts-firacode` | リガチャの種類が最も多い — デモ映えする |
| **Cascadia Code** | `=>` `->` `!=` `<=` | [GitHub releases](https://github.com/microsoft/cascadia-code/releases) | Microsoft 製コーディングフォント |

```toml
[font]
main = "JetBrains Mono"    # または "Fira Code"
```
