---
title: "Nix: a wallpaper color beside Stylix's image is refused, and only the log says so"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-04"
---

# Nix: a wallpaper color beside Stylix's image is refused, and only the log says so

Filed 2026-10-03, from the review of PR #407 (the compositor's Stylix
target). Serves **daily-drive**: a Stylix user who wants a solid-color
background gets the Stylix scheme's `base00` instead, with no visible reason.

## The gap

With `stylix.image` set (the normal Stylix flow), the home-manager module
puts Stylix's `image` and `mode` in `[wallpaper]` at `mkDefault`. A user who
writes `programs.scoot.settings.wallpaper.color = "#101014"` gets a section
with both `image` and `color`; scoot passes it through, scootbg refuses it
(`image` or `color`, not both: exit status 2), and the session falls back to
`background_color` with a warning in the compositor log only. Documented in
`docs/nix.md` and `docs/configuration.md` with the remedies (set your own
`image`, or `programs.scoot.stylix.enable = false`).

The module cannot simply drop Stylix's `image` when the user set `color`:
the condition would read the merged `settings.wallpaper`, which the same
block defines (an evaluation cycle, verified by the review).

## What to do

- Break the cycle with plumbing the user sets explicitly: a
  `programs.scoot.stylix.wallpaper.enable` (default true) that turns off just
  the Stylix wallpaper defaults, documented as "set this false to choose your
  own wallpaper color".
- Or make the refusal loud where the user looks: a home-manager assertion is
  impossible for the same cycle reason, so consider a `scootctl` status line
  or a notification for a refused wallpaper section.
- Pin with the nix eval tests (`nix/tests.nix`).

Also, from the same review (nit): `nix/scootbar-tests.nix` still defines a
`base0A` slot and `scripts/scootbar-stylix-test.sh` still inherits it,
though nothing reads it since the accent moved to `base0D`. Drop both.

## Not in this ticket

Changing scootbg to prefer `color` over `image` instead of refusing: a
deliberate strictness (a typo is an error, not silently ignored).

## Resolution

Took the first option: `programs.scoot.stylix.wallpaper.enable`
(default true) in `nix/modules/home.nix` gates just the Stylix
`[wallpaper]` `image`/`mode` defaults. Set it to `false` to choose your
own wallpaper color; the themed `[appearance]` and cursor defaults stay.
The condition reads the explicit switch, not the merged
`settings.wallpaper`, so evaluation stays acyclic (a home-manager
assertion on the merged section would cycle the same way). Documented in
`docs/nix.md` (Stylix section + home-manager options table) and
`docs/configuration.md` (`[wallpaper]`). Also dropped the dead `base0A`
slot from the `nix/scootbar-tests.nix` Stylix stub and its inherit in
`scripts/scootbar-stylix-test.sh` (nothing read it since the accent
moved to `base0D`).

Evidence: `nix build .#checks.aarch64-linux.scoot-modules` green on the
Asahi M2, including new pins (`hmStylixWallpaperOff`: no `[wallpaper]`
table, `wallpaper.enable` off, no scootbg, appearance still Stylix's;
`hmStylixColor`: user `color = "#101014"` with Stylix's image set renders
just the color; file-content check 9b) that fail before the fix
(`nix/modules/home.nix` reverted: `The option
'programs.scoot.stylix.wallpaper' does not exist`, exit 1) and pass
after; `nix eval
.#checks.aarch64-linux.scootbar-modules.drvPath` green (stub still
satisfies every check); `nix fmt -- --check` clean on the three changed
`.nix` files.
