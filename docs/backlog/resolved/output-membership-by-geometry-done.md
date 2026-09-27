---
title: "Three readers still answer \"which output is this window on\" from its bounding box, not its placement — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Output membership read from geometry — RESOLVED

RESOLVED 2026-09-27 (PR #289, branch `output-membership-by-placement`).
All three sites now read `output_clip::placed_on`:

- **Frame callbacks** (`headless.rs`): the per-output loop fires a window
  exactly when its stamp names that output (`placed_on(window) == Some(id)`,
  using the render loop's own id) — one user-data read per window, where the
  overlap read walked each window's surface tree. Unmapped windows are told
  by none; mapped-but-never-committed ones keep their stamp, so the first
  frame still unsticks them.
- **`wl_surface.enter`/`leave**: new `State::reconcile_surface_membership`
  (in `output_clip.rs`), run after both `Space::refresh()` calls (the render
  tail and `remove_output`), which corrects `refresh`'s overlap bookkeeping
  to the placement: leaves every crossed non-placed output, enters the
  placed output even where the box sits wholly off it (a focused-away
  fullscreen window) with the window's own element-local geometry as the
  overlap. Sound because Smithay's `Output::enter`/`leave` dedupe per
  surface — the corrective leave is a no-op on repeat, and `refresh`'s own
  leave when the overhang ends finds the surface already gone. No
  `Space::outputs_for_element` reader exists in scoot (verified by grep —
  only Smithay's own `refresh` maintains that map, and drawing/hit-testing
  never consult it), so the reconcile owns this membership outright.
  Allocation-free on the steady path (one bbox read plus integer
  intersections per window per output; tree walks only for actual
  overhangs).
- **`output_of_window`**: stamp first (live outputs only), then the core's
  `Placement::output` where the window has one, then the pointer's output,
  then the primary — the ticket's preferred fallback order, so an announced
  but never placed window still opens on the pointer's output.

Caller audit: the one other geometry-overlap frame decision,
`x11_unmanaged_frames` for override-redirect X windows, is consistent by
design (those windows have no placement and are *drawn* on every output
they overlap, so being paced by each is correct) and was left alone. No
fourth reader turned up.

Evidence: four new harness pins, each verified fail-first against the old
code (stashed impl, tests-only tree — all four fail there): rendering only
the neighbouring output leaves an overhanging window's callback pending
(`an_overhanging_window_is_paced_by_its_own_output`), an overhanging
window ends entered only on its own output with every neighbouring enter
paired to a corrective leave (`an_overhanging_window_enters_only_its_own_output`,
plus the same assertion for the wholly-off fullscreen case),
`output_of_window` names the placed output across an overhang and the
core placement for an unmapped window
(`a_window_overhanging_the_first_output_is_reported_on_the_second`,
`an_unmapped_window_falls_back_to_its_core_placement`). Full workspace
nextest: 2384 passed, 1 failed — the failure is
`scootbg … a_live_socket_with_a_full_backlog_is_refused_without_hanging`,
which fails identically on unmodified `main` (another agent's lane, left
alone). `clippy -D warnings` and `fmt --check` clean; `smoke-test.sh`
rc=0 (22 oks). Benchmark (`render_frame_cost_multi_output`, best of 5,
pixman, dev VM): ranges overlap main's on every scene (e.g. 8 windows /
two outputs 323–357 µs vs main 195–310 µs across runs, empty desktop
66–68 / 132–136 µs vs main 71–85 / 133–174 µs) — VM noise dominates, no
regression signal; per-frame bbox walks strictly decrease (16 vs 24 in
that scene). No README/protocols change: no doc promises callback pacing
or enter membership, and no config/flag/IPC surface moved. Multi-output
headless harness only, per the ticket (no hardware needed).

Filed 2026-09-23 while fixing
[windows bleeding across outputs](../resolved/windows-bleed-across-outputs-done.md),
which made drawing and hit-testing follow the output a window is *placed*
on. Three other sites still decide by bounding-box overlap, so a window
whose rect crosses a shared edge (a column scrolled part-way off its output,
a fullscreen window focused away from) counts as being on the neighbour too.
None of them draws or delivers input to the wrong place, which is why they
were left out of that fix; each is a smaller inconsistency:

- **Frame callbacks** (`headless.rs`, the per-output loop after each frame):
  a window overlapping an output's geometry is sent a frame callback with
  that output as the token. An overhanging window's callback is fired by
  whichever of the two outputs renders first, so it is paced (and its
  presentation feedback attributed) by a screen it is not shown on.
- **`wl_surface.enter`/`leave`** (Smithay's `Space::refresh`, called from
  `headless.rs`): the client is told its surface entered the neighbouring
  output. Harmless while every output shares one scale; once per-output
  scale lands (`per-output-scale-mode.md`) a client picks its buffer scale
  from the outputs it has entered, and would pick the neighbour's.
- **`foreign_toplevel_management.rs`'s `output_of_window`**: reads the first
  output whose geometry overlaps the window's bbox, so a handle announced
  (or a `wl_output` bound) while a window overhangs the output before its
  own names the wrong `output_enter`; the next `apply()`'s membership diff
  (which reads the placement) corrects it.

## What to do

Read the placed output (`output_clip::placed_on`) at all three. It is set
by `apply()` from the placement beside `map_element` and cleared beside
every `unmap_elem`, so it is `Some` exactly while the window is mapped:
`None` for a window that is invisible, closed or never placed. That is what
the frame-callback and `wl_surface.enter` sites want (an unmapped window is
on no screen), but `output_of_window` also announces windows that are not
mapped -- keep its fallback for `None` (today the pointer's output; the
core's `Placement::output` is the better answer where the window has one),
rather than reading `placed_on` as "the output this window belongs to". The frame-callback change is the one with an
observable effect (which output's frame fires an overhanging window's
callback); pin it with a harness test that renders only the neighbouring
output and asserts the callback stays pending. The
`wl_surface.enter` half means not relying on `Space::refresh`'s overlap
bookkeeping for windows -- check what else reads `Space::outputs_for_element`
before replacing it.
