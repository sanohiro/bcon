#!/usr/bin/env python3
"""Temporary Linux uinput bridge for an exclusively reserved bcon VT."""

import argparse
import ctypes
import json
import os
from pathlib import Path
import pwd
import re
import select
import signal
import socket
import stat
import struct
import subprocess
import sys
import time

MAX_PACKET = 4096
MODIFIERS = {"LEFTCTRL", "LEFTSHIFT", "LEFTALT", "LEFTMETA"}
ALIASES = {"CTRL": "LEFTCTRL", "SHIFT": "LEFTSHIFT", "ALT": "LEFTALT",
           "SUPER": "LEFTMETA", "ESCAPE": "ESC", "RETURN": "ENTER",
           "PRINTSCREEN": "SYSRQ", "PAGEUP": "PAGEUP", "PAGEDOWN": "PAGEDOWN"}
KEYS = set("ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789") | MODIFIERS | {
    "ENTER", "ESC", "BACKSPACE", "TAB", "SPACE", "MINUS", "EQUAL", "LEFTBRACE",
    "RIGHTBRACE", "BACKSLASH", "SEMICOLON", "APOSTROPHE", "GRAVE", "COMMA",
    "DOT", "SLASH", "CAPSLOCK", "NUMLOCK", "SCROLLLOCK", "SYSRQ", "PAUSE",
    "HOME", "END", "INSERT", "DELETE", "UP", "DOWN", "LEFT", "RIGHT",
    "PAGEUP", "PAGEDOWN", "KPDOT", "KPPLUS", "KPMINUS", "KPASTERISK",
    "KPSLASH", "KPENTER",
} | {"F" + str(i) for i in range(1, 13)} | {"KP" + str(i) for i in range(10)}


def chord(value):
    if not isinstance(value, str):
        raise ValueError("keys must be a chord string, e.g. CTRL+SHIFT+ENTER")
    names = [ALIASES.get(k.upper(), k.upper()) for k in value.split("+")]
    if not 1 <= len(names) <= 5 or len(set(names)) != len(names):
        raise ValueError("use 1-5 distinct keys")
    if any(k not in KEYS for k in names):
        raise ValueError("unknown/unsupported key")
    normal = [k for k in names if k not in MODIFIERS]
    if len(normal) != 1:
        raise ValueError("a tap requires exactly one non-modifier key")
    # Do not expose the kernel's VT-switch or magic SysRq chords.
    if "LEFTALT" in names and ("SYSRQ" in names or
                               ("LEFTCTRL" in names and
                                (re.fullmatch(r"F[0-9]+", normal[0]) or normal[0] == "DELETE"))):
        raise ValueError("VT-switch, reboot, and Alt+PrintScreen chords are disabled")
    return [k for k in names if k in MODIFIERS] + normal


def text_chords(text):
    if not isinstance(text, str) or not 1 <= len(text) <= 256:
        raise ValueError("text must contain 1-256 printable ASCII characters")
    plain = dict(zip("`-=[]\\;',./ ", ["GRAVE", "MINUS", "EQUAL", "LEFTBRACE",
                     "RIGHTBRACE", "BACKSLASH", "SEMICOLON", "APOSTROPHE",
                     "COMMA", "DOT", "SLASH", "SPACE"]))
    shifted = dict(zip('~_+{}|:"<>?', ["GRAVE", "MINUS", "EQUAL", "LEFTBRACE",
                       "RIGHTBRACE", "BACKSLASH", "SEMICOLON", "APOSTROPHE",
                       "COMMA", "DOT", "SLASH"]))
    shifted.update(dict(zip("!@#$%^&*()", "1234567890")))
    result = []
    for c in text:
        if "a" <= c <= "z" or "0" <= c <= "9":
            result.append([c.upper()])
        elif "A" <= c <= "Z":
            result.append(["LEFTSHIFT", c])
        elif c in plain:
            result.append([plain[c]])
        elif c in shifted:
            result.append(["LEFTSHIFT", shifted[c]])
        else:
            raise ValueError("text requires a US layout, printable ASCII only; send ENTER separately")
    return result


