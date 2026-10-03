---
title: "Nix: a wallpaper color beside Stylix's image is refused, and only the log says so"
status: "open"
area: "packaging"
priority: "low"
blocked: null
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
