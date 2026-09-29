---
title: "Volume module: level, mute, scroll to change, click to mute"
status: "open"
area: "scootbar"
priority: "medium"
blocked: "pointer-and-interactions, icons-and-fonts"
milestone: "M5"
---

# Volume module

Filed 2026-09-29. Serves **daily-drive**, and is one of the two modules
(with [network](network-module.md)) that must exercise the module API before it
is frozen.

## Source

PipeWire and PulseAudio both answer the PulseAudio native protocol (via
`pipewire-pulse`). Options, decided by measurement:

1. **Interim: `exec`** around `pactl subscribe` plus `wpctl get-volume` on each
   event. Works today; costs a child process and its memory.
2. **Native**: a minimal PulseAudio-protocol client on the bar's `poll` loop
   subscribing to sink events, no libpulse. Fuzz the parser (it reads bytes from
   a same-user socket); cap frame sizes.
3. **PipeWire native**: heavier; only if 2 cannot do what is needed.

## What to build

- Default sink volume and mute, updated on subscription events; the default
  sink changing (headphones plugged in) is an event too.
- `on-scroll-up/down` step (config, default 5%), `on-click` toggle mute,
  `on-right-click` a mixer command; maximum (100% or allow over-amplification).
- Class `muted`; an icon by level (see [icons](icons-and-fonts.md)).
- `query` returns `{volume, muted, sink}`; `invoke raise|lower|toggle-mute`
  ([agent-interface](agent-interface.md)).
- A separate `microphone` variant (source volume, mute) shares the code.

## Edge cases

No audio server running (`Unavailable`, retry on the socket appearing, not by
polling), the server restarting mid-session, a scroll flood at a touchpad's
rate (coalesce to one set per frame, and set absolute values rather than
accumulating relative ones that can drift), volume set to a value the server
clamps.

## Done when

Scroll and click change and show the level within a frame on a real
PipeWire session, the interim and native paths are measured against each other,
and idle cost is zero events with no audio activity.
