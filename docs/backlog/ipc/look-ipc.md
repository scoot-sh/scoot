---
title: "IPC: look-apply and looks requests, a look event, answered off the loop"
status: "open"
area: "ipc"
priority: "high"
blocked: "look-apply-cli"
---

# IPC: look-apply and looks requests, a look event, answered off the loop

Filed 2026-10-12, child of [`live-looks`](../core/live-looks.md). Serves
**computer use**: an agent, or a Mac driving a VM, can pin a look and know
when it is on screen.

## The gap

`scoot look apply` acts on local files, but the `scoot` binary on a Mac is
a client only that drives a compositor in a VM over the socket
(`site/src/content/docs/msg/index.md:40-41`). Without a socket request a
remote agent cannot change the look, and a screenshot test cannot know the
swap has landed.

## What to do

- `Request::LookApply { name, for_ms }` → `Response::LookApplied { name,
  applied, refused }`; `Request::Looks` → the catalog and the active look.
  Additive requests in the existing style; the reply variant moves
  `PROTOCOL_VERSION` the way `Reloaded` did (`crates/scoot-ipc/src/lib.rs:49-50`).
- `EventKind::Look` on `Subscribe`: `{ name }` after each swap.
- The compositor never renders or validates on its loop thread: it spawns
  `scoot look apply NAME --from-compositor` as a child (the pattern of the
  scootbg run, `crates/scoot/src/compositor/wallpaper.rs:219-243`) and answers when
  the child exits, off the loop the way `WaitIdle` is answered
  (`crates/scoot/src/compositor/ipc.rs:65,725`). One swap at a time; a second request while
  one runs is refused, not queued.
- `scoot msg look-apply NAME`, `scoot msg looks`; docs in the same PR
  (`site/src/content/docs/msg/requests.md`, `events.md`).

## Evidence to record

Tests: the reply arrives after the wallpaper is on screen, a refused look
answers `error` with the reason and changes nothing, a concurrent request is
refused, the event fires once per swap. A Mac-to-VM run recorded: `scoot
msg look-apply` from the Mac, then `scoot msg screenshot`, both images
attached.

## Not in this ticket

Golden images ([`look-golden-screenshots`](../testing/look-golden-screenshots.md)).

The design doc this implements against: [live looks](../../../dev/research/live-looks-design.md).
