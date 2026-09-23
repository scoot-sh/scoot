---
title: "Honour client fullscreen: a real fullscreen state in the layout"
status: "open"
area: "core"
priority: "high"
blocked: null
---

# Honour client fullscreen

Filed 2026-09-22 out of the exporter-widening review (PR #222). Serves
**daily-drive** first (a video player's or game's fullscreen button does
nothing today, on every renderer) and is the precondition for zero-copy
scanout on the GPU tier (see [format gate](./gpu-primary-direct-format-gate.md)
and [candidates](./gpu-scanout-candidates.md), which depends on this).

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
