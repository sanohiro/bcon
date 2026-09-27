#!/usr/bin/env python3
"""Bounded interactive rendering stress test for bcon rootless sessions."""

import base64
import datetime as dt
import os
import select
import shutil
import struct
import subprocess
import sys
import tempfile
import termios
import time
import tty
import zlib
from pathlib import Path

PHASES = ("text", "scroll", "sixel", "kitty-inline", "kitty-file")
PHASE_SECONDS = 15
FRAME_INTERVAL = {"text": 0.12, "scroll": 0.12, "sixel": 0.30,
                  "kitty-inline": 0.25, "kitty-file": 0.25}


def log(path, message):
    line = f"{dt.datetime.now().astimezone().isoformat(timespec='seconds')} {message}\n"
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o644)
    try:
        os.write(fd, line.encode())
        os.fsync(fd)
    finally:
        os.close(fd)


def watch(path, parent):
    while True:
        try:
            os.kill(parent, 0)
        except ProcessLookupError:
            log(path, "WATCH parent exited")
            return
        log(path, "WATCH host heartbeat")
        time.sleep(5)


def png_bytes(variant):
    width = height = 192
    rows = []
    colors = ((240, 35, 35), (30, 215, 65), (40, 85, 245), (245, 215, 25))
    for y in range(height):
        row = bytearray([0])
        for x in range(width):
            quadrant = (y // 96) * 2 + x // 96
            r, g, b = colors[(quadrant + variant) % 4]
            shade = (x * 2 + y + variant * 19) % 64
            row.extend((max(0, r - shade), max(0, g - shade), max(0, b - shade)))
        rows.append(row)

    def chunk(kind, body):
        data = kind + body
        return struct.pack(">I", len(body)) + data + struct.pack(">I", zlib.crc32(data))

    return (b"\x89PNG\r\n\x1a\n"
            + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(b"".join(rows)))
            + chunk(b"IEND", b""))


def kitty_inline(data, image_id):
    encoded = base64.b64encode(data)
    chunks = [encoded[i:i + 4096] for i in range(0, len(encoded), 4096)]
    packets = []
    for index, chunk in enumerate(chunks):
        if index == 0:
            params = f"a=T,t=d,f=100,i={image_id},q=2".encode()
        else:
            params = b""
        if index + 1 < len(chunks):
            params += b",m=1" if params else b"m=1"
        packets.append(b"\x1b_G" + params + b";" + chunk + b"\x1b\\")
    return b"".join(packets)


def frame(phase, elapsed, duration, count, images, sixels, paths):
    size = shutil.get_terminal_size()
    width = max(25, size.columns)
    height = max(12, size.lines)
    title = (f"bcon rootless stress | {phase} | {elapsed:4.0f}/{duration:.0f}s "
             f"| frame {count} | q/Ctrl+C: stop")
    prefix = b"\x1b_Ga=d,d=A,q=2\x1b\\\x1b[2J\x1b[H"
    output = bytearray(prefix + title[:width].encode() + b"\r\n")
    output.extend(("=" * min(width, 72) + "\r\n").encode())
    variant = count % len(images)

    if phase in ("text", "scroll"):
        lines = height - 4 if phase == "text" else height + 9
        for row in range(lines):
            red = (count * 11 + row * 7) % 256
            green = (count * 5 + row * 13) % 256
            blue = (count * 3 + row * 17) % 256
            line = (f"\x1b[38;2;{red};{green};{blue}m{count:05d} / {row:02d} "
                    "ABC 123 日本語 かな カナ 😀 ─── 画面更新\x1b[0m\r\n")
            output.extend(line.encode())
    elif phase == "sixel" and sixels:
        output.extend("Sixel 192x192 / 赤 緑 青 黄\r\n".encode())
        output.extend(sixels[variant])
    elif phase == "kitty-inline":
        output.extend("Kitty inline chunks / PNG 192x192\r\n".encode())
        output.extend(kitty_inline(images[variant], 100 + count % 64))
    elif phase == "kitty-file":
        output.extend("Kitty file path / PNG 192x192\r\n".encode())
        path = base64.b64encode(str(paths[variant]).encode())
        output.extend(b"\x1b_Ga=T,t=f,f=100,i=1,q=2;" + path + b"\x1b\\")
    return output


