---
title: "scootnotify: the control socket, `msg`, JSON, `subscribe` and `wait`, scoped `reload`"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M7"
resolved: "2026-10-19"
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

## Docs (same PR)

Part of this ticket's work, not a follow-up: update `site/src/content/docs/scootnotify/cli.md and site/src/content/docs/msg/` with the control socket requests and the `scootctl`/`scootnotify` subcommands. Task-first, type/default/example/reload per option, `> **Symptom:**` boxes, snippet gates green (`site/OUTLINE.md`). A change with no user-visible effect says so in the PR instead.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.

## Not in this ticket

DND, history and rules (`notify-dnd-history-rules`).

## Resolution

Shipped in PR #684 (`feat/notify-control` branch): the control socket
through `scootui` `control` (`list`, `dismiss`, `invoke`, `reload`
plain and `scope = "look"`, `subscribe`, client-side `wait`,
`version`), the protocol-1 JSON view documented on the site, and the
`msg` CLI. A refused reload keeps the running config, state and look;
a scoped reload never reads the user file.

Evidence (dev VM, aarch64, rustc 1.97.1): `cargo nextest run -p
scootnotify` 178 passed and `cargo test -p scootnotify` full green on
the opposite runner (both with `SCOOTNOTIFY_REQUIRE_DBUS_DAEMON/SCOOT/
SWAY=1`); `cargo clippy -p scootnotify --all-targets -- -D warnings`
and `cargo fmt --check -p scootnotify` clean; no `libc` crate in the
normal tree and a 1315552-byte release binary linking only
libc/libm/libgcc_s; the fuzz workspace locked-check clean; five
mutation proofs red-then-green (subscriber cap, dismiss-unknown,
protocol mismatch, bad-reload-keeps-config, unknown verb). Bugs found
by testing, fixed: e2e clients missing the `AddMatch` rule, `wait`
conflating timeout with EOF, the subscribe pin tripped by an
overtaking probe event.
