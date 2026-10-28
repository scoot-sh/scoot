---
title: Night light
description: "Warm the screen after dark with wlsunset or gammastep."
---

Warm the screen after dark, on with the profile: the day's blue
fades to amber across the evening instead of snapping at sunset.

```nix title="configuration.nix and home.nix"
programs.scoot.desktop = {
  enable = true;
  # Warmer nights, or a fixed schedule of your own:
  # nightlight.nightTemp = 3000;
  # nightlight.sunrise = "06:30";
  # nightlight.sunset = "21:30";
  # No warming at all on this box:
  # nightlight.enable = false;
};
```

What runs: `wlsunset` as a user unit in `scoot-session.target` (so no
other desktop starts it), driving every output's gamma ramp through
`wlr-gamma-control-v1` — one control per output. The schedule is manual
by default (07:00/19:00, over 15 minutes), with no location, no
geoclue and no network involved. Unlike home-manager's
`services.wlsunset`, which starts in the shared
`graphical-session.target`, this unit starts in scoot's own session
scope, past the display import.

Why wlsunset, measured at the pinned rev (`8ce4ef6`,
`aarch64-linux`: full closures by `nix path-info --closure-size`,
marginals as new store paths over the profile's own tools, idle
wakeups as context-switch deltas over 60 s steady state under a
headless scoot on the M2, RSS as VmRSS while running):

| Tool | Version | Full closure | New over profile | Wakeups in 60 s steady | RSS steady | Schedule with no location |
|---|---|---|---|---|---|---|
| wlsunset | 0.4.0 | 47.6 MiB (7 paths) | 1 path, ~75 KiB (itself — glibc and wayland-client already ship with the profile) | 0 | ~2.3 MB | yes (`-S`/`-s`) |
| gammastep | 2.0.11 | 673.6 MiB (165 paths) | 19 paths, ~33.7 MiB (geoclue, modemmanager, polkit, the ayatana indicator stack…) | 16 | ~6.1 MB | no (always needs `-l` or geoclue) |

