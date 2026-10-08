---
title: "Data-source modules (umbrella): rules shared by every module that reads the system"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "ongoing"
resolved: "2026-10-07"
---

# Data-source modules

Filed 2026-09-29; split into one entry per module 2026-09-29. Serves
**daily-drive**. This entry holds the rules they share; the work is in:

- [window title](../resolved/window-title-module-done.md) — RESOLVED 2026-10-01
- [battery](../resolved/battery-module-done.md) — RESOLVED 2026-10-02
- [volume](../resolved/volume-module-done.md) — RESOLVED 2026-10-02
- [network](../resolved/network-module-done.md) — RESOLVED 2026-10-02
- [brightness](../resolved/brightness-module-done.md) — RESOLVED 2026-10-02
- [media (MPRIS)](../resolved/media-module-done.md) — RESOLVED 2026-10-03
- [bluetooth](../resolved/bluetooth-module-done.md) — RESOLVED 2026-10-03
- [system tray](../resolved/tray-done.md) — RESOLVED 2026-10-03

**Not built in, by decision**: CPU, memory, temperature and disk
([system-stats-decision](../system-stats-decision.md)). **Keyboard layout**, the most
requested module of all, is blocked on scoot: no standard protocol carries it, so
it needs [an IPC event](../../../backlog/resolved/keyboard-layout-event-done.md) first.

## Rules for every module

- **An fd the loop polls, or nothing.** No periodic timer unless the source has
  no event and a measurement justifies it; then only while it matters (visible,
  discharging), and the rate is published.
- **`Unavailable` means zero cost**: no fds, no width, no timer, no log spam.
  Retry when the thing appears (a socket, an inotify watch), never by polling.
- **A state class, a `query` entry and an `invoke` action** for each
  ([agent-interface](../resolved/agent-interface-done.md)), and interactions through the shared
  config keys.
- **Text from outside is untrusted**: bound its length, strip control
  characters, never let it grow a cache without limit.
- **Tests through the module harness** with fake events, plus a real-source test
  where the machine allows, and a row in the [resource ratchet](../lightest.md).
- **Freeze the module API only after volume and network have exercised it.**

`blocked` above lists the children, so this entry becomes ready (and can be
resolved) only when all of them are done.

## Done when

Every child is resolved or explicitly refused, and each has its cost published.

## Resolution (2026-10-07)

Done-condition quoted from above: "Every child is resolved or explicitly
refused, and each has its cost published." All eight children are resolved;
none was refused. Costs: ratchet rows where the module got one, in-ticket
idle measurements otherwise.

| Child | Resolved | Cost published |
|---|---|---|
| window title | 2026-10-01 (`window-title-module-done.md`, module PR #374) | in-ticket: title flood 10 jiffies (0.5% CPU) capped vs 27 uncapped, 0 jiffies idle |
| battery | 2026-10-02 (`battery-module-done.md`) | in-ticket: idle 75 s on AC Full, voluntary 14→14 / nonvoluntary 3→3, no steady-state wakeup |
| volume | 2026-10-02 (`volume-module-done.md`, PR #376) | in-ticket: native ~73 µs/event vs interim 11–13 ms + ~11 MB transient; idle 60 s subscribed: 0 event wakes, 0 bytes |
| network | 2026-10-02 (`network-module-done.md`) | ratchet row "M5 network" (`lightest.md`): 2 netlink sockets, timerfd only while a WiFi network is shown |
| brightness | 2026-10-02 (`brightness-module-done.md`) | ratchet row "M5 brightness": one netlink uevent socket |
| media (MPRIS) | 2026-10-03 (`media-module-done.md`) | ratchet row "M6 media": 0 wakeups with bus + paused/playing players |
| bluetooth | 2026-10-03 (`bluetooth-module-done.md`) | ratchet row "M6 bluetooth" |
| system tray | 2026-10-03 (`tray-done.md`, menus PR #405) | ratchet rows "M6 tray and the D-Bus client" + "M6 tray hardening" |

The API-freeze precondition in the rules ("only after volume and network
have exercised it") is satisfied: both landed (volume PR #376 native client,
network module 2026-10-02); the freeze itself is tracked by the resolved
`module-api-and-clock` entry. `blocked` is null: the children unblocked this
entry as each resolved. CPU/memory/temperature/disk stay out by the linked
`system-stats-decision` (still open); keyboard layout waits on the linked IPC
event entry.
