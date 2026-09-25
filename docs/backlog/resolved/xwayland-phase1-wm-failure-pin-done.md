---
title: "XWayland Phase 1 follow-up: pin the WM-attach-failure clear (review recipe from PR #221) — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# XWayland Phase 1 follow-up: pin the WM-attach-failure clear

**RESOLVED (2026-09-25) with XWayland Phases 2+3** -- pinned by
`a_window_manager_that_cannot_attach_withdraws_the_display`
(`compositor/xwayland/tests/mod.rs`), but **not with this ticket's recipe,
which does not hold**, and with one small production change the ticket said
would not be needed.

- **Why the recipe fails.** XWayland accepts *no* X client until the window
  manager owns the `WM_S0` selection -- Smithay's `start_wm` says so in as
  many words ("No X11 clients are accepted before this"), and it is what
  happened live: the rival's `x11rb::connect` never returned, and nextest
  killed the test at its 120 s timeout (dev VM, xwayland build, run log in
  the PR). So a rival can never claim the root before our WM attaches.
  Separately, `start_wm` does not check its own `ChangeWindowAttributes`, so
  a rival that did get in first would not make it fail either.
- **What is pinned instead.** The failure `start_wm` does report is its
  connection failing -- what XWayland dying between `READY` and the attach
  looks like. The test spawns the server the way `start` does, records
  `READY` instead of acting on it, shuts the recorded WM socket down, and
  hands it to `attach_window_manager`; it asserts `xwm` and `xdisplay` are
  `None`, `withdrawing DISPLAY` is logged, and the session still spawns.
- **The production change.** `start`'s `READY` arm is extracted, verbatim,
  into `xwayland::attach_window_manager` so the test can drive it without
  racing a dispatch. Behaviour is identical; the function is what `start`'s
  callback calls.

The original gap and recipe follow, as filed.

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
