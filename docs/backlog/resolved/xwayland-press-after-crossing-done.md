---
title: "XWayland: a press batched with a move onto another X window reached no X window — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# XWayland: a press batched with a move onto another X window reached no X window — RESOLVED

RESOLVED 2026-09-27 (branch `claude/scoot-backlog-issues-3rfkfv`, on
`560c63b`). Filed 2026-09-27 as a harness-only side finding of
[the X drag first-motion race](./xwayland-x-drag-first-motion-race-done.md).
Served computer use first (every `scoot msg pointer click` onto the other
of two X apps was lost) and daily use second.

## Resolution

**Verdict: a product bug, not a harness artifact, and not a batching
race.** Any click after the pointer crossed from one X window onto
another (or from an X window onto the background, then onto an X window)
went to the wrong place, however long the pointer sat still first.

**Mechanism.** scoot credits a move's relative motion to the surface the
pointer is leaving (`relative_pointer.rs`: pre-move focus), so a crossing
move sent XWayland, captured with `WAYLAND_DEBUG=server` on the harness:

```
-> zwp_relative_pointer_v1@22.relative_motion(0, 525000, 194.0000, 0.0000, 194.0000, 0.0000)
-> wl_pointer@21.leave(8, wl_surface@25[1])
-> wl_pointer@21.frame()
-> wl_pointer@21.enter(8, wl_surface@27[1], 91.0000, 188.0000)
-> wl_pointer@21.frame()
-> wl_pointer@21.button(10, 741, 272, 1)
```

That order is correct Wayland. XWayland 24.1.13 (`hw/xwayland/xwayland-input.c`)
mishandles it:

- `relative_pointer_handle_relative_motion` stores the delta in
  `pending_pointer_event` (`has_relative = TRUE`);
- `pointer_handle_leave` clears `focus_window`;
- `pointer_handle_frame` returns early when `focus_window` is `NULL`,
  **without clearing the pending state**;
- `pointer_handle_enter` sets the X pointer to the enter position
  (`SetCursorPosition`, 206 + 91 = 297 here);
- the next `pointer_handle_frame` has a focus, so it dispatches the stale
  delta: `dispatch_relative_motion` queues it as `POINTER_RAWONLY`, which
  still moves the sprite (`dix/getevents.c`, `fill_pointer_events`:
  `moveRelative` and `positionSprite` run before the `RAWONLY` check).
  297 + 194 = 491, clamped to 399 on the 400-wide screen.

No absolute motion follows the enter, so nothing corrects the position
until the next move, and the press goes to whatever is under the displaced
pointer: the root here, or a third window.

**Fix (scoot-side, `compositor/relative_pointer.rs` `leaves_an_x_window`).**
A move that takes pointer focus off an X window sends no relative event.
Wayland clients keep the pre-move-focus rule. Under a grab (a button held
on an X window) focus does not move, so a drag keeps its deltas.
`move_absolute` now works out whether focus moves once (`focus_moves`,
replacing `pointer_entered`), and the relative gate and the `enter` record
both use that answer. The Clamped arm's `under.clone()` is gone too. It is
not a Smithay fix: Smithay sends only what scoot asks it to send, in that
order. The XWayland half is an upstream bug. No upstream report was filed
(standing rule: no agent files upstream).

**What X clients lose.** XWayland no longer gets the one crossing delta
of such a move as an `XI_RawMotion`, the same way scoot already sends
nothing for a teleport *onto* a surface. On `--tty` a crossing is one
device event of a few pixels, so what is lost is one small raw delta.

## Evidence (raw)

All captured in the Claude Code web container (no dev VM, no tty), with
XWayland `/nix/store/iz5m97qfzkdv6ahnq0v673cc28sbznlp-xwayland-24.1.13`.

**Harness, before (tree `560c63b` plus the new test module, no fix).**
The three new tests failed:

```
Batched ... left: ((0, 0), (0, 0), (399, 200))   right: ((1, 1), (0, 0), (297, 200))
Settled ... left: ((0, 0), (0, 0), (399, 200))
ViaBackground ... left: ((0, 0), (0, 0), (394, 200))
```

(`(target presses, releases)`, `(left-window presses, releases)`, X
pointer while pressed.) The via-background row is displaced by the 97 px
leave-to-background delta, which confirms the delta is applied after the
enter.

