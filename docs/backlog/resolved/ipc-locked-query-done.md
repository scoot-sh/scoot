---
title: "A side-effect-free way to ask whether the session is locked"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-05"
---

# A side-effect-free way to ask whether the session is locked

Filed 2026-10-05 from PR #443's review (clipboard, N2). Serves **computer use**
(an agent must know whether the session is locked before acting) and
**daily-drive** (the clipboard must not record while locked).

## The gap

No IPC request reports lock state without side effects. `Response::Ok` is the
only reply that carries `locked` (`crates/scoot-ipc/src/response.rs`), and
while locked the only request answering `Ok` is the mutating `output-power`;
`version`, `outputs`, `windows` and `keyboard` carry no lock state. The
desktop clipboard therefore probes with `action focus-window-id
18446744073709551615` (refused while locked, a no-op otherwise), and every
`focus-window-id`, the probe included, clears `clicked_layer`
(`crates/scoot/src/compositor/ipc.rs`), so each copy drops the keyboard focus
an on-demand layer surface (an open bar dropdown) had.

## What to do

- Add lock state to a read-only reply (a `locked` field on `version` or a
  small `session` query; decide by the protocol's own versioning rules in
  `docs/ipc.md`), available whether or not the session is locked, and a
  `locked`/`unlocked` subscribe event if one does not exist. Bump the IPC
  protocol version per its rules.
- Make `focus-window-id` with an id that matches no window leave
  `clicked_layer` alone (a miss must not change focus state).
- Switch the desktop clipboard's probe (`nix/modules/clipboard-guard.sh`,
  once #443 lands) to the new query.

## Not in this ticket

Locking or unlocking over IPC.

## Resolution

Landed 2026-10-05 in PR #463 (`feat(scoot,scoot-ipc,scootctl,nix):
side-effect-free session lock query and event`), all three parts:

- Chose the small query over the `version` field (a new request tag needs
  no bump on the request half; the two new reply tags move the protocol
  7 → 8): `Request::Locked` answers `Response::Locked{locked}` locked or
  not, and `EventKind::Lock` carries `Response::LockChanged{locked}` once
  per transition. Old clients never receive the new tags; an older server
  answers the new tag/kind with an ordinary `Error`, so a probe against
  one fails open.
- A `focus-window-id` miss keeps `clicked_layer` (and all focus state);
  the old-behavior test is replaced by
  `an_unknown_window_id_leaves_focus_state_alone`, which fails before and
  passes after.
- `clipboard-guard.sh` asks `msg locked` with the fail-open behavior
  kept; the nix module checks (stub scenarios + content pins) are green.

Evidence: nextest workspace 4350 passed (22 `scootbar::*` environmental
failures, package untouched), `cargo test -p scoot` 2127 passed, clippy
and fmt clean, smoke rc=0 (36 ok), `scoot-modules` and `docs-site` nix
checks green, IPC benchmark on the report.
