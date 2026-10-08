---
title: "Keyboard navigation of popup lists (arrows, Enter)"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M6"
resolved: "2026-10-08"
---

# Keyboard navigation of popup lists (arrows, Enter)

Filed 2026-10-03, from [popup-network-list](resolved/popup-network-list-done.md). Serves
**daily-drive** (picking WiFi without a pointer).

## The gap

[popup-network-list](resolved/popup-network-list-done.md) lands the list with pointer
scroll and selection only: arrows do nothing and Enter selects nothing in
a popup, while the popup grab already gives the popup the keyboard for
Escape (`crates/scootbar/src/daemon/popup/events.rs`, `KEY_ESC`). A list
that needs a pointer is half a picker.

## What to do

- Move the hover/selection with Up/Down (wrapping or clamping: decide),
  following the scroll so the selected row stays visible.
- Activate the selected row with Enter (the button's action, with its
  `closes` flag honored).
- Type-ahead or number shortcuts are out (nothing in the popup reads text).

## Not in this ticket

Anything the list ticket left out besides the keyboard: a password prompt
(the `connect-command` does that), tooltips in popups.

## Resolution (2026-10-08, PR #508)

Landed as filed, with the ticket's open decision taken as **clamping**
(no wrapping; the most conservative option, one branch to reverse):
Up/Down move the hover among the popup's button rows, clamped at the
ends (text and slider rows skipped; Down with no hover takes the first
button, Up the last), the scroll following so the hovered row stays
fully visible; Enter activates the hovered row's action with its
`closes` flag honored, exactly as a release over it would. Keys are the
input-event codes (no keymap, no xkb), press only. Only a popup that
grabbed has a keyboard (an `invoke`-opened popup stays pointer-only, as
with Escape); tooltips never take one. This holds for every button list
(the network list, the power menu, tray menus), not just WiFi.

- **Semantics checked against the pinned scoot source**: the bar's layer
  surface asks `KeyboardInteractivity::None` and never takes focus; the
  keyboard reaches a popup only through its explicit `xdg_popup.grab`
  (`crates/scoot/src/compositor/popup.rs`), which is also why a
  grab-less popup has no keyboard at all.

### Evidence

Unit tests through the popup state machine (each proven to fail without
the fix by reverting `interact.rs`/`events.rs` to `origin/main`:
`E0599 no method named move_selection`, exit 101; restored → green):
hover among buttons with clamping at both ends, scroll-follow down and
back up a 10-row list cut to two visible, Enter with the `closes` flag,
Enter with no hover or a held pointer press doing nothing. The
warm-popup allocation test drives the new paths and still counts 0
allocations. Full verification on the Asahi M2: `cargo fmt --check`
clean; `cargo clippy --all-targets -D warnings` 36/36 across the
feature matrix; `cargo nextest run` 1375 passed / 4 skipped / 13 failed
(all 13 `no sway on PATH` — no sway on the M2, left to CI) and
`cargo test` the same shape with
`SCOOTBAR_REQUIRE_SCOOT=1 SCOOTBAR_REQUIRE_SWAY=1
SCOOTBAR_REQUIRE_DBUS_DAEMON=1`; `cargo deny check` ok; `nix build
.#docs-site` exit 0. Ratchet (release, same toolchain): file +0 B,
`.text` +928 B (+0.05%), loaded sections ≈ +1,088 B total; idle
network-placed 60 s rows level with base on both sides (~30 wakeups
each from this box's live WiFi traffic); no-module rows exactly level
(4400 kB, 0 wakeups, 7 fds, 1 thread). The `.text` row is reported for
the maintainer to waive or not; no waiver claimed.
