# Shell motion: what an animating notification stack costs

Part of #807 (slice 2, motion). What a stack costs per frame while cards move,
what starting a motion costs, and that a stack at rest or a look without
motion costs what it did.

## Method

`crates/scootnotify/examples/bench_motion.rs` lays out five cards (mixed
heights, a progress bar, two buttons, three urgencies, a `soft` shadow), then
for each scenario starts a motion (`Motion::retarget` and the layers'
rasterization, timed once) and times 400 animated frames after 20 warm-up
frames, spread over the first half of a 300 ms spring (every card still
moving). A frame is the whole per-tick path: `Motion::settle`, the stale-layer
check, the static paint, the composites, into a pooled `ARGB8888` buffer
(pixman-tier: no GPU anywhere). Nine interleaved rounds per scenario, best
round reported, median and 5th percentile as in [`shell-depth`](../shell-depth/README.md)
(the dev VM is shared; its noise comes in bursts that move the median and
leave the low tail alone).

```sh
cargo build --release -p scootnotify --example bench_motion --example bench_depth
taskset -c 3 target/release/examples/bench_motion --font DejaVuSans.ttf --iters 400
```

The "motion = none costs nothing" claim is measured against `main` at
`20a758eab`, built from a `git archive` into its own target dir, running the
unchanged `bench_depth` of that tree alternately with this branch's, six
times each.

Machine: aarch64 dev VM (6 vCPU, NixOS), release profile, DejaVu Sans 13 px,
`taskset -c 3`. Branch commit `HEADSHA`. Raw output:
[`raw.txt`](raw.txt).

## Result

Microseconds per frame, at commit `HEADSHA` (three runs on a busy VM; the
earlier quiet-window figures were 97 static and 152 to 231 animated and are in
the first run of the previous `raw.txt`). Best median and best p5 of the three:

| Scenario | Surface | Median | p5 |
|---|---|---|---|
| static repaint, 5 cards (what any stack at rest repaints) | 384x498 | 306 | 225 |
| enter `slide`, one arrival on 5: animated frame | 384x522 | 624 | 459 |
| enter `scale`, one arrival on 5: animated frame | 384x522 | 1178 | 584 |
| restack, close the middle of 5: animated frame | 384x508 | 393 | 295 |
| restack, bottom anchor: animated frame | 384x508 | 442 | 329 |
| start a motion (retarget and rasterize the layers) | | 177 to 1063 | |

The worst animated frame is 1.2 ms median (`scale`), against a 16.6 ms frame at 60 Hz.

Reading it:

- **An animated frame is 1.3 to 2 times a static repaint, and well inside
  the budget.** Every card in a stack that restacks is a layer being
  composited (a pure blit with an alpha and, for `scale`, a bilinear
  resample), plus the clear and the pill. Text is not re-rasterized per frame:
  the layers are painted once when the motion starts and again only when a
  card's content or hover changes.
- **Starting a motion costs about one repaint** (the layers' rasterization,
  paid on the content change that causes it, never per frame).
- **A frame tick does not lay out.** The daemon turn that a frame callback
  wakes skips the relayout and the store's ordered view when nothing but the
  Wayland fd spoke. One card sliding in over 300 ms: 14 frames requested, 16
  commits, 7 calls to `Popup::refresh` with the gate and 35 without
  (`raw.txt`; those gate on/off counts come from an uncommitted, instrumented tree, not from the committed code).
- **`motion = none` costs nothing.** `bench_depth` on main and on this branch,
  the default look and `soft`, six alternating runs: the 5th percentile is
  131 to 134 µs (default look) and 221 to 226 µs (`soft`) on both in the
  runs the VM was busy; the one quiet window each side caught read 82 and
  137 µs on the branch. The static paint path is `paint_stack` with a
  closure that skips nothing; the motion state is never built.

## Idle

A stack at rest with a motion look asks for nothing. Pinned by
`crates/scootnotify/tests/motion_tests.rs` against a real headless scoot:

- after the last frame of a 300 ms slide (14 frame requests observed), five
  more seconds add no `wl_surface.frame` request and no voluntary context
  switch of the daemon (`test_support::wakeups`);
- the same after a restack, after a burst of 20 arrivals, and after the last
  card fades out (surface and buffers unmapped, no frames).

There is no timer: the animation is paced by one outstanding frame callback at
a time, and the last animated frame requests none.
