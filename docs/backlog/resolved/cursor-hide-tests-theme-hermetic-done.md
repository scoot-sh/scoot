---
title: "cursor_hide tests read the host's XCURSOR theme instead of the built-in arrow"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-03"
---

# cursor_hide tests read the host's XCURSOR theme instead of the built-in arrow

Filed 2026-09-29 (PR #332 review). Serves **daily-drive** (a contributor's clean `nextest` on their own desktop).

## The gap

The three `cursor_hide::*` tests sample one pixel of the built-in fallback arrow. On a host with an Adwaita cursor theme reachable through `XCURSOR_PATH`/`XCURSOR_THEME` (the Asahi box, `~/.icons/Adwaita`), the themed arrow is transparent at that pixel and the tests read the background `[60,60,60]`. They fail on `main` there and pass with `env -u XCURSOR_THEME XCURSOR_PATH=<empty dir>` (19/19). Not aarch64-specific; found in the PR #332 review.

## What to do

Pin the theme inside the fixture (unset `XCURSOR_THEME`, point `XCURSOR_PATH` at an empty directory) so the built-in arrow is what is sampled, whatever the host has.

## Not in this ticket

Any product change: this is test hygiene only.

## Resolution (2026-10-03, PR #394)

Landed as `test(scoot): pin cursor_hide fixtures to the built-in arrow` (`0ccd9963`). Fixture passes `cursor_theme = "scoot-test-no-such-theme"` by value (the `cursor::tests::NO_THEME` pattern) instead of env-pinning, so parallel `cargo test` threads cannot race. Reviewer reproduced the 3 pixel failures on base with a hostile transparent theme and verified 22/22 green hostile+clean on the fix, clippy/fmt clean, CI green.
