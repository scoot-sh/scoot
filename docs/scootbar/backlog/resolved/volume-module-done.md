---
title: "Volume module: level, mute, scroll to change, click to mute"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M5"
resolved: "2026-10-02"
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
- Class `muted`; an icon by level (see [icons](resolved/icons-and-fonts-done.md)).
- `query` returns `{volume, muted, sink}`; `invoke raise|lower|toggle-mute`
  ([agent-interface](resolved/agent-interface-done.md)).
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

## What landed (PR #376, 2026-10-02)

The `volume` module and its `microphone` twin (`crates/scootbar/src/modules/volume/`,
`microphone/`, Cargo features `volume`/`microphone`, both on by default),
reference in [cli.md](../../cli.md#volume). Decided by measurement: the
**native** client (option 2). The module API is extended, not frozen
(`microphone/` is a second registry id sharing the code; volume and network
have still not both exercised it).

- Default sink (source for `microphone`) level and mute, updated on
  subscription events; a server change re-reads the defaults, so a
  headphone plug-in follows the new default.
- `step` (default 5, 1 to 50) per scroll notch and raise, `max-volume`
  (default 100, 100 to 150) as the cap; click toggles mute and scrolls
  raise/lower with no binding, a binding runs instead;
  `on-right-click` has no default (point it at a mixer).
- Class `muted`, four built-in level icons (muted/low/medium/high) or a
  static configured one; `query` reports
  `{volume, muted, sink}` (`source` for `microphone`); `invoke` takes
  `raise|lower|toggle-mute` through the shared interaction keys.
- One socket while up, one inotify watch while down, no timers ever;
  `Unavailable` shows nothing. Sets are absolute levels (one in flight,
  one coalesced behind it), every answered set re-reads, so a touchpad
  flood cannot drift and server clamping shows as-is.

### Interim vs native, measured on the Asahi M2 (NixOS aarch64, pipewire-pulse 1.6.8)

| Path | Per event | Steady state |
| --- | --- | --- |
| Interim (`pactl subscribe` + `wpctl get-volume` per event) | `wpctl get-volume` 11–13 ms wall (30 runs ×2: 10955, 12730 us/op), ~11 MB transient RSS (sampled) | `pactl subscribe` child 5.2 MB RSS |
| Native (this module) | ~73 µs per server round trip (50× `GET_SERVER_INFO`, two runs: 72.9, 74.5 us/op), parsed in place, no allocation | one socket, no child, no timer |

About 150× the latency and 11 MB transient per event vs none: native wins
decisively, and PipeWire-native (option 3) is unnecessary — the PA
protocol does everything the module needs. `pactl` itself is not installed
on that box (measured via `nix shell nixpkgs#pulseaudio`); the subscribe
child's own wakeups were not counted, which flatters the interim column.

### The wire, mapped against the real server (not just the fake)

A throwaway std-only probe spoke to `/run/user/1000/pulse/native`
(read-only, except two net-zero sets): AUTH answers the version u32,
`SET_CLIENT_NAME` answers its client index (184, 185, … — **not** the
empty ack the other commands send; the WIP parsed it as empty and the
handshake stalled on it), `SUBSCRIBE` acks empty, `GET_SERVER_INFO` is 4
strings, a sample spec, the two defaults, a u32 and a channel map, and a
`GET_SINK_INFO` reply is 1302 bytes here (the 64 KiB frame cap holds it
easily; only the prefix to mute is parsed). Two refusals mapped:
**sets must go by name with index `INVALID_INDEX`** — index 74 with the
name errored (`INVALID`), name-only acked and applied; and a malformed
proplist length is refused, not crashed. The real replies are committed
as fuzz seeds (`fuzz/corpus/volume/real-*`).

### Bugs the cycle caught (all fixed, all with tests)

- `SET_CLIENT_NAME` parsed as an empty ack (handshake stalled; fixed by
  the probe above, `fake::ack_name` pins the u32).
- Sets sent index+name (server `INVALID`; fixed to name-only, tests pin
  `INVALID_INDEX`).
- Subscribe-ack re-read the server info instead of asking for the device
  (extra round trip per handshake; now `DeviceQuery`).
- A refused device query reported `Changed` with nothing shown (now only
  on a real transition; both arms tested).
- `conn.quiet` treated a dropped peer's EOF as a frame (now silence).
- Lean builds broke twice (the icon module's gate, `init` reading the
  other variant's settings field); the feature matrix is green for every
  feature alone.

### Evidence (Asahi, `nix develop`, `CARGO_TARGET_DIR=/tmp/scootbar-vol-target`)

- `cargo nextest run -p scootbar`: 824 passed, 0 failed (incl. the
  corpus replay with 5 real-packet seeds, and `against_a_real_server`
  against live pipewire).
- `cargo test -p scootbar`: all suites ok (712 + satellites).
- `cargo clippy -p scootbar --all-targets -- -D warnings`: clean on
  default, `--no-default-features`, and each of the 9 features alone.
- `cargo fmt --check -p scootbar`: clean.
- `cargo check` of the fuzz workspace: clean; `cargo fuzz run volume`
  200,000 runs, `-max_len=70000`, seed 1: none.
- Live round trip (`SCOOTBAR_TEST_LIVE_AUDIO=1`, gated, net-zero ±1
  step): raise showed in **1.65 ms**, restored to 0.49 after.
- Idle: a subscribed connection polled 60 s with no audio activity: **0
  event wakes, 0 bytes**, 1 voluntary context switch.
- Not run: `nix build`, sway/tty hardware, the Asahi ratchet (as with
  window-title: numbers above are the row).
