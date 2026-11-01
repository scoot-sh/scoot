---
title: "Bar opacity pixman benchmark: the gate for translucent bars"
status: "open"
area: "scootbar"
priority: "medium"
blocked: null
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
