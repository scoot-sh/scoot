---
title: "Desktop capture stack: portal backends, grim/slurp, screenshot keys"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-05"
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

## Resolution (2026-10-05, PR #459)

Landed as proposed, with two adjustments the work found. First, the
chooser runs through a `scoot-screencast-chooser` wrapper script
rather than a long `chooser_cmd`: xdpw parses its config with inih
(200-character lines), which cut a fully-flagged command mid-flag
live on the M2 (fuzzel refused a truncated `--selection-color` and
the cast died with `no output found`). Second, a third screenshot
bind (`ctrl+print`, region to clipboard) beside the two file binds,
so the "standard three" are all bound.

What runs with the profile: the portal backends system-wide
(xdg-desktop-portal-wlr 0.8.4+, `-gtk` for the file chooser and the
rest, the `scoot` backend selection on the system config and the
per-user one), PipeWire for the cast, grim 1.5.0+ plus slurp for the
keymap's three binds (`Print` every output to dated files in
`~/Pictures`, `Shift+Print` a region to file, `Ctrl+Print` a region
to the clipboard), and the output chooser xdpw asks before each cast
(a dmenu list through fuzzel by default, click-to-pick through
slurp, or no picker on a fixed output — themed by the look unless
`theme.targets.capture.enable` opts out). Too-old backends fail eval
(the flake's loud-at-eval style). `scoot msg screenshot` is
unchanged and documented as the agent path (site `desktop` page,
which replaces the old `docs/nix.md` reference). Window sharing
falls back to screens (scoot captures outputs only): stated plainly
on the site page, with what a user sees in Meet's picker.

Evidence: `nix build .#checks.aarch64-darwin.scoot-modules` (Mac)
and `.#checks.aarch64-linux.scoot-modules` (Asahi M2) green
(packages, version floors with fail-before pins, binds, chooser and
script contents); `nix build .#docs-site` green (snippet/nix-parse
gates); live on the M2 in a `scoot-test` login (seat free,
`--override-input` switch only, lock and tree restored after):
portal stack on demand (1.22.1 + wlr 0.8.4 + gtk 1.15.3), Firefox
`getDisplayMedia` streaming eDP-1 (2560x1600) and DP-1 (1920x1080)
at once (two ACTIVE xdpw→firefox PipeWire links), an IPC screenshot
showing the shared screen inside the `<video>`, grim per output,
the portal Screenshot returning a URI, the `Print` bind writing
`~/Pictures`, locked captures holding only lock pixels, and zero
portal CPU over 30 s idle (no portal processes at all before first
use). The M2 was left on its pre-proof generation (`nix-store --gc`
after, `flake.lock` byte-identical).