Steady state is the whole point of the wakeup column: outside a
transition wlsunset sleeps until the next boundary (zero wakeups —
the maintainer's battery goal, stated in full), while gammastep polls
about every few seconds. And only wlsunset warms on fixed times with
nothing to locate it — gammastep without `-l` reaches for geoclue,
which needs its system service and the network behind it, so the
manual schedule is impossible there. `gammastep` stays as the
location-based fallback (`daemon = "gammastep"` with
`latitude`/`longitude` set): sunrise/sunset computed for where the
box is, instead of fixed times.

The night temperature follows the look — each look warms to its own
default (its palette's warmth: vinyl-sunset 3200, moonrise 3400,
radial-burst 3500, music-desk 4000), a value you set in
`nightTemp` winning per key, `theme.targets.nightlight.enable =
false` keeping the plain 3500 while the rest follows the look. Day
stays 6500 everywhere (changes nothing).

Over the session lock the warming stays: gamma is output state, not
pixels, so locking changes nothing about the ramp ([protocols](../scoot/protocols.md#night-light-wlr-gamma-control-v1)).
A second output warming later is the same path: the daemon holds one
control per output and re-pushes after a CRTC move, so hotplug needs
no configuration. On `--headless`/`--nested` the ramp is accepted
and changes nothing on screen — and no error.

There is deliberately no screenshot on this section: `scoot msg
screenshot` reads the framebuffer, which is pre-LUT and pre-CTM, so a
capture shows the unmodified frame either way. See
[Screenshots](../msg/screenshots.md#pre-lut-captures-show-the-unmodified-frame).
To see the ramp itself, read the CRTC color state back (`drm_info` or
`modetest` on the `--tty` session).

Every value is an option, applied on rebuild/switch (the unit
restarts into the new flags; no re-login):

| Option | Type | Default | Meaning |
|---|---|---|---|
| `desktop.nightlight.enable` | bool | `true` with the profile | warm the screen (the daemon plus its unit) |
| `desktop.nightlight.daemon` | enum (`"wlsunset"`, `"gammastep"`) | `"wlsunset"` | the program behind `enable` |
| `desktop.nightlight.dayTemp` | int (Kelvin, 1000–10000) | `6500` | day color temperature (neutral: changes nothing) |
| `desktop.nightlight.nightTemp` | int (Kelvin, 1000–10000, below `dayTemp`) | `3500` (or the look's, above) | night color temperature (lower is warmer) |
| `desktop.nightlight.sunrise` / `.sunset` | string (`HH:MM`) | `"07:00"` / `"19:00"` | manual schedule (read only without `latitude`/`longitude`) |
| `desktop.nightlight.latitude` / `.longitude` | number or null | `null` (manual schedule) | where the box is, in decimal degrees (set both for location mode; `gammastep` needs both) |
| `desktop.nightlight.duration` | int (seconds, 0–7200) | `900` | how long the day/night transition takes (manual schedule only; `0` snaps) |
| `desktop.nightlight.gamma` | number (0.1–10) | `1.0` | extra gamma multiplier (neutral) |
| `desktop.nightlight.package` | package or null | the daemon's tool (Linux-only: null off Linux) | point at your own build of either daemon (its flags must agree with `daemon`); null with the switch on fails evaluation naming it |
| `desktop.theme.targets.nightlight.enable` | bool | `true` | warm to the look's own night temperature; `false` keeps the plain default (or your value) |

```nix title="configuration.nix and home.nix"
programs.scoot.desktop = {
  enable = true;
  # Sunset where the box is, not fixed times:
  nightlight.daemon = "gammastep";
  nightlight.latitude = 37.33;
  nightlight.longitude = -121.89;
};
```

Force tonight now (wlsunset only): `systemctl --user kill -s
USR1 scoot-nightlight.service` cycles forced-day, forced-night,
then back to the schedule. There is no keybind for it: the cycle has
three modes, while gammastep's SIGUSR1 toggles it on and off, so a
shared bind would do something different on each daemon.

Troubleshooting, by symptom:

- *The screen never warms.* Check the unit:
  `systemctl --user status scoot-nightlight` — and that it started
  with the display (a hand-started session must reach
  `scoot-session.target` with `WAYLAND_DISPLAY` imported,
  [on the desktop page](./index.md#which-sessions-start-the-units)). Then check the clock
  against the schedule: at noon with 07:00/19:00 nothing should be
  warm yet. Run the daemon by hand to hear its own cause
  (`wlsunset -S 00:00 -s 23:59` warms at once in the evening state —
  kill it after, so two daemons do not fight over the outputs).
- *Two daemons fight over the outputs.* Only one control holds an
  output at a time: the second `get_gamma_control` steals it and the
  first hears `failed`. Turn one off (this unit, or home-manager's
  `services.wlsunset`, or a hand-started one) — whichever stays owns
  every output.
- *A screenshot shows no warming.* Expected: captures read
  pre-LUT/pre-CTM (above). Read the CRTC color state
  back instead (`drm_info`, `modetest`).
- *Warms nowhere, ever, on Apple silicon.* The Apple display
  controller reports no gamma LUT (`drm_info` shows no `GAMMA_LUT`
  and gamma size 0 on both CRTCs), but it does expose a per-CRTC
  `CTM` color matrix, which scoot drives instead: the same unit
  warms normally, with the ramp's white endpoints applied as a
  diagonal matrix (exact at the default gamma). If a push is still
  refused with `failed`, check `drm_info` for the `CTM` property —
  without it there is nothing to drive. The unit still runs with
  the right flags either way (check
  `systemctl --user status scoot-nightlight`).
- *`gammastep` exits at once.* Without
  `latitude`/`longitude` it reaches for geoclue, which is not wired.
  Set both (or stay on `wlsunset`, whose manual schedule needs
  none) — evaluation refuses the missing pair before it ever runs.
- *A bad time or temperature fails the rebuild.* The schedule reads
  24-hour `HH:MM`, temperatures sit inside 1000–10000 with the night
  at or under the day, the transition inside 0–7200 seconds — each
  refusal names the switch.
- *Headless or nested shows nothing.* The ramp is accepted
  with no hardware LUT behind it, and no error. The unit still runs
  (and still sleeps between boundaries).
