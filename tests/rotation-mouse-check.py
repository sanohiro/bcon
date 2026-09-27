#!/usr/bin/env python3
"""Check that clicks land on visible targets with a rotated bcon display."""

import os
import re
import select
import shutil
import sys
import termios
import tty

EVENT = re.compile(rb"\x1b\[<([0-9]+);([0-9]+);([0-9]+)([Mm])")
OUT = sys.stdout.buffer


def emit(data):
    OUT.write(data.encode() if isinstance(data, str) else data)
    OUT.flush()


def draw(targets, active, result=""):
    emit("\x1b[2J\x1b[H")
    emit("マウス位置テスト: 緑の [番号] の中央をクリックしてください。q で終了。\n")
    emit("カーソルの見た目とクリック位置が一致するかも確認してください。")
    for index, (x, y) in enumerate(targets):
        color = "\x1b[32;1m" if index == active else "\x1b[90m"
        emit(f"\x1b[{y};{x}H{color}[ {index + 1} ]\x1b[0m")
    rows = shutil.get_terminal_size().lines
    emit(f"\x1b[{rows - 1};1H\x1b[2K{result}")
    emit(f"\x1b[{rows};1H\x1b[2K{active + 1}/{len(targets)} をクリック")


def main():
    size = shutil.get_terminal_size()
    if size.columns < 25 or size.lines < 12:
        raise SystemExit("端末が小さすぎます")
    targets = [
        (5, 5),
        (size.columns - 8, 5),
        (size.columns - 8, size.lines - 5),
        (5, size.lines - 5),
        (size.columns // 2, size.lines // 2),
    ]
    fd = sys.stdin.fileno()
    old = termios.tcgetattr(fd)
    buffer = b""
    active = 0
    try:
        tty.setraw(fd)
        emit("\x1b[?1000h\x1b[?1006h\x1b[?25h")
        draw(targets, active)
        while active < len(targets):
            if not select.select([fd], [], [], 1)[0]:
                continue
            chunk = os.read(fd, 1024)
            if not chunk:
                break
            buffer += chunk
            if b"q" in buffer or b"\x03" in buffer:
                break
            match = EVENT.search(buffer)
            while match:
                button, x, y, edge = match.groups()
                buffer = buffer[match.end():]
                if edge == b"M" and int(button) & 3 == 0:
                    tx, ty = targets[active]
                    if abs(int(x) - tx - 2) <= 3 and abs(int(y) - ty) <= 1:
                        active += 1
                        if active == len(targets):
                            break
                        draw(targets, active, f"OK: {x.decode()},{y.decode()}")
                    else:
                        draw(targets, active, f"ずれ: click={x.decode()},{y.decode()} target={tx + 2},{ty}")
                match = EVENT.search(buffer)
            buffer = buffer[-100:]
    finally:
        emit("\x1b[?1000l\x1b[?1006l\x1b[0m\x1b[2J\x1b[H")
        termios.tcsetattr(fd, termios.TCSADRAIN, old)
    print(f"マウス位置: {active}/{len(targets)} 成功")


if __name__ == "__main__":
    main()
