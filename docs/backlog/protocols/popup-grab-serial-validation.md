---
title: "`xdg_popup.grab` accepts any serial, so any client can take the keyboard whenever it likes"
status: "open"
area: "protocols"
priority: "medium"
blocked: null
---

# `xdg_popup.grab` accepts any serial, so any client can take the keyboard whenever it likes

Split out of `xdg-popup-input.md` when popup grabs landed
(`resolved/xdg-popup-input-resolved.md`), which deliberately did not do this.

`xdg_popup.grab` carries the serial of the input event that caused the
client to open a menu, and the protocol says it "must be used in response to
some sort of user action like a button press, key press, or touch down
event"; a compositor "may ignore" a grab whose serial is not one. flexwm
ignores the serial entirely today — it is handed straight to
`PopupManager::grab_popup`, which uses it only to key the seat grab, never to
authorize it. So a client with no recent interaction at all can map an
`xdg_popup` and hold the keyboard.

What already bounds the damage, and is tested:

- A grab is refused, and the popup dismissed, while the session is locked or
  while an `exclusive` layer surface holds the keyboard — so this is not a
  route past the lock screen or a launcher.
- Clicking anywhere outside the popup dismisses it, and keybindings are
  matched before anything is forwarded, so a grabbing client cannot wedge
  the session.
- It is the same-uid trust boundary flexwm already documents (see
  `README.md`'s trust note): a client that can reach the wayland socket can
  already take the session lock over.

What is still worth closing: a client that is merely *running* can quietly
capture keystrokes meant for another window, with no user action at all, and
nothing on screen has to look wrong.

## Why it was not done with the grab itself

flexwm has the machinery — `input/interaction.rs`'s `Recent`, which
`activation.rs` already uses to gate `xdg-activation-v1` tokens, and which
records the client each serial was *delivered* to (so a serial cannot be
guessed at). The concern that stopped it is a false-negative one:

- `Recent` records **key and button** serials only. `pointer_move_quietly`'s
  motion serials are deliberately excluded, and `wl_pointer.enter` /
  `wl_keyboard.enter` serials are not recorded at all.
- Qt's `QWaylandInputDevice::serial()` — which is what a Qt client passes to
  `grab` — is updated on `pointer_enter` and `keyboard_enter` as well as on
  button/key events. A strict `contains(serial, client)` check would
  therefore refuse a legitimate Qt combo box whenever the most recent event
  the toolkit saw was an enter.
- A refused grab is silent (no protocol error), so the failure mode is "this
  app's menus don't take the keyboard and nobody knows why" — worse to debug
  than the hole it closes, and the hole is inside an already-same-uid trust
  boundary.

## What doing it properly probably needs

- Recording `enter` serials in `Recent` as well, or a second, looser history
  for "serials a client has legitimately seen" — noting that `enter` is *not*
  a user action, so it weakens the check rather than just widening it.
- A real Qt and a real GTK client to test against (neither probed Quickshell
  shell uses `xdg_popup` at all, so this needs an ordinary app toolkit), and
  a decision about what to do on refusal that is debuggable: at minimum a
  `tracing::warn!`, since the protocol gives no way to tell the client.
- The same question exists for `xdg_toplevel.move`/`resize` and for
  `wl_data_device.start_drag`, none of which flexwm validates either — worth
  answering once rather than three times.
