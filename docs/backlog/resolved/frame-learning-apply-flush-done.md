---
title: "A learned column minimum from a client frame is not applied until the next unrelated event — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Frame learning now flushes its own relayout — RESOLVED

RESOLVED 2026-09-27 (PR #TBD). `World::handle_event` answers whether the
event changed the arrangement in a way the shell has not yet pushed out --
today only a `FrameObserved` that raised a learned minimum or moved a
floating window answers true; every other event answers false, since its
callers already re-apply unconditionally. `State::observe_frame` (and the X
arm, `observe_x11_frame`) propagate that answer instead of applying, and
`flush_window_commits` applies once for the whole dispatch when any frame
did -- never per commit, never per window. Learned-minimum semantics
untouched (same tolerance, same caps, same fullscreen/floating exclusions);
the PR #254 flush ordering is unchanged, with one `apply()` at its end.

Filed 2026-09-23 by the PR #223 re-review (pre-existing). `observe_frame`
fed `FrameObserved` into the core (`handlers.rs:~144`), which could raise a
learned minimum and re-scroll, but nothing called `apply()` after it, so the
new column width and positions reached clients and the screen only at the
next event. PR #223's no-op-fullscreen fast path removed one place that used
to flush this by accident.

No existing "changed" signal was reused because none existed: `handle_event`
returned `()`, and `origin_revision` tracks adoption tags, not arrangement.
The new bool rides the existing event path rather than adding a parallel
mechanism. Flag traced at every site: `learn_from_frame` (tiled: the
existing `learned != learned_min` branch; floating: the same
`floating_size` before/after comparison the shell used to do, now read off
the held borrow), `observe_frame`/`observe_x11_frame` (propagate),
`flush_window_commits` (OR and apply once), `size_undrawn_x11_floats`
(ignores it -- already inside `apply()`).

Evidence: harness pin
`fullscreen::tests::transitions::a_learned_minimum_reaches_the_client_without_further_input`
-- a refusing `DrawSized` earns a client configure with the learned size in
the same flush, with no further input. Fails on the old shell code ("the
refusing frame earned no configure"), passes with the fix. Core pins in
`world::tests::frames` (`a_frame_reports_whether_it_changed_the_arrangement`,
`a_fullscreen_frame_reports_no_change`,
`a_floating_frame_reports_only_a_move`,
`only_a_frame_observation_can_report_change`). Full cheap set green on the
dev VM (nextest `scoot` + `scoot-core`: 1996 passed; `--features xwayland`:
1881 passed; clippy `-D warnings`; fmt; smoke-test 22 ok), plus the full
workspace run (2390 passed; the one failure is
`scootbg::control::tests::a_live_socket_with_a_full_backlog_is_refused_without_hanging`,
EMFILE under `ulimit -n 1024` in a binary that does not link `scoot-core` --
environmental, another agent's lane). Hot-path benchmark (200k
steady-state `FrameObserved`, release, dev VM): base 55–92 ns/event vs.
fix 75–94 ns/event -- overlapping ranges, no measurable change, no
allocation added (the first draft added a map lookup per frame and measured
+31 ns; reworked to read off the held borrow).

No README/protocols change: no new config, keybinding, CLI flag or IPC
surface -- a refusing client now learns smoothly instead of jumping at the
next unrelated event, which is not documentable behavior.

Original ticket below.

---

title: "A learned column minimum from a client frame is not applied until the next unrelated event"
status: "open"
area: "core"
priority: "low"
blocked: null
---

# Frame learning never flushes its own relayout

Filed 2026-09-23 by the PR #223 re-review (pre-existing). `observe_frame`
feeds `FrameObserved` into the core (`handlers.rs:~144`), which can raise a
learned minimum and re-scroll, but nothing calls `apply()` after it, so the
new column width and positions reach clients and the screen only at the next
event. PR #223's no-op-fullscreen fast path removed one place that used to
flush this by accident.

Fix: apply (or schedule one coalesced apply per dispatch) when the core
reports that a frame changed the arrangement — never per commit
unconditionally, since commits arrive at frame rate. Pin with a harness test:
a refusing frame changes the placed rect without any further input.
