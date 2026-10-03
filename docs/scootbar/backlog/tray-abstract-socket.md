---
title: "Tray: session buses on abstract sockets"
status: "open"
area: "scootbar"
priority: "low"
blocked: null
milestone: "M6"
---

# Tray: session buses on abstract sockets

Filed 2026-10-03, split out of [tray](resolved/tray-done.md) when its menus landed.
Serves **daily-drive** on machines whose session bus is not a
filesystem path (what `dbus-launch` makes: `unix:abstract=...`).

## The gap

The client dials a filesystem path only
(`crates/scootbar/src/dbus/conn.rs`: `bus_path_for` refuses an address
with no `unix:path=` with a line on stderr and no tray, rather than
replacing it with another bus that happens to exist). The media and
bluetooth modules share the rule through the same client.

## What to do

- Dial `unix:abstract=` session addresses (Linux abstract namespace:
  connect to the name, watch what for disappearance — there is no
  directory to inotify, so the wait/retry half of `dbus::link` needs a
  second mechanism).
- Keep the refusal for truly undialable transports (`tcp:`,
  `autolaunch:`), loudly, as today.
- The inotify wait watches the socket's directory today; an abstract
  bus has none, so decide what re-dialing waits on (poll the name on
  the retry timer only, or watch the runtime directory as an
  approximation — and say which).

## Not in this ticket

Everything else in [tray](resolved/tray-done.md); the system bus (a fixed path).
