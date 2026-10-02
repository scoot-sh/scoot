---
title: "Popups: sliders, lists and menus as `xdg_popup`s parented to the bar"
status: "open"
area: "scootbar"
priority: "low"
blocked: "network-module"
milestone: "M6"
---

# Popups

Filed 2026-09-29. Serves **daily-drive** (a volume slider, a WiFi list, a
power menu).

A module may return declarative popup content (list, slider, buttons) that the
bar draws in an `xdg_popup` parented to its layer surface. The popup exists only
while open, so idle cost stays zero.

## Interim, and why this is low priority

Until this lands, a module hands a list to a dmenu-style launcher and acts on
the selection: pickers and menus work with `fuzzel --dmenu` today and with the
scoot launcher later, with no popup code in the bar. Build native popups only
when that stops being enough.

## What to do

- `xdg_popup` via `zwlr_layer_surface_v1.get_popup` with a positioner anchored
  to the module's rect; scoot handles layer-parented popups and grabs
  (`xdg-popup-input-resolved.md`), and validates the grab serial, so open from the
  click that carries one.
- A tiny widget set (list, slider, button) drawn with the bar's own primitives.
  This is the consumer that justifies [extracting `scootui`](extract-scootui.md).
- Dismissal: click outside, Escape (the popup grab gives it the keyboard only
  while open), and when its module disappears.
- Popups must not defeat the bar's "never takes the keyboard" property outside
  their own lifetime.

## Edge cases

A popup open when the output is removed or the session locks (scoot dismisses
popup grabs on lock), a second click on the same module (toggle), a popup that
would leave the output (constraint adjustment, `popup-constraint-adjustment-done.md`).
