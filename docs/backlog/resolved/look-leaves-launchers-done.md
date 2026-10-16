---
title: "Look leaves for swaylock, fuzzel and the pickers"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-15"
---

# Look leaves for swaylock, fuzzel and the pickers

Filed 2026-10-12, child of [`live-looks`](../core/live-looks.md). Serves
**daily-drive**.

## The gap

swaylock runs with `-C ${swaylockConfig}`, a store path
(`idle-home.nix:83`); fuzzel's flags are inlined into the launcher and picker
scripts (`keys-home.nix:72-79,139-141`, `apps-home.nix:67-73`) and the
capture chooser (`capture-home.nix:41-42,69-76`).

## What to do

Original plan, superseded: the Resolution below explains why neither app is
pointed at the look file.

- swaylock: `-C <state>/look/swaylock` (next lock).
- fuzzel: `--config <state>/look/fuzzel.ini` instead of inlined flags (next
  launch), the launcher, pickers and capture chooser alike.
- Per-format quoting (section 3.5): fuzzel's `launch-prefix` and `terminal`
  keys are exactly why a font name must never carry a newline.

## Evidence to record

A swap observed at the next lock and the next launcher open; a quoting test.

## Not in this ticket

scootlaunch (its own plan, reading the `launch.toml` fragment).

The design this implements against: [live looks](../../../dev/research/live-looks-design.md).

## Resolution (2026-10-15)

PR #631: `scoot-look` renders `fuzzel.ini` (the seven CLI roles as a
`[colors]` table in `rrggbbaa`) and `swaylock` (the fourteen
`lockTheme` leaves as `key=value` in `rrggbb`) from the resolved look;
`apply`, `build` and `validate` render and check all five files
(`RENDER_VERSION` 7), and the standalone `scoot-look build` renders
them for the Nix path. No `[apps.fuzzel]` / `[apps.swaylock]`
overrides in v1: there is no layer loader for these apps, so an
override would have nothing to agree with. Nix takes the state link
at runtime (`nix/modules/look-state.nix`, the same rule `scoot look`
resolves): the launcher, the clipboard picker, the network and
bluetooth pickers and the capture chooser read the same seven colors
out of `<state>/look/fuzzel.ini` as CLI flags once a swap renders it
(`nix/modules/look-leaf.sh`, one helper for all five call sites),
and the new `scoot-locker` wrapper concatenates
`<state>/look/swaylock` with the user's own locker settings into one
private runtime file (look first, settings after, so user keys win per
key); with no applied look every command runs exactly as before. The
user's own configs stay the base throughout: fuzzel is never pointed
at the look file (its `--config` replaces the user's config instead
of layering over it), and neither is swaylock (its `-C` does the
same). Review fix: the first version did point both apps at the bare
look files, silently dropping the user's font, terminal, lock image
and settings after the first swap.

Evidence: renderer exact-output tests (moonrise shapes), the
single-line writer test (newline refused; `=`, `[`, quote and `#`
round-trip), the shipped-looks gate rendering every look into both
targets (seven and fourteen leaves, borders equal the focus ring);
apply/build integration tests asserting all five files with leaf
content, extended to the daemon-driven swap test (all five files at
the link after two live scoped reloads on headless); the pinned fuzzel
1.14.1 binary accepting the rendered `fuzzel.ini` through its own
`--check-config` and refusing a broken one; the helper's exact seven
flags over a fixture look, the real fuzzel validating the fixture user
config it will read at launch (and refusing a broken one, proving the
file is consulted, not bypassed); the live locker merge with the user's
ring color winning last and the merged file parsing in the real
swaylock 1.8.6; `nix eval` of the full module check; the realized
launcher, locker and picker scripts asserting the live-leaf branches
and the unchanged fallbacks, with no bare look file reaching
`--config`/`-C`.
The next-lock / next-launch pickup is by construction (the apps read
these paths at start; the swap itself was observed live on headless,
both looks applied through the scoped reload with the leaves at the
link).
