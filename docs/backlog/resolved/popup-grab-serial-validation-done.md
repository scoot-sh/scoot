---
title: "`xdg_popup.grab` accepts any serial, so any client can take the keyboard whenever it likes — DONE."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# `xdg_popup.grab` accepted any serial — DONE

Split out of `xdg-popup-input.md` when popup grabs landed
(`resolved/xdg-popup-input-resolved.md`), which deliberately did not do this.

`xdg_popup.grab` carries the serial of the input event that caused the
client to open a menu; flexwm handed it straight to
`PopupManager::grab_popup`, which uses it only to key the seat grab, never
to authorize it. A client with no recent interaction at all could map an
`xdg_popup` and hold the keyboard.

## What landed

A serial gate in `State::grab_popup`, checked after the outranked refusal
(lock / `exclusive` layer surface) and before anything is installed:

- The named seat must resolve to the compositor's own seat (same check
  `activation.rs` applies to a token's `set_serial`).
- The serial must be one flexwm delivered to the *requesting* client --
  resolved from the popup's own surface via `client_of`, which no client can
  forge -- as a key, button **or focus `enter`**, within the last 16 such
  deliveries and the last 10 seconds (`contains_seen`, the looser half of
  `input/interaction.rs`; the activation gate's strict `contains` is
  byte-for-byte unchanged).
- A grab that continues the requesting client's own menu session is granted
  without consulting the serial (see below).
- Refusal dismisses the popup like every other refusal, and -- since the
  protocol posts no error -- logs at `warn` with the client and serial, so
  a menu that never takes the keyboard is diagnosable instead of silent.

Recording the `enter` half, with the "delivered literally" discipline the
activation work established:

