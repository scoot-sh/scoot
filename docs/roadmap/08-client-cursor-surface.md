---
item: "8"
title: "Client-supplied cursor images (CursorImageStatus::Surface)"
status: "done"
area: "rendering"
pr: 13
commit: null
---

# Client-supplied cursor images (CursorImageStatus::Surface)

~~Client-supplied cursor images (`CursorImageStatus::Surface`)~~ — piece
(a) of the Backlog's "Custom/client cursor support" entry. Piece (b) (a
config-level override for the *fallback* shape) was untouched here and
landed later as item 13, for size and color only — a theme *name* is still
open, for the license reason that entry gives. Like item 7, this landed
ahead of item 6: it's small, self-contained, and item 5 shipped knowing
it was wrong.

Before this, `cursor.rs` drew its procedural 16x16 triangle for every
status except `Hidden`, so a client that handed the compositor a real
`wl_surface` full of cursor pixels (an I-beam, a resize arrow, a spinner)
got the triangle instead. Now `Cursor::element` renders that surface's
subsurface tree via
`render_elements_from_surface_tree`, at the hotspot the client set.
`Named` still draws the fallback and always will from here: there is no
client buffer to draw for it, only a theme name.

Four things the shape of this fix turned on, all verified against the
pinned Smithay rev (`0ff00983`) rather than assumed:

- **The hotspot isn't on the enum at this rev.** `CursorImageStatus` is
  `Surface(WlSurface)` (`src/input/pointer/cursor_image.rs:42`) — the
  hotspot lives in the surface's own `data_map` as a
  `CursorImageSurfaceData`, written by Smithay's `wl_pointer.set_cursor`
  handler. It is therefore **re-read every frame, never cached**: a
  client may call `set_cursor` again with the *same* surface and a
  different hotspot, producing a `CursorImageStatus` that compares equal
  to the previous one, so "the status didn't change" does not imply "the
  hotspot didn't change." Covered by
  `a_new_hotspot_on_the_same_surface_moves_the_cursor`.
