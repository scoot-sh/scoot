---
title: "System tray (StatusNotifierItem)"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M6"
resolved: "2026-10-03"
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

## Status (2026-10-03): menus landed, entry resolved (PR #405)

Menus are built: `menu N` and a right click open the item's menu in a
popup (an `ItemIsMenu` item opens on a left click instead of
`Activate`); the DBusMenu client reads `GetLayout` (bounded depth and
property list), sends `Event clicked` and `AboutToShow`, and honors the
update signals, accepted only from the item's owner; a layout update
while open re-fills the popup and the item vanishing closes it. Rows
map to text and buttons (mnemonics stripped, toggles as text prefixes,
separators as gaps, disabled rows as plain text, submenus drilling in
place with a back row; icons in items not drawn). A hostile menu
(huge, deep, malformed, flooding updates) loses only itself. An item
with no menu falls back to `ContextMenu`. The reference is
[`docs/scootbar/cli.md`](../cli.md#tray) (Menus, Bounds).

Checked live (a private `dbus-daemon`, `scoot --headless`, a jeepney
item serving a real menu): `menu 0` and a real right-click open it,
`menu-drill`/`menu-back` walk a submenu, `menu-select` reaches the item
as `Event clicked` and closes it, a `LayoutUpdated` re-fills a changed
label, and a `kill -9`'d item closes everything. Screenshots in the
implementer's report. Cost, measured with `readelf -S -W` on release
builds of base `5c19c0333` and this branch (rebased onto `main`,
review fixes included; both built minutes apart with the same
toolchain): `.text` 1,599,176 → 1,614,760 (**+15,584 B, +1.0%**),
`.rodata` 129,823 → 130,143 (+320 B), file bytes 2,101,984 on both
sides (+0); idle tray with one menu item RSS 4,420 kB closed and
4,592 kB open, zero wakeups either way — recorded for the maintainer,
no waiver claimed.

What remains moved to its own entries: [themed icon
names](tray-icon-themes.md), [abstract
sockets](tray-abstract-socket.md), [real
apps](tray-real-apps.md), [runaway
redraws](tray-redraw-coalesce.md).

Out of scope: an item's own tooltip text is not drawn. The module's
tooltip lists the shown items' titles, which identifies every item;
drawing per-item tooltip bodies (often long help text) serves no split
entry, so it stays undrawn rather than gaining one.
