---
title: "Pointer gestures: real-pad check before default-on"
status: "open"
area: "protocols"
priority: "low"
blocked: null
---

# Pointer gestures: real-pad check before default-on

Filed 2026-11-05, split out of
[`pointer-gestures-v1`](../resolved/pointer-gestures-v1-done.md) when its
opt-in half landed. Serves **daily-drive**: pinch-to-zoom and a three-finger
history swipe in a browser are what a laptop user reaches for.

## The gap

`zwp_pointer_gestures_v1` is advertised only with
`[gestures] client_gestures = true` (off by default, read at startup). The
default stayed off because a bound global changes what GTK4, Firefox and
Chromium do with a touchpad, and no real pad was available to check them
against. Everything after the libinput arms is pinned by headless tests with
a real `wayland-client`; nothing on the toolkit side is.

The nested half (item 2) is resolved; see the end of this entry.

## What to do

Item 1 needs a real touchpad and stays open. Item 2 is resolved.

1. **Real pad, each toolkit.** On a laptop touchpad under `--tty` with
   `client_gestures = true`, check that GTK4, Firefox and Chromium (Wayland
   mode) each behave: pinch zooms, a two-finger scroll still scrolls, a
   resting hold does not cancel kinetic scrolling in a way that feels worse
   than without the global, and a three-finger swipe (scoot's) does not
   also move a page. Record what each did with and without. Then decide
   whether the default flips; if it does, that is a changed default, so mark
   it as such in the commit.
2. ~~**`--nested` host gestures.**~~ Resolved, see below.

## Not in this ticket

Touch-screen (multi-touch) gestures, and `scoot msg` injection of gestures.

## Item 2 resolved (nested host gestures)

`--nested` binds the host's `zwp_pointer_gestures_v1` (version 1 to 3, absent
is a quiet no-op) and routes its swipe, pinch and hold into the same
`gesture_*` entry points `--tty` calls (`compositor/nested/gestures.rs`).
Decisions the ticket left open:

- **Swipe is requested always, pinch and hold only with `client_gestures`.**
  Mirrors `--tty`, where the swipe arms call `gesture_swipe_*` unconditionally
  (scoot's own swipe works with the client global off) while pinch and hold
  are dropped without it. Not asking the host for what would be dropped saves
  its traffic.
- **Hold needs host version 3**; a v1/v2 host gets swipe and pinch only.
- **Objects follow the host `wl_pointer`.** Made when the seat gains a
  pointer, destroyed when it loses one. Losing a pointer was not handled
  before (the stale `wl_pointer` was kept, so a returning pointer was never
  picked up); it now releases the pointer, drops its half-buffered scroll and
  ends a floating-window drag as a `leave` does.
- **Begin decides, end matches, one layer earlier.** An update or end with no
  begin from the host never reaches an entry point; a gesture still open when
  the pointer leaves the window or is lost is ended `cancelled`.
- **Host units, host clock.** Deltas and timestamps are passed through as the
  host sends them; `sensitivity` is documented as possibly needing a nested
  value.
- **Cancels carry the host's clock.** A cancel made here is stamped with the
  last event's host timestamp, so a gesture keeps one timeline.
- **Not tested:** a real v1 or v2 host. The pinned Smithay global is fixed at
  v3, so the nested side is made to bind at most v2 in a test, which is the
  same wire a v2 host presents. Nor any real host compositor or touchpad.
