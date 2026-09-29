---
title: "System tray (StatusNotifierItem)"
status: "open"
area: "scootbar"
priority: "low"
blocked: "dbus-client, popups (tray menus)"
---

# System tray

Filed 2026-09-29. Serves **daily-drive**: some apps only expose themselves
here.

A `tray` module implementing the StatusNotifierWatcher and host side over
D-Bus: list items, draw their icons, click for the primary action, and show
an item's menu (the DBusMenu protocol) through [popups](popups.md).

## What to decide first

- **Icon sources**: themed icon names need an icon-theme lookup and image
  decoding; pixmaps arrive as raw ARGB over D-Bus. Measure what each costs
  and support pixmaps first.
- **Whether v1 of this is worth its weight**: the tray is where a lightweight
  bar most often becomes a heavy one. Measure against the
  [release gate](lightest.md) before enabling by default; it is a Cargo
  feature and off in the smallest build.
- Item lifecycle: apps that crash without unregistering, and a watcher that
  is already owned by another process.

## Done when

An item appears, clicks work, a crashed item disappears, and the module's
memory and wakeups are measured and published.
