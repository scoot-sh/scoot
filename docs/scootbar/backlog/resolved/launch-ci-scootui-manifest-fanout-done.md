---
title: "ci: a scootui Cargo.toml-only change should run the scootlaunch job"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-11-04"
---

# ci: a scootui Cargo.toml-only change should run the scootlaunch job

Filed 2026-10-19. Serves **daily-drive** / **computer use** (pick one, say why).

## The gap

What is wrong or missing, with evidence (file paths, measured numbers).

## What to do

The proposed shape, and the edge cases to pin.

## Not in this ticket

What is deliberately out of scope.

Found reviewing #674. The classify arms fan scootui sources to scootlaunch but a manifest-only change misses it. Add the manifest path and prove with a case-statement simulation.

## Docs (same PR)

Update `site/src/content/docs/scootlaunch/panel.md` if the change is user-visible; otherwise say so in the PR.

## Resolution

Resolved 2026-11-04 with no workflow change: the gap was already closed.
`.github/workflows/ci.yml`'s classify job has a dedicated
`crates/scootui/Cargo.toml)` arm (added in b75242b75, #703) that sets
`scoot`, `scootbar`, `scootlaunch` and `scootnotify`. Proven by running the
real case block (lines extracted from the workflow) against single-path
changes: `crates/scootui/Cargo.toml` -> `scootlaunch=true`; a
`crates/scootui/src/entry.rs` change -> `scootlaunch=true`; a
`crates/scootbar/src/main.rs` change -> `scootlaunch=false`; `README.md` ->
all false. Nothing user-visible.
