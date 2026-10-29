# Shell depth: shadow and gradient repaint cost

What a notification stack's repaint costs with each depth token, against the
repaint before the tokens existed. Part of #807 (slice 1, depth).

## Method

`crates/scootnotify/examples/bench_depth.rs`: lays out five cards (mixed
heights, a progress bar, two buttons, three urgencies) under each style, then
times `view::render::paint` into a pooled `ARGB8888` buffer, 500 repaints per
round after 20 warm-up repaints, 9 rounds per style. It reports the best
round's median and 5th percentile. The 5th percentile is the cost when the
machine is not interrupting the code: the dev VM is shared and its noise comes
in bursts, which move a median and leave the low tail alone.

```sh
cargo build --release -p scootnotify --example bench_depth
taskset -c 3 target/release/examples/bench_depth --font DejaVuSans.ttf --iters 500
```

The baseline is `main` at `b233b8ab1` built from a `git archive` into its own
target dir, with the same harness trimmed to the one style that code has (no
`Shadow`, no `shadow` argument to `paint`). Main and branch ran alternately, six
times each. Raw output: [`raw.txt`](raw.txt).

Machine: aarch64 dev VM (6 vCPU, NixOS), release profile, DejaVu Sans 13 px,
`taskset -c 3`. Branch commit: `d5bc1c9d1`.

## Result

Time per full repaint of the stack, microseconds, over the six runs
(median of the six runs' best-round figures; the range is the best and worst
run):

| Style | Surface | Median µs (range) | p5 µs (range) |
|---|---|---|---|
| main, no depth tokens | 360×474 | 1176 (1054–1184) | 732 (730–733) |
| branch, `shadow = "none"` | 360×474 | 158 (59–180) | 133 (59–134) |
| branch, `none` + `fill = "gradient"` | 360×474 | 181 (60–220) | 121 (60–133) |
| branch, `soft` | 384×498 | 266 (94–301) | 224 (94–225) |
| branch, `soft` + `gradient` | 384×498 | 312 (95–382) | 224 (95–226) |
| branch, `glow` | 392×506 | 351 (106–450) | 258 (106–259) |
| branch, `deep` | 416×530 | 489 (147–516) | 365 (147–367) |

Reading it:

- **`shadow = "none"` costs nothing extra: it is faster than main.** It builds,
  pads and paints no shadow; the surface is the same 360×474. The gain is the
  rounded fill, which now writes the straight runs between the corners as runs
  (an opaque run is a plain store) instead of computing coverage and blending
  every pixel. The output is byte-identical: the default look's repaint
  fingerprint (`the_default_look_paints_the_bytes_it_always_did`) was recorded
  on `b233b8ab1` before the change, and a randomized test compares the runs to
  the old per-pixel fill byte for byte.
- **A cached shadow adds about 90 µs (`soft`) to 230 µs (`deep`) at the p5, per
  repaint of five cards.** That is on top of a repaint that already costs
  less than main's did. A repaint happens when the stack changes (an arrival, a
  close, an expiry, a hover); an idle stack repaints nothing and the shadow adds
  no timer or wakeup, so idle cost is unchanged.
- **Surface growth is the compositor-side cost.** `soft` grows the surface by
  12% (170,640 to 191,232 px), `glow` by 16%, `deep` by 29%. The extra area is
  mostly transparent; a shadowed stack costs the compositor one larger
  `ARGB8888` blend, only while it is on screen.
- **The tables are small and built off the repaint.** One 1-D edge table
  (`2 × blur + 1` bytes) and one corner table per distinct corner radius
  (`(radius + blur)²` bytes: 676 B for the default radius 14 and a 12 px blur).
  Building them is part of the "layout+tables" column in `raw.txt`
  (about 40 to 90 µs with the layout, on a content change only). A repaint
  allocates nothing: `a_repaint_with_depth_allocates_nothing_after_warm_up`
  counts it for `soft`, `deep` and `glow`.

Caveat: the baseline moved between sessions. In one quiet window early on, main
measured 317 µs (median and p5) and the branch before the run-fill change 383 µs,
so the 732 µs above is main's figure in the sessions that ran alternately, not its
best. The branch's best (59 µs) was a quiet window too. The ordering in the table
held in every alternating run; the absolute numbers are this VM's.
