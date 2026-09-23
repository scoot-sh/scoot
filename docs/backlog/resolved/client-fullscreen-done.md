---
title: "Honour client fullscreen: a real fullscreen state in the layout — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Honour client fullscreen — RESOLVED

RESOLVED 2026-09-22 (PR #223):
a per-window fullscreen state in `scoot-core` (`world/fullscreen.rs`), wired
to all four entry points. What was decided, where the rules live:

- **Semantics** (the doc on `scoot_core::Action::ToggleFullscreen`, user
  prose in `docs/protocols.md#fullscreen`): niri-like. A fullscreen window
  covers its output's whole area -- gaps, focus ring, bar exclusive zones --
  whenever its column is the focused one of its output's active workspace
  (`World::fullscreen_on`). Its column keeps its strip slot at full output
  width, so focus/workspace switching work unchanged; leaving restores the
  arrangement exactly (preset untouched, entry `view_x` restored). A
  fullscreen window is always its column's focused window (at most one per
  column); focusing a stacked sibling, consume/expel, and moving it to
  another workspace/output end it (no scroll restore); `move-column` and
  `move-window` within the column keep it; ignored moves change nothing.
- **Drawing, pointer and keyboard agree**: while a window covers an output,
  that output's `top` layer is hidden from all three
  (`layer_shell::above_windows`); `overlay` and the lock stay above; no ring,
  no rounded clip.
- **Entry points**: xdg `set/unset_fullscreen` (always answered with a
  configure; honoured while locked as the window's own state; output hint
  honoured only for the focused, unlocked window; discarded on unmap), wlr
  foreign-toplevel `set/unset_fullscreen` + the `fullscreen` state bit on
  v2+ handles, IPC `toggle-fullscreen` and `set-fullscreen ID on|off` plus
  the `fullscreen` snapshot field (additive, no `PROTOCOL_VERSION` bump),
  and `Super+f`.
- **Found on the way**: `observe_frame` paired each commit with the latest
  configure *sent* rather than the one the commit acked, so the first
  full-size frame after leaving fullscreen read as a refusal to shrink and
  widened the column for good (reproduced by a harness test; the old
  pairing fails it). It now reads Smithay's committed state.

Direct scanout was out of scope; [candidates](./gpu-scanout-candidates-done.md)
consumes `World::fullscreen_on` / `Placement::fullscreen`. Original entry
below, kept verbatim.

Filed 2026-09-22 out of the exporter-widening review (PR #222). Serves
**daily-drive** first (a video player's or game's fullscreen button does
nothing today, on every renderer) and is the precondition for zero-copy
scanout on the GPU tier (see [format gate](./gpu-primary-direct-format-gate-done.md)
and [candidates](./gpu-scanout-candidates-done.md), which depends on this).

## What is missing

scoot has no fullscreen state anywhere:

- `xdg_toplevel.set_fullscreen` / `unset_fullscreen` have no handler (the
  `XdgShellHandler` default no-op), so the client is never configured
  fullscreen and never gets the `fullscreen` state bit.
- The wlr foreign-toplevel `SetFullscreen`/`UnsetFullscreen` requests are
  accepted and ignored on purpose (`foreign_toplevel_management.rs` module
  doc: scoot-core has no concept of fullscreen, and inventing one to fill a
  protocol enum was rejected as speculative semantics).

So no window can become the whole-output, nothing-above-it element that
Smithay's primary-direct assignment needs -- on default config the focus
ring (3 px) and the non-black background also rule out every other shape.

## What to decide

Layout design first, wire format second -- this is the decision the
foreign-toplevel module deferred, and it belongs in `scoot-core`:

- What fullscreen *means* in a scrolling-column layout (the window leaves
  its column and covers the output? its column's position is kept for
  unfullscreen?), per output, and what it does to focus, workspace switching
  and the other columns while it holds.
- What is drawn over it: the focus ring and layer-shell surfaces (bars,
  notifications -- `top` layer hidden, `overlay` kept, as most compositors
  do?), the lock screen (always above).
- Which requests may enter it: the client's own `set_fullscreen` (with its
  output hint), the wlr foreign-toplevel request, an IPC action and a
  default bind.

## Evidence expected

Harness tests for the state transitions (enter/leave, output hint, focus
change, workspace switch, client unmapping while fullscreen), a live check
under `--tty` with a real client (e.g. `mpv --fs`), README/configuration/IPC
docs for any new action or bind. Direct scanout is *not* part of this
ticket; the candidates ticket consumes the state.
