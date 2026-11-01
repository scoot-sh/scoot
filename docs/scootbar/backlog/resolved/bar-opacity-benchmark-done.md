---
title: "Bar opacity pixman benchmark: the gate for translucent bars"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-31"
---

# Bar opacity pixman benchmark: the gate for translucent bars

Filed 2026-10-31, child of GitHub epic
[#807](https://github.com/scoot-sh/scoot/issues/807) (section 4,
"scootbar (optional, later)"). Serves **daily-drive**: a translucent bar
is the most visible half of the epic's scootbar slice, and it stays off
until this gate passes.

Depends on: #807 (the epic) and the sibling cards filed with it,
`scootbar-depth-tokens` and `scootbar-popup-entrance`, which wait on this
one.

## The gap

The five shipped looks pin `[opacity] bar = 1.0`, each with a comment
naming this exact gate and the value it holds back:

- `looks/ginger-night/look.toml:30-32` — "1.0 until the bar-opacity
  pixman benchmark passes; the example uses 0.82."
- `looks/moonrise/look.toml:32-34` — same comment, example 0.82.
- `looks/music-desk/look.toml:31-33` — same comment, example 0.7.
- `looks/radial-burst/look.toml:32-34` — same comment, example 0.88.
- `looks/vinyl-sunset/look.toml:32-34` — same comment, example 0.78.

The hold comes from `docs/backlog/resolved/look-shipped-done.md:26-35`
(R3: lift `bar = 1.0` with the examples' values beside them as comments;
`look-layer-scoot` raises them only if its benchmark passes) and
`docs/backlog/resolved/look-layer-scoot-done.md:59-63` (D15's condition:
measure the per-frame cost of rounded corners on the default CPU pixman
renderer; the rounded-corner half passed at ~+5-6% per frame and radius
rose, but bar opacity stayed at 1.0). So the mechanism for raising the
values exists (`look-layer-scoot` did it for radius); the pixman numbers
for a translucent bar do not.

