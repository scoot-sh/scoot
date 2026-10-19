---
title: "scootlaunch: `--run-id` and the documented JSON schema for agents"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M7"
resolved: "2026-10-19"
---

# scootlaunch: the agent interface

Filed 2026-10-12 from the [scootlaunch design](../../../dev/design/scootlaunch.md#the-agent-interface). Serves
**computer use**.

## What to do

- `--run-id ID [--action A]` through the same spawn path, without a surface.
- The JSON schema for `--list`, `--query` and `--run-id`, documented on the site and on the
  agents page.

## Acceptance

An agent-shaped test over the headless harness: list, query, run, then find the window with
`scoot msg windows`. The exit codes and error objects are pinned.

## Docs (same PR)

Part of this ticket's work, not a follow-up: update `site/src/content/docs/scootlaunch/cli.md` with the full agent JSON schema and an example an agent can paste. Task-first, type/default/example/reload per option, `> **Symptom:**` boxes, snippet gates green (`site/OUTLINE.md`). A change with no user-visible effect says so in the PR instead.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.

## Not in this ticket

UI changes.
