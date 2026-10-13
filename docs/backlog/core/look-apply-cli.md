---
title: "scoot look apply: the atomic swap, scoped reloads, rollback, list, current and validate"
status: "open"
area: "core"
priority: "high"
blocked: "look-shipped, look-layer-scoot, look-layer-scootbar"
---

# scoot look apply: the atomic swap, scoped reloads, rollback, list, current and validate

Filed 2026-10-12, child of [`live-looks`](live-looks.md). Serves **both**:
the swap itself, and the end of the minimum slice.

## The gap

Nothing switches a look after evaluation. Each app's reload exists (scoot
`Request::Reload`, `scoot-ipc/src/request.rs:115-122`; scootbar's control
socket, `crates/scootbar/src/control/protocol.rs:1-17`), but nothing renders
a look, points the apps at it, or reloads them together.

## What to do

`scoot look list`, `current`, `show`, `validate`, `build`, `apply NAME`,
each with `--json`, per the design doc's sections 3.1, 4 and 7.1:

- The catalog: source looks (shipped, home-manager, system, user-looks) and
  rendered looks kept apart; `apply` renders into
  `$XDG_CACHE_HOME/scoot/rendered/<name>-<hash>/` when no rendered entry
  exists.
- The effectful checks the pure crate leaves to the tool: fonts through
  their carrier, the wallpaper through `scootbg check-image` (new: byte and
  pixel caps from the header before decoding), `scootbar daemon --check`.
- The transaction of section 4.5 under one `flock` (`<state>/look.lock`):
  render and validate, write `look.json` then the link (repairing the link
  from `look.json` whenever they disagree), scoped reloads in order (scoot,
  waiting for the wallpaper; every scootbar of the session), skip what is not
  running, roll back on a refusal, report "rolled back with errors" when a
  rollback reload also refuses, and list shadowed keys.
- The render cache pruned under the lock (active, previous, four most
  recent; never a directory `look.json` or the link names).
- An unwritable `<state>` refuses loudly.
- **Dogfood note in the docs:** on a home-manager machine before
  `look-nix-catalog`, set `programs.scoot.desktop.look = null`,
  `programs.scoot.stylix.enable = false` and `programs.scootbar.stylix.enable
  = false`, or the Nix-written values in the user's own files shadow every
  look; `scoot look current` lists anything still shadowing.
- Docs in the same PR: a new site page for looks (pick, apply, what is live
  and what is next-launch), and the generated CLI pages.

## Evidence to record

Tests: atomicity (a reader racing a swap sees the old or the new directory),
refuse-and-keep, rollback and rollback-with-errors, the lock (two applies
serialize), the link repaired from `look.json` after a simulated crash, a
missing bar skipped, a shadowed key reported, a pending user edit not applied
by a swap. Benchmarks on headless and `--tty`: swap wall time, frame times
across a swap, zero new per-frame allocations after it. The smoke test
applies two looks and checks the background pixel of each.

## Not in this ticket

`next`/`prev`/`random`/`--for` (`look-cycle-preview`), IPC, Nix,
third-party apps, authoring.

The design this implements against: [live looks](../../../dev/research/live-looks-design.md).
