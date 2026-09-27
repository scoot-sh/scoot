---
title: "XWayland: honour _NET_WM_MOVERESIZE (an X app's own titlebar drag)"
status: "open"
area: "protocols"
priority: "low"
blocked: null
---

# XWayland: honour `_NET_WM_MOVERESIZE`

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
