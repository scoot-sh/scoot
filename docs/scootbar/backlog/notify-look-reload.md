---
title: "scoot look apply must send scootnotify msg reload --look"
status: "open"
area: "scootbar"
priority: "medium"
blocked: null
---

# scoot look apply must send scootnotify msg reload --look

Filed 2026-10-21. Serves **daily-drive**: on the new default a look swap
re-renders `notify.toml` but the running daemon ignores it until a
manual reload or restart (a restart drops cards, history and DND).

## The gap

`scoot look apply` has no scootnotify leaf (`crates/` sends no `msg
reload --look`), and the daemon has no config watcher. Docs
(`scootnotify` Nix section, Migrating-from-mako) now say so plainly: a
look swap needs `scootnotify msg reload --look` (or a restart), not a
live reload.

## What to do

Teach the `scoot look apply` leaf to send `msg reload --look` (scoped
reload, keeping cards/history/DND), with a headless test proving the
real daemon re-renders end to end. Then the M2 real-login proof for the
scootnotify look path, which nobody has run yet.

## Not in this ticket

The `targets.notifications` opt-out (see `notify-look-target.md`).
Only implement the leaf if it stays small and safe with tests proving
the headless end-to-end run; otherwise leave this ticket open.
