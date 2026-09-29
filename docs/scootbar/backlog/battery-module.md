---
title: "Battery module: level, charging state, warn and critical classes"
status: "open"
area: "scootbar"
priority: "medium"
blocked: "icons-and-fonts"
milestone: "M5"
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

**Measured 2026-09-29 (Asahi.md, Test 14), partly.** The M2 exposes
`macsmc-battery` (`capacity`, `status`, `present`, `energy_*`, `charge_*`,
`voltage_now`, `temp`, `time_to_*`, `charge_behaviour`) and `macsmc-ac`
(`online`). On AC at `Full` for 300 s it fired **zero** `power_supply`
uevents even though `voltage_now` and `temp` moved, so jitter is silent.
Whether capacity steps while discharging, or plug/unplug, emit uevents is
**still open**: it needs someone to unplug the machine. At `Full` the SMC
reports `capacity=100` while `charge_now/charge_full` is 96%, so use
`capacity`.

## What to build

- Percentage from `capacity`, state from `status` (charging, discharging,
  full, not charging), the class `warn` and `urgent` at configurable
  thresholds.
- Several batteries: combine, or show the first, by config. No battery at all
  (desktop, VM): `Unavailable`, zero cost, module hidden.
- Time-remaining is **not** in the first version: it needs rate smoothing to be honest, and a
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