class Keyboard:
    """libevdev handles native uinput ABI details on both arm64 and x86_64."""
    def __init__(self, fd):
        self.lib = ctypes.CDLL("libevdev.so.2")
        signatures = {
            "libevdev_new": (ctypes.c_void_p, []),
            "libevdev_free": (None, [ctypes.c_void_p]),
            "libevdev_set_name": (None, [ctypes.c_void_p, ctypes.c_char_p]),
            "libevdev_event_code_from_name": (ctypes.c_int, [ctypes.c_uint, ctypes.c_char_p]),
            "libevdev_enable_event_code": (ctypes.c_int, [ctypes.c_void_p, ctypes.c_uint,
                                                        ctypes.c_uint, ctypes.c_void_p]),
            "libevdev_uinput_create_from_device": (ctypes.c_int, [ctypes.c_void_p, ctypes.c_int,
                                                                ctypes.POINTER(ctypes.c_void_p)]),
            "libevdev_uinput_write_event": (ctypes.c_int, [ctypes.c_void_p, ctypes.c_uint,
                                                         ctypes.c_uint, ctypes.c_int]),
            "libevdev_uinput_destroy": (None, [ctypes.c_void_p]),
        }
        for name, (result, args) in signatures.items():
            fn = getattr(self.lib, name)
            fn.restype, fn.argtypes = result, args
        self.device = ctypes.c_void_p()
        self.held = []
        desc = self.lib.libevdev_new()
        if not desc:
            raise MemoryError("libevdev_new")
        try:
            self.lib.libevdev_set_name(desc, b"bcon SSH test keyboard")
            self.codes = {}
            for name in sorted(KEYS):
                code = self.lib.libevdev_event_code_from_name(1, ("KEY_" + name).encode())
                if code < 0:
                    raise ValueError("libevdev does not know " + name)
                self.codes[name] = code
                self.check(self.lib.libevdev_enable_event_code(desc, 1, code, None))
            self.check(self.lib.libevdev_uinput_create_from_device(desc, fd,
                                                                  ctypes.byref(self.device)))
        finally:
            self.lib.libevdev_free(desc)

    @staticmethod
    def check(result):
        if result < 0:
            raise OSError(-result, os.strerror(-result))

    def event(self, code, value):
        self.check(self.lib.libevdev_uinput_write_event(self.device, 1, code, value))
        self.check(self.lib.libevdev_uinput_write_event(self.device, 0, 0, 0))

    def release(self):
        # Attempt every release even if one write fails. Device destruction is
        # the final fallback; no held state is allowed across requests.
        failed = None
        while self.held:
            try:
                self.event(self.held.pop(), 0)
            except OSError as error:
                failed = error
        if failed:
            raise failed

    def tap(self, names, guard):
        try:
            for name in names:
                guard.check()
                code = self.codes[name]
                self.held.append(code)
                self.event(code, 1)
            time.sleep(0.03)
        finally:
            self.release()
        time.sleep(0.02)

    def close(self):
        try:
            self.release()
        finally:
            self.lib.libevdev_uinput_destroy(self.device)


class Guard:
    def __init__(self, tty, pidfd, active_path="/sys/class/tty/tty0/active"):
        self.tty, self.pidfd, self.active_path = tty, pidfd, active_path
        self.reason = None

    def check(self):
        if self.reason is None:
            try:
                if select.select([self.pidfd], [], [], 0)[0]:
                    self.reason = "target process exited; restart helper"
                elif Path(self.active_path).read_text().strip() != self.tty:
                    self.reason = "foreground VT changed; restart helper"
            except OSError as error:
                self.reason = "cannot verify foreground VT: " + str(error)
        if self.reason:
            raise ValueError(self.reason)


