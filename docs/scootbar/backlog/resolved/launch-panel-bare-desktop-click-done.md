---
title: "scootlaunch: a click outside the panel dismisses it"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-29"
---

# scootlaunch: a click outside the panel dismisses it

Filed 2026-10-19. Serves **daily-drive**: a launcher that stays up after a
click elsewhere reads as stuck.

## The gap

Found reviewing #674. With exclusive keyboard a click on the bare desktop does
not dismiss the panel: `layer_keyboard_focus` keeps an exclusive surface
first, so no `wl_keyboard.leave` ever reaches it, and the click goes to the
surface below, never to the panel. The v1 limitation was documented in
`site/src/content/docs/scootlaunch/panel.md`.

## What to do

The compositor sends the panel `zwlr_layer_surface_v1.closed` on a button
press anywhere but on it, gated on the `scootlaunch` namespace, never while
the session is locked; `scootlaunch` exits 1 on any `closed`. A full-output
transparent input surface was the other option in the ticket: it costs a
full-output buffer and a blended full-screen surface every frame for a panel
that otherwise holds almost nothing, so it was rejected.

## Not in this ticket

Other launchers (fuzzel, wofi) and other `exclusive` surfaces: the protocol
carries no "this is a launcher" bit, and closing a locker or a password
prompt on a stray click is the harm the namespace gate exists to prevent.
Touch input: scoot has no `wl_touch` path.

## What landed

`State::dismiss_launcher_on_outside_press` (`layer_shell.rs`), called from
`pointer_button` on a press before the focus change. It is stateless: the
surface keeps `exclusive` for its whole life and its unmap returns the
keyboard through the existing derivation. `scootlaunch` now treats any
`closed` as a dismiss (it used to re-create the surface once, a retry meant
for a bar). Tests: `layer_shell/tests/launcher_dismiss.rs`, one in
`multi_output.rs`, and `a_click_outside_the_panel_dismisses_it` in
`crates/scootlaunch/tests/panel.rs`.
