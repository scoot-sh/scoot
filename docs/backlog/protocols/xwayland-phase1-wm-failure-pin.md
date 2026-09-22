---
title: "XWayland Phase 1 follow-up: pin the WM-attach-failure clear (review recipe from PR #221)"
status: "open"
area: "protocols"
priority: "low"
blocked: null
---

# XWayland Phase 1 follow-up: pin the WM-attach-failure clear

Filed from the `scoot-reviewer` delta report on PR #221 (Phase 1
skeleton, merge `4af4223`), which landed with a clean gate. One test
gap, with a concrete deterministic recipe — not a behavior doubt (the
two-line clear is verified correct by direct reading; adjacent pins
bound both sides).

## The gap

`start_wm` failure clears `xdisplay` (`xwayland/mod.rs`, WM-attach-failure
arm), but no test pins it: the failure is uninducible with the current
harness, and the adjacent pins cover only fallback-asserts-`None`
(`tests.rs:355`) and live-asserts-`Some` (`tests.rs:376,382` — live-only,
needs the binary on `PATH`).

## The recipe (from the review, verified against pinned Smithay `0ff0098`)

`start_wm` does real X roundtrips ending in
`change_window_attributes(SUBSTRUCTURE_REDIRECT)` (`xwm/mod.rs:804+`),
with errors surfacing at `flush()`/subsequent `reply()`s. A rival
claimant deterministically breaks it: the display number is known
synchronously from `start()`'s return, and dispatch only happens on the
test thread's `settle()` — so a test can `x11rb::connect`
(retry-to-bind loop under the existing `XWAYLAND_PATIENCE` pattern) and
claim `SUBSTRUCTURE_REDIRECT` on the root *before the first settle
dispatches `READY`*. Zero raciness. `start_wm` then fails `Access`
deterministically; assert `xdisplay`/`xwm` are `None` plus
`"withdrawing DISPLAY"` via the file's existing `capture_logs`. Needs
the binary on `PATH` (live-only, like its neighbours) and no machinery
beyond what `tests.rs` already imports.

## What done looks like

The rival-claimant test green on the dev VM (live branch), full gate
green, no production change. Then close this entry to `resolved/`.
