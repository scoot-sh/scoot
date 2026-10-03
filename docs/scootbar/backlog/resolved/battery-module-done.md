---
title: "Battery module: level, charging state, warn and critical classes"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M5"
resolved: "2026-10-02"
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

## What landed (PR #383, 2026-10-02)

The `battery` module (`crates/scootbar/src/modules/battery/`, Cargo
feature `battery`, on by default), reference in
[cli.md](../../cli.md#battery). Both arms of the ticket's measured
question are built: kernel uevents drive it, and a 60 s timer re-reads
while discharging for drivers whose capacity steps are silent.

- Percent from `capacity` (never `charge_now`/`charge_full`), state from
  `status` (`charging`, `discharging`, `full`, `not-charging`, anything
  else `unknown`, never a refusal); `warn` at or below `warn-below`
  (default 20) and `urgent` at or below `urgent-below` (default 10), by
  level alone.
- `batteries = "combine"` (the default: the mean, discharging winning
  the state) or `"first"`; no battery (or an empty `power_supply`
  directory, as on the dev VM) is `Unavailable` with a reason: no fds,
  no width. A runtime removal hides the module and drops the timer; the
  uevent socket stays as the appearance watch, so a reinsert shows again
  with no polling.
- `on-low = { exec = [...] }` stages its command once per downward
  crossing of `urgent-below` (re-armed by rising back above; starting
  below is not a crossing). It is carried out by a new defaulted module
  API method, `Module::take_action`, which the loop performs through the
  same `action::perform` as a binding — the module never spawns, so the
  spawn stays bounded and reaped by the bar's `Spawner`. The API is
  extended, not frozen (volume and network have still not both exercised
  it). The module defines no `invoke` actions of its own, like the clock:
  there is nothing to actuate, and the five interaction keys take
  commands.
- `query` reports `{"percent": 72, "state": "discharging",
  "batteries": 1}`; the tooltip carries `Discharging 72%`.
- Bounds: one short read per sysfs file into a fixed 64-byte buffer (a
  longer file is not a value), `capacity` clamped at 100 and strictly
  digits (overflow skips the battery), at most 32 entries, 64 datagrams
  drained per turn with one re-read (a storm is one redraw per turn), one
  bad directory entry skips itself, never the whole read.

### The open question, measured as far as possible without unplugging

- Test 14's finding replicates: on the Asahi M2 at AC `Full`, 90 s of
  watching a `NETLINK_KOBJECT_UEVENT` tap saw **zero uevents of any
  subsystem** (capacity 100 → 100, status Full throughout).
- Userspace cannot synthesize uevents for tests: a `send` on the uevent
  family fails `EPERM` (measured). The harness tests therefore drive the
  module through a socketpair with crafted NUL-separated datagrams, plus
  a construction-only test of the real netlink socket.
- Still open (needs a human to unplug the machine): whether capacity
  steps while discharging, and plug/unplug, emit uevents. The module is
  correct on either answer (the timer covers silent steps; uevents cover
  plug events). **Pending-human step:** on the Asahi machine, unplug AC
  while running `perl /tmp/uevent-watch.pl`-equivalent (bind group 1,
  print `SUBSYSTEM=power_supply` datagrams) for ~3 minutes as it
  discharges past a percent, then plug back in; record whether capacity
  steps and the unplug/replug each produced a uevent, and paste the
  capacity trace. If even unplug is silent, the design is revisited
  (a timer that also covers the plugged states).
- **Measured 2026-10-02, no longer open**: plug and unplug each emit a
  uevent burst, and capacity steps while discharging emit none (five
  steps over 62 minutes). The module is correct as built; see
  [battery-unplug-uevent-measure-done.md](battery-unplug-uevent-measure-done.md).
- Known narrow hole, recorded not deferred-harm: unplug-during-suspend
  while the shown state is not Discharging leaves the view stale until
  the next `power_supply` uevent (there is no wake source in the fd set,
  and the timer arms from the shown state, so a bar showing Full or
  Charging arms nothing; only a bar already showing Discharging
  self-heals on its next tick after resume). No user-facing harm follows:
  the bar shows the last true state, and the first post-resume event, or
  the timer of a discharging bar, corrects it.

### Evidence (dev VM `cargo`, Asahi reference machine)

- `cargo nextest run -p scootbar`: **926 passed, 0 failed** (26 new
  battery tests: fixtures, crafted uevents, storm coalescing,
  timer-only-while-discharging, removal/return, on-low crossings,
  threshold classes, invented statuses, clamped/strict capacities, the
  Asahi file shape, the real-`power_supply` probe which reads sane on the
  Asahi box and reports `Unavailable` on the battery-less dev VM).
- `cargo test -p scootbar`: all suites ok (814 in the binary, satellites green).
- `cargo clippy -p scootbar --all-targets -- -D warnings`: clean on
  default, `--no-default-features`, and each of the 10 features alone
  (incl. `battery`); every no-clock/no-workspaces/no-window-title arm of
  the CLI help matrix (`battery` × `volume` × `microphone`) check-builds.
- `cargo fmt --check -p scootbar`: clean.
- `CARGO_TARGET_DIR=/dev/shm/sb-batt-target` throughout (the shared
  9p checkout and its target dir untouched); Asahi built under `nix
  develop` into `~/sb-batt-target3`.
- Live on the Asahi M2 (headless scoot + `daemon --left battery`):
  `query battery` →
  `{"batteries":1,"percent":100,"state":"full"}`, text `100%`, class
  `normal`, tooltip `Full 100%`.
- Idle 75 s on AC Full: battery bar voluntary **14→14**,
  nonvoluntary **3→3**, VmRSS 7184→7184 kB; empty bar 5→5 / 0→0. The
  module adds no steady-state wakeup.
- Size (debug, like-for-like `-p scootbar --bin scootbar`, help-verified
  both sides): 37,670,464 without → 38,117,072 with (**+446,608 bytes**,
  no new dependencies). The row for the ratchet.

### Bugs the cycle caught (all fixed, all with tests)

- `the_layout_is_what_is_listed` and `bad_values_name_their_key` used
  `battery` as their unknown-module example; both now use `bluetooth`
  (M6, still unbuilt).
- The `Modules:` help line omitted battery in the four clock arms (the
  list edit missed them while the paragraph landed); caught by the live
  `--help` on Asahi, fixed, and the help test now asserts the list too.
- Two lean-build breaks: the vol/vol+mic/mic-only help arms lost their
  `concat!`, and the vol+mic arm's cfg; the feature matrix is green for
  every feature alone.
- `track_low` cloned the hook argv on every re-read; now only on a
  crossing. One bad directory entry (`to_str()?`) failed the whole read;
  now it skips itself.
