---
title: "Layer-destroy review follow-up from PR #34 (`mapped_layers` can retain a dead entry on implicit disconnect) — DONE."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Layer-destroy review follow-up from PR #34 (`mapped_layers` can retain a dead entry on implicit disconnect) — DONE

## Resolution (PR #77)

Fixed as the ticket sketched: a defensive `retain(alive)` in
`CompositorHandler::destroyed` (`handlers.rs`), next to where that function
already clears the cursor surface, the lock surface and the idle inhibitor
— the exact sites the ticket named (`forget_dead_clicked_layer`'s home in
`refresh_keyboard_focus`, and `destroyed`'s cursor/lock clearing).

Deliberately in `destroyed` only, not folded into
`forget_dead_clicked_layer` itself: that function promises to be cheap
enough for every focus refresh (one atomic read), and `mapped_layers`'s
only reader is `commit_layer_surface`'s unmap transition, which a dead
surface never reaches — so sweeping at death time suffices, and the cost
stays per surface death (one walk over the handful of mapped layer
surfaces plus an aliveness probe each), not per focus derivation and
never per frame. No hot-path benchmark, per the ticket's own cost model.

## What the evidence actually showed

The ticket's exact shape — full implicit disconnect with the unlucky
callback order — does **not** reproduce in-harness, and that is stated
rather than papered over: the pinned backend queues disconnect
destructors in object-id order (wayland-backend 0.3.17
`rs/server_impl/map.rs`: `all_objects` walks an index-ordered `Vec`, so
the `wl_surface` deterministically precedes the role object), and
Smithay's role destructor finds the role by id regardless of surface
aliveness (`wlr_layer/handlers.rs`), so `layer_destroyed` still runs and
removes the entry. The disconnect test pins that shape (passing before
and after) rather than the fix.

The reproducible form is the explicit one: destroy the `wl_surface`
while keeping the role object and the connection alive (new
`Step::DestroyLayerWlSurface`), so `layer_destroyed` genuinely never
runs. Pre-fix that retained the dead entry (fail-first run recorded in
the PR); post-fix the sweep drops it, drops only it (two-surface test),
and the full disconnect still ends empty.

## Premises re-derived, not relayed

- **Memory-only, never a wrong neutralize:** server-side `ObjectId` is
  `{ id, serial, client_id, interface }` with `PartialEq` comparing all
  four (wayland-backend 0.3.17 `rs/server_impl/mod.rs:20,59-67`), so a
  stale entry can never equal a live surface.
- **PR #70 relationship:** `neutralize_destroyed_layer_surface` only
  rewrites pending anchors for role-destroyed-but-surface-alive surfaces;
  it never touches `mapped_layers` — no overlap, no double-handle.
- **Sibling audit:** cursor (`forget_surface` + the `live_surface`
  render-path guard), lock (`forget_lock_surface`), idle
  (`forget_idle_inhibitor`) all clear from `destroyed`; popups are
  Smithay-owned with `PopupManager::cleanup` invoked on flexwm's
  display-source settle and grab paths (mid-grab death covered by PR #44
  tests); both foreign-toplevel lists are window-keyed with subscriber
  pruning on `destroyed` plus a dead-list retain in the open path.
  `mapped_layers` was the only list without a disconnect path; nothing
  bigger found, nothing filed.

No README change: no user-facing behavior (internal retention only).

Original entry, left as written:

# Layer-destroy review follow-up from PR #34 (`mapped_layers` can retain a dead entry on implicit disconnect).

One non-blocking finding from the independent review of PR #34
(`fix/layer-surface-post-destroy-commit-kill`, merged 2026-09-14),
recorded so it doesn't get lost:

- On implicit client disconnect with the unlucky callback order
  (`layer_shell.rs`, `wl_surface` dead first), `layer_destroyed` never
  runs, and `CompositorHandler::destroyed` (`handlers.rs`) clears the
  cursor and lock surfaces but not `mapped_layers` — so a dead entry can
  be retained. Verified memory-only, never a wrong neutralize:
  server-side `ObjectId` includes `client_id` plus a generation serial,
  so a stale entry can never equal a live surface. Suggested fix: a
  defensive `retain(alive)` alongside `forget_dead_clicked_layer`. Can
  land any time, e.g. bundled with the next touch of `layer_shell.rs`.
