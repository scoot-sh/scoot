---
title: "Theme the notification daemon from the look behind a target"
status: "open"
area: "scootbar"
priority: "medium"
blocked: null
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

The `msg reload --look` leaf itself (see `notify-look-reload.md`).
