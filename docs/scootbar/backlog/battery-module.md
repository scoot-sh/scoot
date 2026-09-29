---
title: "Battery module: level, charging state, warn and critical classes"
status: "open"
area: "scootbar"
priority: "medium"
blocked: "module-api-and-clock, icons-and-fonts"
---

# Battery module

Filed 2026-09-29. Serves **daily-drive** on laptops (the Asahi M2 Air
is the reference machine in scoot's hardware tests).

## Source, event-driven if the kernel allows

`/sys/class/power_supply/*` read on change, woken by kernel uevents on a
`NETLINK_KOBJECT_UEVENT` socket filtered to the `power_supply` subsystem. The
open question is measured, not assumed: **does the driver emit uevents as the
capacity changes, or only on plug and unplug?** On the Asahi machine and one
other, record how often a uevent arrives while discharging. If capacity
changes are silent, the fallback is a slow timer (order of a minute) that runs
**only while discharging** and stops when charging or full, with the
wakeup rate published.

## What to build

- Percentage from `capacity`, state from `status` (charging, discharging,
  full, not charging), the class `warn` and `urgent` at configurable
  thresholds.
- Several batteries: combine, or show the first, by config. No battery at all
  (desktop, VM): `Unavailable`, zero cost, module hidden.
- Time-remaining is **not** in v1: it needs rate smoothing to be honest, and a
  wrong estimate is worse than none.
- A low-battery action hook (`on-low = { exec = [...] }`) fires once per
  crossing, not per update.

## Edge cases

Battery removed or hot-swapped, status strings a driver invents, `capacity`
above 100 or a missing file, resume from suspend with stale state (re-read on
wake), a uevent storm.

## Done when

Value and state track a real battery on the reference machine with the
measured wakeup behavior recorded, and the module is absent where there is no
battery.
