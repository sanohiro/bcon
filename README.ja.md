# bcon

Linux コンソール (TTY) 用 GPU アクセラレーション対応ターミナルエミュレータ — X11/Wayland 不要

## 概要

**bcon** は最新のターミナル機能を Linux コンソールに直接提供します — デスクトップ環境のオーバーヘッドなしに、GPU アクセラレーション、スムーズで美しいレンダリングを実現。

「Linux コンソール版 Ghostty」— クリアなテキスト、スムーズなスクロール、True Color、レスポンシブな入力に焦点を当てています。

## なぜ bcon？

AI コーディングツール（Claude Code、Codex、Gemini CLI など）の登場により、開発ワークフローは大きく変わりました。VSCode を開く機会は減り、ターミナルで過ごす時間が増えています。

ふと気づくと、X11/Wayland 上で動かしているのはターミナルエミュレータだけ — それなら、デスクトップ環境ごと省略できるのでは？

**bcon** はその答えです。Ghostty や Alacritty のようなモダンなターミナル体験を、Linux コンソール上で直接実現します。GPU アクセラレーション、True Color、Sixel/Kitty グラフィックス、日本語入力 — X11 なしで。

### bcon の役割

bcon はあくまで**土台**です。画面分割やセッション管理は、それを得意とするツールに任せましょう：

- **セッション管理**: tmux, zellij, screen
- **ファイル操作**: yazi, ranger
- **エディタ**: Neovim, Helix

Unix 哲学に従い、一つのことをうまくやる。bcon は「美しく、速いレンダリング」に集中します。

**楽しい CLI ライフを。**

## 機能

### レンダリング
- **GPU レンダリング**: DRM/KMS + EGL + GBM 経由の OpenGL ES
- **シャープなテキスト**: ピクセルアライン済みグリフレンダリング
- **True Color**: 24bit フルカラーサポート
- **リガチャ**: フォントリガチャ対応 (FiraCode, JetBrains Mono など)
- **絵文字**: カラー絵文字レンダリング (Noto Color Emoji)
- **LCD サブピクセル**: LCD モニター向け最適化
- **HiDPI スケーリング**: 設定可能な表示倍率 (1.0x - 2.0x)
- **HDR 検出**: EDID から HDR 対応を自動検出

### グラフィックス
- **Sixel グラフィックス**: ターミナル内画像表示
- **Kitty グラフィックスプロトコル**: 高速画像転送

### ターミナル
- **スクロールバック**: 設定可能なバッファ (デフォルト: 10,000 行)
- **マウスサポート**: 選択、ホイールスクロール、ボタンイベント (X10/SGR/URXVT)
- **OSC 52 クリップボード**: エスケープシーケンスでクリップボード操作
- **ブラケットペースト**: セキュアなペーストモード
- **ハイパーリンク**: OSC 8 対応

### 入力
- **キーボード**: evdev + xkbcommon による完全キーボードサポート
- **日本語入力**: D-Bus 経由の fcitx5 統合
- **IME 自動無効化**: vim/emacs などで自動的に IME を無効化
- **キーリピート**: 設定可能な遅延/レート

### UX
- **コピーモード**: Vim ライクなキーボードナビゲーション
- **テキスト検索**: スクロールバック内インクリメンタル検索 (Ctrl+Shift+F)
- **スクリーンショット**: PNG で保存 (PrintScreen または Ctrl+Shift+S)
- **フォント拡大縮小**: 実行時フォントサイズ変更 (Ctrl+Plus/Minus)
- **ビジュアルベル**: ベル文字で画面フラッシュ
- **URL 検出**: Ctrl+クリックで URL をコピー
- **モニターホットプラグ**: モニター接続/切断を自動検知・切替
- **外部モニター優先**: HDMI/DP 接続時に自動切り替え (ラップトップ向け)

## 動作要件

- DRM/KMS サポートのある Linux
- OpenGL ES 2.0+ 対応 GPU
- Rust ツールチェイン (1.82+)

### 実行権限オプション

| モード | 必要条件 | ビルドコマンド |
|--------|----------|----------------|
| root モード | root で実行 (`sudo`) | `cargo build --release` |
| rootless モード | systemd-logind または seatd | `cargo build --release --features seatd` |

