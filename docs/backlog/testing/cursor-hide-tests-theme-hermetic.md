---
title: "cursor_hide tests read the host's XCURSOR theme instead of the built-in arrow"
status: "open"
area: "testing"
priority: "low"
blocked: null
---

# cursor_hide tests read the host's XCURSOR theme instead of the built-in arrow

Filed 2026-09-29 (PR #332 review). Serves **daily-drive** (a contributor's clean `nextest` on their own desktop).

## The gap

The three `cursor_hide::*` tests sample one pixel of the built-in fallback arrow. On a host with an Adwaita cursor theme reachable through `XCURSOR_PATH`/`XCURSOR_THEME` (the Asahi box, `~/.icons/Adwaita`), the themed arrow is transparent at that pixel and the tests read the background `[60,60,60]`. They fail on `main` there and pass with `env -u XCURSOR_THEME XCURSOR_PATH=<empty dir>` (19/19). Not aarch64-specific; found in the PR #332 review.

## What to do

Pin the theme inside the fixture (unset `XCURSOR_THEME`, point `XCURSOR_PATH` at an empty directory) so the built-in arrow is what is sampled, whatever the host has.

## Not in this ticket

Any product change: this is test hygiene only.
