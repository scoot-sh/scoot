---
title: "Tray: themed icon names"
status: "open"
area: "scootbar"
priority: "medium"
blocked: null
milestone: "M6"
---

# Tray: themed icon names

Filed 2026-10-03, split out of [tray](tray.md) when its menus landed.
Serves **daily-drive**: most GTK and Ayatana apps send only `IconName`.

## The gap

An item that sends only `IconName` (no `IconPixmap`) is tracked and
clickable by index but takes no room: there is no icon-theme lookup and
no image decoder in this build
(`crates/scootbar/src/modules/tray/item.rs`: `shown()` needs
`!icons.is_empty()`). Attention and overlay icons, tooltip icons and
`IconThemePath` are likewise read for shape and dropped. So on a
real-world desktop a large fraction of tray icons are invisible.

## What to do

- Measure first, per the [tray](tray.md) "what to decide" rule: an
  icon-theme lookup (hicolor search across `$XDG_DATA_DIRS`) plus an
  image decoder (PNG at least, SVG would pull a renderer) against the
  [resource ratchet](lightest.md), before enabling by default. Pixmaps
  came first precisely because this cost was unmeasured.
- Draw themed names through the shared icon cache at the output's
  device pixels, like pixmaps; keep the hostile-item bounds (name
  length, lookup cost per icon version, not per frame).
- Attention/overlay icons and `IconThemePath` with it, or say why not.

## Not in this ticket

Menus (landed with [tray](tray.md)); tooltip icons.
