---
title: "scootnotify: the control socket, `msg`, JSON, `subscribe` and `wait`, scoped `reload`"
status: "open"
area: "scootbar"
priority: "low"
blocked: "notify-popups"
milestone: "M7"
---

# scootnotify: the control interface

Filed 2026-10-12 from the [scootnotify design](../../../dev/design/scootnotify.md#the-control-interface-and-what-an-agent-gets);
split in the #579 fix round. Serves **computer use**: a stable JSON view and `wait`.

## What to do

The control socket through `scootui` `control`, with these verbs: `list`, `dismiss`, `invoke`,
`reload` (plain, and `scope = "look"` per #578 §4.4, answering applied or refused), `subscribe`,
`wait` and `version`. The JSON object is versioned and documented on the site in this PR.

## Acceptance

- Every verb tested end to end.
- `wait` returns on a match and times out.
- A reload with a bad file keeps the running config and state; a scoped reload ignores a
  half-edited user file.
- The subscriber and connection caps hold.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.

## Not in this ticket

DND, history and rules (`notify-dnd-history-rules`).
