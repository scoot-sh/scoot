---
title: "bump-downstream resolves tags via prefix glob; query the exact ref and peel"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-24"
---

# bump-downstream resolves tags via prefix glob; query the exact ref and peel

Filed 2026-10-24 from the #733 review (R733 F1). Serves **daily-drive**: the downstream bump is the release path that keeps try-scoot and scoot-iso on the published compositor.

## The gap

`scripts/bump-downstream` `scoot_rev_for_tag` resolves via prefix glob
`refs/tags/<tag>*` (`git ls-remote https://github.com/scoot-sh/scoot
refs/tags/<tag>*`). Today safe (no colliding tags; `v0.5.0`/`v0.6.2`
probes resolve exactly), but a future `v0.6.20` alongside `v0.6.2` would
make ls-remote list two refs and could pick the wrong peeled commit.
It fails closed (step 4 lock-rev check refuses, no bad PR opens), but the
fix is one line.

## What to do

Query the exact ref plus its peel (`refs/tags/<tag>` and
`refs/tags/<tag>^{}`), preferring the peeled commit as today. Add a test
or a scripted check that works offline (a local bare repo with `v0.6.2`
and `v0.6.20` tags).

## Not in this ticket

Live dispatch plus downstream PR plus downstream CI (needs
`DOWNSTREAM_BUMP_TOKEN`); the ISO build itself.

## Resolution

2026-10-24: `scoot_rev_for_tag` now asks `git ls-remote` for
`refs/tags/<tag>` and `refs/tags/<tag>^{}` and prefers the peel.
`scripts/test_bump_downstream.py` runs offline against a local bare repo,
including a lightweight `v0.6.2` beside an annotated `v0.6.20`, the shape
the glob got wrong; release.yml's lint job runs it (scoot#738).
