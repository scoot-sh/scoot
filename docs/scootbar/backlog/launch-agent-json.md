---
title: "scootlaunch: `--run-id` and the documented JSON schema for agents"
status: "open"
area: "scootbar"
priority: "low"
blocked: "launch-spawn"
milestone: "M7"
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

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.

## Not in this ticket

UI changes.
