---
title: "restore test a_restore_on_outputs_of_two_sizes_decodes_once_on_sway captured an all-black frame once"
status: "open"
area: "scootbg"
priority: "low"
blocked: null
---

# restore test a_restore_on_outputs_of_two_sizes_decodes_once_on_sway captured an all-black frame once

Filed 2026-10-21. Serves **daily-drive**: a flaky test erodes trust in the suite that guards the wallpaper daemon.

## The gap

The screenshot helper takes one shot after a fixed wait, so on a slow runner it can capture before sway presents the restored frame.

## What to do

Poll the screenshot until the flat color shows or a deadline passes (in `tests/common/shots.rs`), so a slow runner waits and a real failure still fails.

## Not in this ticket

Anything in scootbg itself: the daemon painted correctly on the re-run.

## Sighting

First sighting 2026-10-21 on PR #701 (a site-only vite bump, so no scootbg
change), job "scootbg on headless scoot", run 39667263669: the screenshot
helper (`tests/common/shots.rs:81`) saw all nine samples `[0, 0, 0]` ("not one
flat color") on `a_restore_on_outputs_of_two_sizes_decodes_once_on_sway`; the
other 81 tests passed and the re-run of the job was green. Reads as a capture
taken before sway presented the restored frame. Next step if it recurs: poll
the screenshot until the flat color shows or a deadline passes, instead of one
shot after a fixed wait.
