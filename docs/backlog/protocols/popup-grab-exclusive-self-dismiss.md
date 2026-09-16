---
title: "An `exclusive` layer surface's own popup grab is refused and its menu dismissed by the very surface that opened it."
status: "open"
area: "protocols"
priority: "low"
blocked: null
---

# An `exclusive` layer surface's own popup grab is refused and its menu dismissed by the very surface that opened it.

Found by independent review of `docs/backlog/resolved/xdg-popup-input-resolved.md`
(PR #44). Not a crash, and the precedence rule it violates is otherwise
correctly enforced — this is the one case the rule's own doc overclaimed.

`popup.rs`'s `popup_grab_outranked()` refuses a new grab, and `shell.rs`'s
`refresh_keyboard_focus` pre-empts an existing one, whenever
`layer_keyboard_focus()` reports `exclusive: true` — without checking
whether the exclusive surface *is* the grab's own root. So a
`gtk4-layer-shell` launcher on `overlay`/`top` with `keyboard_interactivity:
exclusive` that opens its own dropdown (a settings menu, an emoji picker)
gets refused immediately: `grab_popup` sees its own exclusive surface
outranking it and sends `popup_done` on the spot, so the dropdown flashes
open and instantly closes — dismissed by the surface that opened it, not by
something else winning.

This is exactly the failure `popup.rs`'s module doc and `README.md`'s
"Popup menus" section both claim rule 3 prevents ("a bar's own dropdown is
not dismissed by the bar that opened it") — true for a click-focused
`on_demand` layer surface, false for an `exclusive` one, since the two share
one check today.

Low priority: neither DMS nor Noctalia (the two Quickshell shells probed
against flexwm) route any menu through `xdg_popup` — both use layer
surfaces for their own dropdowns — so this has not been field-hit yet, and
`exclusive` layer-shell clients with their own popups are uncommon (most
exclusive surfaces are launchers, which tend to use their own internal
list-navigation UI rather than a real `xdg_popup`).

What it would take: `popup_grab_outranked()` (and the corresponding
pre-emption check in `shell.rs`) need the grab's *root* surface, not just
whether something else is exclusive — `find_popup_root_surface` already
computes this in `grab_popup`. One clause: don't outrank when the exclusive
surface *is* the root the grab belongs to.
