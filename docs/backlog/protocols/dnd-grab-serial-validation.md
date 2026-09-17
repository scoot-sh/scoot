---
title: "`wl_data_device.start_drag` accepts any serial, so any client can start a pointer drag whenever a button is held"
status: "open"
area: "protocols"
priority: "low"
blocked: null
---

# `wl_data_device.start_drag` accepts any serial

Split out of `protocols/popup-grab-serial-validation.md` (resolved as
`resolved/popup-grab-serial-validation-done.md`), which deliberately did
not cover it: same "answer once rather than three times" question, different
answer.

`WaylandDndGrabHandler::dnd_requested` (`handlers.rs`) installs a pointer
`DnDGrab` for whoever asks, keyed by the request's serial, without checking
the serial against anything. A fabricated serial from a client that received
no input starts a drag that routes the pointer into the attacker's menus
until the button comes up.

What already bounds it, and why this is low priority next to the popup gate
it was split from:

- A drag requires a *live implicit button grab*: the handler cancels unless
  `pointer.grab_start_data()` exists, which only holds while a button is
  physically down. The attacker needs somebody -- the user -- holding a
  button right now, and the grab ends at release. The popup hole needed no
  such accomplice.
- Touch drags are already refused outright (`GrabType::Touch =>
  source.cancel()`).
- Keybindings run before any grab, so the session keeps its escape hatch.

What doing it properly needs, once (not re-derived):

- The serial semantics differ from popup/activation: for a pointer drag the
  serial names the *implicit grab* (the button press being converted), not
  "the event that caused this". The check is probably
  `interaction_serials.contains_seen(serial, requesting_client)` -- the
  press is recorded under whoever the pointer focus named -- but that needs
  confirming against the data-device protocol text and a real drag client,
  plus a decision about whether the *held* button's serial (still pressed,
  so trivially recent) makes the window moot.
- The requesting client is the data source's client, not a surface owner --
  resolving it is different plumbing from `client_of`.
- A real drag (GTK file-manager drag, or at minimum a raw client doing
  `start_drag` after a real press) proving legitimate drags still work,
  fail-first tests in the harness shape `popup_serial.rs` established, and
  the same `warn`-on-refusal debuggability, since a refused drag is as
  silent as a refused grab.
