---
title: "Touchpad scrolling does not reach Chrome (axis source/v120/stop dropped)"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-04"
---

# Touchpad scrolling does not reach Chrome (axis source/v120/stop dropped)

Filed 2026-10-04. Serves **daily-driving** (the maintainer's own
trackpad does nothing in Chrome on `--tty`) and **computer use** (an agent
injecting scrolls needs clients to see the same events a real device sends).

## The gap

Two-finger trackpad scrolling does nothing in Google Chrome (native Wayland
client) on scoot `--tty`. Other apps scroll. Injected scrolls
(`scoot msg pointer scroll 0 15`) scroll Chrome under headless scoot.

Cause: every scroll reaching a client is rebuilt as
`AxisFrame::new(..).source(AxisSource::Wheel)` with continuous values only
(`State::scroll`, `crates/scoot/src/compositor/input.rs`). The tty libinput
arm (`compositor/tty/mod.rs`, `InputEvent::PointerAxis`) drops the event's
real source (`PointerScrollAxis::{Wheel,Finger,Continuous}`), its
`amount_v120` (wheel detents), its `relative_direction` (natural scroll),
and never sends `stop` for the zero-amount event that ends a finger scroll.
The nested arm (`compositor/nested_dispatch.rs`, host `wl_pointer::Axis`)
likewise forwards only the continuous value, dropping the host's
`axis_source`/`axis_value120`/`axis_discrete`/`axis_stop`/`axis_relative_direction`.
Chrome's Wayland backend keys wheel-vs-finger behavior off `axis_source`,
`axis_stop` and `axis_value120`, so a touchpad arriving as a sourceless
wheel with tiny values and no discrete steps is ignored; more tolerant
toolkits scroll anyway.

Verified against the pinned Smithay fork (`035d447`,
`~/.cargo/git/checkouts/smithay-*/035d447`): `PointerAxisEvent` carries
`source()`/`amount_v120()`/`relative_direction()`; the libinput backend maps
`PointerScrollWheel` to `Wheel` (with v120), `PointerScrollFinger` to
`Finger` (amount only, a zero amount terminating the sequence),
`PointerScrollContinuous` to `Continuous`; each libinput scroll event yields
exactly one `InputEvent::PointerAxis` (no deprecated `POINTER_AXIS` double
path in this rev); server-side `AxisFrame::source/v120/stop` become
`wl_pointer.axis_source/axis_value120(+axis_discrete` below v8`)`/`axis_stop`
and `relative_direction` becomes `axis_relative_direction` (v9+, only with a
nonzero value).

## What to do

- Pass the real source, `v120` for wheel events, `stop` for a finger axis
  that reaches 0, and the relative direction, from the tty libinput arm and
  from the nested host-pointer arm (accumulate the host's axis events and
  flush one frame on the host `Frame`).
- Keep `scoot msg pointer scroll` a wheel, with a `v120` so wheel-only
  clients see a detent.
- Unit-test the event-to-frame translation (pure function: every source, the
  stop case, the v120 case, a horizontal+vertical frame); prove live on the
  dev VM with a virtual touchpad (uinput) and a `WAYLAND_DEBUG=client`
  observer showing `axis_source(1 = finger)`, values and `axis_stop`.
- Hot path: no allocation on the per-event input path.

## Not in this ticket

- Gesture (pinch/swipe) forwarding; kinetic/inertial scrolling synthesis.
- Adopting the host keymap in nested mode (separate gap, noted in
  `nested_dispatch.rs`).

## What landed

PR #XXX (`fix/scoot-touchpad-scroll`): `scroll_frame` in
`compositor/input.rs` (pure event-to-frame translation: real source,
wheel `v120`, finger-zero `stop`, relative direction, `None` for
nothing-to-send), used by the tty libinput arm, the nested host-pointer
arm (new `PendingAxis` buffer flushed on host `Frame`), and injected
`pointer scroll` (wheel + detents at 8 v120 units per delta unit).
Unit tests for every source, the stop case, the v120 case and two-axis
frames; wire tests proving an injected scroll arrives as wheel+value120
and a finger scroll arrives with source+stop (both fail without the
fix). Live on the dev VM `--tty` with a uinput touchpad and a
`WAYLAND_DEBUG=client` foot: before `axis_source(0)` wheel with no stop,
after `axis_source(1)` finger with `axis_stop`. Full suite green
(nextest 4213 passed, clippy, fmt, smoke 36 ok).
