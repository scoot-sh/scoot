---
title: "No `xdg_popup` ever receives its initial configure, so no popup maps at all"
status: "open"
area: "protocols"
priority: "medium"
blocked: null
---

# No `xdg_popup` ever receives its initial configure, so no popup maps at all

No `xdg_popup` ever receives its initial configure, so no popup maps at
all (found while implementing item 14; pre-existing and unrelated to
layer shell). `handlers.rs`'s `new_popup` tracks the popup in
`PopupManager` and `commit` calls `PopupManager::commit`, but nothing
calls `PopupSurface::send_configure` — and the pinned rev's
`PopupManager::commit` only moves a popup from unmapped to mapped, it does
not configure (checked: `desktop/wayland/popup/manager.rs:38-52`). Anvil
does this in its own `ensure_initial_configure`. Consequence: a client
menu, dropdown or tooltip never appears, from a window *or* a layer
surface — which is why item 14 deliberately does not implement
`WlrLayerShellHandler::new_popup` either: tracking a popup that can never
map would be dead code. Fix is small (configure on first commit, the same
shape `send_initial_configure` already has for toplevels) but wants its
own tests, since it makes popups appear for the first time and nothing in
the render path has ever drawn one.

Reproduced, not just traced: `no_xdg_popup_is_configured_yet` (in
`layer_shell/tests.rs`) has a real client create an `xdg_popup` on a
mapped toplevel with a valid positioner and round-trip ten times — no
`xdg_surface.configure` ever arrives, and the compositor stays up. Turning
that assertion around is what the fix should do; delete the test and this
entry together.
