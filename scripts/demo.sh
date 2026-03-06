#!/bin/bash
# bcon demo script — showcases terminal capabilities
# Usage: ./scripts/demo.sh [--fast] [--no-pause]
#
# Designed to be recorded with screen capture for README demo video.

set -eu

FAST=false
NO_PAUSE=false
for arg in "$@"; do
  case "$arg" in
    --fast) FAST=true ;;
    --no-pause) NO_PAUSE=true ;;
  esac
done

# === Helpers ===

RESET="\033[0m"
BOLD="\033[1m"
DIM="\033[2m"
ITALIC="\033[3m"
UNDERLINE="\033[4m"

type_text() {
  local text="$1"
  local delay=${2:-0.03}
  if $FAST; then delay=0.005; fi
  for ((i=0; i<${#text}; i++)); do
    printf '%s' "${text:$i:1}"
    sleep "$delay"
  done
}

pause() {
  if $NO_PAUSE; then
    sleep 0.5
  else
    sleep "${1:-1.5}"
  fi
}

section() {
  printf "\n"
  printf "\033[38;2;100;200;255m━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}\n"
  printf "\033[1;38;2;100;200;255m  %s${RESET}\n" "$1"
  printf "\033[38;2;100;200;255m━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}\n"
  pause 1
}

clear

# =====================================================================
# Title
# =====================================================================
printf "\n"
printf "\033[1;38;2;80;200;120m"
cat << 'LOGO'
   ██████╗  ██████╗ ██████╗ ███╗   ██╗
   ██╔══██╗██╔════╝██╔═══██╗████╗  ██║
   ██████╔╝██║     ██║   ██║██╔██╗ ██║
   ██╔══██╗██║     ██║   ██║██║╚██╗██║
   ██████╔╝╚██████╗╚██████╔╝██║ ╚████║
   ╚═════╝  ╚═════╝ ╚═════╝ ╚═╝  ╚═══╝
LOGO
printf "${RESET}"
printf "\n"
printf "\033[38;2;180;180;180m   GPU-accelerated terminal emulator for Linux console${RESET}\n"
printf "\033[38;2;120;120;120m   DRM/KMS + OpenGL ES — No X11, No Wayland${RESET}\n"
pause 2

# =====================================================================
# 1. True Color (24-bit)
# =====================================================================
section "True Color (24-bit RGB)"

printf "  "
for i in $(seq 0 2 179); do
  r=$((255 * (180 - i) / 180))
  g=$((255 * i / 180))
  b=80
  printf "\033[48;2;${r};${g};${b}m \033[0m"
done
printf "\n  "
for i in $(seq 0 2 179); do
  r=80
  g=$((255 * (180 - i) / 180))
  b=$((255 * i / 180))
  printf "\033[48;2;${r};${g};${b}m \033[0m"
done
printf "\n  "
for i in $(seq 0 2 179); do
  r=$((255 * i / 180))
  g=80
  b=$((255 * (180 - i) / 180))
  printf "\033[48;2;${r};${g};${b}m \033[0m"
done
printf "\n"
pause

# =====================================================================
# 2. Colored Underlines (SGR 58/59)
# =====================================================================
section "Colored Underlines (SGR 58/59)"

printf "  \033[4:1m\033[58;2;255;100;100mSingle underline${RESET}  "
printf "\033[4:2m\033[58;2;100;255;100mDouble underline${RESET}  "
printf "\033[4:3m\033[58;2;100;100;255mCurly underline${RESET}\n"
printf "  \033[4:4m\033[58;2;255;200;50mDotted underline${RESET}  "
printf "\033[4:5m\033[58;2;200;100;255mDashed underline${RESET}\n"
pause

# =====================================================================
# 3. Multilingual Text
# =====================================================================
section "Multilingual Text Rendering"

printf "  \033[1;38;2;255;220;100mEnglish${RESET}    : The quick brown fox jumps over the lazy dog\n"
printf "  \033[1;38;2;255;100;100m日本語${RESET}     : 吾輩は猫である。名前はまだ無い。\n"
printf "  \033[1;38;2;100;200;255m中文${RESET}       : 天地玄黄，宇宙洪荒。日月盈昃，辰宿列张。\n"
printf "  \033[1;38;2;100;255;200m한국어${RESET}     : 하늘과 땅이 처음 열리니 만물이 생겨났다.\n"
printf "  \033[1;38;2;200;150;255mРусский${RESET}    : Съешь ещё этих мягких французских булок.\n"
printf "  \033[1;38;2;255;180;100mDeutsch${RESET}    : Zwölf Boxkämpfer jagen Viktor quer über den Sylter Deich.\n"
printf "  \033[1;38;2;100;180;255mFrançais${RESET}   : Portez ce vieux whisky au juge blond qui fume.\n"
printf "  \033[1;38;2;255;100;180mEspañol${RESET}    : El veloz murciélago hindú comía feliz cardillo y kiwi.\n"
printf "  \033[1;38;2;100;255;100mPortuguês${RESET}  : À noite, vovô Kowalsky vê o ímã cair no pé do pingüim.\n"
pause

# =====================================================================
# 4. Color Emoji
# =====================================================================
section "Color Emoji (Noto Color Emoji)"

printf "  Faces : 😀 😎 🤖 👻 🎃 🥳 😈 🤯 🥶 🤠\n"
printf "  Hands : 👍 👎 ✌️  🤞 🫶 👏 🙌 💪 🤝 ✊\n"
printf "  Nature: 🌸 🌺 🌻 🌴 🍁 🌊 ⛰️  🌈 ☀️  🌙\n"
printf "  Food  : 🍣 🍜 🍕 🍔 🌮 🥐 🧁 🍰 🍩 🍺\n"
printf "  Tech  : 💻 ⌨️  🖥️  📱 🔧 ⚙️  🚀 🛸 🤖 🧠\n"
printf "  Flags : 🇯🇵 🇺🇸 🇬🇧 🇩🇪 🇫🇷 🇪🇸 🇧🇷 🇷🇺 🇰🇷 🇨🇳\n"
pause

# =====================================================================
# 5. Font Ligatures
# =====================================================================
section "Font Ligatures (FiraCode / JetBrains Mono)"

printf "  \033[38;2;180;220;255m"
printf "  Arrows  : -> => <- >> << |> <| >>= =<< ->> <<-\n"
printf "  Compare : == != === !== >= <= <> =/=\n"
printf "  Logic   : && || !! :: .. ... ..<\n"
printf "  Types   : :: :> <: |> <|\n"
printf "  Other   : #{  #[ #( #_ #{ www *** /// /**\n"
printf "  ${RESET}"
pause

# =====================================================================
# 6. Sixel Graphics
# =====================================================================
section "Sixel Graphics"

# Generate a test Sixel image inline (color gradient)
if command -v img2sixel &>/dev/null; then
  printf "  (img2sixel available — generating test image...)\n"
  # Create a small PPM gradient
  TMP_PPM=$(mktemp /tmp/bcon_demo_XXXXXX.ppm)
  {
    printf "P6\n200 60\n255\n"
    for y in $(seq 0 59); do
      for x in $(seq 0 199); do
        r=$((x * 255 / 200))
        g=$((y * 255 / 60))
        b=$(( (200 - x) * 255 / 200 ))
        printf "\\$(printf '%03o' $r)\\$(printf '%03o' $g)\\$(printf '%03o' $b)"
      done
    done
  } > "$TMP_PPM"
  printf "  "
  img2sixel "$TMP_PPM" 2>/dev/null || printf "  (Sixel rendering...)\n"
  rm -f "$TMP_PPM"
else
  printf "  \033[33m(img2sixel not installed — skipping Sixel demo)\033[0m\n"
  printf "  Install libsixel-bin to see Sixel graphics in action.\n"
fi
pause

# =====================================================================
# 7. AI CLI Tools Showcase
# =====================================================================
section "Built for AI-Powered Development"

# Claude Code
printf "\n"
printf "  \033[48;2;30;30;30m\033[38;2;200;160;255m ◆ Claude Code \033[38;2;120;120;120m─────────────────────────────────────────────── ${RESET}\n"
printf "  \033[48;2;30;30;30m                                                                        ${RESET}\n"
printf "  \033[48;2;30;30;30m  \033[38;2;200;160;255m❯\033[38;2;255;255;255m Add authentication to the API                                  ${RESET}\n"
printf "  \033[48;2;30;30;30m                                                                        ${RESET}\n"
printf "  \033[48;2;30;30;30m  \033[38;2;100;200;255m⠋ Reading src/api/routes.rs ...                                    ${RESET}\n"
printf "  \033[48;2;30;30;30m  \033[38;2;80;180;80m✓ Created src/middleware/auth.rs                                   ${RESET}\n"
printf "  \033[48;2;30;30;30m  \033[38;2;80;180;80m✓ Updated src/api/routes.rs (+42 -3)                               ${RESET}\n"
printf "  \033[48;2;30;30;30m  \033[38;2;80;180;80m✓ Added JWT validation with RS256                                  ${RESET}\n"
printf "  \033[48;2;30;30;30m                                                                        ${RESET}\n"
pause 1

# Codex
printf "\n"
printf "  \033[48;2;25;25;35m\033[38;2;100;255;180m ◇ Codex \033[38;2;120;120;120m──────────────────────────────────────────────────────── ${RESET}\n"
printf "  \033[48;2;25;25;35m                                                                        ${RESET}\n"
printf "  \033[48;2;25;25;35m  \033[38;2;100;255;180m>\033[38;2;255;255;255m Review this codebase for security issues                        ${RESET}\n"
printf "  \033[48;2;25;25;35m                                                                        ${RESET}\n"
printf "  \033[48;2;25;25;35m  \033[38;2;255;200;80m⚠ Found 3 potential issues:                                        ${RESET}\n"
printf "  \033[48;2;25;25;35m  \033[38;2;255;100;100m  Critical: Unvalidated file path in graphics protocol             ${RESET}\n"
printf "  \033[48;2;25;25;35m  \033[38;2;255;180;80m  High: Integer overflow in image dimensions                       ${RESET}\n"
printf "  \033[48;2;25;25;35m  \033[38;2;255;255;100m  Medium: Unbounded memory allocation in registry                  ${RESET}\n"
printf "  \033[48;2;25;25;35m                                                                        ${RESET}\n"
pause 1

# Gemini CLI
printf "\n"
printf "  \033[48;2;30;30;25m\033[38;2;100;180;255m ◈ Gemini CLI \033[38;2;120;120;120m────────────────────────────────────────────────── ${RESET}\n"
printf "  \033[48;2;30;30;25m                                                                        ${RESET}\n"
printf "  \033[48;2;30;30;25m  \033[38;2;100;180;255m✦\033[38;2;255;255;255m Explain the rendering pipeline                                  ${RESET}\n"
printf "  \033[48;2;30;30;25m                                                                        ${RESET}\n"
printf "  \033[48;2;30;30;25m  \033[38;2;220;220;220mThe pipeline: VT Parser → Text Shaper → Glyph Atlas → GPU          ${RESET}\n"
printf "  \033[48;2;30;30;25m  \033[38;2;180;180;180mOpenGL ES renders glyphs via instanced draw calls...               ${RESET}\n"
printf "  \033[48;2;30;30;25m                                                                        ${RESET}\n"
pause 1

# =====================================================================
# 8. TUI Applications
# =====================================================================
section "Modern TUI Applications"

# Zellij-like layout
printf "\n"
printf "  \033[48;2;40;40;50m\033[38;2;150;200;255m Zellij \033[38;2;80;80;100m━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━ ${RESET}\n"
printf "  \033[48;2;40;40;50m \033[38;2;80;80;100m┌─── Tab 1: dev ──────────────┬─── Tab 2: logs ──────────────┐${RESET}\n"
printf "  \033[48;2;40;40;50m \033[38;2;80;80;100m│\033[38;2;100;255;100m ~/src\033[38;2;255;255;255m \$ cargo build --release\033[38;2;80;80;100m│\033[38;2;255;200;100m[INFO] Server started on :8080\033[38;2;80;80;100m│${RESET}\n"
printf "  \033[48;2;40;40;50m \033[38;2;80;80;100m│\033[38;2;80;200;80m   Compiling bcon v0.5.0     \033[38;2;80;80;100m│\033[38;2;180;180;180m[DEBUG] Connection accepted    \033[38;2;80;80;100m│${RESET}\n"
printf "  \033[48;2;40;40;50m \033[38;2;80;80;100m│\033[38;2;80;200;80m    Finished in 12.3s        \033[38;2;80;80;100m│\033[38;2;180;180;180m[DEBUG] Request: GET /api/v1   \033[38;2;80;80;100m│${RESET}\n"
printf "  \033[48;2;40;40;50m \033[38;2;80;80;100m└──────────────────────────────┴──────────────────────────────┘${RESET}\n"
printf "  \033[48;2;40;40;50m \033[38;2;80;80;100m Ctrl+p ▸ Panes  Ctrl+t ▸ Tabs  Ctrl+s ▸ Scroll  Ctrl+q ▸ Quit${RESET}\n"
pause 1

# Yazi-like file manager
printf "\n"
printf "  \033[48;2;35;35;40m\033[38;2;255;200;100m 🦆 Yazi \033[38;2;80;80;100m─────────────────────────────────────────────────────────${RESET}\n"
printf "  \033[48;2;35;35;40m \033[38;2;80;80;100m│\033[38;2;100;180;255m 📁 src/           \033[38;2;80;80;100m│\033[38;2;255;255;255m 📁 config/      \033[38;2;80;80;100m│\033[38;2;180;180;180m [security]               \033[38;2;80;80;100m│${RESET}\n"
printf "  \033[48;2;35;35;40m \033[38;2;80;80;100m│\033[38;2;180;180;180m 📁 scripts/       \033[38;2;80;80;100m│\033[48;2;60;60;80m\033[38;2;255;200;100m 📄 mod.rs       \033[48;2;35;35;40m\033[38;2;80;80;100m│\033[38;2;180;180;180m allow_kitty_remote = false\033[38;2;80;80;100m│${RESET}\n"
printf "  \033[48;2;35;35;40m \033[38;2;80;80;100m│\033[38;2;180;180;180m 📄 Cargo.toml     \033[38;2;80;80;100m│\033[38;2;180;180;180m 📁 drm/         \033[38;2;80;80;100m│\033[38;2;180;180;180m                          \033[38;2;80;80;100m│${RESET}\n"
printf "  \033[48;2;35;35;40m \033[38;2;80;80;100m│\033[38;2;180;180;180m 📄 README.md      \033[38;2;80;80;100m│\033[38;2;180;180;180m 📁 font/        \033[38;2;80;80;100m│\033[38;2;180;180;180m [appearance]              \033[38;2;80;80;100m│${RESET}\n"
printf "  \033[48;2;35;35;40m \033[38;2;80;80;100m│\033[38;2;180;180;180m 📄 CLAUDE.md      \033[38;2;80;80;100m│\033[38;2;180;180;180m 📁 gpu/         \033[38;2;80;80;100m│\033[38;2;180;180;180m font_size = 20            \033[38;2;80;80;100m│${RESET}\n"
printf "  \033[48;2;35;35;40m \033[38;2;80;80;100m└───────────────────┴──────────────────┴──────────────────────────┘${RESET}\n"
pause 1

# =====================================================================
# 9. Performance Stats
# =====================================================================
section "Performance"

printf "\n"
printf "  \033[38;2;80;200;120m▐█████████████████████████████████████████\033[38;2;180;180;180m  GPU Render   : < 2ms / frame${RESET}\n"
printf "  \033[38;2;100;180;255m▐██████████████████████████████████████   \033[38;2;180;180;180m  Text Shaping : < 1ms / line${RESET}\n"
printf "  \033[38;2;255;200;100m▐████████████████████████████████████     \033[38;2;180;180;180m  Input Latency: < 5ms${RESET}\n"
printf "  \033[38;2;200;100;255m▐█████████████████████████████            \033[38;2;180;180;180m  Memory Usage : ~30MB base${RESET}\n"
printf "\n"
printf "  \033[38;2;120;120;120mMeasured on: Intel i5 + Mesa / AMD Radeon + AMDGPU${RESET}\n"
pause

# =====================================================================
# Finale
# =====================================================================
printf "\n"
printf "\033[38;2;80;200;120m━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}\n"
printf "\n"
printf "  \033[1;38;2;80;200;120mbcon${RESET} — \033[38;2;180;180;180mBringing modern terminal experience to bare metal Linux${RESET}\n"
printf "\n"
printf "  \033[38;2;120;120;120m  GitHub : https://github.com/sanohiro/bcon${RESET}\n"
printf "  \033[38;2;120;120;120m  License: MIT${RESET}\n"
printf "\n"
printf "\033[38;2;80;200;120m━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}\n"
printf "\n"
