# Agent Handoff

Status: No active task. Input helper and focused screenshot fix verified locally.

## User direction

- Prioritize stability and focused bug reviews with reproducible tests. Defer
  issues #5/#6; leave #19 unchanged unless instructed otherwise.
- Operate via Codex started over SSH on the Linux test machine, whether Parallels
  or physical hardware. Use actual bcon input/captures for feature/app checks.
- All commit, PR, release and issue messages must be English.
- Never push/tag without an explicit instruction. No new publication requested.

## Completed in this working tree

- tests/automation/probe.py: read-only environment inventory.
- input_helper.py: sudo bootstrap opens uinput and pins the target bcon PID;
  drops UID/GID, then serves bounded keyboard requests on a private Unix socket.
  Peer UID checks, release-on-interruption, VT/process guard, and cleanup.
- bcon-test-input shell launcher, symlinked into /home/hiro/.local/bin; works
  from any directory. No-argument invocation starts helper via sudo for tty2.
- tests/automation/test_input_helper.py: 18 protocol/lifecycle tests passed.
- src/main.rs: moved PNG capture from the content FBO to after screen composition,
  just before buffer swap, within presentation guards. Success flash starts after
  capture. No other product source changed.
- cargo test --locked: 65 passed, 1 hardware-dependent seatd test ignored.
- cargo build --release --locked: passed (CIFS cache hard-link warning in tests).
- User installed binary and restarted bcon: PID 20247, start 12:45:23 JST.
  Installed SHA256 55308c4bf690505d26d0fbc0ab314022360dcc6ec308f0ebc1bc5b97323f41eb.
- Live helper test confirms new PNGs contain tab bar, divider, terminal cursor,
  and 42% progress overlay. Focus, zoom/unzoom and test-tab closure exercised.
  PID stayed 20247 and restart count stayed 0. Original login/shell tab restored.

## Environment and evidence

- Helper remains running in the user's separate SSH session; query status before
  using it and reserve the seat exclusively. It pins the PID, so restart helper
  after bcon restarts. User stops it with Ctrl+C.
- User added [paths] screenshot_dir to /etc/bcon/config.toml. Do not append again.
  Directory /home/hiro/.local/state/bcon-test/screenshots: hiro:hiro, 0700;
  PNG files root:root, 0644, readable by hiro but directory excludes other users.
- Evidence/report: composed-frame-regression.md in that screenshot directory.
  The earlier first-helper-check.md describes the pre-fix run.
- Both focus states have tab-background RGBA (31,31,38,255) at (20,920),
  divider RGBA (76,76,89,255) at (512,300). A fleeting visual suspicion of a
  missing tab bar was disproved by PNG pixel checks; it is not a known defect.

## Limits and possible future work

- CI runs the helper protocol/lifecycle tests and shell syntax check.
- No complete application scenario runner or automatic PNG collector yet.
- Hardware pointer planes are outside screenshots. Rotation, sync-update
  deferral, VT recovery and host-display flicker were not live-tested here.
- Earlier lines move out of view on split/unzoom and reappear on zoom. Observed
  before and after the screenshot fix; separate resize/reflow review candidate.
- Earlier pane-close logs reported SIGHUP/SIGKILL timeouts; not diagnosed here.
- Do not silently expand into rendering refactors or mark these candidates fixed.

## Working tree / publication

- Change includes src/main.rs, tests/README.md, tests/automation/, HANDOFF.md,
  and CI coverage for the input helper. No release or tag is requested.
- CIFS reports all files executable. Use git -c core.filemode=false status/diff
  and preserve tracked modes when staging. The shell launcher needs executable mode.
- Main baseline is v1.5.0, e3751738f4361cea94b081a2ae7b0e3483321d0c.
- Earlier PR #20 merged and v1.5.0 released; artifact checksums verified.
- Issue replies already posted; do not repeat:
  https://github.com/sanohiro/bcon/issues/17#issuecomment-5842086768
  https://github.com/sanohiro/bcon/issues/18#issuecomment-5842088706
