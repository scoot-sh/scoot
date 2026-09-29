---
title: "Module API, layout, theme tokens and the clock"
status: "open"
area: "scootbar"
priority: "high"
blocked: "skeleton-layer-surface"
milestone: "M1"
---

# Module API, layout, theme tokens and the clock

Filed 2026-09-29. Serves **daily-drive**.

The contract every module is written against, proven by the first one.

## The trait

A module (one file in `modules/`, one registry line, one Cargo feature):

- `init` probes and returns `Available` or `Unavailable`; an unavailable
  module registers no fds and takes no space, so absent hardware costs nothing.
- `sources` names the fds and timers it wants polled.
- `on_ready(source)` handles one and returns whether the view changed.
- `view(output, &mut View)` fills a small declarative `View`: icon, text,
  state class (`normal`, `warn`, `urgent`, `muted`), optional tooltip. A module
  never touches pixels.
- `on_input` is added in [pointer-and-interactions](pointer-and-interactions.md).

Built once at startup as trait objects (allocation at load or reload only),
never per frame. Dispatch cost is a handful of virtual calls per redraw.

## Layout, theme and text

`left`, `center`, `right` lists of module ids; per-module padding and
spacing; semantic color tokens (bg, fg, accent, dim, urgent, plus the state
classes) so a theme source maps onto them. Text through the rasterizer the
[spikes](resolved/dependencies-done.md) chose (`ab_glyph`), glyph cache lazily filled and bounded
(see [robustness-and-limits](robustness-and-limits.md)), grayscale AA, no shaping. Recompute layout
only when a module's measured width changes; damage only the changed
module's rect.

## The clock

Absolute realtime timerfd on the next minute (second, if configured) with
cancel-on-clock-set, so suspend, NTP steps and DST are handled without
polling; timezone from `/etc/localtime`. Format string from a flag (a config key
later), with a small documented set of specifiers.

M0 measured this design (one wakeup per minute, 10 in 600 s) and chose the
parts; [the record](resolved/dependencies-done.md#what-this-changes-in-the-plan)
lists what to carry over: re-check the clock after arming, port the spike's
TZif reader with its `zdump` check as a fixture test, honour `TZ`, `statx`
the zone file on each wake, and prove suspend/resume on hardware (M0 could
only read the kernel for it).

## First frame first

Paint the bar and the clock before anything else is initialized. A module whose
`init` is slow (a bus connection, a scan) must never block the loop or delay the
first frame; it joins when ready. Slow startup is a recurring bar complaint
(Waybar #1093), and a bar that shows up late is not the lightest one.

## Tests

Pure-drawing snapshot tests (no Wayland), a module test harness that feeds
fake events and asserts on the `View`, a fuzz target over the format string,
and a headless-scoot pixel test showing the time. Measure: idle wakeups are
exactly one per minute.

## Done when

The clock renders on every output and adding a second trivial module is one
file, one line and a test.
