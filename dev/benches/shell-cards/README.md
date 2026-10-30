# Shell card styles: marks, pill and typography repaint cost

What a notification stack's repaint and layout cost with each card style,
against `main`. Part of #807 (slice 3, card styles and typography).

## Method

`crates/scootnotify/examples/bench_styles.rs`: lays out five cards (mixed
heights, a progress bar, two buttons, three urgencies, five app names) under
each style, then times `view::render::paint` into a pooled `ARGB8888` buffer,
500 repaints per round after 20 warm-up repaints, 9 rounds per style, and
`layout::layout_styled` 200 times a round. It reports the best round's median
and 5th percentile (the cost when the machine is not interrupting the code:
the dev VM is shared and its noise comes in bursts).

The baseline is `main` at `39c0cd4b1` built from a `git archive` into its own
target dir, running the existing `bench_depth` (the default look, `shadow =
"none"`, solid fill). The branch ran the same `bench_depth` binary source, so the
two are the same harness. Main and branch ran alternately, six times each.

```sh
cargo build --release -p scootnotify --example bench_depth --example bench_styles
taskset -c 3 target/release/examples/bench_depth --font DejaVuSans.ttf --iters 500
taskset -c 3 target/release/examples/bench_styles --font DejaVuSans.ttf --iters 500
```

Machine: aarch64 dev VM (6 vCPU, NixOS), release profile, DejaVu Sans 13 px,
`taskset -c 3`. Final branch commit: `95af2f594`. Raw output, including the
first set of runs taken before the corner-search change: [`raw.txt`](raw.txt).

## Result

Repaint of the stack, microseconds, p5 (median of the runs' best-round
figures; the range is the best and worst run). Main and the branch's
`bench_depth` rows are six runs; the `bench_styles` rows are three:

| Style | Surface | p5 µs (range) |
|---|---|---|
| main, default look | 360x474 | 127.8 (121.8-132.6) |
| branch, default look (`bench_depth`) | 360x474 | 128.4 (84.9-130.0) |
| branch, default look (`bench_styles`, three runs) | 360x474 | 130.7 (117.3-131.5) |
| `card.mark = stripe` | 360x474 | 121.2 (87.2-132.6) |
| `card.mark = dot` | 360x474 | 132.6 (80.5-135.2) |
| `card.mark = badge` | 360x474 | 139.4 (139.1-139.8) |
| `badge` + `card.app-accent` | 360x474 | 139.0 (84.2-139.8) |
| `card.shape = pill` | 360x474 | 170.4 (103.5-170.8) |
| pill + soft shadow + badge + accent | 384x498 | 322.5 (195.8-323.1) |
| `title.weight = 400` | 360x474 | 122.8 (77.3-125.0) |
| `title.weight = 900`, `title.spacing = 2` | 360x474 | 137.7 (137.5-139.8) |

Layout (a content change: an arrival, a close, an expiry), microseconds for
five cards: 19.8 default, 21.7 stripe, 19.0 dot, 20.9 badge, 27.3 badge with
the app accent (the hash and the contrast search, once per card), 22.1 pill.

Reading it:

- **A look that sets none of the tokens costs what it did.** The default look's
  repaint is 128 µs p5 on main and 128 µs on the branch, byte-identical output
  (every existing golden is unchanged).
- **A mark costs nothing measurable at the p5.** A stripe is three columns of
  stores, a dot one small fill, a badge a different plate color; the badge's
  extra 10 µs is the icon slot every card now shows.
- **The pill is the one real cost: about 40 µs for five cards** (170 against
  130). A 36 px corner has 36 corner rows with a few anti-aliased pixels each,
  against 8 for the default radius. The fill finds each row's clear, edge and
  whole stretches by binary search instead of evaluating every pixel; before
  that change the pill was 471 µs p5 (3.5 times the default), as the first set
  of runs in `raw.txt` shows. The output is byte-identical (the
  pixel-by-pixel reference test covers radii up to 400).
- **A heavier weight costs one more strike of the summary glyphs** (the summary
  is 5 short lines), and spacing is one add per character.
- Idle cost is unchanged: nothing here adds a timer or a wakeup, and the accent
  is computed at layout, not at repaint. The repaint still allocates nothing
  (`a_repaint_with_every_style_allocates_nothing_after_warm_up`).