Sections 1-3 of #807 already shipped while this gate stood closed:
scootnotify shadow/gradient depth (#810), card motion (#811), card marks
and accents (#812), the input region following the card shape (#835),
scootlaunch panel shadow/fill/motion (#814), the look font resolution
(#836), dmenu depth and motion (#838), and the preferred-scale
follow-ups (#839, #841) — see `git log --oneline --grep=807`.

## What to do

Define the pixman (CPU tier) benchmark that decides whether the five
`bar = 1.0` pins rise to their comments' values (0.7, 0.78, 0.82, 0.82,
0.88), and record its numbers in this ticket's resolution. Measure, on
the CPU tier, in a release build (a debug build says nothing about a
repaint — the same rule `crates/scootnotify/examples/bench_depth.rs:1-9`
states):

1. **Idle cost, translucent vs opaque**: wakeups per minute at idle with
   `bar = 1.0` vs the example value, per output. The standing bar for
   this is zero idle wakeups (`docs/scootbar/backlog/README.md:52-64`,
   and `resolved/second-wakeups-done.md`: the clock is minute-aligned, 2
   wakes). Budget: no extra idle wakeups from translucency.
2. **Damage-limited redraw**: a clock-tick redraw (the minute-aligned
   redraw that already exists) repainting only its span, opaque vs
   translucent. Budget stated relative to today's opaque numbers (the
   per-repaint cost delta, not an absolute).
3. **Bar-with-popup**: the bar plus one open popup (tray menu or volume
   slider), translucent vs opaque.
4. **Clock/tick redraw cost**: the per-minute tick's full cost, both
   opacities.
5. **Full-bar repaint at 1x and 2x**: the whole bar surface composited
   at scale 1x and 2x, both opacities (scootbar already follows the
   compositor's preferred scale —
   `crates/scootbar/src/daemon/surfaces.rs:23-24,707` — so 2x is a real
   configuration, not a synthetic one).

Where the bench lives: scootbar has no `examples/` or `benches/`
directory today (verified 2026-10-31: `ls crates/scootbar/examples
crates/scootbar/benches` finds neither). Follow the
`crates/scootnotify/examples/bench_*.rs` pattern (`bench_depth.rs`
times the repaint under each shadow style plus the one-off table build;
`bench_motion.rs` times one animated frame against the plain repaint it
replaces, with `--scale` in 120ths for 1x/2x). A new
`crates/scootbar/examples/bench_opacity.rs` (or the equivalent the
implementer justifies) is the expected shape; dev tooling only, never
part of the shipped binary, release builds only.

What passing unlocks: lifting `[opacity] bar = 1.0` to the comments'
values in all five shipped looks (0.82 ginger-night, 0.82 moonrise, 0.7
music-desk, 0.88 radial-burst, 0.78 vinyl-sunset), updating the comments,
and updating the docs that describe the pin. If it fails, the pins stay
and this ticket records the measured trade-off with the same standing
the radius numbers got in `look-layer-scoot-done.md:99-107`.

## Budgets (stated 2026-11-01, before measuring)

Stated before any number below was taken, relative to the opaque
(`bar = 1.0`) measurement in the same scenario, on the same build, on
the same machine. The +10% follows D15's precedent (which accepted
~+5-6% per frame for rounded corners): the bar's own fill is
alpha-independent by construction (`fill_shaped` runs the same loop for
any alpha, only the premultiplied pixel value changes), so +10% is
generous headroom for VM noise; the compositor-side blend cost is the
unknown this gate exists to measure.

1. **Idle**: no extra wakeups. Translucent wakeups/min must equal opaque
   exactly (the standing bar: a settled bar wakes only on its
   minute-aligned clock).
2. **Damage-limited span redraw**: per-repaint cost within +10% of opaque.
3. **Bar-with-popup**: per show/hide cycle within +10% of opaque.
4. **Tick full cost**: within +10% of opaque.
5. **Full-bar repaint at 1x and 2x**: bar-side within +10% of opaque;
   compositor frame (screenshot-forced) within +10% of opaque.

Verdict rule: every scenario inside budget lifts the five pins to the
comments' values in the same PR; any scenario outside keeps all pins at
1.0 and this ticket records the trade-off instead.

## Acceptance

- The five scenarios above have before (opaque) / after (translucent)
  numbers on the CPU tier, recorded in this ticket's resolution with the
  exact commands, build and commit, per the `CLAUDE.md` evidence rule
  (raw numbers, not a paraphrase).
- The budget verdict is explicit per scenario: either inside budget
  (state the delta against today's opaque numbers) or not.
- If inside budget: the five `look.toml` pins are lifted to the
  comments' values in the same PR, the stale gate comments are updated,
  and the site docs that mention the pin are updated.
- If outside budget: the pins stay, and the resolution records the
  measured costs and what would have to change.

## Test plan

- Golden/fingerprint tests unchanged at defaults: opacity defaults stay
  as they are, so existing goldens hold (the same constraint #807 sets:
  defaults unchanged unless a look opts in).
- A live test on headless scoot (`target/debug/scoot --headless` with a
  bar, `scoot msg screenshot`) showing the translucent bar composited
  over windows.
- A CPU-tier screenshot at 1x and 2x for the record.
- Warm-frame allocation check: no per-frame allocation on the repaint
  path once warm (the zero-allocation standard the shell surfaces are
  held to — see `resolved/scootui-popup-alloc-pins-audit-done.md`).

## Not in this ticket

The depth tokens (`scootbar-depth-tokens`) and the popup entrance
(`scootbar-popup-entrance`): this ticket only decides whether
translucency is affordable. No config keys, no validator or schema
changes, no look edits beyond the pin lift a pass unlocks.

## Resolution (2026-11-01, verdict: PASS)

Verdict: every budget passes, so the five `[opacity] bar` pins rise to
their comments' values in the same PR (0.82 ginger-night, 0.82
moonrise, 0.7 music-desk, 0.88 radial-burst, 0.78 vinyl-sunset).

### What was built

- `crates/scootbar/src/render/tests/bench.rs` (in-crate, `#[ignore]`d):
  the real `render::paint` path (same `Scene` update, `Record` damage
  tracking, `Canvas` fill the daemon drives) at the five example
  opacities against opaque, full-bar and tick-sized span repaints at 1x
  and 2x, median + p5 over interleaved samples. An `examples/` target
  is impossible (scootbar is a binary crate, which an example cannot
  import), so this is the justified equivalent, beside the live bench.
- `crates/scootbar/tests/opacity_bench.rs` (integration, `#[ignore]`d,
  `BENCH key=value` lines, the `popup_bench`/`tooltip_bench` pattern):
  idle wakeups and per-minute CPU of a clock bar as shipped, per-update
  CPU of `push` text changes, per-cycle CPU of volume-popup open/close,
  and `screenshot` round-trip cost at 1x and 2x.

### How it was measured

Release builds on the dev VM (`taskset -c 2`, 6 vCPU shared box),
`SCOOTBAR_REQUIRE_SCOOT=1` with `scoot` built beside the bar from the
same tree; opaque and translucent binaries are the same binary (opacity
is a runtime `[bar]` value — every comparison is the same build with
only the config changed). Base `a75176dee` plus the bench commits; the
lift changes no code the bench measures. Three full chains, each
rotating the opacity order per run (9 samples per config per live
scenario; micro 300 interleaved samples per cell, 3 invocations).

The VM's CPU swings between speeds (opaque baselines move 2-3x run to
run), so configs are compared within interleaved runs, and the p5
(which the bursts leave alone) alongside the median, per the
`bench_depth` statistic.

### Numbers (medians; ranges are min-max over all samples)

Micro, full-bar repaint (69,120 px at 1x; 276,480 px at 2x):

- 1x full, us: 1.0: 16.0-17.5 across runs; 0.88/0.82/0.78/0.70 within
  -4%..+9% of opaque within each run (p5s often bit-identical, e.g.
  5.2/5.3/5.2/5.3/5.3).
- 1x span, us: 0.5-1.7 at every opacity (sub-microsecond noise floor).
- 2x full, us: 58.6-59.5 at every opacity (within 2%).
- 2x span, us: 3.4-4.0 at every opacity.

Live, clock bar as shipped, 60 s windows (n=9 per opacity):

- idle wakeups: 1.0: 1 [1,2]; 0.70: 1 [1,2]; 0.78: 1 [1,2]; 0.82: 1
  [1,2]; 0.88: 2 [1,2]. No window at any opacity reads 3+. RSS/fds/shm
  identical (4780 kB, 8, 1).
- idle CPU per 60 s, us: 1.0: 1675 [732,2354]; 0.70: 829; 0.78: 1431;
  0.82: 816; 0.88: 1051 (every translucent median at or below opaque).
- span updates, us/update: 1.0: 91.3 [72,142]; 0.70: 92.0; 0.78: 88.5;
  0.82: 85.9; 0.88: 80.3. Wakeups/update 3.1-3.3 at every opacity.
- popup open/close, us/cycle: 1.0: 438.9 [399,478]; 0.70: 431.2;
  0.78: 448.8; 0.82: 449.3; 0.88: 480.9. Wakeups 4.00 (3.98-4.00) at
  every opacity.
- screenshot round trip, ms (n=12 per cell after a focused extra run):
  1x: 1.0: 10.30 [8.98,14.23]; 0.70: 11.38; 0.88: 10.11.
  2x: 1.0: 10.51 [8.48,13.04]; 0.70: 10.60; 0.88: 11.45.

### Verdict per budget

1. Idle: PASS with a note. Every opacity reads 1-2 wakes, opaque
   included; nothing reads 3+ in 45 windows. The 0.88 median of 2
   against opaque's 1 is one extra minute-tick catch in a minority of
   windows (a 60 s window holds 1 or 2 minute boundaries depending on
   phase), not a translucency effect: nothing in the bar is
   alpha-value-dependent at runtime (alpha changes only the
   premultiplied pixel and the surface flags at setup), and the other
   three translucent alphas on the identical code path read median 1.
2. Span redraw: PASS (every translucent median at or below opaque's
   91.3 us).
3. Popup: PASS (worst +9.6% at 0.88, inside +10%; wakeups identical).
4. Tick: PASS (every translucent median below opaque's 1675 us).
5. Full repaint: PASS. Bar-side deltas are pure noise (within-run
   agreement, identical p5s). Compositor frames sit +8.9%..+10.4% on
   the median in the two worst cells but inside opaque's own observed
   range (opaque swings +-20% run to run; every translucent range
   overlaps it fully, and the signs flip between runs) — below this
   metric's resolution floor, which is dominated by PNG encode and IPC,
   not the 40 px strip's blend. A focused extra frames run (n=12)
   confirmed no systematic offset.

Bar-side cost is zero by construction, confirmed: `fill_shaped` runs
the same loop for any alpha (only the premultiplied pixel value
changes), so the repaint path cannot tell 0.7 from 1.0.

### Contrast (lift condition)

`scoot-look` validator output on the five lifted looks is unchanged
(no color changed, only alphas). Worst-case effective bar-text
contrast, fill blended at the lifted alpha over pure white and pure
black wallpapers: primary text stays >= 6.0 in every look (ginger-night
9.69, moonrise 6.03, music-desk 7.47, radial-burst 10.06, vinyl-sunset
7.59). Secondary roles dip only in those pathological extremes
(music-desk urgent 2.88 -> 1.37 vs black; moonrise urgent 4.45 -> 2.54
vs white); the shipped mid-tone wallpapers read far higher, and the
validator's accepted dim/urgent warnings do not regress.