- **Two element types, one return.** The fallback needs
  `R: ImportMem` + `MemoryRenderBufferRenderElement`; a surface needs
  `R: ImportAll` + `WaylandSurfaceRenderElement`, and a tree can produce
  more than one. `cursor.rs` now defines a `CursorElement<R>`
  (`render_elements!`, still renderer-generic — this module and
  `decorations.rs` deliberately don't hard-code `PixmanRenderer`) and
  `element()` returns a `Vec` of it: empty for hidden/no-content, one for
  the fallback, N for a tree. `headless.rs`'s `Elements::Cursor` variant
  wraps that instead of the bare memory element.
- **Frame callbacks, or animated cursors freeze.** A cursor surface is
  never in `self.space`, so `render()`'s per-window `send_frame` loop
  could never reach it — and a well-behaved client attaches one frame,
  asks for a callback, and waits. `render()` now also calls
  `send_frames_surface_tree` for the cursor surface, **only** on frames
  that actually drew it (`Cursor::surface()` and `Cursor::element()` both
  go through one private `live_surface()`, so "was it drawn" and "does it
  get a callback" cannot drift apart).
- **Nothing upstream drops a destroyed cursor surface.** Confirmed by
  reading `wayland/seat/pointer.rs`: its destruction handling covers the
  `WlPointer` object, not the cursor surface, so a client that destroys
  its cursor surface without setting a replacement leaves
  `CursorImageStatus::Surface(<dead>)` in place indefinitely. Two
  independent defenses, both exercised: `CompositorHandler::destroyed`
  (newly overridden) calls `Cursor::forget_surface`, which resets to the
  default named shape and requests a redraw under `--tty`; and
  `live_surface()` independently refuses a non-`alive()` surface, so the
  render path stays safe even if `dispatch.rs`'s hand-written
  `Dispatch::destroyed` forwarding (see item 7's maintenance hazard) ever
  regresses. Falling back to the built-in shape rather than to `Hidden`
  is deliberate: the pointer still exists and is still being moved.

`with_states` on a destroyed surface is safe, not lucky: the `WlSurface`
proxy owns an `Arc` of its object data (`wayland-scanner-0.31.11`'s
`server_gen.rs:139-141`), so `Resource::data()` keeps working after
destruction — which is also why Smithay's own `IsAlive for WlSurface` can
unwrap it. The hotspot lookup deliberately does no rendering inside its
`with_states` closure: that guard is a plain non-reentrant `Mutex` and
walking the tree to render re-locks the same one.

Seven new integration-style tests in `cursor/tests.rs` drive a real
`wayland-client` connection through a real `State` (the harness
`dispatch/tests.rs` introduced, extended with a step-at-a-time script
channel so the client and compositor halves can interleave), then render
with a real `PixmanRenderer` and assert on **read-back pixels**, not on
which enum variant came out — a variant assertion would pass on a
wrongly-positioned or wrongly-imported element. Covered: the happy path
and hotspot placement, a re-set hotspot on the same surface, a cursor
surface with no buffer yet (draws nothing — deliberately *not* the
fallback, which would flash a wrong shape between `set_cursor` and the
client's first commit), destroy-while-active, 8 rounds of
hidden/visible alternation, a cursor surface with a subsurface (two
elements), and an `i32::MIN`/`i32::MAX` hotspot — the one
client-controlled integer that reaches element geometry, which
`wl_pointer.set_cursor` takes raw and unbounded. That one places a real
element at the saturated coordinate and asserts nothing is drawn and
nothing panics (a debug build is where the damage tracker's own
`loc + size` would blow up), then that an ordinary hotspot still works
afterwards. Every test was confirmed non-vacuous against at least one of
three negative controls (`live_surface` forced to `None`, `surface_hotspot`
forced to `(0, 0)`, `forget_surface` stubbed to a no-op), each disabling a
different piece of production code and watching the relevant tests fail
with the specific wrong value the old fallback behavior would produce —
see the PR for the raw output.

**Hardware verification** (dev VM, real `--tty` on its `virtio-gpu` KMS
device at 1600x1000, all against `03cd51c` — no production code changed
since; a later commit added one more test and this paragraph). Driven
by a throwaway raw-protocol client (built in `/tmp` on the VM, nothing
committed, no image asset involved — the pixels come from the client over
the wire) that maps a toplevel and hands over a flat-coloured cursor
surface of a chosen size and hotspot. Exact commands and raw
pixel/jiffies output are in the PR description. Summary:
a 24x24 magenta cursor with hotspot `(4,6)` lands pixel-exact around the
pointer; a 128x128 one does too, including clipped against the output's
top-left corner; leaving the client's surface falls back to the built-in
shape (Smithay resets to `default_named()` on focus leave,
`input/pointer/mod.rs:823`); an `--animate` client's frame-callback
counter sat at **0 for 187s** while its cursor wasn't presented, then
climbed at ~46/s the moment the pointer entered, and **froze again**
(1156 → 1156 over 3s) when the pointer left — the callback gating works
in both directions; screenshots caught both animation colours. Destroying
the cursor surface while active, and `kill -9` on the client while
active, both leave the compositor up and drawing the fallback, with no
panic or error in its log.

**Benchmarked** (same jiffies-delta method as item 5), because the render
path changed shape: the fallback now goes through an enum wrapper and
`element()` returns a `Vec`. 12 interleaved reps per side of the
expensive case (150 corner-to-corner pointer jumps, near-full-frame
damage), alternating the pre-change `a1693e4` binary and this one so VM
drift hits both: **before mean 53.58 (42–59), after mean 52.58 (44–62)**
— no measurable difference. A static client cursor parked under the
pointer costs **5 jiffies over 10s**, i.e. no render-loop spin was
introduced (a frame callback only goes to a client that asked for one).
An animated one costs 80 jiffies over 10s while it animates, which is
just what drawing an animation at frame rate costs; `wait-idle` never
settles while one is running, expected and worth knowing.
