---
title: "Keyboard navigation of popup lists (arrows, Enter)"
status: "open"
area: "scootbar"
priority: "low"
blocked: null
milestone: "M6"
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
