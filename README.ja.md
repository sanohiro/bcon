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

- **セッション管理**: tmux (推奨), screen, zellij*
- **ファイル操作**: yazi, ranger
- **エディタ**: Emacs, Neovim, Helix

*zellij は Kitty graphics protocol のパススルーに対応していません。zellij 経由では yazi の画像プレビューが動作しません。

Unix 哲学に従い、一つのことをうまくやる。bcon は「美しく、速いレンダリング」に集中します。

**楽しい CLI ライフを。**

## 機能

### レンダリング
- **GPU レンダリング**: DRM/KMS + EGL + GBM 経由の OpenGL ES
- **シャープなテキスト**: ピクセルアライン済みグリフレンダリング
- **True Color**: 24bit フルカラーサポート
- **リガチャ**: フォントリガチャ対応 (FiraCode, JetBrains Mono など)
- **絵文字**: カラー絵文字レンダリング (Noto Color Emoji)
- **Powerline**: ピクセル精度の Powerline/Nerd Font グリフ
- **LCD サブピクセル**: LCD モニター向け最適化
- **HiDPI スケーリング**: 設定可能な表示倍率 (1.0x - 2.0x)
- **HDR 検出**: EDID から HDR 対応を自動検出

### グラフィックス
- **Sixel グラフィックス**: ターミナル内画像表示
- **Kitty グラフィックスプロトコル**: 高速画像転送

### ターミナル
- **スクロールバック**: 設定可能なバッファ (デフォルト: 10,000 行)
- **マウスサポート**: 選択、ホイールスクロール、ボタンイベント (X10/SGR/URXVT/SGR-Pixels)
- **OSC 52 クリップボード**: エスケープシーケンスでクリップボード操作
- **ブラケットペースト**: セキュアなペーストモード
- **ハイパーリンク**: OSC 8 対応
- **OSC 4/10/11/12**: パレット、前景色、背景色、カーソル色の動的変更
- **通知**: OSC 9 (iTerm2) / OSC 99 (Kitty) 通知プロトコル — トーストオーバーレイ＋プログレスバー

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
- **通知パネル**: 通知履歴の閲覧 (Ctrl+Shift+N)、ミュート切替 (Ctrl+Shift+M)
- **ビジュアルベル**: ベル文字で画面フラッシュ
- **URL 検出**: Ctrl+クリックで URL をコピー
- **モニターホットプラグ**: モニター接続/切断を自動検知・切替
- **外部モニター優先**: HDMI/DP 接続時に自動切り替え (ラップトップ向け)

## 動作要件

- DRM/KMS サポートのある Linux (Debian/Ubuntu 推奨)
- OpenGL ES 2.0+ 対応 GPU

## インストール (Debian/Ubuntu)

### 基本セットアップ

