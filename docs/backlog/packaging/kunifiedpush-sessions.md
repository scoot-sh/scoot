---
title: "kunifiedpush distributor runs in minimal sessions at 15 MB PSS"
status: "open"
area: "packaging"
priority: "low"
blocked: null
---

# kunifiedpush distributor runs in minimal sessions at 15 MB PSS

Filed 2026-10-05 from the five-desktop idle benchmark (Asahi M2,
`docs/benchmarks.md`). Serves **daily-drive** (dead weight in every
session's footprint: push notifications nobody on this box uses).

## The gap

`kunifiedpush-distributor` runs in the scoot, niri and KDE sessions
(started through the xdg-desktop-portal stack) at ~15 MB PSS and
~22 MB RSS, zero CPU at idle (`ev/scoot-c1-delta.tsv` and siblings:
`pss0=14989`). It is the single biggest "plumbing" process after the
user manager in the scoot session — bigger than mako, the bar and the
idle daemon combined.

It arrives as a portal dependency (the distributor backs the Push
portal some backends advertise). Nothing in a minimal scoot session
sends or receives push.

## What to do

Find which portal backend pulls it in and whether the Push portal can
be masked out of the session's portal config without breaking the
backends the session needs (file chooser, screencast, Secret,
settings). Expected saving: ~15 MB PSS per session. If it cannot go,
say which backend hard-requires it. Check the other portal fellow
travelers while there (geoclue, `ibus` under the gtk portal in niri
sessions at ~60 MB — that one is niri's recommended stack, not ours,
so note it in the benchmark, not here).

## Not in this ticket

wireplumber/pipewire (~40 MB together): a deliberate system choice,
identical everywhere measured, not portal-pulled.
