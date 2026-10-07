---
title: "Tray: real Qt, GTK and Electron apps"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M6"
resolved: "2026-10-07"
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

## Status (2026-10-07): live round landed (PR #496)

Ran real StatusNotifierItem apps headless against the bar (private
dbus-daemon, `scoot --headless`, Xvfb/offscreen, dbus-monitor +
`scoot msg screenshot` evidence): pasystray 0.8.2 (GTK/Ayatana),
CopyQ 16.0.0 and KeePassXC 2.7.12 (Qt), Joplin 3.6.16 (Electron).

Fixed:
- `GetLayout` replies ride bare (`u(ia{sv}av)`): the bar demanded a
  struct-wrapped `(u(ia{sv}av))`, so every real menu stayed shut
  (proven with CopyQ's reply on the wire: `sig=u(ia{sv}av)`,
  `parsed=false`). The fakes emitted the wrapped form, encoding the
  bug. New test `a_real_qt_layout_parses_bare_out_args` fails on the
  old parser and passes on the new one.
- `Scroll` sign flipped to GTK (up negative, matching Waybar and
  libayatana-appindicator's mapping, which reads positive vertical as
  scroll-down; the SNI spec is silent; Plasma/Qt use the opposite):
  wheel-up sent `+1`, which pasystray would read as scroll-down
  (quieter on wheel-up). Verified post-fix on the wire against
  pasystray (`Scroll(-1)`) and CopyQ.

Recorded (docs), not fixed here:
- Themed-only icons stay invisible ([tray-icon-themes](tray-icon-themes.md)):
  pasystray sends `IconName` only, and its menu is unreachable (the
  popup has no span to open from).
- Ayatana items expose neither `Activate` nor `ContextMenu`: a left
  click does nothing.
- Electron serves its menu from a second same-PID connection and
  answers no introspection (the bar never introspects, and
  well-known-name calls accept any sender, so both already work); row
  ids renumber per revision (CopyQ); fresh configs leave the tray off
  (KeePassXC, Joplin).
- `Event` zeros drive real actions (a CopyQ row set its clipboard,
  Joplin's Quit quit); no item read the data or the timestamp.

Remainder split to
[tray-qt-scroll-scale](tray-qt-scroll-scale.md): Qt-scale deltas for
KDE items (kmix needs `+120`/notch).

Ratchet (release, Asahi M2, same toolchain): file +0 B, `.text` −896 B
(−0.05%), `.rodata` −64 B against `origin/main`; idle 60 s windows 0
tray wakeups (2 with the clock placed: the clock's), RSS level
(4416→4512 kB no items, 4800 kB with four real items against 4784 kB
on the base), 1 thread, 9 fds. No row regresses; nothing waived.
