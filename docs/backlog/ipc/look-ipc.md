---
title: "IPC: look-apply and looks requests, a look event, answered off the loop"
status: "open"
area: "ipc"
priority: "medium"
blocked: "look-apply-cli"
---

# IPC: look-apply and looks requests, a look event, answered off the loop

Filed 2026-10-12, child of [`live-looks`](../core/live-looks.md). Serves
**computer use**: an agent, or a Mac driving a VM, can pin a look and know
when it is on screen. Medium: a pinned config already gives agents
deterministic screenshots; this makes switching remote and awaited.

## The gap

The `scoot` binary on a Mac is a client only, driving a VM's compositor over
the socket (`site/src/content/docs/msg/index.md:40-41`). A swap that touches
local files cannot be reached that way, and
`Request::Action(Spawn "scoot look apply …")` answers before the swap and
never says whether it refused.

## What to do

- `Request::LookApply { name, for_ms }` → `Response::LookApplied { name,
  applied, refused, shadowed }`; `Request::Looks` → the catalog, the active
  look and the session's state directory. The reply variant moves
  `PROTOCOL_VERSION` the way `Reloaded` did (`crates/scoot-ipc/src/lib.rs:49-50`).
- The compositor validates `name` against `[a-z0-9-]{1,32}`, never a path.
- It spawns `scoot look apply NAME --from-compositor` with its own
  `SCOOT_LOOK_STATE` (`crates/scoot/src/compositor/wallpaper.rs:219-243`'s
  pattern) and answers when the child exits, off the loop like `WaitIdle`
  (`crates/scoot/src/compositor/ipc.rs:65,725`). One swap at a time; a second
  is refused.
- `scoot msg look-apply NAME`, `scoot msg looks`; `EventKind::Look` is
  already emitted by the reload (`look-layer-scoot`). Docs in the same PR
  (`site/src/content/docs/msg/requests.md`, `events.md`).

## Evidence to record

Tests: the reply after the wallpaper is on screen, a refused look changing
nothing, a concurrent request refused, a path refused as a name, a headless
session's swap leaving the user state untouched. A Mac-to-VM run: `scoot msg
look-apply` then `scoot msg screenshot`, both images attached.

## Not in this ticket

Golden images.

The design this implements against: [live looks](../../../dev/research/live-looks-design.md).
