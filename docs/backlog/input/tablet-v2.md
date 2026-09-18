---
title: "`zwp_tablet_manager_v2` (drawing-tablet input): libinput plumbing, TabletSeat, tool focus/cursor — needs its own design, not a bare advertisement"
status: "open"
area: "input"
priority: "low"
blocked: null
---

# `zwp_tablet_manager_v2` (drawing-tablet input)

Split out of the niche protocol bundle
(`../resolved/protocol-gaps-niche-done.md`, item 1) 2026-09-18: triage
found this is an input epic, not a three-line advertisement, so it gets
its own entry rather than a line in a closed bundle.

## Why it is not a bare advertisement

Smithay carries `TabletManagerState` at the pinned rev, so the global
could be advertised in the same shape as the rendering-hint globals. But
advertising it would be dishonest: a client would bind the manager and
get zero tools, because flexwm has no tablet input path. What is missing,
roughly smallest-first:

1. **libinput tablet-event plumbing** into the motion core (`input.rs`
   handles pointer/keyboard; no tablet-tool events are read anywhere).
2. **A `TabletSeat`** driving `TabletTool` lifetimes from those events
   (Smithay's `tablet_manager` module owns the wire state; the seat owns
   the tool set).
3. **Tool focus and cursor integration**: proximity/focus routing to
   surfaces, absolute positioning through the existing clamp path, and a
   tool cursor (the `TabletSeatHandler` impl in `handlers.rs` exists only
   to satisfy `wp_cursor_shape_v1`'s bound -- see its doc -- and carries
   no tools).

## Demand

None observed: neither the DMS nor the Noctalia probe records any
`zwp_tablet_manager_v2` bind, and no toolkit on the dev VM asks for it.
A Wacom-style tablet on `--tty` hardware plus a tablet-aware client
(Krita, Xournal++) is the scenario that would promote this; until then
it sits at low priority, as the bundle always had it.

## Acceptance (when it is built)

- Real tablet hardware (or `libinput debug-events` + a virtual tablet
  device) drives proximity, motion, pressure and buttons end to end on
  `--tty`, not just the global listing.
- Advertise-honestly rule from the `foot` record: no client may regress
  by the global appearing (a client that drew its own tool cursor must
  not get a worse compositor one).
- Fail-first harness tests per piece, in the shared `test_support`
  shape; no heap allocation on the per-event path, per the project bar.
