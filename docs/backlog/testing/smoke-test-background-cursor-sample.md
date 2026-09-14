---
title: "`scripts/smoke-test.sh`'s background-colour check samples the cursor under `MODE=--tty`."
status: "open"
area: "testing"
priority: "low"
blocked: null
---

# `scripts/smoke-test.sh`'s background-colour check samples the cursor under `MODE=--tty`.

`scripts/smoke-test.sh`'s background-colour check samples the cursor
under `MODE=--tty`. The check reads the pixel at (3,3) and expects the
configured background; under `--tty` the pointer starts at (0,0) and the
built-in arrow cursor is drawn there, so it reads the arrow's black
outline and the script ends in `BUG: one or more decoration pixel checks
failed`. Pre-existing and backend-specific (`--headless`/`--nested` draw
no cursor, and both pass), confirmed identical on `e2c7971` and on item
15's branch — a flaw in the test's sample point, not in the compositor: a
pixel map of that corner shows the 16x16 arrow. Fix is a line: sample
somewhere the cursor isn't, or move the pointer over IPC before capturing.
Left out of item 15 to keep that diff to its own ticket.
