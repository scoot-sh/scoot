---
title: "Data-source modules (umbrella): rules shared by every module that reads the system"
status: "open"
area: "scootbar"
priority: "medium"
blocked: "module-api-and-clock, pointer-and-interactions"
---

# Data-source modules

Filed 2026-09-29; split into one entry per module 2026-09-29. Serves
**daily-drive**. This entry holds the rules they share; the work is in:

- [window title](window-title-module.md)
- [battery](battery-module.md)
- [volume](volume-module.md)
- [network](network-module.md)
- [brightness](brightness-module.md)
- [media (MPRIS)](media-module.md)
- [system tray](tray.md)

**Not built in, by decision**: CPU, memory, temperature and disk
([system-stats-decision](system-stats-decision.md)). **Not yet filed**: keyboard
layout, which no standard protocol carries; it needs a scoot IPC event first
(file it in the compositor's backlog when wanted).

## Rules for every module

- **An fd the loop polls, or nothing.** No periodic timer unless the source has
  no event and a measurement justifies it; then only while it matters (visible,
  discharging), and the rate is published.
- **`Unavailable` means zero cost**: no fds, no width, no timer, no log spam.
  Retry when the thing appears (a socket, an inotify watch), never by polling.
- **A state class, a `query` entry and an `invoke` action** for each
  ([agent-interface](agent-interface.md)), and interactions through the shared
  config keys.
- **Text from outside is untrusted**: bound its length, strip control
  characters, never let it grow a cache without limit.
- **Tests through the module harness** with fake events, plus a real-source test
  where the machine allows, and a row in the [release gate](lightest.md).
- **Freeze the module API only after volume and network have exercised it.**

## Done when

Every child is resolved or explicitly refused, and each has its cost published.