def peer_uid(connection):
    return struct.unpack("3i", connection.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12))[1]


def handle(connection, uid, keyboard, guard):
    if peer_uid(connection) != uid:
        return {"ok": False, "error": "peer UID is not authorized"}
    packet, _, flags, _ = connection.recvmsg(MAX_PACKET)
    if flags & socket.MSG_TRUNC:
        raise ValueError("request too large")
    request = json.loads(packet)
    if not isinstance(request, dict):
        raise ValueError("request must be an object")
    op = request.get("op")
    if op == "status":
        try:
            guard.check()
        except ValueError:
            pass
        return {"ok": True, "armed": guard.reason is None, "reason": guard.reason,
                "tty": guard.tty, "uid": os.getuid()}
    if op == "tap":
        sequence = [chord(request.get("keys"))]
    elif op == "text":
        sequence = text_chords(request.get("text"))
    else:
        raise ValueError("supported operations: status, tap, text")
    # Validate the complete request before injecting anything.
    for names in sequence:
        # Detect an abandoned client between taps and individual presses.
        class RequestGuard:
            def check(self):
                guard.check()
                if select.select([connection], [], [], 0)[0]:
                    raise ValueError("client disconnected or sent additional data")
        keyboard.tap(names, RequestGuard())
    return {"ok": True, "taps": len(sequence)}


def serve(args):
    if os.geteuid() != 0:
        raise ValueError("start serve using sudo in a separate SSH session")
    account = pwd.getpwnam(args.user)
    if account.pw_uid == 0:
        raise ValueError("--user must be a non-root account")
    if not re.fullmatch(r"tty[1-9][0-9]*", args.tty):
        raise ValueError("--tty must name a text VT")
    pid = args.pid
    if pid is None:
        result = subprocess.run(["/usr/bin/systemctl", "show", "bcon@" + args.tty + ".service",
                                 "--property=MainPID", "--value"], check=True,
                                capture_output=True, text=True, timeout=5,
                                env={"PATH": "/usr/bin:/bin", "LC_ALL": "C"})
        pid = int(result.stdout.strip())
    if pid <= 1:
        raise ValueError("no running target bcon process")
    pidfd = os.pidfd_open(pid)
    try:
        # Pin identity before dropping privileges; root-owned bcon /proc entries
        # are not readable by the client user. pidfd prevents PID-reuse confusion.
        executable = os.readlink(f"/proc/{pid}/exe").removesuffix(" (deleted)")
        fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
        if Path(executable).name != "bcon" or int(fields[4]) != os.stat("/dev/" + args.tty).st_rdev:
            raise ValueError("target must be bcon with the specified controlling VT")
        guard = Guard(args.tty, pidfd)
        guard.check()
        fd = os.open("/dev/uinput", os.O_RDWR | os.O_NONBLOCK | os.O_CLOEXEC)
        try:
            # Retain only the already-open uinput and pidfd. Never serve as root.
            os.setgroups([])
            os.setgid(account.pw_gid)
            os.setuid(account.pw_uid)
            os.umask(0o077)
            if os.getuid() != account.pw_uid or os.geteuid() != account.pw_uid:
                raise RuntimeError("privilege drop failed")
            run_server(args.socket or default_socket(account.pw_uid), account.pw_uid,
                       fd, guard, pid)
        finally:
            os.close(fd)
    finally:
        os.close(pidfd)


