---
title: "Baselines and spikes: measure the competitors and the four open choices before writing the bar"
status: "open"
area: "scootbar"
priority: "high"
blocked: null
---

# Baselines and spikes

Filed 2026-09-29. Serves **daily-drive**. Nothing else starts until this is
done, the way scootbg's
[dependency choices](../../scootbg/backlog/resolved/dependencies-done.md)
came before its daemon.

## Baselines

On one machine, published in the eventual `docs/scootbar/README.md`: `yambar`
and `waybar` (add `i3status-rust` or `ironbar` if cheap) showing a clock and
workspaces on scoot headless and one other compositor. Rows: idle RSS and PSS,
idle wakeups per minute, CPU over a fixed window, peak memory, binary size,
startup to first frame. Record the method (waited for idle before sampling,
how wakeups were counted) so later runs are comparable. These rows become the
[release gate](lightest.md).

## Four spikes, each a throwaway prototype with numbers

1. **Font rasterizer**: `fontdue`, `ab_glyph`, `swash`; not `cosmic-text`
   (too heavy for a clock and digits). Measure idle RSS and binary size with
   a glyph cache filled from a fixed string set, at 1x and a fractional
   scale. Font discovery is a file path in config; no fontconfig. Decide
   whether the font file is mmapped or read.
2. **Config parser**: the `toml` used elsewhere vs a smaller one
   (`basic-toml`) vs a hand-rolled subset. Size and parse time on the schema in
   [config-cli-and-reload](config-cli-and-reload.md); reject-with-a-clear-error
   behavior on malformed input.
3. **Clock and timezone**: an absolute `CLOCK_REALTIME` timerfd on the next
   minute boundary with cancel-on-clock-set; a TZif reader for `/etc/localtime`
   (`tz-rs`-sized, or hand-rolled) rather than `time`'s local-offset path.
   Prove it across a DST change, a manual clock step and suspend/resume, and
   that idle is exactly one wakeup per minute.
4. **D-Bus**: `zbus` vs libdbus bindings vs a hand-rolled minimal client
   (auth EXTERNAL, `Hello`, `RequestName`, method calls, signal match rules).
   Measure a process that owns a name and receives a signal. The result
   decides [`dbus-client`](dbus-client.md).

Every dependency is licence-checked (MIT-compatible) and recorded with its
alternatives and numbers, as scootbg's record does.

## Done when

The record exists (a `resolved/dependencies-done.md` shaped like scootbg's),
each choice states its numbers and the runner-up, and the baselines table is
published.
