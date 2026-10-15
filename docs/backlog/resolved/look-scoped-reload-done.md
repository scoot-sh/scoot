---
title: "A look-scoped reload in scoot: the user file's raw snapshot, only the layer swapped"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-15"
---

# A look-scoped reload in scoot: the user file's raw snapshot, only the layer swapped

Filed 2026-10-12, split out of [`look-layer-scoot`](look-layer-scoot-done.md) for
the [`live-looks`](live-looks.md) epic. Serves **both**: a swap must never
apply the user's pending edits, and an agent's swap must not depend on
whatever the user is typing into `config.toml`.

## The gap

A reload re-reads the whole file (`config::reload_from`, `config.rs:756-767`),
so a swap driven through it would also apply pending edits: new
`[autostart]` entries spawn (`reload.rs:101-112`), binds swap, and a
half-written file refuses the reload and rolls the swap back over an
unrelated file. There is nothing to scope against: the reload diffs a fresh
`LoadedConfig` against live `State` fields (`reload.rs:291-301,343-480`) and
keeps no raw user table.

## What to do

Per the design doc's section 4.4:

- Keep a snapshot of the user file's raw table at every successful load
  (startup and plain reload): new state.
- A distinct `Request::ReloadLook`: it merges the freshly read layer
  under that snapshot (the section 4.3 rules) and swaps only what changed;
  no autostart, no binds, no pending edits. A plain reload keeps its
  meaning and its wire shape. SIGHUP stays a plain reload.
- The reply says which scope ran.

## Evidence to record

Tests: a scoped reload not spawning a new autostart entry, not applying a
pending bind, not refusing over a half-written user file; a plain reload
afterwards applying all three; the snapshot advancing only on a successful
load.

## Not in this ticket

scootbar's scoped reload (in [`look-layer-scootbar`](../../scootbar/backlog/resolved/look-layer-scootbar-done.md)).

The design this implements against: [live looks](../../../dev/research/live-looks-design.md).

## Resolution (PR #617, branch `look-scoped-reload`)

Landed as designed, mirroring scootbar's PR #612 semantics. One shape
decision: the snapshot is the user file's raw bytes (`State::user_snapshot`,
`Some` only when those exact bytes parsed), not a parsed table -- so the
scoped path re-parses bytes that already parsed once, and
`config::FileConfig` stays private to `config.rs`. No `PROTOCOL_VERSION`
move: the scoped reload is a distinct request tag (`reload_look`), additive
like any new tag, and the reply's `scope` is omitted on a plain reload;
an older server meets the scoped tag with an ordinary unknown-request
error, never a silent plain reload. (A `scope` field on `reload` would not
hold: serde's internally-tagged unit variants drain and ignore extra
fields, so an old compositor would decode
`{"type":"reload","scope":"look"}` as a plain reload.)

Evidence: seven scoped tests (swap-only + scope named, no autostart
spawn, pending bind waits, half-written file, plain applies all three
after, snapshot-advances-only-on-success, refused-leaves-everything,
no-layer refusal); wire round-trips for both tags plus the older-server
proof (the new tag fails the old request shape's decode, while
`{"type":"reload"}` still decodes to the plain reload); a live headless
e2e over the real socket (plain silent without scope, `reload --look`
swaps only the look, pending bind applies on the plain reload after,
scoped applies over a half-written file, plain refuses it). Full matrix on the dev VM (Linux): nextest + cargo test on
default and xwayland, nextest + cargo test on `--no-default-features`
(all green there), workspace clippy, `fmt --check`. One unrelated
failure stands everywhere including pristine main (verified by stashing
this diff and re-running):
`xwayland refused_cost::a_refused_managed_window_costs_the_server_nothing`
(273 fds vs ≤264) in fd-ledger code this diff does not touch. No Nix is
involved on any path: the snapshot is the plain file, the layer is plain
rendered output, and the e2e ran with only files, a link and
`SCOOT_LOOK_STATE`.
