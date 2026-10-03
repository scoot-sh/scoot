---
title: "The first pointer move after a client ends a popup grab lands at the old position (an IPC click misses)"
status: "open"
area: "core"
priority: "medium"
blocked: null
---

# The first pointer move after a client ends a popup grab lands at the old position (an IPC click misses)

Filed 2026-10-03, found while testing scootbar's tooltips. It predates that
work: no scoot code changed there. Serves **computer use**: an agent's
`click x y` that follows a popup's Escape can land on the wrong target, and a
`click` is the one request whose whole point is where it lands.

## The gap

After a client destroys an `xdg_popup` that held a grab (scootbar's volume
popup, closed by Escape: the client ends it, the compositor did not dismiss
it), the **first** `pointer_move` over scoot's IPC does not deliver the position
it was given. Seen in `WAYLAND_DEBUG` traces of the bar on headless scoot, with
the pointer resting on a module at x = 25 and the bar surface the pointer's focus
throughout, in the sequence the test
`under_a_click_popup_there_is_no_tooltip_and_after_it_there_is`
(`crates/scootbar/tests/tooltip.rs`) now guards with two nudges:

- `{"type":"click","x":800,"y":20,"button":"left"}` delivered
  `wl_pointer.motion(…, 25.0000, 20.0000)` (the old position, not 800), then the
  press and release at that position: the click landed on the module the
  pointer was already on, not the one asked for.
- `{"type":"pointer_move","x":400,"y":20}` delivered no `motion` at all in one
  run and `motion(…, 25.0000, 20.0000)` in another; the move after it (x 800)
  was delivered as asked, and every later one.

The sequence: a click opens the volume popup (`xdg_popup.grab` with the press's
serial), a move to another module of the same bar, Escape (the bar destroys the
popup), then the move or click above. A popup that never grabbed does not
do it: a tooltip (no grab, an empty input region) shown and then left with
`pointer_move` delivers the leave properly, in every run of the tooltip tests.

## What to do

Reproduce without scootbar (a client that opens a grabbing popup, destroys it on
a key, then two IPC `pointer_move`s, with a trace of what its `wl_pointer`
receives), then find why the first motion after the grab's end carries the old
position. The suspect is the grab's own state: Smithay's popup pointer grab is
only unset when the next input reaches it, so the first motion after the client
destroyed the popup may go through the stale grab, which holds the position the
grab began at. Pin it with a test in `crates/scoot` that the first move after a
popup grab ended is delivered as asked.

Until it is fixed an agent can work around it by moving the pointer once, to
anywhere, before a `click` that follows a popup's end (the tooltip tests do:
two moves after an Escape).

## Not in this ticket

A grab the compositor dismissed (a click outside, the session lock): whether
those behave the same is part of the reproduction, not assumed here.
