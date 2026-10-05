---
title: "A side-effect-free way to ask whether the session is locked"
status: "open"
area: "ipc"
priority: "medium"
blocked: null
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
