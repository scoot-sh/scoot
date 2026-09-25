---
title: "Rounded clip and focus ring follow the layout slot, not the client's committed size: a client that doesn't fill its slot shows mismatched corners (gh #205, reopened) — DONE"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Clip and ring the committed size; tell clients they're tiled — DONE

RESOLVED 2026-09-24 (PR #240), together with
[`ring-outer-corner-shoulders-done.md`](./ring-outer-corner-shoulders-done.md)
in one PR (the user asked for both in one). The ticket as filed is kept
verbatim below the resolution.

**Scope of "done".** Default foot, mpv, and any client that fills its slot
or draws a plain rectangle now match at every corner. One residual is
filed rather than fixed: libadwaita dialogs round their own corners wider
than scoot's radius, so a crescent of background still shows between their
curve and the ring (all four corners fail the pixel check, 53–59
background px each at 1.5). That is much closer than `main`, where the ring
circled the empty slot, but it is not a match. See
[`core/client-rounded-corners-vs-ring.md`](../core/client-rounded-corners-vs-ring.md).

**SHA mapping.** The branch was rebased onto `85c662a` (a backlog-only
commit) during review, so the evidence SHAs below are pre-rebase:
`b755f11` → `e1bab6d`, `7164226` → `c71a2fb`, `557fa8e` → `cec98d5`. Each
pair has an identical `crates/` tree (`git diff <old> <new> -- crates` is
empty); the rebase added only `docs/backlog/core/floating-windows.md` and
its index line.

## Resolution

Both halves, as the ticket suggested.

**(a) Tiled states.** `fullscreen::set_layout_states` (was
`set_fullscreen_state`) puts a window's pending state in step with the
layout: `fullscreen`, or all four `tiled_*` states. The two are exclusive,
and scoot has no floating windows. Its two callers, `apply()` and
`answer_fullscreen_request`, each send it in the same configure as the size,
so PR #223's size-and-bit atomicity holds for the tiled bits too. The
initial configure carries them (`apply()` runs in `add_window`). A
`foot --fullscreen` first frame carries `fullscreen` and no tiled state
(pinned). Smithay's `into_filtered_states` removes the tiled states for a
client bound below xdg_toplevel v2. `observe_frame` reads only size and
`fullscreen` from the acked state, so it is untouched.

**(b) Clip and ring what was drawn.** New `drawn.rs`. The drawn rect is
the slot's origin plus, per axis, `min(slot, committed window geometry)`.
With nothing committed it falls back to the slot. The origin is the slot's
top-left because that is where the window's geometry origin is mapped, so
a short client only moves its right and bottom edges. It has three readers
and no writers. None of them configures a client or teaches the core:

- the rounded clip in `window_elements`, from the geometry it already
  reads;
- both ring paths, through a `drawn` closure that `ring_elements` builds
  from `&state.windows`. The compiler forced all three callers: the frame
  body, the scanout tier and the capture-cursor re-render. The square ring
  follows too, so the ring and IPC `rect` never disagree at radius 0;
- IPC `windows` `rect`.

**Rect semantics (decided).** `rect` is what is drawn and clickable, and
it never reaches past the slot. Hit-testing was always by surface, so a
click in the undrawn part of a slot never reached the window. The old
`rect` promised an area that was not clickable. The clamp applies only to
visible windows: a hidden one reports its layout frame unchanged. (Before
the review this also applied to a window stacked behind a fullscreen
sibling, which then reported the sibling's origin with its own size.) A
fullscreen window reports the output's rect as long as it draws the whole
output. `docs/ipc.md` says both.

**Re-map (review round 1).** Smithay discards everything pending when a
toplevel unmaps (`xdg/mod.rs`, the `got_unmapped` reset). Nothing
re-arranged on a re-map, so the configure answering it carried no size,
no tiled states, no `activated`, and, because the decoration mode is
re-sent with every initial configure, `ClientSide`. That told a re-mapped
foot to size itself, round to cells, and draw its own titlebar. The review
found the size and states; the decoration mode came to light while tracing
the same reset. `State::restore_layout_state` now rebuilds all four from
the layout before `send_initial_configure`:

- the size, only for a visible placement (a hidden one may be stacked
  behind a fullscreen sibling, whose frame is not its size);
- the layout states;
- `activated` if the window is focused;
- `ServerSide` under `prefer_no_csd`.

On a first map every value is already pending, so nothing changes there.

Pinned in `fullscreen/tests/transitions.rs`:
`a_re_mapped_window_is_told_its_size_and_that_it_is_tiled` (on `fbb1551`
the re-map configure was `0x0`, `tiled: false`) and
`a_re_mapped_window_keeps_server_side_decorations` (on `fbb1551` the modes
were `[ServerSide, ClientSide]`). The same file pins
`a_window_hidden_behind_a_fullscreen_sibling_reports_its_layout_rect`, and
`rounded/tests/committed.rs` has `a_version_one_client_is_never_sent_tiled_states`
as a guard; that one passes on both sides.

Live on `340f3c7` (release `9d03ae27…`, headless 2952x1660, default foot).
Open foot, `action close`, open it again: both rounds report the full
slot, 966x1083 at 1.5 and 1458x1636 at 1.0, and all four corners pass in
both rounds. A newly spawned foot is a fresh toplevel, not a re-map (foot
never unmaps a live window), so the re-map path itself is covered by the
harness tests above. Evidence is in `~/evidence/r205/review1/`.

**Resize behaviour (decided).** The drawn rect is read fresh each frame
from committed state, so the ring and clip move on the frame the client's
commit lands. A growing slot keeps the ring around the old content until
the larger frame arrives. On `main` the ring jumped to the new slot at
once and circled empty slot for a frame or two. A shrinking slot clamps
the ring at once, as before. Nothing is tracked per resize.

**Cost.** Per placed window per frame there is one extra `Window::geometry()`
on the ring path: two uncontended locks and no allocation. The clip path
already read the geometry.

There is one new trigger, and it is client-driven. The painted ring's key is
the drawn size, so a client whose committed size changes on every commit
repaints its two strips on every such commit. Each repaint is two
strip-sized `MemoryRenderBuffer` copies plus imports, about 158 KB each for
a 966-wide window at 1.5. On `main` only a layout change could trigger a
repaint. The trigger is bounded by the client's commit rate, and each of
those commits also uploads a full-window buffer. Measured below.

## Evidence

Fail-first commit `b755f11` (main `1a8c5c1` plus tests only), fixed at
`7164226`. Dev VM, 9p mount, `cargo nextest run -p scoot`:

- `rounded/tests/committed.rs`:
  - a client drawing `(13, 7)` logical px short of each configure gets a
    matching clip and ring at 1.0, 1.5 and 2.0;
  - the square ring hugs it too;
  - the ring follows short, full, then short-by-another-amount redraws;
  - IPC `rect` is the drawn rect: short, full, and clamped past the slot.

  All six failed on `b755f11` under pixman and GLES and pass at `7164226`.
  On `b755f11` the red census counted the unclipped short corners: at 1.5,
  54824 against 54686 expected. IPC reported `116x243` for a `103x236`
  window.
- `fullscreen/tests/transitions.rs` (client bound at xdg_wm_base v3):
  - tiled on every edge from the first sized configure;
  - fullscreen replaces tiled, and leaving restores it;
  - the fullscreen-first frame is not tiled;
  - an invisible window made fullscreen over IPC loses tiled.

  The first two failed on `b755f11` (`tiled: false` everywhere).
- `drawn/tests.rs`: `clamp_to_slot` edge cases (zero or negative sizes,
  per-axis clamp, `i32::MAX`).

Live repro (release binaries, sha256 `ffce3c34…` before, `f9eec41e…` after,
`~/evidence/r205/` on the dev VM). Default `foot` (`resize-by-cells` on,
alpha 0.95), `corner_radius = 10`, `focus_ring_width = 4`, the coordinator's
`check_corners.py` with an outer-arc check added:

| run | before (`1a8c5c1`) | after (`7164226`) |
| --- | --- | --- |
| headless 2952x1660 @1.5 | TL inner pass; TR, BL, BR fail (bg inside clip 242/506/518); outer arc 11/21 rows at every corner | all four pass, outer arc 21/21 |
| headless @1.0 | TR, BL, BR fail; outer 8/14 | all four pass, 14/14 |
| `--tty` pixman 1600x1000 @1.5 | BL, BR fail; outer 11/21 | all four pass, 21/21 |
| `--tty` pixman @1.0 | TR, BL, BR fail; outer 8/14 | all four pass, 14/14 |

foot's commits under `WAYLAND_DEBUG` at 1.5. Before, it was told 966x1083
with states `array[4]` (activated) and committed a viewport of 958x1068.
After, it is told `array[20]` (activated plus four tiled) and commits
966x1083. It fills its slot.

(b) was also checked on its own, with an uncommitted experiment binary
that never sends tiled states (`2afb3097…`). foot under-fills again (IPC
`rect` 958x1068 at 1.5, the 1437x1602 physical of the report) and all four
corners still pass, 21/21 outer rows. At 1.0 it reports 1452x1628, also all
pass.

Other clients, headless @1.5, before → after (Qt and GTK 3 apps are not
installed on the dev VM, so not measured):

- `zenity --info` (GTK 4): a fixed-size dialog. It keeps a 300x223
  geometry either way, and its CSD margin shrinks 22 → 20 once tiled.
  Before, the ring circled the 966x1083 slot. After, it hugs the dialog and
  IPC `rect` is 300x223.
- `mpv` testsrc (`--vo=wlshm`): keeps its own 644x722 size tiled or not.
  After, the ring and rounded corners follow the video.
- `weston-terminal`: ignores the tiled states. It commits a 967x1087
  geometry, larger than its slot, and keeps it for good (not only mid-resize).
  So the rect is clamped to the 966x1083 slot, the same as before.
- Residual: re-measured on `340f3c7`, `check_corners.py` fails all four of
  zenity's corners (the libadwaita crescent described at the top): 53–59
  background px inside the clip per corner at 1.5, 19 at 1.0. The outer arc
  passes.

Everything above (binaries with `SHA256SUMS`, logs, PNGs, `windows`
JSON, `WAYLAND_DEBUG` client logs, gate logs) is under `~/evidence/r205/`
on the dev VM: `live/`, `clients/`, `bin/`, `gate-<sha>/`.

**Cache key.** The live runs, the client measurements and the screenshots
were all captured on `7164226`. Everything after it in `crates/` is:

- doc comments;
- a parameter rename (`placement` → `rect`) in `rounded::clip_rect` and
  `ring_layout`;
- `#[cfg(test)]` code only: the two benches below, plus the bench client
  now destroying the buffer it replaces, as real clients do (the per-client
  fd bound of PR #239 otherwise refuses a client that re-attaches every
  frame).

None of it changes behaviour.

## Benchmarks

Release *test* binaries OOM the 3.9 GB dev VM and are banned there, so
these are dev-profile runs. They are good for before/after on the same
machine, not for absolute numbers. Command:

```
cargo test -p scoot --bin scoot -- --ignored --nocapture --test-threads=1 --exact \
  compositor::headless::bench::render_frame_cost compositor::headless::bench::rounded_corners_cost
```

Pixman ran three times per tree and GLES once (`SCOOT_TEST_RENDERER=gles`).
Before is `1a8c5c1`, after is `7164226`. The files are
`~/evidence/r205/bench-{before-1a8c5c1,after-7164226}-{pixman*,gles}.out`.
The table gives square/radius-12 medians per frame, the paired median delta
in brackets, and one cell per run:

| scene | before | after |
| --- | --- | --- |
| render, empty (BEST) | 23.85 / 23.96 / 23.66 µs | 23.88 / 23.78 / 23.81 µs |
| render, 8 windows (BEST) | 77.88 / 77.75 / 76.98 µs | 77.53 / 77.21 / 77.45 µs |
| rounded single | 140.5/158.0 (+13.7%), 138.9/157.4 (+13.2%), 141.2/155.4 (+11.2%) | 140.6/157.1 (+11.6%), 138.0/154.2 (+12.0%), 138.7/157.8 (+13.9%) |
| rounded tiled3 | 239.7/274.4 (+15.6%), 239.9/276.5 (+14.4%), 238.7/274.3 (+14.9%) | 238.7/271.8 (+14.6%), 237.3/271.3 (+14.4%), 237.0/272.2 (+14.2%) |
| rounded overhang3 | 305.0/330.3 (+8.1%), 305.3/333.0 (+9.1%), 303.2/327.7 (+8.1%) | 301.9/327.3 (+7.8%), 302.6/328.6 (+8.4%), 302.2/328.1 (+8.6%) |
| GLES rounded single / tiled3 / overhang3 | +34.9% / +40.6% / +18.6% | +30.0% / +40.0% / +16.1% |

Every after number sits inside the before runs' spread, so there is no
measurable change. `render_frame_cost`'s windows are core-only (no
`Window`), so they cannot see the new lookup. `rounded_corners_cost` has
real clients and does.

The client-driven repaint, from the two new ignored benches:

- `headless::bench::rounded_resize_churn_cost`: one real client at radius
  12 commits a fresh full-window buffer every frame, either the same size
  each time or alternating 7 px smaller. Pixman paired median was
  +0.7% / +0.7% over two runs (about 30.8 → 31.0 ms per commit+frame in
  the dev profile, mostly the client filling its buffer). GLES was +0.5%.
- `decorations::tests::ring_repaint_cost`: one `build_strips` took
  432–436 µs at 966x1083 @1.5 and 287–291 µs at 1458x1636 @1.0 in the dev
  profile, against a 194–204 ns cache hit.

Filed 2026-09-24 from a live re-verification of gh #205 on `main` `1f2fe5c`
(scale 1.5, `corner_radius = 10`, `focus_ring_width = 4`). Serves
**daily-drive** (the default look of the default terminal) and
**computer use** (window rects that match what is drawn).

## What is wrong

foot (default `resize-by-cells=yes`) commits a buffer rounded down to whole
cells: 1437x1602 physical in a 1449x1625 slot. scoot's rounded clip and ring
use the layout slot (`clip_rect(placement)`), so the ring rounds a corner
the content never reaches (a green gap between square content and a curved
ring) at the right and bottom corners. Not fractional-specific (reproduces at
1.0). With `resize-by-cells=no` all four corners pass a per-pixel check, so
PR #207's fix itself holds.

foot's CHANGELOG says cell-rounding applies to *floating* windows; scoot
sends no `xdg_toplevel` tiled states (`tiled_left/right/top/bottom`, v2+).

## What to do (both, likely)

1. Send tiled states for windows in the scrolling layout (not for
   fullscreen, which has its own state; check what floating means for
   scoot, which has none today). Measure that foot, GTK, Qt then fill the
   slot exactly.
2. Clip and ring what the client actually committed (the window geometry
   of its last commit, clamped to the slot, centred/aligned per layout
   rules), so any client that still under-fills (e.g. a fixed-size dialog,
   an old client) gets a matching ring. Decide how the ring and rounded clip
   follow a mid-resize commit without flicker.

## Evidence

Repro artifacts: scratchpad `r205/` (the coordinator has the paths) and
`check_corners.py` (a per-corner pixel check that can become a harness
test). Fail-first harness test with a client committing a buffer smaller
than its slot; live foot defaults at 1.0 and 1.5 on headless and `--tty`,
all four corners passing; screenshot for the issue.
