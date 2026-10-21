---
title: "restore test a_restore_on_outputs_of_two_sizes_decodes_once_on_sway captured an all-black frame once"
status: "open"
area: "scootbg"
priority: "low"
blocked: null
---

# restore test a_restore_on_outputs_of_two_sizes_decodes_once_on_sway captured an all-black frame once

Filed 2026-10-21. Serves **daily-drive** / **computer use** (pick one, say why).

## The gap

What is wrong or missing, with evidence (file paths, measured numbers).

## What to do

The proposed shape, and the edge cases to pin.

## Not in this ticket

What is deliberately out of scope.

First sighting 2026-10-21 on PR #701 (a site-only vite bump, so no scootbg
change), job "scootbg on headless scoot", run 39667263669: the screenshot
helper (`tests/common/shots.rs:81`) saw all nine samples `[0, 0, 0]` ("not one
flat color") on `a_restore_on_outputs_of_two_sizes_decodes_once_on_sway`; the
other 81 tests passed and the re-run of the job was green. Reads as a capture
taken before sway presented the restored frame. Next step if it recurs: poll
the screenshot until the flat color shows or a deadline passes, instead of one
shot after a fixed wait.
