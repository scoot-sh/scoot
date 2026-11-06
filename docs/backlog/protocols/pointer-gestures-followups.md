---
title: "Pointer gestures: real-pad check before default-on, and nested host gestures"
status: "open"
area: "protocols"
priority: "low"
blocked: null
---

# Pointer gestures: real-pad check before default-on, and nested host gestures

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

Separately, `--nested` does not receive its host's swipes, so `[gestures]`
and the forwarding do nothing in a nested session.

## What to do

The first item needs a real touchpad; the second does not.

1. **Real pad, each toolkit.** On a laptop touchpad under `--tty` with
   `client_gestures = true`, check that GTK4, Firefox and Chromium (Wayland
   mode) each behave: pinch zooms, a two-finger scroll still scrolls, a
   resting hold does not cancel kinetic scrolling in a way that feels worse
   than without the global, and a three-finger swipe (scoot's) does not
   also move a page. Record what each did with and without. Then decide
   whether the default flips; if it does, that is a changed default, so mark
   it as such in the commit.
2. **`--nested` host gestures.** Bind the host's `zwp_pointer_gestures_v1`
   and route its swipe events through `gesture_swipe_begin/_update/_end`
   (and pinch and hold through `gesture_pinch_*` and `gesture_hold_*`,
   `gesture/client.rs`), so `[gestures]` works in a nested session. The
   entry points already hold the begin-decides-and-end-matches rule, so the
   work is the host-side bind and the translation of its events.

## Not in this ticket

Touch-screen (multi-touch) gestures, and `scoot msg` injection of gestures.
