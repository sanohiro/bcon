# Real-VT application testing

Status: environment probe and temporary keyboard bridge implemented. The full
application runner and screenshot collection are not implemented yet.
This infrastructure lives outside bcon's renderer. The goal is repeatable
application checks using real input and rendered frames on Linux, either in a
Parallels VM or on physical hardware.

## Operating model

The user connects to the Linux test machine over SSH and starts Codex there.
Codex runs the controller locally through that SSH session. The tested apps
run inside bcon on a real foreground VT on the same machine. Screenshots and
results are local files that Codex can inspect. No macOS automation or Parallels
API is required for the initial workflow.

The intended loop is: inspect state, launch a fixture scenario in bcon, send
keys, capture the rendered frame, inspect it, and record a verdict with evidence.
This loop is not implemented yet. SSH PTY output alone is not visual evidence
about bcon. Headless machines without a usable DRM display need separate setup.

Initial setup may require the user to prepare a reserved bcon session and grant
the input helper appropriate access. Routine runs should then need no manual
key presses or visual judgments from the user. The controller must stop if the
test session is no longer reserved or the expected application has exited.

## Inventory

Run from the agent's shell or over SSH, outside the tested terminal:

```sh
python3 tests/automation/probe.py --tty tty2 > /tmp/bcon-test-environment.json
```

Python 3 standard library only. This is read-only: no sudo, key injection,
package installation, service restart, or VT switch. Missing tools and access
denials are reported. It does not certify that the environment is ready.

## Temporary input helper

Requires Linux with pidfd support (kernel 5.3+), Python 3.9+, libevdev.so.2
(Ubuntu/Debian package `libevdev2`), and /dev/uinput. No build step is needed.
Reserve the foreground bcon session exclusively and log in as the test user.
Do not type concurrently or use this against a root shell.

In a separate SSH session, from the repository root:

```sh
sudo /usr/bin/python3 -I -B tests/automation/input_helper.py serve --user hiro --tty tty2
```

For a command usable from any directory, install the shell launcher once as
your regular user (from the repository root):

```sh
mkdir -p ~/.local/bin
ln -s "$(pwd)/tests/automation/bcon-test-input" ~/.local/bin/bcon-test-input
```

Ensure `~/.local/bin` is on PATH. The launcher resolves its symlink to locate
the Python helper, so keep this checkout available. On the current development
machine it is already installed for `hiro` and that directory is on PATH.

```sh
bcon-test-input                    # sudo prompt, current user, tty2
bcon-test-input start --tty tty3   # choose another VT
```

Client commands need no sudo: `bcon-test-input status`,
`bcon-test-input screenshot`, and `bcon-test-input tap CTRL+SHIFT+ENTER`.
Stop the foreground helper with Ctrl+C. `bcon-test-input --help` lists usage.

Replace `hiro` with the actual test account. `-I` isolates Python's import path
and environment. The helper looks up the MainPID of `bcon@tty2.service`; for a
manually launched bcon use `--pid PID`. The process must have tty2 as its
controlling terminal and tty2 must be foreground. It does not switch VTs,
log in, restart services, or reload systemd units.

The helper opens uinput, pins the target process with a pidfd, clears supplementary
groups, and drops to the specified user's UID/GID before creating the virtual
keyboard and listening. Wait for the JSON `"ready": true` message. It waits two
seconds for device hotplug; this does not guarantee bcon received the device.

In the Codex SSH session, as that same user:

```sh
python3 -I -B tests/automation/input_helper.py status
python3 -I -B tests/automation/input_helper.py screenshot
```

`screenshot` taps PrintScreen. Its successful response confirms event submission,
not PNG creation or display correctness. Check the configured screenshot binding
and directory, then inspect the new image separately. Do not assume a default
key binding if bcon's configuration was customized.

After establishing the expected screen, examples of deliberate test operations:

```sh
# Default split-right binding (confirm the actual configuration first).
python3 -I -B tests/automation/input_helper.py tap CTRL+SHIFT+ENTER
# Type without submitting; text requires US layout, CapsLock off and IME off.
python3 -I -B tests/automation/input_helper.py text 'echo bcon-test'
python3 -I -B tests/automation/input_helper.py tap ENTER
```

Text accepts up to 256 printable ASCII characters; it does not append Enter.
Key names include A-Z, 0-9, F1-F12, arrows, HOME, END, PAGEUP, PAGEDOWN,
BACKSPACE, DELETE, TAB, ENTER, ESC, PRINTSCREEN, and keypad keys. Modifiers
are CTRL, SHIFT, ALT, and SUPER. Each request is a complete tap, with modifiers
released in reverse order; persistent key-down requests are not supported.

The socket defaults to `/tmp/bcon-input-UID.sock`, mode 0600. Both ends verify
peer UID using SO_PEERCRED. An alternate path may be set with `--socket PATH`
**before** the subcommand, on both server and client. Existing paths are never
overwritten. If SIGKILL leaves a stale socket, first confirm no helper remains,
then remove only that socket as its owner before restarting.

The helper latches a disabled state if it observes a VT change or target-process
exit. Start a new helper to rearm. Requests are bounded; disconnects cancel
remaining taps, and I/O errors terminate the helper and destroy its device.
Ctrl+C, SIGTERM, or SSH hangup also clean up. A failed/aborted request may have
partially typed text: inspect the screen rather than retrying automatically.
VT-switch, Ctrl+Alt+Delete, and Alt+PrintScreen chords are rejected.

Input is seat-wide, not directed to a particular process. Foreground checks
reduce accidental input but cannot make key delivery atomic with VT changes or
concurrent physical keyboard activity. Do not use other sessions on the same
seat during a test. The helper cannot detect whether an app has exited to a shell;
that belongs to the scenario controller. Ctrl+C in the helper SSH session is the
manual stop mechanism.

