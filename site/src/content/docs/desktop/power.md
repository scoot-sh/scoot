---
title: Power
description: "Power profiles, lid and low-battery suspend, and a charge limit (opt-in)."
---

Batteries, lids and charge caps come on only when you ask: unlike
the [idle policy](./idle-lock.md), nothing here runs with the profile. Lid-close
suspend and an 80% charge cap change what the machine does — with real
consequences on a box you drive remotely — so enabling the profile
must not smuggle them in:

```nix title="configuration.nix and home.nix"
programs.scoot.desktop.power.enable = true;
```

What each default does for your battery:

| Default | Battery effect |
|---|---|
| `balanced` profile (PPD's boot state) | none on Apple silicon — there is no PPD driver there (no `platform_profile`, no EPP; the cores already run `schedutil` with the deep idle state), so the daemon owns the bus name for widgets and changes nothing. On Intel/AMD with a driver, the middle ground between the other two. On Apple silicon the charge cap and the lid suspend below are what save battery, not profiles |
| lid close → `suspend` | stops all draw (s2idle on Apple silicon), at the cost of the session — including SSH — sleeping under you |
| docked lid close → `lock` | keeps drawing for the external screen, behind the lock |
| low battery (2%) → `suspend` | RAM stays powered on a dying battery — data safety, not savings |
| charge cap 80% | longevity, not runtime: sitting at 100% on the charger is what wears the cell most |

To stretch a flight on driver hardware: switch to `power-saver`
(`Super+p`, below), dim earlier (`idle.dimTimeout = 60`), and let
the lid suspend as it already does. On Apple silicon there is no
driver for profiles to steer (below), so the charge cap and the lid
suspend here are what save battery. To
hold a charge longer on the shelf, the 80% cap is the whole trick.

## Profiles

`power-profiles-daemon` runs as a system service, owning
`org.freedesktop.UPower.PowerProfiles` on the system bus, which is what
applets and shells read, driver or not. Switch one keypress at
a time:

| Press | Does |
|---|---|
| `Super+p` | cycle power-saver → balanced → performance (`scoot-power-profile cycle`) |

```sh
scoot-power-profile status
scoot-power-profile set power-saver
powerprofilesctl list
```

On Apple silicon the daemon runs a placeholder driver offering
`power-saver` + `balanced` — both no-ops there, so `set` to either
succeeds while changing no CPU behavior, and only `performance` is
refused (loudly, with the daemon's own error). `Super+p` (`cycle`) steps only
between the profiles the daemon lists, so the bind never fails silently
from the keymap; `set` to an unlisted profile stays loud
for terminal use. Where no driver honors the daemon at all, drop
it and its bind entirely:

```nix title="configuration.nix and home.nix"
programs.scoot.desktop.power = {
  enable = true;
  profiles.enable = false;
};
```

TLP and `auto-cpufreq` stay out: both conflict
with the daemon (its module refuses them at eval), and neither has
an Apple-silicon backend.

Holding a profile across plug events is opt-in — the daemon itself
holds whatever is set, treating a profile as your intent rather
than power state:

```nix title="configuration.nix and home.nix"
programs.scoot.desktop.power = {
  enable = true;
  profileOnAC = "performance";
  profileOnBattery = "power-saver";
};
```

Either half alone works (`null` keeps the current profile); the
switch applies on boot too (udev coldplug fires for the present
state), while a manual switch mid-session stays until the next plug
event.

## Lid, power key, low battery

The exact rule, with the profile's docked twin beside it:

| Event | Default | Meaning |
|---|---|---|
| lid close | `suspend` | the laptop sleeps (s2idle on Apple silicon) |
| lid close while docked or multi-output | `lock` | the session stays up behind the external screen — never suspends |
| lid close on external power, no dock | `suspend` | a charger is not a screen |
| power key | `suspend` | short press (long press is the firmware's) |
| battery at 2% | `suspend` | through UPower, which already sees the battery |

```nix title="configuration.nix and home.nix"
programs.scoot.desktop.power = {
  enable = true;
  # Stay awake on lid close (needs the idle policy's locker to mean
  # anything), and power off instead of suspending at 2%:
  # lidSwitch = "lock";
  # lowBattery.action = "PowerOff";
};
```

Lock-before-sleep ordering: the idle policy's `before-sleep` runs
the locker first — swayidle holds logind's delay inhibitor and
waits (`-w`), so `systemd-suspend.service` starts only once the lock
is up. Every sleep path here (lid, power key, low battery) goes
through logind suspend, which that inhibitor covers — the policy
adds no sleep that bypasses it. Suspend keeps the user manager (no
logout; `KillUserProcesses` stays false), so agents and multiplexers
survive it — while an SSH session sleeping under you does not
resume by itself. Working remotely through a lid close: hold it off
for the session, or make the rule permanent:

```sh
systemd-inhibit --what=handle-lid-switch sleep 1d
```

Hibernate is not wired: s2idle is the only sleep state the
reference hardware has, and its swap is zram (no persistent image
to hibernate into) — the UPower default (`HybridSleep`) would fail
there instead of sleeping, which is why the default suspends.
`Hibernate` and `HybridSleep` stay accepted values for hardware with
persistent swap and a deeper sleep state. No idle timer ever
suspends: the idle child owns idle timing and deliberately
suspends nothing (see [Idle and lock](./idle-lock.md)) — only the
battery percentage trips here.

## Charge limit

The battery normally stops at 80%: `charge_control_end_threshold`
on `macsmc-battery` (Apple silicon) or the first supply that owns
the node (`BAT0` on most laptops — one config travels). Sitting at
100% on the charger is what wears the cell; 80% is the day-to-day
default, and the rest covers the exceptions:

```sh
scoot-charge status
scoot-charge toggle
scoot-charge full-once
scoot-charge limit
```

A full charge once (`toggle`, until the next unplug), and an
automatic refill to 100% after thirty minutes on battery (a trip
needs the range), dropping back after a day straight on the
charger:

```nix title="configuration.nix and home.nix"
programs.scoot.desktop.power = {
  enable = true;
  # A lower cap on a named battery, an hourly trip, a half-day trip end:
  # chargeLimit.limit = 70;
  # chargeLimit.battery = "BAT0";
  # chargeLimit.fullAfter = 3600;
  # chargeLimit.tripEndsAfter = 43200;
  # No cap at all on this box:
  # chargeLimit.enable = false;
};
```

Root re-syncs on every AC change and every five minutes (a missed
event is caught within one interval). Where the hardware has no
charge-control node the service logs one line and exits cleanly —
inert, not refused, since one shared config deploys to machines
with and without the node. A limit outside 1–100, or a blank
battery name, fails evaluation instead.

The state reaches the bar through its `push` module — place the
cell and give it the toggle, and the fill unit (which runs when the
bar starts, so a fresh login does not wait for the next sync) does
the rest:

```nix title="home.nix"
programs.scootbar.settings = {
  right = [ "charge" "battery" "clock" ];
  push.charge.on-click.exec = [ "scoot-charge" "toggle" ];
};
```

The button reads `80`, `full` or `trip` (text, which is what reads
in the bar's default font), with the explanation in its tooltip —
no polling, every change pushed. Without the look there is nothing
themed here: the button inherits the bar's own colors.

Every value is an option, applied on rebuild/switch (the services
restart; no re-login):

| Option | Type | Default | Meaning |
|---|---|---|---|
| `desktop.power.enable` | bool | `false` (never with the profile) | run the daemon, the lid/power-key/low-battery policy and the charge service |
| `desktop.power.profiles.enable` | bool | `true` | run the profiles daemon behind `Super+p` (`false` drops the daemon and its bind — for driverless hardware, where the charge limit and the lid policy are what save battery) |
| `desktop.power.profileOnAC` / `.profileOnBattery` | enum or null | `null` (hold) | profile to select on that power state (`"performance"`, `"balanced"`, `"power-saver"`); needs `profiles.enable` |
| `desktop.power.lidSwitch` | logind action | `"suspend"` | lid close (`"lock"` needs the idle policy's locker) |
| `desktop.power.lidSwitchDocked` | logind action | `"lock"` | lid close while docked or multi-output — never suspends |
| `desktop.power.lidSwitchExternalPower` | logind action | `"suspend"` | lid close on external power without a dock |
| `desktop.power.powerKey` | logind action | `"suspend"` | power key |
| `desktop.power.lowBattery.percentage` | int (0–5) | `2` | battery percent tripping the action (above 5 UPower silently uses its own triple) |
| `desktop.power.lowBattery.action` | enum | `"Suspend"` | the trip (`"PowerOff"`, `"Hibernate"`, `"HybridSleep"`, `"Ignore"`) |
| `desktop.power.chargeLimit.enable` | bool | `true` with the policy | cap the charge (inert without the sysfs node) |
| `desktop.power.chargeLimit.limit` | int (percent, 1–100) | `80` | the percent the battery normally stops at |
| `desktop.power.chargeLimit.battery` | string or null | `null` (first node found) | which battery (`"BAT0"`, `"macsmc-battery"`) |
| `desktop.power.chargeLimit.fullAfter` | int (seconds) | `1800` | on-battery time tripping the refill (`0` disables the trip) |
| `desktop.power.chargeLimit.tripEndsAfter` | int (seconds) | `86400` | on-charger time ending the trip (`0` keeps it until unplug) |
| `desktop.keys.binds.powerProfile.enable` | bool | `true` | bind the cycle (`false` leaves `Super+p` unbound) |

Troubleshooting, by symptom:

- *`Super+p` does nothing.* The slot renders that bind only with
  `power.enable` beside the keymap (opt-in, never with the
  profile), and not with `power.profiles.enable = false`; any of
  those leaves the combo unbound. Then check the
  daemon: `powerprofilesctl` from a terminal — outside the session
  it still lists profiles, so a failure there names the daemon's
  own cause.
- *`performance` is refused on Apple silicon.* The
  placeholder driver offers `power-saver` + `balanced` only, both
  no-ops there. The daemon still owns the bus name for widgets — it
  changes no CPU behavior. (To drop the daemon and its bind entirely, set
  `profiles.enable = false`.)
- *Closing the lid suspends over SSH.* That is the default doing
  its job — hold it off per session (`systemd-inhibit`, above) or
  set `lidSwitch = "lock"` (needs the idle policy's locker) or
  `"ignore"`.
- *Closing the docked lid suspends.* Something beat both docked
  rules to logind: your own logind setting wins over the profile's
  (both children default to `lock`).
- *Low battery never suspends.* The trip needs UPower awake and
  the percentage reached: `systemctl status upower` plus
  `upower -d` (the battery must list with a percentage).
- *The charge never passes 80%.* `scoot-charge
  status` names the mode (`trip` refills on its own; `toggle`
  fills once). Past 80 with the mode already `limit`, the sysfs
  node stopped answering: read it
  (`/sys/class/power_supply/BAT0/charge_control_end_threshold`)
  and the service log (`systemctl status scoot-charge-sync`).
- *The bar button is empty.* The cell needs placing (the two
  lines above) and the fill unit needs running:
  `systemctl --user status scoot-charge-push`.
