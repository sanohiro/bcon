# CLAUDE.md - Project Context for Claude Code

## Project Overview

**bcon** - A GPU-accelerated terminal emulator running directly on Linux console (TTY) without X11/Wayland.

Think "Ghostty for the console" - bringing modern terminal features (True Color, Sixel, GPU rendering) to bare metal Linux.

## Why This Exists

- AI coding tools (Claude Code, aider, Codex) are all CLI-based
- Developers spend most time in terminal, rarely need GUI except browser
- X11/Wayland adds unnecessary overhead for terminal-only workflows
- No existing solution combines: GPU acceleration + Sixel + True Color + Japanese input on raw console

## Design Philosophy

**bcon は「縁の下の力持ち」— 基盤レイヤーに徹する。**

- セッション管理は tmux / screen / zellij に任せる（車輪の再発明をしない）
- 画面分割・ペイン管理も既存ツールに任せる
- bcon が提供するのは：美しく、ヌルヌル動く、モダンなレンダリング基盤

```
┌─────────────────────────────────┐
│  tmux / zellij / screen        │  ← セッション管理
├─────────────────────────────────┤
│  bcon                          │  ← GPU レンダリング基盤
├─────────────────────────────────┤
│  DRM/KMS + OpenGL ES           │  ← ハードウェア
└─────────────────────────────────┘
```

## Technical Architecture

```
┌─────────────────────────────────────────────────────────┐
│                        bcon                             │
├─────────────────────────────────────────────────────────┤
│  VT Parser      │ ANSI/DEC escape sequences, Sixel     │
│  Text Shaping   │ rustybuzz (HarfBuzz compatible)      │
│  Font Rendering │ fontdue (Pure Rust FreeType)         │
│  GPU Backend    │ OpenGL ES via EGL + GBM              │
│  Display        │ DRM/KMS direct                        │
│  Input          │ evdev + xkbcommon                     │
│  IME            │ fcitx5 via D-Bus (zbus)              │
└─────────────────────────────────────────────────────────┘
```

## Implemented Features

### Rendering
- GPU rendering via OpenGL ES (DRM/KMS + EGL + GBM)
- Pixel-aligned glyph rendering for sharp text
- Full 24-bit True Color support
- Font ligature support (FiraCode, JetBrains Mono, etc.)
- Color emoji rendering (Noto Color Emoji)

### Terminal Emulation
- Sixel graphics display
- Kitty graphics protocol support
- Configurable scrollback buffer (default: 10,000 lines)
- Mouse support: selection, wheel scroll, button events (X10/SGR/URXVT)
- OSC 52 clipboard integration
- Bracketed paste mode

### Input
- Full keyboard support via evdev + xkbcommon
- Japanese input via fcitx5 D-Bus integration
- Automatic IME disable for vim/emacs/etc.
- Configurable key repeat delay/rate

### UX
- Vim-like copy mode for text selection
- Incremental search in scrollback (Ctrl+Shift+F)
- Screenshot to PNG (PrintScreen or Ctrl+Shift+S)
- Runtime font size adjustment (Ctrl+Plus/Minus)
- Visual bell on bell character
- URL detection with Ctrl+Click

### Configuration
- TOML-based configuration (`~/.config/bcon/config.toml`)
- Configurable keybinds (multiple keys per action)
- Preset support: `default`, `vim`, `emacs`, `japanese`/`jp`
- Combinable presets: `--init-config=vim,jp`

## File Structure

```
src/
├── main.rs               # Entry point, event loop
├── config/
│   └── mod.rs            # Configuration, keybinds, presets
├── drm/
│   ├── mod.rs
│   ├── device.rs         # DRM device handling
│   └── display.rs        # Mode setting, page flip
├── gpu/
│   ├── mod.rs
│   ├── context.rs        # EGL/OpenGL setup
│   ├── shader.rs         # Shader compilation
│   ├── renderer.rs       # Text rendering
│   ├── emoji_renderer.rs # Color emoji rendering
│   ├── image_renderer.rs # Sixel/Kitty image rendering
│   └── ui_renderer.rs    # UI overlay rendering
├── font/
│   ├── mod.rs
│   ├── atlas.rs          # Glyph texture atlas
│   ├── shaper.rs         # Text shaping (rustybuzz)
│   └── emoji.rs          # Color emoji handling
├── terminal/
│   ├── mod.rs
│   ├── pty.rs            # PTY handling, foreground process detection
│   ├── parser.rs         # VT escape sequence parser
│   ├── grid.rs           # Character grid/cells
│   ├── sixel.rs          # Sixel decoder
│   └── kitty.rs          # Kitty graphics protocol
└── input/
    ├── mod.rs
    ├── evdev.rs          # evdev raw input
    ├── keyboard.rs       # Keyboard event processing
    └── ime.rs            # fcitx5 D-Bus integration
```

## Rendering Philosophy

**美しく、見やすく、速く。** これが bcon のレンダリングの最優先原則。