日本語環境が必要な場合は [日本語環境セットアップ](#日本語環境セットアップ) を参照してください。

```bash
# 1. bcon をインストール
curl -fsSL https://sanohiro.github.io/bcon/install.sh | sudo sh
sudo apt install bcon

# 2. (任意) Nerd Font をインストール (yazi, lsd 等のアイコン表示用)
sudo apt install fontconfig curl  # 未インストールの場合
mkdir -p ~/.local/share/fonts
cd ~/.local/share/fonts
curl -OL https://github.com/ryanoasis/nerd-fonts/releases/latest/download/Hack.tar.xz
tar xf Hack.tar.xz && rm Hack.tar.xz
fc-cache -fv

# 3. 設定ファイルを生成 (Nerd Font があれば自動検出)
sudo bcon --init-config=system           # デフォルトキーバインド
sudo bcon --init-config=system,vim       # Vim 風キーバインド
sudo bcon --init-config=system,emacs     # Emacs 風キーバインド

# 4. systemd サービスを有効化 (tty2)
sudo systemctl disable getty@tty2
sudo systemctl enable bcon@tty2
sudo systemctl start bcon@tty2

# 5. bcon に切り替え
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

# 2. (任意) Nerd Font をインストール (yazi, lsd 等のアイコン表示用)
sudo apt install fontconfig curl  # 未インストールの場合
mkdir -p ~/.local/share/fonts
cd ~/.local/share/fonts
curl -OL https://github.com/ryanoasis/nerd-fonts/releases/latest/download/Hack.tar.xz
tar xf Hack.tar.xz && rm Hack.tar.xz
fc-cache -fv

# 3. fcitx5 自動起動を設定
echo 'fcitx5 -d &>/dev/null' >> ~/.bashrc
# または ~/.zshrc

# 4. 設定ファイルを生成 (Nerd Font があれば自動検出)
sudo bcon --init-config=system,jp        # デフォルトキーバインド
sudo bcon --init-config=system,vim,jp    # Vim 風キーバインド
sudo bcon --init-config=system,emacs,jp  # Emacs 風キーバインド

# 5. systemd サービスを有効化 (tty2)
sudo systemctl disable getty@tty2
sudo systemctl enable bcon@tty2
sudo systemctl start bcon@tty2

# 6. bcon に切り替え
# Ctrl+Alt+F2

# IME 切り替え: Ctrl+Space (fcitx5 デフォルト)
```

**Emacs ユーザー向け:** Ctrl+Space は `set-mark-command` (C-SPC) とバッティングします。Super(Win)+Space に変更する場合:

```bash
mkdir -p ~/.config/fcitx5
cat >> ~/.config/fcitx5/config << 'EOF'
[Hotkey/TriggerKeys]
0=Super+space
EOF
```

### ユーザー権限で起動 (sudo 不要)

GDM/SDDM などのログインマネージャーから直接起動:

```bash
# 1. bcon をインストール
curl -fsSL https://sanohiro.github.io/bcon/install.sh | sudo sh
sudo apt install bcon

# 2. セッションファイルをインストール
sudo cp /usr/share/bcon/bcon-session /usr/local/bin/
sudo chmod +x /usr/local/bin/bcon-session
sudo cp /usr/share/bcon/bcon.desktop /usr/share/xsessions/

# 3. ユーザー設定ファイルを生成
bcon --init-config=vim,jp    # ~/.config/bcon/config.toml に保存

# 4. ログイン画面で「bcon」セッションを選択
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
main = "FiraCode"                   # フォント名 (fontconfig で自動解決)
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

[notifications]
enabled = true               # OSC 9/99 通知を有効化 (デフォルト: true)

[paths]
screenshot_dir = "~/Pictures"
```

### Nerd Fonts (アイコン表示)

**yazi**, **ranger**, **lsd**, **eza**, **fish** などでアイコンを表示するには Nerd Font が必要:

```bash
# Hack Nerd Font をダウンロード・インストール
mkdir -p ~/.local/share/fonts
cd ~/.local/share/fonts
curl -OL https://github.com/ryanoasis/nerd-fonts/releases/latest/download/Hack.tar.xz
tar xf Hack.tar.xz
rm Hack.tar.xz
fc-cache -fv
```

`config.toml` で設定 — フォント名でもファイルパスでも指定可能:

```toml
[font]
symbols = "Hack Nerd Font Mono"    # フォント名で指定 (推奨)
# symbols = "/usr/local/share/fonts/HackNerdFontMono-Regular.ttf"  # パスでも可
```

**systemd 経由（root サービス）**で起動する場合は、フォントを system-wide にインストール:

```bash
sudo mkdir -p /usr/local/share/fonts
sudo cp ~/.local/share/fonts/HackNerdFont*.ttf /usr/local/share/fonts/
sudo fc-cache -fv
```

`symbols` フォントは Powerline グリフ (U+E000-U+F8FF) や Nerd Font アイコンのフォールバックとして使用されます。指定しない場合、bcon は fontconfig 経由でインストール済みの Nerd Font を自動検出します。

注: Powerline 矢印グリフ (E0B0-E0B7) はフォントに関係なくプログラムでピクセルパーフェクトに描画されます。

### キーバインド

| アクション | デフォルト | Vim | Emacs | 説明 |
|-----------|-----------|-----|-------|------|
| コピー | `Ctrl+Shift+C` | 同左 | `Ctrl+Shift+W` | 選択をクリップボードにコピー |
| ペースト | `Ctrl+Shift+V` | 同左 | `Ctrl+Shift+Y` | クリップボードからペースト |
| スクリーンショット | `PrintScreen` | 同左 | 同左 | PNG でスクリーンショット保存 |
| 検索 | `Ctrl+Shift+F` | 同左 | `Ctrl+Shift+S` | スクロールバック内検索 |
| コピーモード | `Ctrl+Shift+Space` | 同左 | `Ctrl+Shift+M` | Vim ライクコピーモード開始 |
| フォント拡大 | `Ctrl+Plus` | 同左 | 同左 | フォントサイズ拡大 |
| フォント縮小 | `Ctrl+Minus` | 同左 | 同左 | フォントサイズ縮小 |
| フォントリセット | `Ctrl+0` | 同左 | 同左 | フォントサイズリセット |
| スクロールアップ | `Shift+PageUp` | `Ctrl+Shift+U` | `Alt+Shift+V` | 上にスクロール |
| スクロールダウン | `Shift+PageDown` | `Ctrl+Shift+D` | `Alt+Shift+N` | 下にスクロール |
| 通知パネル | `Ctrl+Shift+N` | 同左 | 同左 | 通知パネルの開閉 |
| 通知ミュート | `Ctrl+Shift+M` | 同左 | `Alt+Shift+M` | トースト通知のミュート切替 |

1つのアクションに複数のキーを割り当て可能:

```toml
[keybinds]
copy = ["ctrl+shift+c", "ctrl+insert"]
paste = ["ctrl+shift+v", "shift+insert"]
```

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
    libseat-dev \
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

bcon はデフォルトで libseat 対応。以下が可能:
- root 権限なしで実行
- セッション追跡 (`loginctl list-sessions`)
- スクリーンロック、電源管理との連携
- GDM/SDDM ログインセッション

同じバイナリが `sudo` でもユーザーセッションでも動作する。

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
