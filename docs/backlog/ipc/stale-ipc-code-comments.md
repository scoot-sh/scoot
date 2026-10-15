---
title: "Stale code comments found during docs2 review"
status: "open"
area: "ipc"
priority: "low"
blocked: null
---

# Stale code comments found during docs2 review

Filed 2026-10-14 during the docs2 IPC slice review. Serves **computer use**: these comments sit on the paths agents read when checking IPC semantics.

## The gap

Three stale comments found while verifying `site/src/content/docs/msg/` against code at main (e5b9dd610); the site pages already match the implementation, the comments do not:

1. `crates/scoot-ipc/src/request.rs:53-58` says "Only `--headless --outputs N` can produce another output today; compositing more than one is future work" — false now (per-output screenshots, `primary_id` in `crates/scoot/src/compositor/screenshot.rs:260-262`, multi-output `outputs`).
2. `crates/scoot-ipc/src/response.rs:456-461` says config binds render "in load order" — the implementation sorts config rows by combo (`crates/scoot/src/compositor/binds.rs:73-75`); the docs match the implementation.
3. `crates/scoot/src/cli.rs:2` says requests are "parsed and run through the `scootctl` library crate" — the standalone client is gone (`scoot msg` is the only client).

Open question for the owner: `site/src/content/docs/msg/actions.md` says `move-floating` is a no-op on "a maximized one" too; `crates/scoot-ipc/src/action.rs:240-244` lists only unknown id, tiled, and fullscreen. Confirm against the compositor `MoveFloating` handler and either fix the docs or the code comment.

## What to do

Update the three comments to match the implementation; answer the `move-floating`-on-maximized question and align docs/code accordingly. No behavior change.

## Not in this ticket

The wider docs-bar Symptom-box pass over `msg/` (tracked in the docs2 report).
