---
title: "System tray (StatusNotifierItem)"
status: "open"
area: "scootbar"
priority: "medium"
blocked: "dbus-client"
milestone: "M6"
---

# System tray

Filed 2026-09-29. Serves **daily-drive**: some apps only expose themselves
here.

A `tray` module implementing the StatusNotifierWatcher and host side over
D-Bus: list items, draw their icons, click for the primary action, and show
an item's menu (the DBusMenu protocol) through [popups](resolved/popups-done.md).

## Why this is medium, and what the research says

The tray is the buggiest piece of other bars (about 144 tray-related issues in
Waybar alone, several long-lived) and the thing users of lightweight bars miss
most: yambar's tray request is among its oldest and its maintainer called it
"rather annoying to implement". Failure modes worth designing out:
the watcher disappearing after some uptime (`No such object path
'/StatusNotifierWatcher'`, Waybar #3468), a bad item crashing the whole bar
(#3616, #4261), apps that started **before** the bar losing their icons, and
wrong icon size at fractional scale (#1175). So:

- **Treat the StatusNotifierWatcher as core infrastructure of the bar**, not just
  a module: own the name, re-acquire it if it is lost, and pick up items that
  registered before the bar started (enumerate the bus at start).
- **A malformed or hostile item can only ever lose itself**: never a panic or a hang.
- **Icons at exact device pixels**, requesting the size the output needs.
- The systemd user unit orders the bar so tray apps are not starved of a host
  ([nix-modules-and-stylix](resolved/nix-modules-and-stylix-done.md)).

## What to decide first

- **Icon sources**: themed icon names need an icon-theme lookup and image
  decoding; pixmaps arrive as raw ARGB over D-Bus. Measure what each costs
  and support pixmaps first.
- **Whether a first version of this is worth its weight**: the tray is where a lightweight
  bar most often becomes a heavy one. Measure against the
  [resource ratchet](lightest.md) before enabling by default; it is a Cargo
  feature and off in the smallest build.
- Item lifecycle: apps that crash without unregistering, and a watcher that
  is already owned by another process.

## Done when

An item appears, clicks work, a crashed item disappears, and the module's
memory and wakeups are measured and published.