To run protocol/lifecycle tests without root or real key injection:

```sh
python3 -B -m unittest discover -s tests/automation -p 'test_*.py' -v
```

These tests use a fake keyboard and real Unix sockets. They do not validate
privilege dropping, libinput hotplug, actual key delivery, or PNG capture.

## Proposed runner

1. Reserve a test VM/session and fixture directory. Fix display dimensions,
   font, locale, keyboard layout, and bcon configuration. Record actual bcon
   and application versions, not just executable paths.
2. Run a user-owned test worker inside bcon. It launches only the chosen scenario
   with a timeout, isolated application configuration, and disposable files.
   Login remains an explicit setup step; do not type passwords automatically.
3. From the controlling shell, send a bounded sequence of virtual-keyboard
   events through uinput. The repository's keypad check is an existing example.
   Verify the expected foreground VT and bcon process before input, release all
   held keys on cancellation, and stop on unexpected state. VT checks cannot
   eliminate races with concurrent human input: reserve the session exclusively.
4. Trigger bcon's screenshot key and wait for a new, complete PNG. Copy it to a
   unique scenario artifact immediately. Existing screenshot names have only
   second precision; serialize captures to avoid collisions. Confirm configured
   screenshot key/path rather than assuming defaults.
5. Check expected application behavior, inspect images, and record evidence.
   Save before/after service counters and relevant logs; report timeouts, missing
   dependencies, and uncertain visual results separately from passes.
6. End the scenario, verify terminal restoration, and remove the virtual input
   device. Do not automatically restart the user's normal bcon service.

The input helper uses the temporary sudo startup above; do not make /dev/uinput
world-writable or run test applications as root. A dedicated VM is preferable;
a reserved test window in the existing VM is an alternative.

## Visual verdicts

### Composed-frame screenshot regression

On a reserved bcon session running the revised binary:

1. Capture a single-pane shell. Check visible terminal text and the terminal
   cursor (take multiple captures if it is blinking).
2. Open a second tab, split it horizontally, and type different labels in each
   pane. Capture after input settles. Expect both labels, the pane divider and
   the bottom tab bar with the active tab indicated.
3. Move focus, zoom, then unzoom. Capture each settled state and check the cursor
   and tab indicator against the active pane/tab. Treat changed text visibility
   on resize as a separate finding, not automatically a screenshot bug.
4. Capture a visible notification/progress overlay when a fixture is available.
   Expect the overlay in the PNG. Wait for the previous screenshot success flash
   to expire between captures; a screenshot must not contain its own success flash.
5. Close only the test tab and confirm return to the original session. Compare
   service PID/restart counters before and after.

Before the fix, capture ran against the content FBO before screen composition:
tab bars, terminal cursors and later overlays were absent. Capture now belongs
after composition and before buffer swap, within the presentation guard. Requests
remain pending while presentation is deferred by synchronized updates or VT state.
The hardware mouse cursor is a separate DRM plane and is not captured. A PNG
still cannot prove successful scanout, absence of flicker, or VT recovery.

Actual post-fix display verification requires deploying the revised binary;
unit tests and a successful build alone do not establish this regression pass.

- Start with explicit assertions: text present, selection moves, borders align,
  CJK/emoji do not overlap, scrolling clears old content, and exit restores the
  terminal. Store the action sequence and expected result with each image.
- Agent image review can find visible defects but can also miss them. Preserve
  images and reasons; use `needs-review` for ambiguous results.
- Establish reviewed baselines before using image differences. Mask dynamic
  regions such as clocks/cursors; a matching baseline alone does not prove
  correctness, and different terminals need not render identical pixels.
- With the screenshot timing fix, bcon screenshots sample the composed default
  framebuffer before buffer swap. They do not independently prove successful
  display scanout or absence of flicker. Host-side VM captures or
  video are a later complementary check, especially for VT recovery.
- Publish results as "tested version/scenario/environment", not blanket support
  for every feature of an application or every GPU.

## First scenarios

Begin with one deterministic input/capture round trip. Then add fzf selection
with a fixed list, Neovim editing/search of a disposable Unicode fixture, and
yazi navigation/preview of a fixture directory. Add btop and lazygit after this
works; their dynamic content needs tailored assertions. These are candidates,
not compatibility claims. Do not install/update applications implicitly.

## Inspection on 2026-09-26

- Parallels, Linux aarch64; bcon@tty2 active with tty2 foreground.
- /dev/uinput exists but is not writable by the agent user.
- bcon has PNG screenshots and existing uinput tests; no new product control API
  is needed for the initial prototype.
- systemd reports NeedDaemonReload=yes. Confirm the intended unit/binary before
  a run; do not silently reload/restart the active session.
- NRestarts=2 at inspection is only a baseline, not evidence of a new crash.
- Found nvim, vim, yazi, btop, fzf, lazygit, and tmux executable paths; their
  versions and compatibility have not been verified by this inventory.

## Live verification on 2026-09-26

The composed-frame fix was deployed and tested through the input helper at
1024x936, rotation 0 on the Parallels VM. Captures contain the terminal cursor,
tab bar, divider and an OSC 9;4 progress overlay (42%). Focus changes, zoom/unzoom
and test-tab cleanup were exercised. Service PID stayed 20247 with zero restarts.
PNG pixel checks confirmed the tab background and divider in both focus states.
Local evidence: ~/.local/state/bcon-test/screenshots/composed-frame-regression.md.
Hardware cursor planes, rotation, sync-update deferral and VT recovery are not
covered by this check. Text visibility changes on resize remain a separate
review candidate; this change does not claim to fix reflow.
