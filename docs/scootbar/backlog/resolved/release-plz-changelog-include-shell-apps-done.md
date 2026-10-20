---
title: "release-plz: list scootlaunch and scootnotify in changelog_include"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-19"
---

# release-plz: list scootlaunch and scootnotify in changelog_include

Filed 2026-10-19. Serves **daily-drive** / **computer use** (pick one, say why).

## The gap

What is wrong or missing, with evidence (file paths, measured numbers).

## What to do

The proposed shape, and the edge cases to pin.

## Not in this ticket

What is deliberately out of scope.

Found reviewing #673. Most user-visible work since v0.3.1 is scoped to the new crates, which the generated changelog omits. Add scootlaunch and scootnotify (libraries stay out) and note it in docs/versioning.md.

## Docs (same PR)

Update `site/src/content/docs/scootlaunch/panel.md` if the change is user-visible; otherwise say so in the PR.

## Resolution

2026-10-19, PR #686: `scootlaunch` and `scootnotify` added to the `scoot` package's `changelog_include` in `release-plz.toml`; the libraries stay out. The release PR body now names scootlaunch among the built binaries.
