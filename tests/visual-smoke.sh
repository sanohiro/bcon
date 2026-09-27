#!/usr/bin/env bash
# Interactive rendering check. Run inside bcon and capture each numbered screen.
set -eu

if [[ ${1:-} == --help ]]; then
    printf 'Usage: bash tests/visual-smoke.sh\nRun inside bcon. Enter=OK, n=NG, s=skip, q=quit. Results: ~/bcon-visual-*.log\n'
    exit 0
fi
if [[ ! -t 0 || ! -t 1 ]]; then
    printf 'bcon の端末内で実行してください。\n' >&2
    exit 1
fi
command -v python3 >/dev/null || { printf 'python3 が必要です。\n' >&2; exit 1; }

tmp_dir=$(mktemp -d)
image_path=$tmp_dir/check.png
log_path=$(mktemp "$HOME/bcon-visual-$(date +%Y%m%d-%H%M%S)-XXXXXX.log")
printf 'START %s TERM=%s TERM_PROGRAM=%s\n' "$(date -Is)" "${TERM:-}" "${TERM_PROGRAM:-}" > "$log_path"
bad=()
skipped=()
passed=()
completed=false
in_alternate=false
title=''

cleanup() {
    local status=$?
    if $in_alternate; then printf '\033[?1049l'; fi
    printf '\033_Ga=d,d=A,q=2\033\\\033[0m\033[?25h'
    rm -rf -- "$tmp_dir"
    printf 'END completed=%s exit=%s OK=%s NG=%s SKIP=%s\n' "$completed" "$status" "${#passed[@]}" "${#bad[@]}" "${#skipped[@]}" >> "$log_path"
    printf '\n結果: OK %s / NG %s / スキップ %s（全10項目）\nログ: %s\n' "${#passed[@]}" "${#bad[@]}" "${#skipped[@]}" "$log_path"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

python3 - "$image_path" <<'PY'
import struct
import sys
import zlib

width = height = 96
rows = []
for y in range(height):
    row = bytearray([0])
    for x in range(width):
        if x < 48 and y < 48:
            rgb = (255, 40, 40)
        elif x >= 48 and y < 48:
            rgb = (40, 220, 60)
        elif x < 48:
            rgb = (40, 90, 255)
        else:
            rgb = (255, 220, 30)
        row.extend(rgb)
    rows.append(row)

def chunk(kind, data):
    body = kind + data
    return struct.pack('>I', len(data)) + body + struct.pack('>I', zlib.crc32(body))

with open(sys.argv[1], 'wb') as output:
    output.write(b'\x89PNG\r\n\x1a\n')
    output.write(chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 2, 0, 0, 0)))
    output.write(chunk(b'IDAT', zlib.compress(b''.join(rows))))
    output.write(chunk(b'IEND', b''))
PY

page() {
    title=$2
    printf '\033_Ga=d,d=A,q=2\033\\\033[0m\033[2J\033[H'
    printf 'bcon 描画チェック %s/10: %s\n' "$1" "$2"
    printf '%s\n\n' '────────────────────────────────────────'
}

check() {
    local answer result note=''
    while true; do
        printf '\n必要なら画面を撮影 → Enter: OK / n: NG / s: スキップ / q: 終了 > '
        IFS= read -r answer </dev/tty || answer=q
        case "$answer" in
            '') passed+=("$1"); result=OK; break ;;
            n|N) bad+=("$1"); result=NG
                printf '症状メモ（空欄でも可）> '
                IFS= read -r note </dev/tty || note=''
                break ;;
            s|S) skipped+=("$1"); result=SKIP; break ;;
            q|Q) printf '%s QUIT %s\n' "$1" "$title" >> "$log_path"; return 1 ;;
        esac
    done
    printf '%s %s %s %s\n' "$1" "$result" "$title" "$note" >> "$log_path"
    return 0
}

page 1 '文字と幅'
printf 'ASCII: ABCDEFGHIJKLMNOPQRSTUVWXYZ 0123456789\n'
printf '日本語: こんにちは、世界。漢字・かな・カナ\n'
printf '全角と半角: |A|あ|B|界|C|  縦線がずれないか\n'
printf '結合文字: e\u0301 / é    記号: ┌─┬─┐ │ └─┘\n'
printf '絵文字: 😀 🚀 🎉 ❤️ 🇯🇵  表示と幅を確認\n'
printf '期待: 欠け・重なり・文字化けがない\n'
check 1 || exit 0

page 2 '色と文字属性'
printf '通常  \033[1m太字\033[22m  \033[3m斜体\033[23m  \033[4m下線\033[24m  \033[9m取消線\033[29m\n'
printf '\033[7m反転表示\033[27m  \033[2m淡い文字\033[22m\n'
printf 'ANSI: '
for color in {0..7}; do printf '\033[4%sm  %s  \033[0m' "$color" "$color"; done
printf '\n256色: '
for color in {16..27}; do printf '\033[48;5;%sm  \033[0m' "$color"; done
printf '\n24bit: '
for step in {0..23}; do
    red=$((step * 255 / 23))
    blue=$((255 - red))
    printf '\033[48;2;%s;40;%sm  \033[0m' "$red" "$blue"
