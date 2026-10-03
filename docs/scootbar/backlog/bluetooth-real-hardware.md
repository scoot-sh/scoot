---
title: "Bluetooth module: real adapter and headset validation"
status: "open"
area: "scootbar"
priority: "medium"
blocked: null
milestone: "M6"
---

# Bluetooth module: real adapter and headset validation

Filed 2026-10-03 as the follow-up the `bluetooth-module` ticket requires.
Serves **daily-drive**: the module merged on VM evidence only (no machine
involved had a Bluetooth adapter), and showing the wrong thing next to a
real headset is a daily-use failure, not a test gap.

## The gap

What the VM proves (see `bluetooth-module.md`'s resolution): the wire
readers against hostile shapes, the state machine against a scripted bus
and a fake BlueZ on a real `dbus-daemon`, the match-rule filtering, the
toggle's `Set`, the draw throttle, the oversize answer keeping the last
state, and module-level idle cost (zero wakeups with no bus, a bus and no
BlueZ, and an idle scripted BlueZ; binary size). None of it touched a
real adapter. A human at the Asahi box (do not run anything there without
one) still has to see, with a real adapter and a real headset:

## What to do

- Pair a headset for real and watch the module: the name shown is the
  headset's, `off`/`on`/`connected` follow the adapter and the link, and
  the click's `Set(Adapter1.Powered)` actually powers the adapter down
  and back up (rfkill and USB-dongle hotplug included).
- Battery: whether this BlueZ reports `Battery1` for the headset at all,
  and whether the charge shown tracks it (BlueZ only sends it for some
  devices, sometimes only while connected and new).
- Scale: what a real `GetManagedObjects` weighs on a machine with the
  author's own paired-device history (the 1 MiB skip path was tested with
  a synthetic 20,000-device answer, never a real one).
- Ratchet rule 2: Waybar's bluetooth module beside scootbar's at the same
  scope (yambar has none), and the Asahi idle/RSS rows the VM cannot give.
- The `menu-command` example in `cli.md` connects something for real
  (the VM only proved the spawn and the list).

## Not in this ticket

No protocol work: BlueZ is used as documented. No new popup widgets (the
native device list stays `popup-network-list.md`'s sibling decision).