### 鮮明さ（ぼやけ防止）
- グリフの頂点座標は必ず **整数ピクセルに丸める** (`.round()`)
- 小数ピクセル座標はテクスチャの LINEAR 補間でにじみの原因になる
- `push_char`, `push_glyph`, `push_text` すべてで適用すること

### テクスチャアトラス
- R8 シングルチャネル。フラグメントシェーダーで `alpha = texture.r` として使用
- アトラス左上 (0,0)-(1,1) に **2x2 の白ピクセル (R=255)** を予約配置
- `push_rect` は `atlas.solid_uv()` でこの白ピクセルを参照し、不透明な矩形を描画する
- グリフの中心ピクセルを矩形描画に使ってはならない（ストロークの隙間で alpha≈0 になる）

### フォント選択
- 主フォントは **等幅フォント** を使うこと（DejaVu Sans Mono, FiraCode 等）
- CJK フォント (Noto Sans CJK) はフォールバック専用。主フォントに指定すると文字間隔が崩れる
- セル幅は主フォントの 'M' advance width で決定される

### 視認性
- テキストの読みやすさを最優先にする
- 背景色・前景色のコントラスト比を十分に確保する
- プリエディット等のオーバーレイは確定テキストと明確に区別できるスタイルにする

## Key Technical Notes

### DRM/KMS without root
Either run as root, or use logind session. For development, root is simpler.

### GBM + EGL Setup
```rust
// Platform display with GBM
let display = egl.get_platform_display(
    khronos_egl::PLATFORM_GBM_KHR,
    gbm_device.as_raw() as *mut _,
    &[khronos_egl::NONE]
)?;
```

### OpenGL ES Shaders
Use GLSL ES 3.00. Vertex shader for positioning, fragment shader for glyph texture sampling with color.

### VT Parser
Uses `vte` crate for parsing. Implements Performer trait for handling escape sequences.
Reference: https://vt100.net/emu/dec_ansi_parser

### Sixel Format
- Starts with `DCS q` (or `ESC P q`)
- Ends with `ST` (or `ESC \`)
- 6 vertical pixels per character row
- Reference: https://github.com/saitoha/libsixel

### Foreground Process Detection
- Uses `tcgetpgrp()` to get foreground process group
- Supports tmux/screen/zellij: detects inner foreground process
- tmux: `tmux display-message -p '#{pane_current_command}'`
- screen: `screen -Q title` + process tree walking
- zellij: Process tree walking via `/proc/[pid]/task/[tid]/children`

### Configurable Keybinds
- Keybinds accept both single string and array format in TOML
- Custom serde deserializer handles flexible input
- `ParsedKeybinds` struct manages multiple bindings per action
- Supports: Ctrl, Shift, Alt modifiers + alphabets, numbers, function keys, special keys

## Code Style

- Rust 2021 edition
- Use `anyhow` for application errors, `thiserror` for library errors
- Prefer returning `Result` over panicking
- Use `log` macros for debugging (trace/debug/info/warn/error)
- Keep unsafe blocks minimal and well-documented
- Module structure: one file per major component

## Language Guidelines

Use **English** for wider adoption:
- README.md, CLAUDE.md
- Release notes / CHANGELOG
- Commit messages
- GitHub Issues/PR titles
- Code comments

Japanese is fine for:
- Discussions with Japanese users
- Internal notes during development

## Release Checklist

**IMPORTANT: Do NOT tag or push without explicit user instruction.**

- Commits: OK to make
- Version update in Cargo.toml: Ask user first
- **Tag + Push: Wait for user's explicit "release" instruction**

When releasing a new version (after user approval):
1. Update version in `Cargo.toml`
2. Commit changes
3. Tag: `git tag vX.Y.Z`
4. Push: `git push && git push --tags`

## Testing

Run on actual TTY (Ctrl+Alt+F2), not inside X terminal.

```bash
# Build
cargo build --release

# Switch to TTY2 and run
sudo chvt 2
sudo ./target/release/bcon

# Return to graphical session
sudo chvt 1  # or 7

# Test Sixel (requires libsixel)
img2sixel test.png
```

## Reference Projects

- **Ghostty** (Zig): https://github.com/ghostty-org/ghostty
- **foot** (C, Wayland): https://codeberg.org/dnkl/foot
- **yaft** (C, framebuffer + Sixel): https://github.com/uobikiemukot/yaft
- **kmscon** (C): https://github.com/dvdhrm/kmscon
- **alacritty** (Rust): https://github.com/alacritty/alacritty

## Commands for Claude Code

When implementing, prefer:
- Small, focused commits
- Test each component in isolation before integration
- Add debug logging liberally during development
- Check Ghostty source for reference when stuck on specific problems

### When asked to "release"

**Always push to remote.** Never say "done" after just local commit/tag.

1. Update CHANGELOG
2. Update version in Cargo.toml
3. Commit
4. Create tag (`git tag vX.Y.Z`)
5. **Push (`git push origin main && git push origin vX.Y.Z`)**
6. Confirm push succeeded before reporting "release complete"

### Before pushing any code changes

**Always build locally first** to verify no compile errors:
```bash
cargo build --release
```
Do NOT push until the build succeeds. Fix errors locally, then push.