- Pointer `enter`: `pointer_move_quietly` records the motion serial under
  the entered client only when focus really moves (read before `motion()`
  updates it), something is actually entered, and no grab holds the pointer
  (under one the recipient is the grab's logic, not the hit test). Costs
  one seat lock and a handle clone per motion event on the no-change path;
  noise next to the hit test and socket write the path already pays.
- Keyboard `enter`: `refresh_keyboard_focus` records only when focus really
  moves to a new surface and no popup grab is live (a live
  `PopupKeyboardGrab` swallows the `set_focus`, and the install path
  deliberately focuses the popup with the grab's *own* serial, which must
  never be filed as fresh evidence). An input-method grab forwards
  `set_focus`, so enters delivered through one record normally.
- Plain motion serials stay excluded: only a motion that delivers an
  `enter` is recorded, bounded by focus changes rather than the event rate.

## The two design details the ticket deferred, both settled with evidence

**1. A strict key-and-button check refuses legitimate Qt menus.** Qt's
`QWaylandInputDevice::serial()` -- what a Qt client passes to `grab` -- is
updated on `pointer_enter` (verified against Qt 6.8's
`qwaylandinputdevice.cpp`: `Pointer::pointer_enter` assigns
`mParent->mSerial`; `keyboard_enter` and `modifiers` deliberately do not).
A hover-opened menu has had no button or key event to draw a fresh serial
from, so strictness breaks it silently. Pinned by
`a_grab_with_a_pointer_enter_serial_is_accepted` and
`a_grab_with_a_keyboard_enter_serial_is_accepted`, which both FAIL against
a strict gate (measured) and pass against the looser one.

Accepting enters weakens the check with eyes open: a client that merely
received an enter (mapped-and-auto-focused, or hovered) can grab within the
window. Accepted because the grab is bounded in ways activation is not --
lock and `exclusive` pre-empt and refuse it, click-outside dismisses it,
keybindings run before it, a grant is visible (a menu appears, and
`msg windows` reports `popup_grab`), and a refusal is now warned about.
Activation keeps refusing focus serials.

**2. Menu sessions outlast the serial window.** Measured against real
KeePassXC (Qt 5.15): click Database (grab with button serial 13, accepted),
wait 33s, hover-switch to Entries -- Qt reuses serial 13 for the
replacement grab, long past the 10s window, and a serial-only gate refuses
it with `popup_done` (reproduced live, warn in the log). A menu read for a
minute then hovered deeper, and every menubar hover-switch, reuses the
opening serial; refusing reads as an attack only if "recent interaction"
is the whole question.

So a grab is also granted when it continues the requester's own session
(`grab_session_continues`): it holds the live grab (root client ==
requester -- nested submenu, second grab while one is up), or its previous
grab ended within `GRAB_REOPEN_GRACE` (2s). Either way the keyboard was
this client's moments ago; a *different* client grabbing off someone
else's menu still faces the serial check. The ended half is filed in one
place only -- `CompositorHandler::destroyed`, gated on the dying surface
belonging to the grabbing client -- because toolkits destroy the old popup
before grabbing the new one *in the same flush*: destroy and grab dispatch
adjacently with no reap in between, so reap-time filing lands a dispatch
too late. Deliberately not filed at dismiss (click outside, lock,
`exclusive` layer) or at reap: a session the user or the compositor ended
must not lend its serial to a reopen, and another client's surface churn
must not refresh someone else's timestamp; a grant clears whatever stamp
came before, so the grace cannot renew across sessions. Pinned by
`ReplacePopup` (same-flush destroy + re-grab), which fails with reap-only
filing and passes with destroy-time filing, while the separate-steps grace
test passes under both -- and by the click-outside dismiss test, which pins
that a dismissed menu's stale serial is refused on reopen.

## Verified against real toolkits, not only in tests

Dev VM, headless, IPC-driven, `WAYLAND_DEBUG=1` on the clients:

- **KeePassXC 2.7.12 (Qt 5.15.19 + qtwayland, nixpkgs):** click-opened
  menubar menus grab with the button-press serial -- accepted, menu
  renders, keyboard enters the popup, arrows reach it, Escape closes it.
  After 15s idle, a menubar hover-switch reuses the aged-out button serial
  -- accepted via the session grace, zero refusals, the Entries menu open
  and holding the keyboard (screenshot). Pre-grace binary refused the same
  switch with the warn + `popup_done`.
- **Minimal GTK 3.24 app (menubar + button-opened `GtkMenu`, compiled on
  the VM):** click-opened menus grab with the button-press serial --
  accepted, menu renders, keyboard enters it, arrows highlight items,
  click-inside reaches the popup, Escape/click closes it. A nested submenu
  grabs while the parent is live -- accepted, keyboard moves onto it.
- Qt5 keeps its button serial across pointer enters (a hover-switch
  re-grabbed with the 15s-old press serial, never an enter serial); the
  enter-update half is verified in Qt6 source and in harness tests. Both
  shapes are accepted.

## The "answer once" question: move, resize, drag

- `xdg_toplevel.move` / `resize` / `show_window_menu`: flexwm does not
  implement them (trait defaults, no-ops). There is nothing to gate; gating
  a no-op would be theater. When interactive move/resize lands, it should
  reuse `contains_seen` plus the session rule -- stated here so the next
  implementer does not re-derive it.
- `wl_data_device.start_drag`: implemented (`dnd_requested`), unvalidated,
  and NOT covered here. It needs its own analysis (a DnD grab requires a
  live implicit button grab, so the threat is narrower but different, and
  the serial semantics are the implicit-grab serial, not an event serial).
  Filed as `docs/backlog/protocols/dnd-grab-serial-validation.md`.

## What this deliberately does not cover

- Touch: flexwm delivers no touch events at all, so no touch serial exists
  to spend. A touch-driven grab names nothing real and is refused; that
  flow cannot exist here until a touch input path does.
- No hot-path benchmark: the grab path runs per menu opened, and the two
  recording sites add one seat lock plus a handle clone per motion event
  (pointer) and per focus derivation (keyboard) -- no allocation, next to
  work both paths already pay per event. Stated, not measured.
- Two live-driving observations, neither caused by this change (it adds no
  serial draws, no wire traffic, and an identical install path on grant):
  IPC `msg windows` rects are arrangement coords while the pointer takes
  screen coords, which diverge once the columns scroll -- agent drivers
  must account for scroll, a pre-existing IPC semantic; and a repeated GTK
  File menu twice mapped at a 50x8 geometry it never repositioned out of
  (likely GTK measuring against unready state under rapid IPC driving, and
  non-monotonic -- a correct submenu mapped in between). Both noted for
  whoever next drives toolkits live, neither blocks this gate.
