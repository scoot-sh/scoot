---
title: "XWayland: honour _NET_WM_MOVERESIZE (an X app's own titlebar drag) — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# XWayland: honour `_NET_WM_MOVERESIZE` — RESOLVED

RESOLVED 2026-09-28 (branch `claude/scoot-backlog-issues-3rfkfv`). An X
app's own titlebar or border drag moves or resizes a floating X window
through the same floating grab an `xdg_toplevel.move` / `.resize` starts
(`compositor/xwayland/moveresize.rs`), and a tiled or fullscreen X window's
request is ignored as an xdg one is. The shape below held, with these
differences:

- **The gate is shared where it can be.** `floating/grab.rs` gained
  `held_click` (the pointer's grab is Smithay's `ClickGrab`, under a serial
  when the request has one) and `begin_client_drag` (the floating and
  on-screen check, then the grab); `client_floating_drag` is now those two
  plus its own same-client check. The X gate (`x11_moveresize_gate`) is a
  pure function over the lock state and the held press, so the lock branch
  -- which live input cannot isolate, since locking drops the press first
  -- is tested on its own.
- **The button check.** `data[3]` is mapped from XWayland's numbering
  (1-3 left, middle, right; 4-7 scroll, never held; 8 up from `BTN_SIDE`)
  and must be the held press's; `0` rides whichever is held (some toolkits
  send none).
- **`_NET_WM_MOVERESIZE_CANCEL` and the keyboard directions (9-11) are
  not honoured: Smithay's window manager drops them before any handler
  runs**, which the ticket noted. Keyboard moves are refused by design
  (scoot has no keyboard move mode; bindings and `scoot msg` move a
  floating window). Cancel would need a window-manager hook in the fork;
  it is not needed to stop a drag sticking, because a request handled
  after its release finds no click grab (the release reaches scoot before
  XWayland, so no X client can act on a release scoot has not seen) and the
  release ends a running drag. Split out:
  [`_NET_WM_MOVERESIZE_CANCEL`](../protocols/xwayland-net-wm-moveresize-cancel.md).
- **A known limit, pinned:** any X client can name any window in the
  request, so a stranger naming the window the press is held on drags it
  until the release -- the X drag-and-drop owner's limit (`dnd.rs`). No
  pointer is captured without a press held on the named window's client.
- **An X window's resize is configured when the drag ends**, not per
  motion: `FloatingGrab` sends the `resizing` configure to xdg toplevels
  only. The modifier drag has always resized X windows this way; the
  window's outline in the layout follows the pointer and the X client is
  told its size on release.

Tests: `compositor/xwayland/tests/moveresize.rs` (16 live, with a real X
client pressing through scoot), `compositor/xwayland/moveresize/tests.rs`
(button and edge mapping). Fail-first and mutation records are in the PR.

The original entry follows.


Split out of [XWayland support](../resolved/xwayland-support-done.md) when
its Phases 5–7 closed it (2026-09-27). Serves daily use, not computer use:
an agent moves a floating window with `scoot msg` actions, never a titlebar.

## Today

An X app that draws its own titlebar (GTK apps with CSD over
`GDK_BACKEND=x11`, Chromium, Electron) asks the window manager to move or
resize it with a `_NET_WM_MOVERESIZE` client message. Smithay's XWM decodes
it into `XwmHandler::move_request` / `resize_request`
(`smithay/src/xwayland/xwm/mod.rs`, the `_NET_WM_MOVERESIZE` arm; keyboard
moves, direction 9–10, and `_NET_WM_MOVERESIZE_CANCEL`, 11, are dropped
there). scoot logs both at DEBUG and does nothing
(`compositor/xwayland/wm.rs`, `resize_request` / `move_request`). The
`[floating] modifier` drag already moves and resizes a floating X window, so
nothing is unreachable -- the titlebar just does not drag.

## Shape of the fix

The xdg path is the model: `XdgShellHandler::move_request` /
`resize_request` go through `State::client_floating_drag`
(`compositor/floating/grab.rs`), which honours a request only for a
*floating* window, only while the press it rides on is still held
(Smithay's implicit `ClickGrab` under that serial), and only when that press
went to the requesting client -- and refuses while locked.

X has no serial, so the gate has to be restated:

- the pointer's current grab is a `ClickGrab` (a button is held) whose
  start focus is an X surface of the *same X client* as the window (the
  window-id client bits, as `xwayland/dnd.rs`'s drag gate compares them), and
  its button matches the message's `data[3]` when that is non-zero;
- the window is managed and floating (a tiled or fullscreen one: refuse, as
  for xdg);
- not locked, not touch.

Then start the same floating grab `client_floating_drag` starts. Generalise
that function to take a window id plus a "the held press is this client's"
predicate, rather than a `ToplevelSurface`, so both paths share one gate.

## Sizing

- **Files:** `compositor/xwayland/wm.rs` (two handlers),
  `compositor/floating/grab.rs` (split the gate from `ToplevelSurface`), a
  new `compositor/xwayland/tests/moveresize.rs` plus a
  `XClient::request_moveresize` helper in `tests/x11.rs`, docs
  (`protocols.md`'s XWayland "Not yet", README's XWayland bullet).
  Roughly 150–250 lines with tests.
- **Fork change:** none. The handler hooks exist at the pinned rev.
- **Risk:** medium. The gate is security-relevant (a background X client
  must not be able to start a drag that captures the pointer: the
  fail-first test is "a `_NET_WM_MOVERESIZE` with no held press, or with a
  press held on another client's window, is refused"), and XWayland holds
  its own implicit X pointer grab during the press, which the grab start must
  not fight -- measure with a real CSD app (GTK4 `zenity` over
  `GDK_BACKEND=x11`, which the Phase 2/3 live matrix already used) before
  trusting a synthetic test alone.
