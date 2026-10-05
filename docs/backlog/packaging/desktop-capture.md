---
title: "Desktop capture stack: portal backends, grim/slurp, screenshot keys"
status: "open"
area: "packaging"
priority: "medium"
blocked: null
---

# Desktop capture stack: portal backends, grim/slurp, screenshot keys

Filed 2026-10-04, child 5 of `desktop-paved-path`. Serves
**daily-drive** (screenshots, screenshare in calls, file choosers) and
**computer use** (agents screenshot via portal or `scoot msg`).

## The gap

The portal *config* is provided (`portals.enable`,
`resources/scoot-portals.conf`: ScreenCast/Screenshot → `wlr` needing
xdpw ≥ 0.8.0, avoid 0.8.3; rest → `gtk`) but no module installs the
backends (`xdg-desktop-portal`, `-wlr`, `-gtk`) or `grim` (≥ 1.5.0,
required — `scoot-portals.conf:33-36`). No screenshot binds exist in the
defaults; `grim`/`slurp` are on the user's PATH by luck.

## What to do

Fill the `desktop.capture` slot:

- Install the portal backends as the profile's system packages with version
  floors (xdpw ≥ 0.8.4 — 0.8.3 stalls recordings per the conf comments;
  `grim` ≥ 1.5.0 which speaks only ext-image-copy-capture). Eval-fail or
  loud-warn on too-old pins — say which, per the flake's loud-at-eval
  style.
- Screenshot binds in the defaults (region via `slurp`+`grim`, full output,
  copy-to-clipboard vs save-to-file — the standard three; say the exact
  binds and where files land under XDG).
- `scoot msg screenshot` stays the agent path (no change); document when to
  use which in `docs/nix.md`.
- Edge cases: locked session (captures hold no locked pixels — compositor
  behavior, assert it in the proof); multi-output (per-output capture);
  screencast permission dialog flow via the `gtk` backend (FileChooser +
  browser screenshare smoke on the M2).

Acceptance: eval pins in `nix/tests.nix` (packages present, version floors,
binds resolve to installed binaries); real-login proof on the M2
(screenshot bind → file, region select, a browser screenshare session);
docs in `docs/nix.md`.

## Not in this ticket

Recording UI beyond portal wiring (OBS/xdpw screencast path working is
enough); color pickers.
