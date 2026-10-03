---
title: "Brightness module: backlight level, scroll to adjust"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M5"
resolved: "2026-10-02"
---

# Brightness module

Filed 2026-09-29. Serves **daily-drive** on laptops.

## Reading

`/sys/class/backlight/*/brightness` against `max_brightness`. Whether the
value changing is announced (inotify does not fire for many sysfs attributes;
udev change events do for some drivers) is measured, not assumed, on the Asahi
machine and one other. The fallback is to re-read only when the bar itself
changed it, plus once on resume, so it never polls.

**Measured 2026-09-29 on the Asahi M2 (Asahi.md, Test 14).** Writing
`/sys/class/backlight/apple-panel-bl/brightness` as root emits exactly one
`change` uevent per write (kernel and udev), with no value in its properties,
so re-read `brightness` on each. Firmware-originated changes (an SMC
brightness key) were not observed, and inotify on the file was not tried.
`actual_brightness` can differ from `brightness` by 1 (rounding). Raw range
is 0-509 (`max_brightness`), `scale=linear`. Still open: a second machine.

## Writing

Setting the level needs permission: logind's `SetBrightness` over D-Bus (the
[shared client](dbus-client-done.md)) needs no setup on a logind session; a udev
rule and the `video` group work elsewhere; `exec` of `brightnessctl` is the
interim. Ship read-only first and say why writes are gated.

## What to build

Level as a percentage, scroll adjusts by a config step (absolute set,
clamped, never below a minimum that blanks the panel), class `muted` unused.
`Unavailable` on machines with no backlight (desktops, external monitors, which
are DDC/CI and out of scope at first).

## Edge cases

Several backlights (`intel_backlight` plus `acpi_video0`: choose by config),
a raw range that is tiny, a scroll flood (coalesce), suspend/resume.

## Done when

Level shows and scroll adjusts on a real laptop, and there is no periodic
wakeup.

## Resolution (2026-10-02)

Shipped in the `brightness` Cargo feature (on by default):
`crates/scootbar/src/modules/brightness/` (`mod.rs`, `tests.rs`), wired
through the registry, `[brightness]` config (`device`, `step`, the five
interaction keys), `--help`, `docs/scootbar/cli.md` and the
[ratchet row](lightest.md#m5-brightness-module-level-cost-measured-2026-10-02-full-bench-pending).

- **Reading** is the ticket's plan on the battery module's transport: one
  `NETLINK_KOBJECT_UEVENT` tap filtered to whole `SUBSYSTEM=backlight`
  fields, one re-read per turn however many datagrams arrive. The
  2026-09-29 Asahi measurement (one `change` uevent per write, no value
  in its properties) is reused, not re-taken: re-verified 2026-10-02
  that `brightness` reads 107 of 509 and that the bar's rounding (21%)
  agrees with `brightnessctl`'s independent 21%. `actual_brightness` is
  ignored (the set point is what is shown). Firmware-key changes and a
  second machine stay open, as filed.
- **Writing departs from the ticket's "read-only first"**: the build
  order put `invoke` (`raise`/`lower`/`set`) in this module's scope, so
  writes ship now, as direct absolute writes to the device's
  `brightness` file — no daemon, no child, no D-Bus. Permission is the
  gate the ticket named: without a udev rule or the `video` group the
  action is refused naming that (proven live: `steve` on the Asahi box
  is in neither, and the write is refused). `logind SetBrightness`
  still waits for M6's shared client, as filed. Every write is absolute
  from the shown percent and re-read before returning, so no coalescing
  slot is needed (a touchpad flood is N synchronous syscalls, never
  accumulated drift); a write never lands below raw 1, so no scroll can
  blank the panel.
- **No timer exists**, so "re-read only when the bar itself changed it"
  needs no fallback machinery: the synchronous re-read after each write
  is it, and the uevent the write emits finds nothing new. "Once on
  resume" is not built: the bar has no resume notification for any
  module, and the tap survives resume.
- **Unavailable is zero-cost** (no backlight on the dev VM: empty class
  directory, no fds, no width); a runtime removal hides and keeps the
  socket as the appearance watch. Class is always `normal` (`muted`
  unused, as filed). `query` reports `{"percent", "device"}`.
- **Evidence.** `cargo nextest run -p scootbar`: 900 passed (23 new);
  the full feature matrix (clippy `--all-targets` on default, none and
  each feature alone; nextest on default, `--all-features`, none and
  each alone; all 16 no-clock lean combos checked) green; `cargo test`
  green; `fmt --check` clean. The 23 module tests also pass on the
  Asahi M2 against the real backlight. Release binary +65,528 bytes on
  aarch64 against the network module's number, same box and profile;
  still libc/libm/libgcc_s only, no new dependencies.
