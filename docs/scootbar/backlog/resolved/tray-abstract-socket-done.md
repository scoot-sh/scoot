---
title: "Tray: session buses on abstract sockets"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M6"
resolved: "2026-10-08"
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

## Status (2026-10-08): landed (PR #523)

`unix:abstract=` session addresses are dialled: `bus_addr_for` parses
them (`%xx` escapes decoded; a path still wins when the address names
both, as before; empty and NUL-holding names refused like `tcp:`),
`connect_abstract` dials in the abstract namespace, and the tray and
media modules start on either address form through `Addr::Abstract`.
The system bus is untouched (a fixed path, out of scope). Truly
undialable transports (`tcp:`, `autolaunch:`) are still refused loudly,
as before. Re-dialing waits on the retry timer only (30 s one-shot; an
abstract socket has no directory to inotify, and a runtime-directory
watch would wake on unrelated files without ever seeing the socket): a
poll that finds no bus resets the quick-death count, the latch is
unchanged. Evidence in the PR body: 4 new tests (each proven to fail
with the fix toggled off), the full clippy matrix, nextest and cargo
test (only the sway-missing environmental failures), `cargo deny`,
release sizes (file +0 B, `.text` +1,760 B reported unwaived), and idle
RSS/wakeups (path rows 0 wakeups both sides; abstract polls at 2–3
wakeups per 60 s).
