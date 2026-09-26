#!/usr/bin/env python3
"""Read-only inventory for real-VT automation; never injects input or uses sudo."""

import argparse
import datetime
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess


def command(argv):
    try:
        result = subprocess.run(argv, capture_output=True, text=True, timeout=10)
        return {"argv": argv, "exit_code": result.returncode,
                "stdout": result.stdout.strip(), "stderr": result.stderr.strip()}
    except (OSError, subprocess.TimeoutExpired) as error:
        return {"argv": argv, "error": str(error)}


def read(path):
    try:
        return Path(path).read_text().strip()
    except OSError as error:
        return {"error": str(error)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tty", default="tty2")
    args = parser.parse_args()
    if not re.fullmatch(r"tty[1-9][0-9]*", args.tty):
        parser.error("--tty must be a text VT, e.g. tty2")
    service = "bcon@" + args.tty + ".service"
    state = command(["systemctl", "show", service,
                     "--property=ActiveState,SubState,MainPID,NRestarts,Result,"
                     "NeedDaemonReload,ExecStart,ExecMainStartTimestamp"])
    props = dict(line.split("=", 1) for line in state.get("stdout", "").splitlines()
                 if "=" in line)
    pid = props.get("MainPID", "0")
    executable = None
    if pid.isdigit() and pid != "0":
        try:
            executable = os.readlink("/proc/" + pid + "/exe")
        except OSError as error:
            executable = {"error": str(error)}
    warnings = []
    if props.get("NeedDaemonReload") == "yes":
        warnings.append("systemd's loaded unit differs from disk; reconcile before testing")
    if props.get("ActiveState") != "active":
        warnings.append("target bcon service is not confirmed active")
    active_vt = read("/sys/class/tty/tty0/active")
    if active_vt != args.tty:
        warnings.append("target VT is not confirmed foreground; do not inject keys")
    writable = os.access("/dev/uinput", os.W_OK)
    if not writable:
        warnings.append("current user cannot write /dev/uinput; input setup is required")
    report = {
        "captured_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "platform": platform.platform(), "uid": os.getuid(),
        "virtualization": command(["systemd-detect-virt"]),
        "target_service": service, "active_vt": active_vt,
        "service": state, "running_executable": executable,
        "uinput": {"exists": Path("/dev/uinput").exists(), "writable": writable},
        "drm_devices": sorted(str(p) for p in Path("/dev/dri").glob("*")),
        "tools": {name: shutil.which(name) for name in
                  ("bcon", "python3", "cc", "nvim", "vim", "hx", "yazi",
                   "btop", "fzf", "lazygit", "tmux", "img2sixel")},
        "warnings": warnings,
        "notes": ["Inventory only: no application compatibility result.",
                  "Tool paths do not establish versions or the running bcon binary.",
                  "NRestarts is a baseline counter, not a diagnosis.",
                  "No keys sent, screenshots captured, or services changed."],
    }
    print(json.dumps(report, indent=2, ensure_ascii=False))


if __name__ == "__main__":
    main()