done
printf '\n期待: 属性と色の階調が見える\n'
check 2 || exit 0

page 3 '位置と罫線'
printf '下の枠が四角くつながり、中の文字が枠内に収まるか\n\n'
printf '┌──────────────────────────────┐\n'
printf '│ %-28s │\n' 'left       center      right'
printf '│ %-28s │\n' '1234567890 ABCDEFGHIJKLMNO'
printf '└──────────────────────────────┘\n\n'
printf '行末テスト: 次の行の X が右端にあるか\n'
cols=$(tput cols 2>/dev/null || printf 80)
if (( cols > 20 )); then
    printf '%*s' "$cols" X
    printf '\n'
fi
printf '期待: 行末の X と次の行が正常\n'
check 3 || exit 0

page 4 'スクロール'
printf '連続行を表示します。最後に 51〜60 が順番に見えるか\n'
for number in {1..60}; do printf '行 %02d  ABC 日本語 123\n' "$number"; done
check 4 || exit 0

page 5 'Sixel 画像'
if command -v img2sixel >/dev/null 2>&1; then
    printf '期待: 赤・緑・青・黄の 4 色の正方形が出る\n\n'
    if img2sixel "$image_path"; then
        check 5 || exit 0
    else
        bad+=(5)
        printf '5 NG Sixel encoder failed\n' >> "$log_path"
    fi
else
    printf 'img2sixel がないためスキップ\n'
    skipped+=(5)
    printf '5 SKIP img2sixel unavailable\n' >> "$log_path"
fi

page 6 'Kitty 画像（分割転送）'
printf '期待: 左上=赤、右上=緑、左下=青、右下=黄の正方形\n\n'
# Force multiple chunks even for this small PNG; omit m on the final chunk.
python3 - "$image_path" <<'PY'
import base64
import pathlib
import sys
data = base64.b64encode(pathlib.Path(sys.argv[1]).read_bytes())
for offset in range(0, len(data), 64):
    params = 'a=T,t=d,f=100,q=2' if offset == 0 else ''
    if offset + 64 < len(data):
        params += ',m=1' if params else 'm=1'
    sys.stdout.buffer.write(b'\x1b_G' + params.encode() + b';' + data[offset:offset + 64] + b'\x1b\\')
PY
check 6 || exit 0

page 7 'Kitty 画像（ファイル転送）'
printf '期待: 同じ 4 色の正方形が出る。casty が使う転送方式\n\n'
encoded_path=$(printf '%s' "$image_path" | base64 -w0)
printf '\033_Ga=T,t=f,f=100,q=2;%s\033\\' "$encoded_path"
printf '\n'
check 7 || exit 0

page 8 '部分更新・残像（約3秒）'
printf '期待: 固定行は動かず、更新行の右端に古い文字や背景が残らない\n'
printf '\033[6;1H固定行: ABC 日本語 ──────────'
for frame in {1..24}; do
    printf '\033[8;1H\033[2K'
    if (( frame % 2 )); then
        printf '\033[44;97m長い更新行 ABCDEFGHIJKLMNOPQRSTUVWXYZ 日本語\033[0m'
    else
        printf '\033[41;97m短い行\033[0m'
    fi
    sleep 0.12
done
printf '\033[10;1H'
check 8 || exit 0

page 9 '代替画面からの復帰'
printf '復帰確認: この文字と見出しが戻れば OK\n'
printf 'Enter で代替画面へ > '
IFS= read -r _ </dev/tty || exit 0
in_alternate=true
printf '\033[?1049h\033[2J\033[H代替画面: Enter で元の画面に戻る > '
IFS= read -r _ </dev/tty || exit 0
printf '\033[?1049l'
in_alternate=false
check 9 || exit 0

page 10 'カーソル・VT復帰'
printf '期待: カーソルが下の白黒文字上を往復し、文字を壊さない\n'
printf 'この画面で別の VT に切り替えて戻り、表示が復元されるかも確認\n'
printf '\033[6;1H\033[47;30mABCD日本語0123456789\033[0m\033[?25h'
for column in 1 2 3 4 5 7 9 11 12 13 12 11 9 7 5 4 3 2 1; do
    printf '\033[6;%sH' "$column"
    sleep 0.15
done
printf '\033[8;1H'
check 10 || exit 0

completed=true
printf '\033_Ga=d,d=A,q=2\033\\\033[2J\033[H'
printf '終了: NG %s 件、スキップ %s 件\n' "${#bad[@]}" "${#skipped[@]}"
if (( ${#bad[@]} )); then printf 'NG: %s\n' "${bad[*]}"; fi
if (( ${#skipped[@]} )); then printf 'スキップ: %s\n' "${skipped[*]}"; fi
printf 'スクリーンショットと NG 番号を共有してください。\n'
