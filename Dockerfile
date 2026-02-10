# bcon開発用Dockerfile
# DRM/KMS対応のLinux環境でビルド・テスト

FROM rust:1.82-bookworm

# 必要なシステムライブラリをインストール
RUN apt-get update && apt-get install -y \
    # DRM/KMS/GBM
    libdrm-dev \
    libgbm-dev \
    # EGL/OpenGL ES
    libegl1-mesa-dev \
    libgles2-mesa-dev \
    # Wayland（gbm依存）
    libwayland-dev \
    wayland-protocols \
    # Input
    libxkbcommon-dev \
    libinput-dev \
    libudev-dev \
    # D-Bus (fcitx5 IME)
    libdbus-1-dev \
    # ビルドツール
    pkg-config \
    cmake \
    clang \
    # CJKフォント（日本語等）
    fonts-noto-cjk \
    # リガチャテスト用
    wget unzip fontconfig \
    # デバッグ用
    gdb \
    strace \
    && rm -rf /var/lib/apt/lists/*

# Fira Code フォントをインストール（リガチャテスト用）
RUN mkdir -p /usr/share/fonts/truetype/firacode && \
    wget -q "https://github.com/tonsky/FiraCode/releases/download/6.2/Fira_Code_v6.2.zip" -O /tmp/firacode.zip && \
    unzip -q /tmp/firacode.zip -d /tmp/firacode && \
    cp /tmp/firacode/ttf/*.ttf /usr/share/fonts/truetype/firacode/ && \
    fc-cache -f && \
    rm -rf /tmp/firacode /tmp/firacode.zip

# 作業ディレクトリ
WORKDIR /app

# 依存関係のキャッシュのために先にCargo.tomlをコピー
COPY Cargo.toml Cargo.lock* ./

# ダミーのsrc/main.rsを作成して依存関係だけビルド（キャッシュ効率化）
RUN mkdir -p src && echo "fn main() {}" > src/main.rs
RUN cargo build --release 2>/dev/null || true
RUN rm -rf src

# 実際のソースをコピー
COPY . .

# ビルド
RUN cargo build --release

# デフォルトコマンド
CMD ["cargo", "run", "--release"]