def run_server(path, uid, fd, guard, pid):
    listener = socket.socket(socket.AF_UNIX, socket.SOCK_SEQPACKET)
    bound = False
    keyboard = None
    try:
        # No unlink-before-bind: never replace another helper or follow a stale
        # socket/symlink. Filesystem work takes place after dropping root.
        listener.bind(path)
        bound = True
        identity = os.lstat(path)
        os.chmod(path, 0o600)
        listener.listen(1)
        listener.settimeout(0.2)
        keyboard = Keyboard(fd)
        # Let libinput discover the hotplugged keyboard; first real capture is
        # still required to demonstrate that the target consumed the events.
        time.sleep(2)
        guard.check()
        print(json.dumps({"ready": True, "socket": path, "uid": os.getuid(),
                          "tty": guard.tty, "bcon_pid": pid}), flush=True)
        while True:
            try:
                guard.check()
            except ValueError:
                pass  # Latch disarmed state; allow status requests only.
            try:
                connection, _ = listener.accept()
            except socket.timeout:
                continue
            with connection:
                connection.settimeout(2)
                fatal = None
                try:
                    response = handle(connection, uid, keyboard, guard)
                except (ValueError, UnicodeError, OSError) as error:
                    response = {"ok": False, "error": str(error),
                                "note": "input may be partial; do not retry automatically"}
                    if isinstance(error, OSError) and not isinstance(error, socket.timeout):
                        fatal = error
                try:
                    connection.sendall(json.dumps(response).encode())
                except OSError:
                    pass
                if fatal is not None:
                    # A failed release cannot leave the virtual device alive.
                    # Destroy it and require an explicit new helper session.
                    raise fatal
    finally:
        try:
            if keyboard is not None:
                keyboard.close()
        finally:
            listener.close()
            if bound:
                try:
                    current = os.lstat(path)
                    if current.st_ino == identity.st_ino and current.st_dev == identity.st_dev:
                        os.unlink(path)
                except FileNotFoundError:
                    pass


def default_socket(uid=None):
    return f"/tmp/bcon-input-{os.getuid() if uid is None else uid}.sock"


def client(path, request):
    info = os.lstat(path)
    if not stat.S_ISSOCK(info.st_mode) or info.st_uid != os.getuid() or stat.S_IMODE(info.st_mode) != 0o600:
        raise ValueError("expected a mode-0600 socket owned by the current user")
    with socket.socket(socket.AF_UNIX, socket.SOCK_SEQPACKET) as connection:
        connection.settimeout(20)
        connection.connect(path)
        if peer_uid(connection) != os.getuid():
            raise ValueError("helper UID does not match client UID")
        connection.sendall(json.dumps(request).encode())
        return json.loads(connection.recv(MAX_PACKET))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--socket", help="default: /tmp/bcon-input-UID.sock")
    commands = parser.add_subparsers(dest="command", required=True)
    server = commands.add_parser("serve", help="run in the foreground via sudo")
    server.add_argument("--user", required=True)
    server.add_argument("--tty", default="tty2")
    server.add_argument("--pid", type=int, help="manual bcon PID; default: service MainPID")
    commands.add_parser("status")
    tap = commands.add_parser("tap")
    tap.add_argument("keys", help="e.g. CTRL+SHIFT+ENTER or PRINTSCREEN")
    text = commands.add_parser("text", help="US layout ASCII; does not append Enter")
    text.add_argument("text")
    commands.add_parser("screenshot", help="tap PrintScreen; does not verify PNG creation")
    args = parser.parse_args()
    if args.command == "serve":
        def stop(signum, frame):
            raise KeyboardInterrupt
        for signum in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
            signal.signal(signum, stop)
        serve(args)
        return 0
    request = {"op": args.command}
    if args.command == "tap":
        chord(args.keys)
        request["keys"] = args.keys
    elif args.command == "text":
        text_chords(args.text)
        request["text"] = args.text
    elif args.command == "screenshot":
        request = {"op": "tap", "keys": "PRINTSCREEN"}
    response = client(args.socket or default_socket(), request)
    print(json.dumps(response, ensure_ascii=False))
    return 0 if response.get("ok") else 1


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        print("Input helper stopped.", file=sys.stderr)
        sys.exit(0)
    except (ValueError, KeyError, OSError, subprocess.SubprocessError) as error:
        print(f"Error: {error}", file=sys.stderr)
        sys.exit(1)
