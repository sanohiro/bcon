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
- **エディタ**: Emacs, Neovim, Helix

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

- DRM/KMS サポートのある Linux (Debian/Ubuntu 推奨)
- OpenGL ES 2.0+ 対応 GPU

## インストール (Debian/Ubuntu)

### 基本セットアップ

```bash
# 1. bcon をインストール
curl -fsSL https://sanohiro.github.io/bcon/install.sh | sudo sh
sudo apt install bcon

# 2. 設定ファイルを生成
sudo bcon --init-config=system           # デフォルト
sudo bcon --init-config=system,vim       # Vim ユーザー
sudo bcon --init-config=system,emacs     # Emacs ユーザー

# 3. systemd サービスを有効化 (tty2)
sudo systemctl disable getty@tty2
sudo systemctl enable bcon@tty2
sudo systemctl start bcon@tty2

# 4. bcon に切り替え
# Ctrl+Alt+F2
```

### 日本語環境セットアップ

```bash
# 1. bcon と日本語関連パッケージをインストール
curl -fsSL https://sanohiro.github.io/bcon/install.sh | sudo sh
sudo apt install bcon fonts-noto-cjk fonts-noto-color-emoji

# fcitx5 最小インストール (推奨)
# 通常の fcitx5 は Qt/GTK の GUI モジュールを大量にインストールする。
# bcon は X11/Wayland を使わないため GUI は不要。--no-install-recommends で最小構成に。
sudo apt install --no-install-recommends fcitx5 fcitx5-mozc

# 2. fcitx5 自動起動を設定
echo 'fcitx5 -d &>/dev/null' >> ~/.bashrc
# または ~/.zshrc

# 3. 設定ファイルを生成 (日本語プリセット)
sudo bcon --init-config=system,vim,jp    # Vim ユーザー
sudo bcon --init-config=system,emacs,jp  # Emacs ユーザー

# 4. systemd サービスを有効化 (tty2)
sudo systemctl disable getty@tty2
sudo systemctl enable bcon@tty2
sudo systemctl start bcon@tty2

# 5. bcon に切り替え
# Ctrl+Alt+F2

# IME 切り替え: Ctrl+Space (fcitx5 デフォルト)
```

### ユーザー権限で起動 (sudo 不要)

GDM/SDDM などのログインマネージャーから直接起動:

```bash
# 1. bcon をインストール (rootless ビルド版)
curl -fsSL https://sanohiro.github.io/bcon/install.sh | sudo sh
sudo apt install bcon

# 2. ユーザー設定ファイルを生成
bcon --init-config=vim,jp    # ~/.config/bcon/config.toml に保存

# 3. ログイン画面で「bcon」セッションを選択
```

デスクトップ環境なしで直接 bcon にログイン。メモリ節約・起動時間短縮に効果的。

## 設定

設定ファイルの場所:
- `/etc/bcon/config.toml` (システム設定、systemd サービス用)
- `~/.config/bcon/config.toml` (ユーザー設定)

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

## その他のインストール・起動方法

### ソースからビルド

```bash
# ビルド依存パッケージ
sudo apt install \
    libdrm-dev libgbm-dev \
    libegl1-mesa-dev libgles2-mesa-dev \
    libxkbcommon-dev libinput-dev libudev-dev \
    libdbus-1-dev libwayland-dev \
    libfontconfig1-dev libfreetype-dev \
    pkg-config cmake clang

# Rust ツールチェイン (1.82+) が必要
cargo build --release

# 設定ファイル生成
./target/release/bcon --init-config=vim,jp
```

### 手動起動

TTY (仮想コンソール) から直接実行:

```bash
# TTY に切り替え
Ctrl+Alt+F2

# bcon を実行
sudo ./target/release/bcon

# グラフィカルセッションに戻る
Ctrl+Alt+F1  # または F7
```

### ログインセッション (GDM/SDDM)

ログイン画面から bcon をセッションとして選択できます:

```bash
# セッションファイルをインストール
sudo cp bcon.desktop /usr/share/wayland-sessions/
```

デスクトップ環境を起動せずに直接 bcon にログイン。メモリ節約、起動時間短縮に効果的。

### rootless モード

root 権限なしで実行するには libseat を使用:

```bash
# 追加パッケージ
sudo apt install libseat-dev

# rootless ビルド
cargo build --release --features seatd

# root なしで実行可能
./target/release/bcon
```

メリット:
- root 権限不要
- セッション追跡 (`loginctl list-sessions`)
- スクリーンロック、電源管理との連携

## 制限事項

- **マルチシート (DRM リース)**: 非対応。bcon は GPU を排他的に使用します。
- **マルチモニタ**: 現在は1つのモニタにのみ出力。

## ライセンス

MIT

## 謝辞

インスピレーション:
- [Ghostty](https://github.com/ghostty-org/ghostty)
- [foot](https://codeberg.org/dnkl/foot)
- [yaft](https://github.com/uobikiemukot/yaft)
- [Alacritty](https://github.com/alacritty/alacritty)
