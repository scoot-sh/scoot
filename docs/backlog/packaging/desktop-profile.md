---
title: "Desktop profile: programs.scoot.desktop.enable plus a look choice"
status: "open"
area: "packaging"
priority: "high"
blocked: null
---

# Desktop profile: programs.scoot.desktop.enable plus a look choice

Filed 2026-10-04, child 1 of `desktop-paved-path`. Serves
**daily-drive** (one switch for a working desktop) and **computer use** (an
agent enabling the same profile in a VM gets the human session's daemons).

## The gap

The flake gives `programs.scoot` (compositor + session + greeter),
`scootbg`, and `programs.scootbar` as three separate enables with no
umbrella: a user assembles a desktop from `docs/nix.md` prose. See the epic
for the inventory.

## What to do

Add `programs.scoot.desktop` to both modules (NixOS + home-manager),
initially wiring only what exists today: session entry, greeter option
passthrough, wallpaper, bar, portal config, XWayland package choice.

- `desktop.enable = true` turns on the session wiring (`session.enable`)
  and the bar + wallpaper defaults; `look = "vinyl-sunset" |
  "music-desk" | "radial-burst" | null` (default `null`: no theming)
  applies that example's palette to every piece the flake owns today
  (compositor `[appearance]`, bar `colors`, Stylix left as the override
  path where present — user values win per key, the `mkDefault`
  pattern from `nix/modules/home.nix:313-372`).
- One boolean + package override per *future* slot, declared now as
  disabled no-ops (e.g. `desktop.idle.lock.enable = false`,
  `desktop.notifications.enable = false`), so later children only fill
  bodies, never rename options (the native-replacement contract).
- NixOS side owns system services/packages; HM side owns user units, binds
  and theme files; either side alone degrades (follow the existing
  `package = null` files-only precedent).
- Edge cases: `look` naming an unknown value is an eval error naming the
  valid ones; `enable` without `programs.scoot.enable` asserts loudly (as
  the greeter's assertions do, `nixos.nix:366-377`); the profile never sets
  `defaultSession` / autologin / replaces the login screen (never-strand
  rule).

Acceptance: eval pins in `nix/tests.nix` (enable turns on session+bar units;
`look` renders the example palette into compositor + bar config; user value
wins per key; unknown look fails eval; profile without `enable` asserts);
docs in `docs/nix.md` (a "Desktop profile" section with the
enable+look snippet); a real greeter-started login on the M2 showing the
look applied.

## Not in this ticket

Any new daemon (children 2-12 fill the slots); compositor changes; the
non-Stylix GTK/Qt/dark-mode derivation (child `desktop-theme-look`).
