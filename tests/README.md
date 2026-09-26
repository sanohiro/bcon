# Interactive checks

For SSH-driven application testing, see [automation/README.md](automation/README.md).
It includes a read-only environment probe and a temporary sudo-started keyboard
helper for use from a separate SSH session.

Run these inside bcon on a text VT, including a session started through
`bcon@tty2.service`. A terminal window or SSH session alone cannot verify bcon's
rendering or local keyboard handling. Stop a test before switching VTs when it
injects virtual keyboard events.

## Rendering smoke check

```bash
bash tests/visual-smoke.sh
```

Requires Bash and Python 3. `img2sixel` is optional; its absence records a skip.
Ten pages cover text/emoji, colors/attributes, box drawing and the right edge,
scrolling, Sixel, Kitty chunked/file transfers, partial updates, alternate screen
restoration, and cursor/VT restoration. Enter records OK, `n` records NG and a
note, `s` skips, and `q` stops. Results are saved to `~/bcon-visual-*.log`.
Screenshots record static results; watch the animations and actually switch
away/back on the final page to check flicker and VT restoration.

## Rotation and pointer coordinates

Set `[display] rotation` to each of `0`, `90`, `180`, and `270`, restart bcon,
and run the rendering check followed by:

```bash
python3 tests/rotation-mouse-check.py
```

Click the five highlighted targets. The visible pointer and reported click
should agree, including near the edges. `q` or Ctrl+C stops the check.

## Keypad navigation

```bash
cc -O2 -Wall -Wextra -o /tmp/bcon-keypad-check tests/keypad-uinput-check.c
sudo /tmp/bcon-keypad-check
```

Requires Linux `/dev/uinput` and libinput device hotplug. It creates a temporary
virtual keypad and checks the resulting bytes in the current bcon PTY. Run at
the shell prompt, with no other keys held; it expects normal cursor-key mode.

## Bounded stress checks

```bash
python3 tests/rootless-stress.py 20
cc -O2 -Wall -Wextra -o /tmp/bcon-input-stress tests/input-stress-uinput.c
sudo /tmp/bcon-input-stress 10
```

Run the commands separately. Durations are in minutes. Despite its filename,
the rendering stress script also works in systemd-launched bcon. It writes
`~/bcon-stress-*.log`; virtual-keyboard stress writes `/var/tmp/bcon-input-stress-*`.
Both accept `q` or Ctrl+C to stop. A completed stress run without a reproduction
does not by itself prove an intermittent issue is fixed.