**rootless モード**は [libseat](https://sr.ht/~kennylevinsen/seatd/) を使用してセッション管理を行います:
- root 権限不要
- セッション追跡 (`loginctl list-sessions`)
- スクリーンロック、電源管理との連携
- Wayland/X11 セッションとのクリーンな VT 切り替え

### システムパッケージ (Debian/Ubuntu)

```bash
# ビルド依存
sudo apt install \
    libdrm-dev libgbm-dev \
    libegl1-mesa-dev libgles2-mesa-dev \
    libxkbcommon-dev libinput-dev libudev-dev \
    libdbus-1-dev libwayland-dev \
    libfontconfig1-dev libfreetype-dev \
    pkg-config cmake clang

# オプション: rootless ビルド (--features seatd)
sudo apt install libseat-dev

# ランタイム (フォント)
sudo apt install fonts-dejavu-core

# オプション: 日本語サポート
sudo apt install fonts-noto-cjk fcitx5 fcitx5-mozc

# オプション: カラー絵文字
sudo apt install fonts-noto-color-emoji
```

## インストール

### apt (Debian/Ubuntu)

```bash
# リポジトリ追加
curl -fsSL https://sanohiro.github.io/bcon/install.sh | sudo sh

# インストール
sudo apt install bcon
```

### ソースからビルド

```bash
# 標準ビルド (root で実行)
cargo build --release

# rootless ビルド (logind/seatd で実行)
cargo build --release --features seatd

# 設定ファイル生成
./target/release/bcon --init-config

# 日本語ユーザー向け
./target/release/bcon --init-config=vim,jp
```

## 使い方

### 手動起動

TTY (仮想コンソール) から実行。X11/Wayland 内からは実行不可：

```bash
# TTY に切り替え
Ctrl+Alt+F2

# bcon を実行 (標準ビルド)
sudo ./target/release/bcon

# bcon を実行 (rootless ビルド: --features seatd)
./target/release/bcon

# グラフィカルセッションに戻る
Ctrl+Alt+F1  # または F7
```

### systemd サービス (常用におすすめ)

```bash
# バイナリとサービスファイルをインストール
sudo cp target/release/bcon /usr/local/bin/
sudo cp bcon@.service /etc/systemd/system/

# システム設定を生成
sudo bcon --init-config=system,vim,jp

# tty2 で有効化 (tty1 はフォールバック用に残す)
sudo systemctl disable getty@tty2
sudo systemctl enable bcon@tty2
sudo systemctl start bcon@tty2

# bcon に切り替え
Ctrl+Alt+F2
```

### ログインセッション (GDM/SDDM)

rootless ビルドでは、ログイン画面から bcon をセッションとして選択できます：

```bash
# セッションファイルをインストール
sudo cp bcon.desktop /usr/share/wayland-sessions/

# GDM/SDDM のセッション選択に "bcon" が表示される
```

デスクトップ環境を起動せずに直接 bcon にログインできます。メモリ節約、起動時間短縮に効果的。

### rootless systemd サービス

rootless ビルド (`--features seatd`) 用のサービス設定：

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

# rootless: 一般ユーザーで実行
User=youruser
Group=youruser
SupplementaryGroups=video input

[Install]
WantedBy=multi-user.target
```

### 日本語入力 (IME) を使う場合

```bash
# fcitx5 デーモンを起動
fcitx5 -d

# bcon を実行 (D-Bus のために環境変数を保持)
sudo -E ./target/release/bcon

# IME 切り替え: 設定キー (デフォルト: Ctrl+Shift+J)
```

## 設定

設定ファイルの優先順位:
1. `~/.config/bcon/config.toml` (ユーザー設定)
2. `/etc/bcon/config.toml` (システム設定)
3. ビルトインデフォルト

### 設定ファイル生成

```bash
# デフォルト
bcon --init-config

# プリセット指定 (カンマで複数指定可)
bcon --init-config=vim,jp
bcon --init-config=emacs,japanese
```

### 利用可能なプリセット

| プリセット | 説明 |
|-----------|------|
| `default` | 標準キーバインド (Ctrl+Shift+C/V など) |
| `vim` | Vim ライクスクロール (Ctrl+Shift+U/D) |
| `emacs` | Emacs ライクスクロール (Alt+Shift+V/N) |
| `japanese` / `jp` | CJK フォント + IME 自動無効化 |
| `system` | /etc/bcon/config.toml に出力 |

### 設定例

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
ime_toggle = "ctrl+shift+j"

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

[paths]
screenshot_dir = "~/Pictures"
```

### キーバインド

| アクション | デフォルト | 説明 |
|-----------|-----------|------|
| コピー | `Ctrl+Shift+C` | 選択をクリップボードにコピー |
| ペースト | `Ctrl+Shift+V` | クリップボードからペースト |
| スクリーンショット | `PrintScreen` | PNG でスクリーンショット保存 |
| 検索 | `Ctrl+Shift+F` | スクロールバック内検索 |
| コピーモード | `Ctrl+Shift+Space` | Vim ライクコピーモード開始 |
| フォント拡大 | `Ctrl+Plus` | フォントサイズ拡大 |
| フォント縮小 | `Ctrl+Minus` | フォントサイズ縮小 |
| フォントリセット | `Ctrl+0` | フォントサイズリセット |
| スクロールアップ | `Shift+PageUp` | 上にスクロール |
| スクロールダウン | `Shift+PageDown` | 下にスクロール |
| IME 切り替え | `Ctrl+Shift+J` | IME のオン/オフ |

### コピーモードキー (Vim ライク)

| キー | アクション |
|-----|----------|
| `h/j/k/l` | カーソル移動 |
| `w/b` | 単語前方/後方 |
| `0/$` | 行頭/行末 |
| `g/G` | バッファ先頭/末尾 |
| `v` | 選択開始/切り替え |
| `y` | ヤンク (コピー) して終了 |
| `/` | 検索 |
| `Esc` | コピーモード終了 |

## 制限事項

- **マルチシート (DRM リース)**: 非対応。bcon は GPU を排他的に使用します。1台の PC で複数ユーザーが別々のモニター/キーボードを使う構成には、従来の X11/Wayland をご利用ください。
- **マルチモニタ**: 現在は1つのモニタにのみ出力。複数モニタが接続されている場合、最初に検出されたディスプレイを使用します。

## ライセンス

MIT

## 謝辞

インスピレーション:
- [Ghostty](https://github.com/ghostty-org/ghostty)
- [foot](https://codeberg.org/dnkl/foot)
- [yaft](https://github.com/uobikiemukot/yaft)
- [Alacritty](https://github.com/alacritty/alacritty)
