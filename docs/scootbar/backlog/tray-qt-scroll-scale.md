---
title: "Tray: Qt-scale scroll deltas for KDE items"
status: "open"
area: "scootbar"
priority: "medium"
blocked: null
milestone: "M6"
---

# Tray: Qt-scale scroll deltas for KDE items

Filed 2026-10-07, split out of [tray-real-apps](resolved/tray-real-apps-done.md)
when its sign question closed with a measured answer (GTK sign, up
negative). Serves **daily-drive**: KDE volume items scroll the wrong
way, or not at all, under that choice.

## The gap

- The bar sends `Scroll(±notches, "vertical")`, up negative — correct
  for GTK/Ayatana items (pasystray 0.8.2 turns volume up on scroll-up;
  libayatana-appindicator maps positive vertical to scroll-down) and
  matching Waybar. But kmix connects
  `KStatusNotifierItem::scrollRequested` to its volume and reads
  **positive as louder**, rounding the delta by 120 (Qt wheel units;
  Plasma's tray forwards `+angleDelta`, up positive at 120 a notch).
  Our `−1` per wheel-up reads to kmix as *quieter*, and would need 120
  repeats for one step anyway.
- No per-item toolkit detection exists, and one sign cannot serve both
  stacks: `+120`/notch would fix kmix and invert every appindicator
  direction handler.

## What to do

- Decide: per-item scroll convention, Qt-scale magnitude for KDE
  items, or wontfix with the docs saying so. Evidence in
  `site/src/content/docs/scootbar/modules.md` (Tray: clicks) and the
  sources it cites (libayatana-appindicator's mapping, kmix's
  `trayWheelEvent`, Plasma's `StatusNotifierItem.qml`).
- Any per-item detector (Ayatana path? missing `Activate`? KSN item
  props?) must be spoof-resistant: same-user peers lie about who they
  are. Any magnitude change must keep pasystray correct (it reads the
  sign only) and Waybar parity on the sign.
- Measure idle RSS/wakeups before/after per the [resource
  ratchet](lightest.md); never claim a row waived.

## Not in this ticket

The sign itself (closed in [tray-real-apps](resolved/tray-real-apps-done.md));
themed icons ([tray-icon-themes](tray-icon-themes.md)).
