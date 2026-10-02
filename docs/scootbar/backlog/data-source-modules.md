---
title: "Data-source modules (umbrella): rules shared by every module that reads the system"
status: "open"
area: "scootbar"
priority: "medium"
blocked: "media-module, bluetooth-module, tray"
milestone: "ongoing"
---

# Data-source modules

Filed 2026-09-29; split into one entry per module 2026-09-29. Serves
**daily-drive**. This entry holds the rules they share; the work is in:

- [window title](resolved/window-title-module-done.md) — RESOLVED 2026-10-01
- [battery](resolved/battery-module-done.md) — RESOLVED 2026-10-02
- [volume](volume-module.md)
- [network](resolved/network-module-done.md) — RESOLVED 2026-10-02
- [brightness](resolved/brightness-module-done.md) — RESOLVED 2026-10-02
- [media (MPRIS)](media-module.md)
- [bluetooth](bluetooth-module.md)
- [system tray](tray.md)

**Not built in, by decision**: CPU, memory, temperature and disk
([system-stats-decision](system-stats-decision.md)). **Keyboard layout**, the most
requested module of all, is blocked on scoot: no standard protocol carries it, so
it needs [an IPC event](../../backlog/ipc/keyboard-layout-event.md) first.

## Rules for every module

- **An fd the loop polls, or nothing.** No periodic timer unless the source has
  no event and a measurement justifies it; then only while it matters (visible,
  discharging), and the rate is published.
- **`Unavailable` means zero cost**: no fds, no width, no timer, no log spam.
  Retry when the thing appears (a socket, an inotify watch), never by polling.
- **A state class, a `query` entry and an `invoke` action** for each
  ([agent-interface](resolved/agent-interface-done.md)), and interactions through the shared
  config keys.
- **Text from outside is untrusted**: bound its length, strip control
  characters, never let it grow a cache without limit.
- **Tests through the module harness** with fake events, plus a real-source test
  where the machine allows, and a row in the [resource ratchet](lightest.md).
- **Freeze the module API only after volume and network have exercised it.**

`blocked` above lists the children, so this entry becomes ready (and can be
resolved) only when all of them are done.

## Done when

Every child is resolved or explicitly refused, and each has its cost published.
