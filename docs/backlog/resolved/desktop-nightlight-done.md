---
title: "Desktop night light from the flake"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-06"
---

# Desktop night light from the flake

Filed 2026-10-04, child 9 of `desktop-paved-path`. Serves
**daily-drive** (night light is stock on every other lightweight desktop;
its absence is felt nightly, literally).

## The gap

`wlr-gamma-control-v1` is implemented and names `gammastep`/`wlsunset` as
working (`docs/protocols.md:34, 2095-2120`, including `--tty` CRTC-LUT
behavior, `gamma_size` re-reads on hotplug, and gamma surviving session
lock). Nothing in the flake installs, configures, or starts either.

## What to do

Fill the `desktop.nightlight` slot (`wlsunset` default — single purpose,
tiny; `gammastep` fallback for location-based sunrise/sunset — measure both
closures, say why; manual schedule must work with neither geoclue nor
network):

- User unit with a schedule (manual times + optional geoclue/location),
  temperature range, per-`look` warm default; transition time bounded (say
  the default; a 2-hour ramp surprises nobody but must be stated).
- Per-output behavior (one control per output; hotplug re-push — the
  compositor retires controls with `failed` on CRTC moves per
  `protocols.md:2104-2110`, so the daemon must re-read `gamma_size` —
  assert this in the proof on multi-output).
- Edge cases: `scoot msg screenshot` reads pre-LUT (documented ibid.) —
  say so in the docs so agents are not confused; headless/nested (ramp
  accepted, no visible change — no error); disable switch per slot
  (`desktop.nightlight.enable = false`).

Acceptance: eval pins in `nix/tests.nix`; real-login proof on the M2
(schedule fires, temperature visibly shifts, survives lock, re-pushes after
hotplug); docs in `docs/nix.md`.

## Not in this ticket

Full redshift-style color management / ICC profiles; per-app temperature
exceptions.

## Resolution (PR #472)

Landed as `programs.scoot.desktop.nightlight`, on with the profile, in
`nix/modules/desktop.nix` (options), `nix/modules/nightlight-home.nix`
(user unit), `nix/modules/nixos.nix` + `home.nix` (system tool, profile
default, session scope), `nix/tests.nix` (`_nightlightPins` +
`_darwinNightlightPins`) and the site's desktop Night light section
(docs live in `site/`, not the `docs/nix.md` stub).

- Daemon: `wlsunset` default, `gammastep` for location mode — measured,
  not picked by feel, on the Asahi M2 at nixpkgs `8ce4ef6`: wlsunset
  0.4.0 (47.6 MiB closure in 7 paths, 1 new path ~75 KiB over the
  profile's tools, 0 wakeups in 60 s steady state, RSS 2.3 MB,
  `-S`/`-s` with no location/geoclue/network) vs gammastep 2.0.11
  (673.6 MiB in 165 paths, 19 new paths ~33.7 MiB — geoclue,
  modemmanager, polkit, ayatana — ~16 wakeups/60 s, RSS 6.1 MB,
  always needs `-l` or geoclue). Wakeups as `/proc` context-switch
  deltas under a headless scoot, steady state, no transition inside
  the window.
- Schedule: manual 07:00/19:00 default; day 6500, night 3500 fallback
  (vinyl-sunset 3200, moonrise 3400, radial-burst 3500, music-desk 4000
  per look, user value winning, opt-out per target); transition 900 s
  default, 0..7200, manual mode only; gamma 1.0, 0.1..10; lat/lon pair
  for location mode (one without the other refused; gammastep without
  both refused — geoclue is not wired). No toggle bind: wlsunset's
  SIGUSR1 cycles three modes and gammastep answers none, so a shared
  bind would lie on one daemon or the other.
- Per-output: one control per output (seen live: eDP-1 + DP-1 each get
  one, each fails independently); hotplug needs no configuration
  (re-push on `failed` is the daemon's). Headless/nested accepts the
  ramp with no visible change and no error. Disable per slot
  (`nightlight.enable = false`).

Evidence: `checks.aarch64-linux.scoot-modules` +
`checks.aarch64-darwin.scoot-modules` eval-green (every default,
per-look temp, exact unit ExecStart, off/standalone/target-off,
located/gammastep renderings, every refusal by message, NixOS halves,
Darwin nulls); `check-nix` (37 blocks) + `test-snippets` green over
the new docs. Live on the M2 (scoot-test login, generation-100 test
switch with the module from this branch, restored to 99 after per
the standing rule — profile, boot default, flake.lock and result
link all verified back): the unit ran with the exact module flags;
the schedule computed correctly (spy trace: sun trajectory for the
window, `setting temperature to 3500 K`); every push was refused
with `failed` on both outputs — Apple DCP reports gamma size 0 and
no `GAMMA_LUT` on either CRTC (the code comment already knew), so
this hardware cannot show warming and the daemon idles output-less
at 0 wakeups (bounded ~250-switch dawn activity, settled flat, no
spin, NRestarts 0); lock cycle survived (same daemon PID throughout,
session recovered to the identical desktop frame); DP-1 was
attached at login but no plug event occurred during the run (cannot
be unplugged remotely), so re-push after a physical hotplug is
covered by the daemon's `failed` path, not observed here. Screenshots
read pre-LUT — stated on the desktop page and in its own section on
the screenshots page (there is deliberately no warming screenshot:
a capture can never show it).
