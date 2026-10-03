---
title: "Battery: measure uevents on unplug and capacity steps on the Asahi M2"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M5"
resolved: "2026-10-02"
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

## Resolution (2026-10-02)

**Measured on the Asahi M2 Air** (`macsmc-battery` and `macsmc-ac`, the
charger on USB-C port `0-003a`), with a person at the machine. A
`udevadm monitor --kernel --subsystem-match=power_supply` log with
millisecond timestamps ran beside a 15 s sampler of `online`, `status`,
`capacity` and `voltage_now`. The session started at 100%, `Full`, on AC.

| Case | `power_supply` uevents |
|---|---|
| Unplug, 19:46:54 | 8 in 0.5 s (4 `macsmc-ac`, 3 `macsmc-battery`, 1 `tps6598x`) |
| Plug in, 19:47:50 | 16 in 1.4 s |
| Unplug, 19:48:16 | 11 |
| Plug in, 20:50:10 | 17 in 2 s |
| Capacity steps while discharging | **0**, over five steps |
| AC at `Full`, idle | 0 (the earlier Test 14 result) |

The five capacity steps, from the sampler, with no uevent between 19:48:30
and 20:50:10:

| Time | Capacity |
|---|---|
| 20:23:07 | 100 → 99 |
| 20:29:22 | 99 → 98 |
| 20:35:52 | 98 → 97 |
| 20:42:08 | 97 → 96 |
| 20:48:38 | 96 → 95 |

That is 62 minutes on battery (19:48:20 to 20:50:23) at about 340 mA, a step
every 6 to 6.5 minutes once they start. Two things worth knowing:

- **The first step lags the real charge.** The SMC held `capacity` at 100
  until `charge_now / charge_full` had fallen to about 94.6% (about 35
  minutes in). A freshly full battery shows 100 for a long time.
- The load was the idle machine's own; no extra load was applied.

**Decision.** Unplug and plug are reliable and capacity steps are silent, so
the ticket's own rule applies: the current design is right. A uevent is how
the module learns it has started discharging (it arms the timer from the
shown state), and the one-minute timer while discharging is the only thing
that sees the steps. The stale-state worry this ticket was filed for (a
silent unplug leaving a bar showing `Full` or `Charging` with no timer and
no event) does not occur on this driver. No code changed; the module header
and `docs/scootbar/cli.md#battery` now state the measurement in place of the
pending-human note.

**Not covered.** Other drivers (a laptop with ACPI `BAT0` may emit on every
step, or on none), suspend and resume (the narrow unplug-during-suspend hole
in [battery-module-done.md](battery-module-done.md) stands, unmeasured), and
a second machine. The silence is plausibly the driver's choice: a
`power_supply` driver emits only when it calls `power_supply_changed()`, and
this one evidently does so on SMC power events and not on gauge ticks. That
is an inference from the logs, not from the driver source.

**Evidence.** Raw logs, not committed: `events.log` (52 uevent lines
across the four bursts) and `samples.log` (about 240 samples), captured on
the machine under `~/unplug-test/`. The tables above are read from them.
