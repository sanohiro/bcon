# インストールガイド

**[English](installation.md)** | Japanese

## 基本セットアップ (Debian/Ubuntu)

日本語環境が必要な場合は [日本語環境セットアップ](#日本語環境セットアップ) を参照してください。

```bash
# 1. bcon をインストール
curl -fsSL https://sanohiro.github.io/bcon/install.sh | sudo sh
sudo apt install bcon

# 2. (任意) Nerd Font をインストール (yazi, lsd 等のアイコン表示用)
sudo apt install fontconfig curl  # 未インストールの場合
sudo mkdir -p /usr/local/share/fonts
cd /usr/local/share/fonts
sudo curl -OL https://github.com/ryanoasis/nerd-fonts/releases/latest/download/Hack.tar.xz
sudo tar xf Hack.tar.xz && sudo rm Hack.tar.xz
sudo fc-cache -fv

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

## 日本語環境セットアップ

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
sudo mkdir -p /usr/local/share/fonts
cd /usr/local/share/fonts
sudo curl -OL https://github.com/ryanoasis/nerd-fonts/releases/latest/download/Hack.tar.xz
sudo tar xf Hack.tar.xz && sudo rm Hack.tar.xz
sudo fc-cache -fv

# 3. 設定ファイルを生成 (Nerd Font があれば自動検出)
sudo bcon --init-config=system,jp        # デフォルトキーバインド
sudo bcon --init-config=system,vim,jp    # Vim 風キーバインド
sudo bcon --init-config=system,emacs,jp  # Emacs 風キーバインド

# 4. systemd サービスを有効化 (tty2)
sudo systemctl disable getty@tty2
sudo systemctl enable bcon@tty2
sudo systemctl start bcon@tty2

# 5. bcon に切り替え
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

## ユーザーログインセッション (sudo 不要)

GDM/SDDM などのログイン画面から直接起動:

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

## ソースからビルド

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

## 手動起動

TTY (仮想コンソール) から直接実行:

```bash
# TTY に切り替え
Ctrl+Alt+F2

# bcon を実行
sudo ./target/release/bcon

# グラフィカルセッションに戻る
Ctrl+Alt+F1  # または F7
```

## rootless モード

bcon はデフォルトで libseat 対応。以下が可能:
- root 権限なしで実行
- セッション追跡 (`loginctl list-sessions`)
- スクリーンロック、電源管理との連携
- GDM/SDDM ログインセッション

同じバイナリが `sudo` でもユーザーセッションでも動作します。
