# Agent Handoff

Status: Active — review complete; v1.5.0 merge and release pending.

## Goal

Merge the reviewed display rotation (#17), NumLock-off keypad navigation (#18),
and related rendering/config fixes. The user authorized merging and necessary
documentation updates. The user also explicitly authorized a release. Use v1.5.0 because rotation adds
a feature. All commit, PR, release, and issue messages must be English.
Leave issue #19 unchanged.
After release, reply to #17 and #18 in English, briefly apologizing for the slow reply
(as requested by the user). Do not send those replies before release.

## Completed

- Reviewed all application, documentation, and test-tool changes.
- Fixed review findings: relative-path config watch filtering, grayscale mode
  selection and R8-to-RGB atlas upload, and out-of-bounds absolute pointer input
  after rotation. Added regression coverage.
- Fixed final-target handling in the interactive pointer check and NumLock
  restoration in the keypad check.
- Updated English/Japanese configuration, keybind, and installation docs;
  added an Unreleased changelog section and tests/README.md.
- Earlier systemd visual smoke run: 12 screenshots showed correct text, colors,
  Sixel, both Kitty transfers, partial update cleanup, and alternate-screen
  restoration. Service had zero restarts during that run. This predates the
  additional code-review fixes; screenshots do not verify all rotation angles
  or VT switching animations.

## Verification

- `cargo test --locked`: 65 passed, 1 hardware-dependent seatd test ignored.
- `cargo test --locked --no-default-features`: passed.
- `cargo build --release --locked`: passed after the review fixes.
- Shell/Python parsing and both C test utilities' strict syntax checks passed.
- Relative-config atomic-save regression failed before the fix and passed after.
- User reports overnight stress did not reproduce issue #19; not proof of a fix.

## Next steps

- Publish the reviewed branch/PR, verify CI, merge, tag v1.5.0, and verify the
  release artifacts. Then reply to issues #17 and #18 in English with a brief
  apology for the slow reply. Update this handoff once complete.
- Update this file after merge so future agents do not repeat stale work.

## Uncommitted files and blockers

- Changes are being prepared on a dedicated review branch. Check git status and
  the PR before resuming. No application restart is needed for GitHub operations.
- The development share is CIFS and reports every file as executable. Preserve
  existing Git modes; ignore mode-only differences when reviewing/staging.
- Privileged deployment requires the user's sudo interaction. The user offered
  to execute commands themselves; never request their password.

- CIFS rejected writing loose Git objects during staging. A local review clone
  is recorded in /tmp/bcon-review-path; it has the same reviewed file contents
  and preserves repository file modes. Use it for publication if needed.
