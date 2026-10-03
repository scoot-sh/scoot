---
title: "System tray (StatusNotifierItem)"
status: "open"
area: "scootbar"
priority: "medium"
blocked: null
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

## Status (2026-10-02): the first version landed (PR PRNUM), menus remain

Everything in "Done when" is met and the entry stays open only for the
menus, which have their prerequisite now: [popups](resolved/popups-done.md)
landed (declarative content: text, a slider, buttons; a list is a column of
buttons), so a DBusMenu layout can be drawn as one. Landed, on the
[shared D-Bus client](resolved/dbus-client-done.md): the watcher owned
(and re-taken when its owner leaves), hosting against another watcher when
one owns it, items that registered before the bar found by listing the bus,
pixmap icons at the output's device pixels, click and middle click and the
wheel as `Activate`, `SecondaryActivate` and `Scroll`, an item vanishing
with its owner, and the cost published in
[lightest.md](lightest.md#m6-tray-and-the-d-bus-client-module-level-cost-measured-2026-10-02).
The reference is [`docs/scootbar/cli.md`](../cli.md#tray).

Checked live (a private `dbus-daemon`, `scoot --headless`, items written
with jeepney, an independent D-Bus marshaller, and `busctl` reading the
bar's watcher object): one bar owning the name and another hosting against
it, the owner `kill -9`'d and the host taking the name and keeping the
item, an item registered before the bar and one after, a changed icon, a
click, and an item killed without unregistering.

**What remains**, none of it in the first version:

- **Menus**: `ContextMenu` and the DBusMenu protocol, through
  [popups](resolved/popups-done.md), which exist now: what is left is the
  DBusMenu client (`GetLayout`, `Event`, `AboutToShow`, layout-updated
  signals, bounded like the rest), mapping a layout to popup content (it has
  no submenu, checkmark or icon widgets yet) and binding `menu N` and a
  right click to open it. The `menu N` action exists and is refused naming
  popups; a right click does nothing by default. An item with
  `ItemIsMenu` true is clicked with `Activate`, which such an item may
  ignore: its menu is the whole point of it.
- **Themed icon names**: an item that sends only `IconName` (most
  GTK and Ayatana apps) is tracked but not drawn: it needs an icon-theme
  lookup and an image decoder, the "what to decide first" cost, still
  unmeasured. Attention and overlay icons and `IconThemePath` are read for
  shape and dropped. [Tooltips](tooltips.md) (the title list on the module
  is what exists).
- **Session buses on abstract sockets** (`unix:abstract=...`, what
  `dbus-launch` makes): the client dials a path only.
- **Real applications**: the items tried are an independent marshaller's,
  not a Qt, GTK or Electron app, and the sign of `Scroll`'s delta is KDE's
  by reading, not by testing a host against an item that cares.
- **A runaway item's redraws**: one that re-announces its icon continuously
  costs a bus round trip and a redraw every 50 ms; measured in the cost
  table. Nothing coalesces redraws across items.
