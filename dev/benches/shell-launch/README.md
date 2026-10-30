# Shell launcher: what the panel's depth and motion cost

Part of #807 (slice 4, scootlaunch). What an open panel costs at rest and
after a keystroke, what a frame costs with a shadow, and what a frame of the
entrance, the fade-out and the sliding highlight costs, against the default
look (`shadow = none`, `motion = none`), which paints the bytes and takes the
code path it always did.

## Method

Frames: `bench_effects` (an `#[ignore]` test in
`crates/scootlaunch/src/panel/staged_tests.rs`) draws the apps panel (13 px
font, 30 em wide, 10 rows, 6 entries, the test font) into a pooled buffer
the way the app does, on a made-up clock, at scale 1 and scale 2. 300 timed
frames after 20 warm-up frames, nine rounds, median and 5th percentile of
all 2,700 (the dev VM is shared; its noise comes in bursts that move the
median and leave the low tail alone). Animated frames are spread across the
run at 16 ms steps, so each is a different pose. Release profile
(`opt-level = 3`, fat LTO), `taskset -c 3`:

```sh
cargo test --release -p scootlaunch --bin scootlaunch bench_effects -- --ignored --nocapture
```

The "zero cost" claim is measured against `main` at `4e8293664`, extracted
with `git archive` into its own tree and target dir, running the same
panel at the same sizes through `main`'s own `paint_panel`, three runs each.

Idle and keystroke CPU: `bench_idle_cpu` (`crates/scootlaunch/tests/idle.rs`)
opens the real `scootlaunch` on a headless `scoot`, lets it settle, reads the
process's user plus system ticks from `/proc/PID/stat` over 5 s idle, types
one character, waits 1 s, and reads 5 s more. `SCOOTLAUNCH_TEST_BIN` runs the
same test against another build: here the debug binary built from `main`
(copied aside before the first edit) against this branch's debug binary,
alternating, three times each.

Machine: aarch64 dev VM (6 vCPU, NixOS). Raw output: [`raw.txt`](raw.txt).
The frame benches ran on the working tree on top of `4e8293664`, before the
PR's commits; the non-test sources of the final commit (`panel.rs`, `stage.rs`,
`motion.rs`, `look.rs` and scootui's `layer.rs`) are the ones they ran, and only
tests and docs changed afterwards. The idle and keystroke CPU numbers were
captured by the verification run at the final commit.

## Result

### The default look costs what it did

Microseconds per frame, `paint_panel` straight into the buffer (every open and
every keystroke of a look with no shadow and no motion):

| Surface | `main`, median | `main`, p5 | branch, median | branch, p5 |
|---|---|---|---|---|
| 390x282 (scale 1) | 292.5 to 311.9 | 158.0 to 162.0 | 311.4 to 311.8 | 157.9 to 168.1 |
| 780x564 (scale 2) | 1115.4 to 1179.4 | 612.7 to 676.0 | 1043.5 to 1177.2 | 598.0 to 657.6 |

The same bytes, too: the panel paints one FNV fingerprint over three looks,
three scales, four list sizes and an error line, recorded on `main` before the
first edit and pinned in `the_default_look_paints_the_bytes_it_always_did`.

### An open panel at rest, and after a keystroke

CPU ticks (1/100 s), debug builds, three alternating runs at the final commit
(an earlier run of the same three at the tree before the last test edits gave
471, 473 and 472 for `main`, and 0 for the branch):

| | `main` | branch |
|---|---|---|
| 5 s idle after the open | 0, 0, 0 | 0, 0, 0 |
| 5 s after one keystroke | 473, 472, 472 | 0, 0, 0 |

`main` redraws forever after a keystroke (a `wl_buffer.release` marked the
panel dirty, so each commit's release drew again): 109 commits in 1.5 s
after the key, then about 70 a second, 94% of a core. The branch commits once
for the keystroke. That is a fix in this PR, separate from the effects; the
numbers above are why it is one. With every effect on (glow, gradient, a
spring), an open panel asks for 3 to 4 frame callbacks and 5 to 6 commits
while it opens and nothing in the 3 s after (`a_panel_at_rest_asks_for_nothing_with_every_effect_on`);
a shadow alone never asks for a frame callback.

### What a frame costs with the effects on

Medians of three runs, microseconds, `[p5]` in brackets:

| Frame | Scale 1 (414x314 surface) | Scale 2 (828x628 surface) |
|---|---|---|
| default look, paint straight | 311 [158] | 1100 [600] |
| `soft`, repaint (a key) | 556 to 683 [347 to 396] | 2340 to 2570 [1420 to 1520] |
| `deep`, repaint (a key) | 652 to 771 [392 to 422] | 2630 to 2910 [1660 to 1780] |
| `glow`, repaint (a key) | 636 to 696 [360 to 370] | 2420 to 2500 [1480 to 1590] |
| a panel at rest, present (one copy) | 15 to 21 [13 to 16] | 95 to 105 [53] |
| entrance frame, scaled and faded | 2230 to 2430 [405 to 460] | 9040 to 9050 [1750 to 1870] |
| fade-out frame, scaled and faded | 2450 to 2670 [590 to 680] | 9980 to 10390 [2550 to 2690] |
| sliding highlight, repaint and present | 657 to 685 [26] | 2140 to 2390 [880 to 1490] |

Reading it:

- **A shadow roughly doubles a repaint** (the panel is painted into a layer,
  the shadow under it, then the two composed) and a repaint happens once per
  keystroke, never per frame of a motion.
- **A frame of a motion that only moves the panel costs one blit.** At rest
  the present is a single copy of the scene (15 to 105 us). A scaled frame
  resamples the whole surface bilinearly and is the expensive one: 2.3 ms at
  scale 1, 9 ms at scale 2, against 16.6 ms at 60 Hz. The p5 column is the
  frames near the end of a run, where the scale is within a 256th of one and
  the copy is exact. The entrance is about 10 frames at 160 ms, so the cost
  is a few tens of milliseconds of CPU per open, then nothing.
- **The sliding highlight repaints the panel** each frame it moves (the
  highlight is part of the panel's pixels) and presents it: about the cost of
  a `soft` repaint.
- Skipping the clear source pixels in `Layer::composite` and reading its two
  source rows as slices made the scaled frames about 10% cheaper
  (`raw.txt`: 10.0 to 10.4 ms before, 9.0 ms after, at scale 2). Filed as
  `launcher-scaled-composite-cost` for the rest.
- Pinned, not benchmarked: a frame that only advances a motion allocates
  nothing (`an_advancing_frame_allocates_nothing`) and repaints nothing
  (`a_frame_that_only_advances_the_motion_does_not_repaint_the_panel`).
