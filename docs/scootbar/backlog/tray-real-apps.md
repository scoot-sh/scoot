---
title: "Tray: real Qt, GTK and Electron apps"
status: "open"
area: "scootbar"
priority: "medium"
blocked: null
milestone: "M6"
---

# Tray: real Qt, GTK and Electron apps

Filed 2026-10-03, split out of [tray](resolved/tray-done.md) when its menus landed.
Serves **daily-drive**: the items tried so far are an independent
marshaller's (jeepney), not real apps.

## The gap

- No Qt, GTK or Electron app has been tried as an item: themed-only
  icons ([tray-icon-themes](tray-icon-themes.md)), real menu trees
  (deep, lazy submenus, radio groups), and toolkit quirks are all
  untested against the bar.
- The sign of `Scroll`'s delta is KDE's by reading, not by testing: a
  Qt wheel's against a GTK one, and hosts disagree (Waybar sends GTK's,
  up negative). No item that cares has been checked against it
  (`docs/scootbar/cli.md`: Tray).
- Menu `Event` data and timestamp are zeros no item reads
  (`crates/scootbar/src/modules/tray/menu.rs`): confirm against an app
  that logs what it receives.

## What to do

Run the live round (private bus, `scoot --headless`, screenshots)
against at least one Qt app, one GTK/Ayatana app and one Electron app
with a menu: register, icon shown, menu opens, every row kind clicks,
submenu drills, update while open, crash without unregistering. Fix
what breaks, record the sign findings in `cli.md`, and close the
`Scroll` question with a measured answer.

## Not in this ticket

Themed icons themselves ([tray-icon-themes](tray-icon-themes.md));
anything the apps need that is already split out.
