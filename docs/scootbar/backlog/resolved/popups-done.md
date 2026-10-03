---
title: "Popups: sliders, lists and menus as `xdg_popup`s parented to the bar"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M6"
resolved: "2026-10-02"
---

# Popups

## Resolution (2026-10-02)

Declarative popups landed: a module fills a `Content` (text, a slider,
buttons) and the bar draws it in an `xdg_popup` parented to its layer surface
(`zwlr_layer_surface_v1.get_popup`). The volume module (and `microphone`,
which shares its code) is the one consumer: `on-click = "popup"` opens a slider
for the level and a Mute button, and drag and click turn into the ordinary
module actions `set N` and `toggle-mute`. Reference: [cli.md](../../cli.md#popups).

- **Opt-in over the interim path.** Nothing changes until a binding names
  `popup`: the click is still the mute, the network picker is still the
  dmenu-style `menu-command`. A bar with no such binding binds no `xdg_wm_base`
  (traced: zero `bind` requests for it) and takes no keyboard.
- **Shape.** `src/popup/` is pure (content, layout, the pointer state machine,
  paint) and shaped for [extraction](../extract-scootui.md) without being
  extracted, because that entry waits for a second binary; `src/daemon/popup/`
  is the Wayland half. A `popup` Cargo feature (default on) gates all of it.
- **One way to act.** What a widget does is a module action by name through
  `action::perform`, like a binding or `invoke`. `scootbar msg invoke ID popup`
  opens one with no grab (an agent has no input serial), toggled by invoking
  again. Volume gained `set N` (percent, held to `max-volume`), the absolute
  action a slider needs; brightness already had a `set`.
- **Press, not release.** A popup bound to a click opens on the press, the
  one exception to "clicks fire on release", so the grab carries the serial of
  a button still held, the shape the strict compositors check. A press on the bar
  while one is open closes it and spends the click, which is what makes a
  second click a toggle.
- **Every ending**, tested while open: click outside and Escape, a press on the
  bar, the module losing its device, the output unplugged (sway), `hide` and
  `reload`, and constraint adjustment at the right edge and on a bottom bar.
- **The keyboard** is taken only while a popup that grabbed is open, for
  Escape, and released with it (traced: no `get_keyboard` with no popup open).
- **A scoot bug found on the way, fixed in its own PR.** Destroying a popup that
  never grabbed left its pixels on screen, since only a grab's dismissal asked
  for a frame (`popup_destroyed` now does; negative control in that PR). Every
  future tooltip would have hit it.

### Evidence

Everything below is from the dev VM (6 CPUs, another agent building on it at
the same time, load average 1 to 3), against `origin/main` at `2d5da91` as the
"before". The scoot used as the compositor was this branch's own debug build.
`crates/scootbar/tests/popup_bench.rs` is the measurement (`--ignored`; its header has the commands, and `cycles` needs `cargo test`, not nextest, for the time limit), and
`SCOOTBAR_BENCH_NO_POPUP=1` leaves the binding out of the config.

Release build (`lto = "fat"`, stripped), the volume module placed on headless
scoot against a live PulseAudio-protocol server, no popup open, 60 s idle:

| Row | before (`2d5da91`) | after, no binding | after, `on-click = "popup"`, closed |
| --- | --- | --- | --- |
| idle RSS | 4216 kB | 4020 kB | 4284 kB |
| idle peak RSS | 4216 kB | 4020 kB | 4284 kB |
| idle wakeups per minute | 0 | 0 | 0 |
| threads, fds | 1, 8 | 1, 8 | 1, 8 |
| shm mappings | 2 | 1 | 2 |
| stripped binary | 1,708,768 B | 1,774,304 B (+65,536, +3.8%) | same binary |

The RSS spread is the bar's own second buffer, which a redraw racing a release
makes or does not (one 1600 x 40 buffer is 250 kB; the mapping count says which
runs had it), not popup code: the three rows are inside one buffer of each other.

Open and close, release build, 100 cycles each checked to have mapped and
drawn (a screenshot shows the popup, then not) before the next step, the
bar's CPU from `/proc/PID/schedstat`:

| Row | Measured |
| --- | --- |
| CPU per open and close | 679 us (67.9 ms over 100 cycles) |
| wakeups per open and close | 4.00 |
| RSS closed, open, after 100 cycles | 4032 kB, 4412 kB (+380), 4284 kB |
| peak RSS | 4396 kB |
| fds, closed and after | 8 and 8 |
| shm mappings closed, open, after | 1, 3, 2 (the popup's one, and the bar's second buffer) |

Idle with a popup open and nothing changing: 0 wakeups over 2 s
(`it_is_idle_while_open_and_while_closed`), and a warm popup (refill, layout,
motion, drag and paint, 200 rounds) allocates nothing
(`a_warm_popup_allocates_nothing`, `count_allocations`).

Tests: 22 popup unit tests (layout, the pointer state machine, paint, the
allocation test), six volume `set` and popup-content tests, one pointer and
two action tests, and 16 integration tests (`tests/popup.rs`: 14 on headless
scoot, 2 on headless sway, one of them with a virtual pointer). The full
command list and results are in the PR description.

### Not done, deliberately

- **Not verified on a compositor that checks the grab serial** (KDE, GNOME,
  Smithay's anvil). scoot takes any recent input serial and wlroots takes any
  serial at all (a bogus one was accepted, measured), so the press-time open is
  unproven against the thing it is for.
- **The session lock** while a popup is open is not tested end to end here
  (scoot's own tests cover its dismissing the grab; the bar closes on the
  `popup_done` it sends, which the click-outside test exercises).
- **No list widget and no network consumer**: a list is a column of buttons, and
  a native WiFi list needs scrolling and a closing selection, filed as
  [popup-network-list](../popup-network-list.md).
- **A drag ends when the pointer leaves the popup** (the compositor's grab moves
  the focus off it); no scroll over the popup; no keyboard navigation but
  Escape; one popup at a time; a scale change closes it.
- **`scootui` is not extracted** (above): no second binary needs it yet.

Original entry, left as written:

Filed 2026-09-29. Serves **daily-drive** (a volume slider, a WiFi list, a
power menu).

A module may return declarative popup content (list, slider, buttons) that the
bar draws in an `xdg_popup` parented to its layer surface. The popup exists only
while open, so idle cost stays zero.

## Interim, and why this is low priority

Until this lands, a module hands a list to a dmenu-style launcher and acts on
the selection: pickers and menus work with `fuzzel --dmenu` today and with the
scoot launcher later, with no popup code in the bar. Build native popups only
when that stops being enough.

## What to do

- `xdg_popup` via `zwlr_layer_surface_v1.get_popup` with a positioner anchored
  to the module's rect; scoot handles layer-parented popups and grabs
  (`xdg-popup-input-resolved.md`), and validates the grab serial, so open from the
  click that carries one.
- A tiny widget set (list, slider, button) drawn with the bar's own primitives.
  This is the consumer that justifies [extracting `scootui`](extract-scootui.md).
- Dismissal: click outside, Escape (the popup grab gives it the keyboard only
  while open), and when its module disappears.
- Popups must not defeat the bar's "never takes the keyboard" property outside
  their own lifetime.

## Edge cases

A popup open when the output is removed or the session locks (scoot dismisses
popup grabs on lock), a second click on the same module (toggle), a popup that
would leave the output (constraint adjustment, `popup-constraint-adjustment-done.md`).
