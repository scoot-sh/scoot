---
title: "M0: competitor baselines, and only the two spikes the first milestone needs"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M0"
resolved: "2026-09-29"
---

# Baselines and spikes — RESOLVED

Resolved 2026-09-29. What landed:

- **The record**: [dependencies-done.md](dependencies-done.md), shaped like
  scootbg's, which later entries extend.
- **Font rasterizer: `ab_glyph`**, over a mapped font file; runner-up
  `swash` (hinting, at +770 KB and ~1 MB RSS more); `fontdue` rejected
  (+19 MB of heap, it outlines every glyph at load).
- **Clock: an absolute `CLOCK_REALTIME` timerfd with cancel-on-set**,
  measured at exactly one wakeup per minute (10 in 600 s), plus a
  **hand-rolled TZif reader** (+8 KB, 0 mismatches against `zdump` over
  414,100 instants in 598 zones, fat and slim); runner-up `tz-rs`.
  Suspend/resume is covered by the kernel source, not exercised; to confirm
  on hardware in [module-api-and-clock](../module-api-and-clock.md).
- **Baselines** for yambar, Waybar, ironbar and ashell on scoot headless and
  sway headless, published in
  [`docs/scootbar/README.md`](../../README.md#baselines), with the method.
  yambar leads on every row but closure size; nobody idles at one wakeup a
  minute. i3status-rust was not measured (it needs `swaybar`, so sway only,
  and is not a bar by itself; see the record's §4a).
- The spike code, kept for re-derivation, in
  [`docs/scootbar/spikes/m0/`](../../spikes/m0/README.md).

The entry as filed:

Filed 2026-09-29; rescoped the same day for iteration. Serves **daily-drive**.
The first milestone (a clock) waits only on this entry, and this entry is small:
baselines plus the two spikes the clock needs. Every other choice is measured
**by the entry that needs it**, in the manner of scootbg's
[dependency choices](../../../scootbg/backlog/resolved/dependencies-done.md), so
no decision is made months before the code that uses it.

## Baselines

On one machine, published in the eventual `docs/scootbar/README.md`: `yambar`
and `waybar` (add `i3status-rust` or `ironbar` if cheap; **`ashell`, a Rust/iced bar
(GPL-3.0-or-later), added 2026-09-29 at the user's request**) showing a clock and
workspaces on scoot headless and one other compositor. Rows: idle RSS and PSS,
idle wakeups per minute, CPU over a fixed window, peak memory, binary size,
startup to first frame. Record the method (waited for idle before sampling,
how wakeups were counted) so later runs are comparable. These rows become the
[resource ratchet](../lightest.md).

## Two spikes now

1. **Font rasterizer**: `fontdue`, `ab_glyph`, `swash`; not `cosmic-text`
   (too heavy for a clock and digits). Measure idle RSS and binary size with
   a glyph cache filled from a fixed string set, at 1x and a fractional
   scale. Font discovery is a file path (a flag now, config later); no
   fontconfig. Decide whether the font file is mmapped or read.
2. **Clock and timezone**: an absolute `CLOCK_REALTIME` timerfd on the next
   minute boundary with cancel-on-clock-set; a TZif reader for `/etc/localtime`
   (`tz-rs`-sized, or hand-rolled) rather than `time`'s local-offset path.
   Prove it across a DST change, a manual clock step and suspend/resume, and
   that idle is exactly one wakeup per minute.

## Spikes that moved to the entry that needs them

- **Config parser** (`toml` vs `basic-toml` vs a hand-rolled subset):
  [config-cli-and-reload](../config-cli-and-reload.md).
- **D-Bus** (`zbus` vs libdbus vs hand-rolled): [dbus-client](../dbus-client.md).
- **Icons** (glyph font vs built-in vs PNG cost): [icons-and-fonts](../icons-and-fonts.md).

Every dependency is licence-checked (MIT-compatible) and recorded with its
alternatives and numbers, as scootbg's record does.

## Done when

The record exists (a `resolved/dependencies-done.md` shaped like scootbg's,
extended by each later entry), the two choices state their numbers and the
runner-up, and the baselines table is published.
