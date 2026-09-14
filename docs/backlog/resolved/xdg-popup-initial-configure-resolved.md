---
title: "No `xdg_popup` ever receives its initial configure, so no popup maps at all — RESOLVED."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# No `xdg_popup` ever receives its initial configure, so no popup maps at all — RESOLVED.

## Resolution (2026-09-14)

Fixed flexwm-side in `handlers.rs`: `State::commit` now calls
`send_popup_initial_configure` after `PopupManager::commit`, which sends
the initial configure on the popup surface's first commit — the same
shape `send_initial_configure` already had for toplevels. Guarded twice
(role check first, so ordinary commits never pay for the popup-tree
lookup; `is_initial_configure_sent` after, so later commits stay quiet
rather than tripping `AlreadyConfigured`/`NotReactive` on a
non-reactive positioner). An `Err` from `send_configure` on the initial
one is logged and retried on the next commit rather than treated as
fatal, since both error variants require a first configure already sent.

Nothing else was needed for popups to appear — verified, not assumed:

- **Rendering:** Smithay's `Window` element already draws each mapped
  window's popups itself (`PopupManager::popups_for_surface` inside
  `render_elements`), so the render path needed no changes — popups had
  simply never mapped to be drawn.
- **Frame callbacks:** `Window::send_frame` already walks
  `popups_for_surface`, so a mapped popup's frame callbacks complete
  like any window's.
- **Cleanup:** `PopupManager::cleanup` already runs per frame in
  `headless.rs`'s render pass, so destroyed popups don't accumulate.

The pinned test (`no_xdg_popup_is_configured_yet`) is deleted as the
entry asked; its replacement (`an_xdg_popup_configures_maps_draws_and_tears_down`)
inverts the assertion and goes further — configure arrives, the popup
acks/attaches/maps, its pixels reach the framebuffer, exactly one
configure arrives across further commits, its frame callback completes,
and destroying it leaves the compositor serving with its pixels gone.

**What is still open, filed as `xdg-popup-input.md`:** popup *input*.
Pointer hit-testing stops at the window tree (`WindowSurfaceType::ALL`
finds popups for input, but keyboard focus never moves onto a popup and
`XdgShellHandler::grab` is still a no-op) — a menu shows but cannot be
clicked with the keyboard yet. Layer-surface-parented popups remain
untracked (`WlrLayerShellHandler::new_popup` is still not implemented):
they now *would* configure if tracked, but a bar's tooltips and menus
still need that half.

Original entry, left as written:

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
