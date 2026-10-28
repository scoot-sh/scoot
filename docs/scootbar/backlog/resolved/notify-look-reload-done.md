---
title: "scoot look apply must send scootnotify msg reload --look"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-21"
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

The `targets.notifications` opt-out (see `notify-look-target-done.md`).
Only implement the leaf if it stays small and safe with tests proving
the headless end-to-end run; otherwise leave this ticket open.

## Resolution

Landed: `scoot look apply` drives the scootnotify look-scoped reload
itself (protocol 2 `{"protocol":2,"type":"reload","scope":"look"}`, the
`msg reload --look` equivalent, scope receipt checked, 30 s bound, never
a hang). Best-effort leaf semantics: a missing daemon is a reported skip,
a refusal or stall a loud note; when the mako leaf already reloaded the
daemon is mako so no scootnotify reload is attempted. The site/CHANGELOG
"needs a manual reload" disclosures now say swaps apply live. Evidence:
`cargo test -p scootctl look::wiring` 100/100 green plus `cargo nextest
run -p scootctl -p scootnotify -p scoot-look` 763/763 green on the VM
venue, new fake-socket unit rows (good/refused/mismatch/scopeless/stall/
no-socket) plus a real-daemon headless e2e (card survives reload and the
apply leaf under the same pid; bad fragment refuses, daemon keeps the
last good look), clippy `-D warnings` and `fmt --check` green. PR: #711.

Still open: the M2 real-login proof for the scootnotify look path, and
the `targets.notifications` opt-out (`notify-look-target-done.md`, since resolved:
the layer still applies unconditionally beside scootnotify).
