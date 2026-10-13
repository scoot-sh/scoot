---
title: "A look-scoped reload in scoot: the user file's raw snapshot, only the layer swapped"
status: "open"
area: "core"
priority: "high"
blocked: "look-layer-scoot"
---

# A look-scoped reload in scoot: the user file's raw snapshot, only the layer swapped

Filed 2026-10-12, split out of [`look-layer-scoot`](look-layer-scoot.md) for
the [`live-looks`](live-looks.md) epic. Serves **both**: a swap must never
apply the user's pending edits, and an agent's swap must not depend on
whatever the user is typing into `config.toml`.

## The gap

A reload re-reads the whole file (`config::reload_from`, `config.rs:756-767`),
so a swap driven through it would also apply pending edits: new
`[autostart]` entries spawn (`reload.rs:101-112`), binds swap, and a
half-written file refuses the reload and rolls the swap back over an
unrelated file. There is nothing to scope against: the reload diffs a fresh
`LoadedConfig` against live `State` fields (`reload.rs:291-301,343-480`) and
keeps no raw user table.

## What to do

Per the design doc's section 4.4:

- Keep a snapshot of the user file's raw table at every successful load
  (startup and plain reload): new state.
- `Request::Reload` gains a scope: `look` merges the freshly read layer
  under that snapshot (the section 4.3 rules) and swaps only what changed;
  no autostart, no binds, no pending edits. A plain reload keeps its
  meaning. SIGHUP stays a plain reload.
- The reply says which scope ran.

## Evidence to record

Tests: a scoped reload not spawning a new autostart entry, not applying a
pending bind, not refusing over a half-written user file; a plain reload
afterwards applying all three; the snapshot advancing only on a successful
load.

## Not in this ticket

scootbar's scoped reload (in [`look-layer-scootbar`](../../scootbar/backlog/look-layer-scootbar.md)).

The design this implements against: [live looks](../../../dev/research/live-looks-design.md).
