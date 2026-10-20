---
title: "scootnotify: do not disturb, the history ring and `restore`, `[[rule]]` with `exec`"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M7"
resolved: "2026-10-19"
---

# scootnotify: DND, history, rules

Filed 2026-10-12 in the #579 fix round, split out of `notify-control`. Serves **daily-drive**.

## What to do

From the [design](../../../dev/design/scootnotify.md#do-not-disturb-history-rules):

- the DND mode and `msg dnd`;
- the in-memory history ring, with `msg history` and `msg restore`;
- `[[rule]]` matching and actions, with `exec` under the bar's exec bounds (at most 4 running,
  argv, process group, pidfd).

## Acceptance

- DND holds and releases correctly, with critical cards bypassing it.
- History caps and `transient` exclusion.
- Rule order and merging.
- A rule `exec` flood bounded at 4 children with RSS and fds flat.

## Docs (same PR)

Part of this ticket's work, not a follow-up: update `site/src/content/docs/scootnotify/configure.md (rules, history, do-not-disturb)` with rule syntax with examples, history limits, DND behavior. Task-first, type/default/example/reload per option, `> **Symptom:**` boxes, snippet gates green (`site/OUTLINE.md`). A change with no user-visible effect says so in the PR instead.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.

## Not in this ticket

The bar feed.

## Resolution

Shipped in the `feat/notify-dnd-history-rules` PR: the DND mode (`msg
dnd on|off|toggle|status`, criticals through while
`dnd_allows_critical` holds), the in-memory history ring (`msg
history`, `msg restore`, `history_max` caps, `transient` excluded,
pixels never stored, a new card with a new id on restore), and
`[[rule]]` matching (app, desktop entry, `class.` category prefixes,
urgency, summary substrings; later keys win) with actions
(`set_urgency`, `timeout_ms`, `hide`, `history = false`, per-card
`output`, `dnd_bypass`, and `exec` under the spawner bound: argv not
shell, own process group, pidfd reaping, at most 4 running). The
control protocol rides at version 2 (`dnd`, `history`, `restore`,
the `dnd` event and reply); the site `configure.md` and `cli.md`
document every key, verb and view.

Evidence (dev VM, aarch64, rustc 1.97.1): `cargo nextest run -p
scootnotify` 217 passed and `cargo test -p scootnotify` full green on
the opposite runner; `cargo clippy -p scootnotify --all-targets -- -D
warnings` and `cargo fmt --check` clean; the fuzz workspace
`cargo fetch/check --locked` clean; a 500-notify rule-`exec` flood
held RSS 5356 to 5360 kB with fds 8 to 8. Every pin mutation-proved
(exec bound, DND holds, history exclusion, later-wins, reap, 0600
reservation, transient, protocol bump, zero-alloc repaint).
