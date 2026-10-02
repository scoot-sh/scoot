---
title: "Battery: measure uevents on unplug and capacity steps on the Asahi M2"
status: "open"
area: "scootbar"
priority: "medium"
blocked: null
milestone: "M5"
---

# Battery: measure uevents on unplug and capacity steps on the Asahi M2

Filed 2026-10-02, from the review of the battery PR (#383). Serves
**daily-drive**: a laptop bar that shows a stale charge state is wrong in the
one place a battery module exists to be right.

## The gap

The resolved [battery module](resolved/battery-module-done.md) has a headline
"Done when" criterion, the measured wakeup behavior on the reference machine,
and only one case of it was measured: on the Asahi M2 on AC at Full, 90 s of
watching saw zero uevents of any kind. Unplug, plug, and capacity steps while
discharging are unmeasured.

That matters for correctness, not just cost. `sync_timer` in
`crates/scootbar/src/modules/battery/mod.rs` arms the one-minute timer only
while the shown state is `Discharging`. If `macsmc-ac` and `macsmc-battery`
emit no `power_supply` uevent on unplug, a bar showing Full or Charging has no
timer and no event, so it keeps showing that stale state until some later
uevent happens to arrive.

## What to do

Needs a human at the Asahi box (`ASAHI.local.md`; memory
`asahi-machine-access`), since it needs a physical unplug. Watch the
`NETLINK_KOBJECT_UEVENT` group (for example `udevadm monitor --kernel
--subsystem-match=power_supply`) and record:

1. whether `macsmc-ac` / `macsmc-battery` emit `power_supply` uevents on
   unplug and on plug;
2. whether capacity steps while discharging emit uevents, and for how long
   the run lasted (minutes, and the capacity range covered).

Then decide from the numbers. If unplug is silent, the design must add a
low-rate timer in the plugged states too (Full, Charging, Not charging), and
the cost line in `docs/scootbar/cli.md#battery` changes with it. If unplug and
plug are reliable but capacity steps are silent, the current timer is right
and the module header's "pending human step" note can be replaced by the
measurement.

## Not in this ticket

A GPU or non-Asahi machine's behavior (other drivers differ), and time
remaining estimation (decided against in the module's reference).
