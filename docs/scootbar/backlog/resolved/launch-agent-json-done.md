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

## Resolution (2026-10-19, PR #680)

`--run-id ID [--action A]` launches one entry headlessly through the panel's
spawn path with no surface and no token (`crates/scootlaunch/src/agent.rs`
for resolution and the `RunOk`/`RunErr` JSON shapes;
`spawn::launch_entry_report` and its twins, which the panel's `launch_entry`
calls and discards, so both share activation-first-with-Exec-fallback, the
environment rules, the systemd scope, the detached fallback and frecency).
Exact id first, then the unique suffix-insensitive form; hostile ids/actions
refuse with an error object and exit 1, never a launch of the wrong entry.
`site/src/content/docs/scootlaunch/cli.md` carries the full schema with a
paste-ready agent loop, and the agents page links it. Evidence: 298 nextest
green with `SCOOTLAUNCH_REQUIRE_SCOOT=1` plus the opposite runner, clippy and
fmt clean, libc only via xkbcommon, release 1315568 bytes linking only
libxkbcommon past std, three mutation pins red-then-green (see the report).
