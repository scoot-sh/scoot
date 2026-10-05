---
title: "Desktop profile: programs.scoot.desktop.enable plus a look choice"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-04"
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

## Resolution

Landed as `feat(nix): desktop profile` (PR #432): `nix/modules/desktop.nix`
(shared option subtree + the three look palettes), wiring in
`nix/modules/home.nix` / `nix/modules/nixos.nix` (session entry, wallpaper
defaults, portal config, the `[xwayland]` knob, `desktop.greeter` as an
alias for `programs.scoot.greeter`) and `nix/modules/scootbar.nix` (the
bar half reads the profile — the profile never sets across the module
boundary, since a conditional set of an undeclared option fails eval
whatever the condition is). Look leaves sit at `mkOptionDefault`, below
Stylix's `mkDefault`: user values win per key, Stylix wins where present.
`vinyl-sunset` sets no `[wallpaper]` keys (its illustration cannot be
committed); the session shows the flat `background_color`.

Evidence: `nix build .#checks.aarch64-linux.scoot-modules` and
`.#checks.aarch64-linux.scootbar-modules` green on the Asahi M2 (eval pins
plus rendered-file content checks, including an unknown `look` failing
with `not of type 'null or one of "vinyl-sunset", "music-desk",
"radial-burst"'`); tree-wide `nixfmt --check` clean (CI's
`git ls-files | xargs nix fmt -- --check` form). Real greeter-started login
as `scoot-test` with `look = "radial-burst"`: wallpaper + ring + bar
colors on screen (screenshot pixels: `#31a9e5` ring 149947,
`#fdef1d` clock text, `#241721` bar zone), all three units active.