```sh
SCOOT_REQUIRE_XWAYLAND=1 PRESS_N=40 cargo test -p scoot --features xwayland --bin scoot \
    press_after_crossing_measurement -- --ignored --nocapture
```

| shape | onto | n | before: press on target / on left / X pointer off target | after |
|---|---|---|---|---|
| Batched (move+press+release, one flush) | second | 40 | 0 / 0 / 40 | 40 / 0 / 0 |
| Batched | first | 40 | 0 / 0 / 40 | 40 / 0 / 0 |
| Settled (move, settle, click) | second | 40 | 0 / 0 / 40 | 40 / 0 / 0 |
| Settled | first | 40 | 0 / 0 / 40 | 40 / 0 / 0 |
| ViaBackground | second | 40 | 0 / 0 / 40 | 40 / 0 / 0 |
| ViaBackground | first | 40 | 0 / 0 / 40 | 40 / 0 / 0 |

**Live, the product path.** `scoot --headless --width 800 --height 600
--xwayland` (debug build, `--features xwayland`), with two separate
`xev -event button` clients (`/nix/store/7z1pbay0rcb26lyq0mhf8vd1rvsdbins-xev-1.2.7`),
tiled at centres (203,300) and (597,300). The script alternates 20 clicks
each way (x nudged by `i % 5`), 0.3 s apart, and counts `ButtonPress`
lines in each `xev`'s output:

| binary | `pointer click X Y` (A+B) | `pointer move` then `pointer button left press/release`, separate invocations (A+B) |
|---|---|---|
| before (`560c63b`) | 0 + 0 of 20 + 20 | 0 + 0 of 20 + 20 |
| after (this change) | 20 + 20 | 20 + 20 |

The second shape is slower than two requests pipelined on one connection,
and it still lost everything. So the pipelined case the entry asked about
is covered: timing does not matter.

**`--tty` (reasoned, not run: no hardware here).** libinput motion goes
through `pointer_move_relative` into the same `move_absolute`, so a
crossing event sent XWayland the same sequence. A continuous hand motion
corrects it on the next event (1-8 ms at 125-1000 Hz), so a click was off
only when it came before that event: a press in the same libinput frame
as the crossing motion, or a click after a hand that stopped on the
crossing event. Either way it was off by one event's delta, a few pixels.
That is enough to miss a small target at a window edge. `--nested` is the
same, with the host's motion deltas.

**Wire after the fix** (temporary probe, removed): a move within an X
window still sends `relative_motion` then `motion`. A crossing sends
`leave`, `enter` and nothing relative. A press on B followed by a drag
over A (implicit grab) still sends
`relative_motion(... -194.0000 ...)` + `motion(-103.0000, 188.0000)` with
no `leave`.

**Benchmark (hot path: `move_absolute`).** Release,
`x11_hot_path_cost` (`--features xwayland`), 20 000 motions x 5 runs per
row. Before and after binaries were interleaved over 8 rounds, with the
order alternating each round. Median ns/motion per round:

- X window, before: 2595 2651 2629 2587 2748 2634 2614 2655 (median of rounds ~2630)
- X window, after:  3044 2609 2623 2609 2582 2621 2650 2617 (median of rounds ~2620)
- Wayland window, before: 2240 2227 2343 2336 2254 2336 2242 2223
- Wayland window, after:  2324 2221 2241 2243 2280 2414 2335 2354

The ranges overlap, so there is no measurable change. An earlier cut
that compared focus a second time for the gate measured X motion
2647-2996 against 2609-2688 before over 6 rounds. It was restructured to
compute the answer once.

## Files

- `crates/scoot/src/compositor/relative_pointer.rs`: `leaves_an_x_window`
  and the module-doc exception.
- `crates/scoot/src/compositor/input.rs`: `focus_moves` (read once) and
  the gate.
- `crates/scoot/src/compositor/xwayland/tests/press_after_crossing.rs`:
  three fail-first tests (batched, settled, via the background) and the
  `#[ignore]`d measurement.

## Unverified

- Not checked against XWayland master. The web container's proxy could
  not fetch freedesktop's GitLab. If a later XWayland clears the pending
  state on `leave`, the gate becomes unnecessary but stays harmless.
- No `--tty` or `--nested` run (above is reasoning from code).
- No test asserts the X side of the grab exception (that a drag held on an
  X window keeps sending raw deltas): the test `x11rb` has no XInput2
  feature, so only the one-off wire capture above shows it.
