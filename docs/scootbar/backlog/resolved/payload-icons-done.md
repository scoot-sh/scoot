---
title: "An icon on exec and push modules"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M6"
resolved: "2026-10-04"
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

## Resolution

Landed in PR #421. `[exec.NAME]` and `[push.NAME]` take `icon`,
`icon-path` (with `icon-viewbox`), `icon-image` and `show-text`, drawn
before the text through the shared `Shown::write_with`; the refusals name
the dotted key like every other icon key. The payload takes an optional
`icon` (one glyph, the static key's rule) that overrides the static icon
for that update; a text line carries none and clears the last one. The
payload stays version 1: `from_value` reads only the keys it knows, so an
older bar shows an `icon`-carrying update as if the key were absent
(pinned by `an_older_version_1_parser_ignores_the_icon_key`). The music
desk and vinyl sunset examples set the icon in `bar.toml` and their
scripts print the value alone, now plain `sh`.

Evidence: each new behavior test failed with the change reverted;
`cargo nextest run -p scootbar` 1329 passed; clippy clean across 12
feature combinations (two of which, push-only and exec-only, failed to
build before the `config.rs` gate fix). The review rendered the music
desk bar on the Asahi M2 from `main` and from the branch with fixed
values: pixel-identical (RMSE 0). Size: `.text` +6,528 B (+0.38%), the
file one 64 KiB page; waived by the maintainer
([lightest.md](../lightest.md#m6-icons-on-exec-and-push-measured-2026-10-04)).
