---
title: "Desktop night light from the flake"
status: "open"
area: "packaging"
priority: "low"
blocked: null
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
