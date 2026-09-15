---
title: "Popup input: keyboard focus never moves onto a popup, grabs are a no-op, layer-parented popups stay untracked"
status: "open"
area: "protocols"
priority: "medium"
blocked: null
---

# Popup input: keyboard focus never moves onto a popup, grabs are a no-op, layer-parented popups stay untracked

The input half of popup support, split out when the mapping half
resolved (`docs/backlog/resolved/xdg-popup-initial-configure-resolved.md`):
a window's `xdg_popup` now configures, maps and draws, but it is
display-only. Three gaps, in the order a real menu needs them closed:

1. **`XdgShellHandler::grab` is a no-op** (`handlers.rs`). A client
   opening a menu asks for a popup grab (`xdg_popup.grab` + seat +
   serial); the protocol says the compositor should direct input to the
   popup tree until it is dismissed. Smithay has the machinery —
   `PopupManager::grab_popup` builds a `PopupGrab` that routes
   keyboard/pointer into the popup and unwinds focus on dismiss — but
   flexwm neither calls it nor falls back, so a menu is shown and then
   clicks/keys keep going to whatever held them. The grab is also what
   dismisses: nothing else ever sends `popup_done` today, so even
   Escape-to-close depends on this item, not just focus routing.
2. **Keyboard focus never moves onto a popup.** Even without a grab,
   keyboard-aware clients (e.g. an app menu with typeahead, a combobox)
   expect focus on the popup surface while it is up. Pointer hit-testing
   does reach popups already (`WindowSurfaceType::ALL` in
   `State::surface_under`), so clicks land on the right surface when the
   pointer is over one — but Escape-to-close, arrow-key navigation and
   typeahead all need the keyboard half.
3. **Layer-surface-parented popups are still untracked.**
   `WlrLayerShellHandler::new_popup` is deliberately not implemented
   (item 14's reasoning: tracking a popup that could never map was dead
   code — no longer true). A bar's own dropdown menus and tooltips
   (`xdg_popup` parented to a layer surface) configure fine once
   tracked, but nothing tracks them, so they still never map. Smithay's
   layer-shell handler signature hands flexwm the parent `LayerSurface`
   and the `PopupSurface`; `PopupManager::track_popup` accepts either
   parent kind.

The same end-to-end test shape the mapping fix used covers 1–2: map a
popup, assert a click over it reaches it, assert keyboard focus lands on
it under a grab, assert focus returns to the parent on dismiss. No
quickshell field confirmation yet — both probed shells route their own
menus through layer surfaces (zero `xdg_popup` wire traffic in either
probe), so the first real client will be an ordinary app toolkit (GTK
menus, Qt comboboxes).

Rough size: M (the grab machinery exists; the work is wiring it into
flexwm's focus model without regressing the layer-shell keyboard rules
`layer_shell.rs` documents — a popup grab must lose to them, or a
launcher's keyboard interactivity and a menu grab will fight).
