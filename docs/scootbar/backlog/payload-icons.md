---
title: "An icon on exec and push modules"
status: "open"
area: "scootbar"
priority: "low"
blocked: null
milestone: "M6"
---

# An icon on exec and push modules

Filed 2026-10-03, from a showcase shoot on the maintainer's Asahi M2 that
recreated their own desktop. Serves **daily-drive** (and looks): after
[module-icons](resolved/module-icons-done.md) every built-in module takes
an `icon`, but `exec` and `push` refuse it (`unknown field icon`), so a
load or CPU readout made with `exec` has to print its glyph in its own
text, and the update payload has no `icon` either.

## What to do

- `[exec.NAME] icon` and `[push.NAME] icon` (one glyph, `icon-path`,
  `icon-image`, as the other modules), drawn before the text.
- An optional `icon` key in the update payload (one glyph), so a script
  can change it per update (a CPU that turns hot, a VPN that drops):
  version 1 ignores unknown keys, so adding it is compatible; say what an
  older bar does with it.
- `show-text = false` as the other modules have it.

## Not in this ticket

Per-level arrays for a payload value (the script picks the glyph).