def run(minutes, end_wall=None):
    if not sys.stdin.isatty() or not sys.stdout.isatty():
        raise SystemExit("bcon の端末内で実行してください")
    duration = minutes * 60
    started = dt.datetime.now().strftime("%Y%m%d-%H%M%S")
    log_path = Path.home() / f"bcon-stress-{started}.log"
    latest = Path.home() / "bcon-stress-last.log"
    latest.unlink(missing_ok=True)
    latest.symlink_to(log_path)
    log(log_path, f"START duration={duration}s pid={os.getpid()} TERM={os.getenv('TERM')} "
                  f"TERM_PROGRAM={os.getenv('TERM_PROGRAM')}")
    watcher = subprocess.Popen([sys.executable, __file__, "--watch", str(log_path), str(os.getpid())],
                               stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                               stderr=subprocess.DEVNULL, start_new_session=True)
    fd = sys.stdin.fileno()
    old = termios.tcgetattr(fd)
    image_dir = tempfile.TemporaryDirectory(prefix="bcon-stress-")
    try:
        paths = []
        images = []
        sixels = []
        for variant in range(4):
            path = Path(image_dir.name) / f"frame-{variant}.png"
            image = png_bytes(variant)
            path.write_bytes(image)
            paths.append(path)
            images.append(image)
            if shutil.which("img2sixel"):
                sixels.append(subprocess.run(["img2sixel", str(path)],
                                             check=True, stdout=subprocess.PIPE,
                                             stderr=subprocess.DEVNULL).stdout)
        log(log_path, f"FIXTURES png_bytes={[len(i) for i in images]} sixel={bool(sixels)}")
        tty.setraw(fd)
        sys.stdout.buffer.write(b"\x1b[?1049h\x1b[?25l")
        sys.stdout.buffer.flush()
        begin = time.monotonic()
        count = 0
        last_progress = -10
        phase = None
        stopped = False
        while True:
            elapsed = time.monotonic() - begin
            if elapsed >= duration or (end_wall is not None and time.time() >= end_wall):
                break
            next_phase = PHASES[int(elapsed // PHASE_SECONDS) % len(PHASES)]
            if next_phase != phase:
                phase = next_phase
                log(log_path, f"PHASE {phase} elapsed={elapsed:.1f}s")
            if elapsed - last_progress >= 10:
                log(log_path, f"PROGRESS elapsed={elapsed:.1f}s frames={count} phase={phase}")
                last_progress = elapsed
            sys.stdout.buffer.write(frame(phase, elapsed, duration, count, images, sixels, paths))
            sys.stdout.buffer.flush()
            count += 1
            if select.select([fd], [], [], FRAME_INTERVAL[phase])[0]:
                keys = os.read(fd, 1024)
                if b"q" in keys or b"\x03" in keys:
                    stopped = True
                    break
        elapsed = time.monotonic() - begin
        log(log_path, f"{'STOPPED' if stopped else 'COMPLETE'} elapsed={elapsed:.1f}s frames={count}")
    except Exception as error:
        log(log_path, f"ERROR {type(error).__name__}: {error}")
        raise
    finally:
        sys.stdout.buffer.write(b"\x1b_Ga=d,d=A,q=2\x1b\\\x1b[0m\x1b[?25h\x1b[?1049l")
        sys.stdout.buffer.flush()
        termios.tcsetattr(fd, termios.TCSADRAIN, old)
        watcher.terminate()
        try:
            watcher.wait(timeout=2)
        except subprocess.TimeoutExpired:
            watcher.kill()
        image_dir.cleanup()
    print(f"bcon stress: {elapsed:.1f}s, {count} frames. Log: {log_path}")


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--watch":
        watch(sys.argv[2], int(sys.argv[3]))
    else:
        end_wall = None
        if len(sys.argv) == 3 and sys.argv[1] == "--until":
            try:
                hour, minute = map(int, sys.argv[2].split(":"))
                if not (0 <= hour < 24 and 0 <= minute < 60):
                    raise ValueError
            except ValueError:
                raise SystemExit("use --until HH:MM (local time)")
            now = dt.datetime.now().astimezone()
            target = now.replace(hour=hour, minute=minute, second=0, microsecond=0)
            if target <= now:
                target += dt.timedelta(days=1)
            end_wall = target.timestamp()
            minutes = (end_wall - time.time()) / 60
        else:
            minutes = int(sys.argv[1]) if len(sys.argv) > 1 else 20
        if not 1 <= minutes <= 1440:
            raise SystemExit("duration must be 1..1440 minutes")
        run(minutes, end_wall)
