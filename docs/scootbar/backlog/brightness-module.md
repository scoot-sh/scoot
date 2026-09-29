---
title: "Brightness module: backlight level, scroll to adjust"
status: "open"
area: "scootbar"
priority: "low"
blocked: "module-api-and-clock, pointer-and-interactions"
milestone: "M5"
---

# Brightness module

Filed 2026-09-29. Serves **daily-drive** on laptops.

## Reading

`/sys/class/backlight/*/brightness` against `max_brightness`. Whether the
value changing is announced (inotify does not fire for many sysfs attributes;
udev change events do for some drivers) is measured, not assumed, on the Asahi
machine and one other. The fallback is to re-read only when the bar itself
changed it, plus once on resume, so it never polls.

## Writing

Setting the level needs permission: logind's `SetBrightness` over D-Bus (the
[shared client](dbus-client.md)) needs no setup on a logind session; a udev
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
