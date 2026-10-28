---
title: "Theme the notification daemon from the look behind a target"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-27"
---

# Theme the notification daemon from the look behind a target

Filed 2026-10-21. Serves **daily-drive**: the look themes every profile
slot except scootnotify, which always applies its notify layer.

## The gap

`programs.scoot.desktop.theme.targets.notifications.enable = false`
keeps mako's own style but does nothing beside the default scootnotify:
the look's own notify layer always applies
(`nix/modules/desktop.nix` says so, pointing here). There is no opt-out,
no per-key story, and no M2 real-login proof for the scootnotify look
path (nothing in-tree exercises it end to end).

## What to do

Add a `targets.notifications` switch for the scootnotify layer (a
value in `settings` winning per key, like mako), or document why the
layer stays unconditional. Prove it with the live look fragment plus a
headless real-daemon run; leave the M2 real-login proof to the reload
ticket.

## Not in this ticket

The `msg reload --look` leaf itself (landed: see
`resolved/notify-look-reload-done.md`). This ticket keeps only the
target-per-daemon remainder: beside scootnotify the look's notify layer
still applies unconditionally (`targets.notifications.enable = false`
keeps mako's style but does nothing beside the default daemon).

## Resolution

Landed as a switch, not a documented-unconditional layer. Read from the
code before choosing:

- The daemon's look layer (`<state>/look/notify.toml`: colors, font size,
  opacity, icon theme) and its config file (geometry, timeouts, rules) do
  not share keys, so "a value in `settings` wins per key, like mako" has
  nothing to win against. The per-key override of a look value is the
  look's own `[apps.scootnotify]` table. The switch is all or nothing.
- The switch is a config key, `look = false` in `notify.toml` (default
  `true`), not a command-line flag. The unit, the D-Bus activation file
  and the NixOS unit all start the daemon, so a flag would need all four
  sites, and a changed `ExecStart` restarts the unit (dropping cards and
  history) where a changed file reloads in place (`X-Reload-Triggers`).
  A non-Nix user can set the key by hand and sees no change by default.
- `programs.scoot.desktop.theme.targets.notifications.enable = false`
  writes `programs.scootnotify.settings.look = false` at `mkDefault`,
  beside the scootnotify daemon only, gated on the target alone (not on a
  look being declared: a look applied at runtime still has a layer).
- With `look = false`, `msg reload --look` answers ok (with a log note)
  rather than refusing, so `scoot look apply` reports no failure.

Evidence is in the PR. The M2 real-login proof stays with the reload
ticket's follow-up.
