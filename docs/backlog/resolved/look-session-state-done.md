---
title: "Per-session look state: headless and --private-look sessions never touch the user's choice"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-15"
---

# Per-session look state: headless and --private-look sessions never touch the user's choice

Filed 2026-10-12, split out of [`look-layer-scoot`](look-layer-scoot-done.md) for
the [`live-looks`](../core/live-looks.md) epic. Serves **computer use**: an agent's
headless test session must be able to swap looks without repointing the
user's real desktop (D10).

## The gap

The look state is per user (`$XDG_STATE_HOME/scoot`) while sessions are per
display (the bar's socket is `scootbar-$WAYLAND_DISPLAY.sock`,
`crates/scootbar/src/control/paths.rs:1-8`). Without a scope, a swap inside a
headless test would show up on the user's desktop at its next reload.

## What to do

Per the design doc's section 4.2:

- `--tty` and `--nested` keep the user state: `--nested` is the webtop
  deployment (`site/src/content/docs/agents/webtop.md:17`) and the "try it
  first" path, and their swaps must survive a restart.
- `--headless` uses a private state under
  `$XDG_RUNTIME_DIR/scoot-look-<display>/`, seeded at startup from the user's
  `look.json`; any backend opts in with `--private-look`.
- Only a private session exports `SCOOT_LOOK_STATE`, to its direct
  children (beside `XCURSOR_*`, `state.rs:1850-1851`). The default path is
  deterministic, so nothing is added to `resources/scoot-session`'s
  environment imports, and systemd user units always act on the user state.
- `scoot look` resolves `<state>` as `--state`, `$SCOOT_LOOK_STATE`,
  `$XDG_STATE_HOME/scoot`, `~/.local/state/scoot`.
- Docs: the flag on the CLI pages; the private-state rule on the looks page.

## Evidence to record

Tests: a headless session's swap leaving the user state byte-identical; a
nested session's swap persisting across a restart; `--private-look` on
nested isolating it; the seed copied at startup.

## Not in this ticket

The IPC requests (`look-ipc`).

The design this implements against: [live looks](../../../dev/research/live-looks-design.md).

## Resolution (PR #616, branch `look-session-state`)

Landed as designed, with two deviations worth naming. First, there is no
`--state` flag yet and no `scoot look` command (both are `look-apply-cli`'s):
the shared resolver takes an explicit dir for the future flag, and
`state_dir_from_env(explicit)` is the call the tool will use. Second, the
compositor ignores an inherited `$SCOOT_LOOK_STATE` when resolving the
*user* state and strips it from non-private sessions' children, so a
`--nested` session started inside a private session's terminal still starts
on the user state instead of following its parent's private dir; the design
names the export but not this shielding, and without it the isolation would
leak one level down.

What landed: `scoot-look`'s `state` module (the state-dir rule once, for
scoot, scootbar and scootbg; the `look.json` record with its grammar;
atomic writes; the link repair from the record; the seed; the bounded
`look.lock`), with scoot's and scootbar's resolvers delegating to it and
scoot's fragment reader sharing its gated read. `--headless` is private by
default, any backend opts in with `--private-look`, the seed copies the
user record and link byte for byte without writing the user dir, and only a
private session exports `SCOOT_LOOK_STATE` to its direct children (units
still see the user state; `resources/scoot-session` is untouched).

Evidence: 27 `scoot-look` state tests (resolution order, private-dir
grammar, record validation, FIFO/oversize refusals, a reader racing a swap
seeing only whole records, lock serialization and bounded refusal, link
repair, seed isolation), the `--private-look` CLI tests on all three
backends, and `crates/scoot/tests/look_session.rs` (a headless session's
seed exporting and isolating, the user state byte-identical across a swap).
A nested session's swap persisting across a restart and `--private-look` on
nested are covered by the resolution unit tests, not live: no host
compositor exists in CI or on the dev VM for a nested session to attach to.

## Follow-up (PR #616 review, same branch)

The review caught six issues, all fixed on this branch before merge:

- The seed left a previous session's record and link in place when the
  user state had neither. The private dir is now wiped and recreated
  owner-only at every seed, so it holds exactly the seed.
- The private dir was never removed and its modes were umask-dependent.
  It is 0700 (files 0600) and removed on a clean exit; a crash or a kill
  leaves it until the next session with that display name reseeds it.
- The module doc claimed startup, sync and apply repair the link and
  every writer takes the lock, but only tests called those helpers. The
  compositor now repairs the session's state dir at startup (the user dir
  for shared sessions, the private dir after the seed for private ones;
  a private session still never writes the user dir), and the doc names
  `look-apply-cli` as the lock/write path's owner.
- The doc said the rule was shared with scootbg, which takes no look
  state. Corrected everywhere it appeared.
- `rendered` now requires an absolute path with no `..` component.
- `write_atomic` uses a unique temp name opened `O_NOFOLLOW|O_EXCL`, and
  the seed resolves a relative link target to absolute.
