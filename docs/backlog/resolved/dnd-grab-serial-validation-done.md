---
title: "`wl_data_device.start_drag` accepts any serial, so any client can start a pointer drag whenever a button is held — DONE."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# `wl_data_device.start_drag` accepted any serial — DONE

Split out of `protocols/popup-grab-serial-validation.md` (resolved as
`resolved/popup-grab-serial-validation-done.md`), which deliberately did
not cover it: same "answer once rather than three times" question, different
answer.

## What landed

A serial gate in `WaylandDndGrabHandler::dnd_requested` (`handlers.rs`),
checked for pointer drags after the existing grab preconditions (a pointer
exists, `grab_start_data()` holds) and before anything is installed:

- The seat must be the compositor's own seat (same check `popup.rs`
  applies; in practice flexwm owns exactly one, so this never fires).
- The serial must be one flexwm delivered to the *requesting* client -- the
  data source's own client, resolved by downcasting the generic `S: Source`
  to the two concrete sources Smithay's dispatch can pass (`WlDataSource`,
  or the origin `WlSurface` when `source` is NULL) -- as a **key or button**
  event, within the last 16 such deliveries and the last 10 seconds
  (`contains`, the strict half of `input/interaction.rs`; the activation
  gate byte-for-byte, deliberately *not* the popup gate's looser
  `contains_seen` -- see answer 1).
- Refusal cancels the source like every other refusal, and -- since
  cancelling posts no protocol error -- logs at `warn` with the client and
  serial, so a drag that never starts is diagnosable instead of silent.

## The three answers the ticket deferred, each settled with evidence

**1. Serial semantics: the serial names the implicit grab, and the window
is not moot.** `wayland.xml` (`wl_data_device.start_drag`) says the serial
is "the serial number of the implicit grab on the origin" and "the client
must have an active implicit grab that matches the serial" -- i.e. the
button press being converted, confirmed by the pinned Smithay rev's
dispatch (`data_device/device.rs` only calls `dnd_requested` past
`pointer.has_grab(serial)`, "in response to a pointer implicit grab"). Two
consequences, both against the ticket's "probably `contains_seen`":

- The gate is the strict `contains`, not `contains_seen`. A focus `enter`
  can only be a live grab's serial while someone's *explicit* popup grab
  holds the seat, and spending that here would bless converting an explicit
  grab into a drag. No real toolkit needs it: measured live (below), GTK
  mints the drag from the press serial, and Qt's last-seen serial -- what it
  passes -- has just been overwritten by the press, since motion never
  updates it (the Qt shape `popup.rs` documents).
- "Still pressed, so trivially recent" is false: still-pressed does not
  imply recent. The ring's age bound is about interaction-to-request
  latency, and a held button produces no new qualifying events while held
  (motion deliberately refreshes nothing), so a press held past
  `INTERACTION_WINDOW` (10s) is refused even still held. Accepted with eyes
  open: real drags cross their motion threshold within milliseconds of the
  press, and a menu-style session rule would only re-open the cross-client
  hole below, so there is none. The pathological shape (hold motionless for
  ten seconds, then drag) reads as a failed drag the user retries by
  repressing -- which is why no README line was added (a refused drag is
  silent by protocol, and there is no drag-and-drop section to hang one
  line on).

**2. Client plumbing: the data source's client, via downcast.** `Source`
carries no client accessor, so `dnd_source_client` downcasts `&S` through
`Any` to `WlDataSource`/`WlSurface` (both `Resource`s, so the id names a
client no sender can forge). Anything else -- a compositor-internal source
type flexwm does not have -- resolves to nothing and fails closed with its
own `warn`: the only producer of these calls is Smithay's dispatch with the
two known types, so an unknown type is unexpected, not a third legitimate
shape. A wrong-client check here would refuse legitimate drags, which the
acceptance test below pins against.

**3. Composition: the check is not vacuous, and the residual threat is
precise.** What reaches `dnd_requested` already proves two things: a button
is physically down (`grab_start_data()`), and the serial equals the live
grab's (`has_grab`). Neither binds the serial to the *requester* --
`has_grab` compares the number alone, and serials are process-global values
any client can read off a configure and spray for free (a wrong guess is a
silent dispatch deny, so spraying costs nothing). The hole the pair check
closes is exactly that: a client naming the victim's live press serial --
guessed, or read -- while the user holds *any* button (on any surface, or
none: a bare-desktop press records under no one, so only its absence is in
the ring) starts a drag from its own source until release. With the gate,
only the client that received the press can spend it. Touch stays refused
outright; `xdg_toplevel.move`/`resize` stay unimplemented no-ops.

## Verified against a real toolkit, not only in tests

Dev VM, headless, IPC-driven, `WAYLAND_DEBUG=1` on the client -- a minimal
GTK 3.24 app (button set as a drag source, `drag-begin`/`drag-end`/
`drag-failed` markers on stdout):

```
wl_pointer#15.button(94, 305037, 272, 1)
-> wl_data_device#19.start_drag(wl_data_source#26, wl_surface#23, wl_surface#31, 94)
wl_pointer#15.button(96, 308061, 272, 0)
wl_data_source#26.cancelled()
```

GTK mints from the **press** serial (94 == 94) -- the wire fact the strict
gate rests on. The drag began (`DRAG-BEGIN`, no `refusing a drag` in the
compositor log), and release ended it (`DRAG-FAILED result=5`,
`GTK_CANCEL`, no target having accepted -- the same lifecycle the
acceptance test pins). A `GtkLabel` drag source never saw its own press
(a client-side event-mask quirk, no compositor traffic involved); the
`GtkButton` source worked first try, which is the shape kept.

Three harness tests in `compositor/selection/dnd.rs` (real
`wayland-client` connections, staged around a real held button -- the
dispatch floor and the compositor gate need different serials to tell
apart):

- `a_drag_with_the_press_serial_is_accepted` -- no cancel while held, the
  device sees drag `enter` mid-drag, release ends the grab and cancels the
  unaccepted source. Passes with and without the gate: it pins that the
  gate refuses nothing legitimate.
- `another_clients_press_serial_starts_no_drag` -- fail-first: FAILS with
  the gate disabled ("should be cancelled while held", measured), passes
  with it. From the second client's side the serial is fabricated -- never
  delivered to it, however right the number.
- `a_serial_nothing_was_pressed_with_starts_no_drag` -- pins Smithay's
  dispatch floor (bogus serials die before the handler, silently: no
  `cancelled`, no drag), which is what makes the previous test the one
  exercising the compositor's own gate.

## What this deliberately does not cover

- No hot-path benchmark: the gate runs once per `start_drag` (a linear
  scan of sixteen entries, like activation and popup), not per event or
  frame -- stated, not measured.
- No README change: a refused drag is silent by protocol (a `cancelled`
  event, no error), and the only legitimate-hit shape is the >10s-held
  press above, which reads as a failed drag the user retries. Stated
  explicitly per the ticket's condition, not silently skipped.
- No session-continuation machinery: forbidden by the ticket unless a real
  client demonstrates needing it, and none did -- a drag ends at release
  by construction, unlike a menu that outlives its serial.
