---
title: "Many popups open side by side stall the compositor roughly quadratically"
status: "open"
area: "core"
priority: "medium"
blocked: null
---

# Popup creation is roughly quadratic in the number of popups open

Filed 2026-09-23 from the review of PR #226 (measured at `10e6b53`;
pre-existing -- `main` and the PR measured the same). Not a crash: the
popup depth bound keeps every tree shallow. But a client that opens
thousands of popups *side by side* (all within the depth cap) stalls the
compositor, and every other client with it, for as long as it takes.

## Measured (release, dev VM)

| popups open, side by side | stall |
|---|---|
| 1954 | 0.73 s |
| 5104 | 5.4 s |

About 2.6x the popups for 7.4x the time: roughly quadratic.

## Likely causes (unverified -- measure before fixing)

- `PopupTree::insert` / `PopupNode::try_insert` search the whole tree for
  the parent's node on every popup tracked (O(n) per popup);
- `PopupManager::find_popup`, which `send_popup_initial_configure` in
  `handlers.rs` calls on every popup commit, scans every tracked popup
  (`iter_popups` over every tree, collecting into a `Vec`);
- Smithay's own `xdg_popup` destructor finds the popup in `known_popups`
  by linear search.

## What a fix needs

A profile naming the actual hot spot, then either a per-client popup
count limit (a protocol error past a generous number, the shape of the
depth cap) or a lookup that is not a scan -- whichever the measurement
supports. Serves daily-drivability (a misbehaving app should not freeze
the desktop) more than computer use.
