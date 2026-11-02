---
title: "apply_config_killed_mid_wait_changes_it_all_the_same took 57 s under load"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-11-02"
---

# apply_config_killed_mid_wait_changes_it_all_the_same took 57 s under load

Filed 2026-10-29. Serves **daily-drive**: a test that takes 21 s alone and 57 s
under load is the next intermittent timeout, and slows every run of the
suite that guards the wallpaper daemon.

## The gap

`config::apply_config_killed_mid_wait_changes_it_all_the_same`
(`crates/scootbg/tests/config.rs`) took 57 s once on the aarch64 dev VM while
another agent ran a release `cargo test` there, and about 21 s alone. It
passed 3 of 3 runs in isolation. It is load-sensitive timing, unrelated to the
sway and rotation fixes (#802 and the sway start-up race fix).

What the test does, read from the source: it starts a daemon with
`start` (`Session::start`, so scoot's default output size, not
`Session::start_sized`), applies a section, writes a 4000x3000 noise PNG
with `write_noise_png` (a debug-build loop over 36 million bytes, then a PNG
encode), runs `apply-config` with it, waits for the state file to name the
image, kills the client, and waits (`query_until`, `PATIENCE` 20 s) for every
output to show the image.

Not measured yet, so not a finding: which of these dominates the 21 s. The
candidates are the test process generating and encoding the noise PNG in a
debug build, the daemon decoding that PNG, and the daemon drawing it on a
default-size output. #802 found the draw of a default 1600x1000 output takes
about 0.7-0.8 s idle and about 2 s under 12 busy loops in a debug build; that
cause is the one named for the rotation `EAGAIN` failures, and may or may not
matter here.

## What to do

Time each phase on the dev VM (test-side PNG generation, `apply-config`
until the state names the image, kill, daemon until `shows.image`), idle and
under busy loops, as #802 did. Then cut what dominates:

- If it is the draw, run the test on a small output (`Session::start_sized`,
  as the rotation tests do after #802) and check the test keeps its meaning:
  it needs a decode slow enough to still be in flight when the client is
  killed, which a smaller output does not change but a smaller image would.
- If it is the PNG, generate a smaller one, or build it once, and re-check
  the kill still lands mid-decode (the state file naming `big.png` is the
  test's signal that the daemon has started).

Keep `PATIENCE` as it is; the point is a test that is fast, not a longer wait.

## What landed (2026-11-02)

The test runs on one 64x64 output (`Session::start_sized`, as the
rotation tests do after #802) with an 800x600 noise PNG, instead of two
default 1600x1000 outputs with a 4000x3000 one. Measured on the 6-core
aarch64 dev VM (debug build), `cargo nextest run -p scootbg --test
config apply_config_killed_mid_wait_changes_it_all_the_same`:

- alone: 20.6/20.6/20.4 s before, 1.31/1.32/1.29 s after (10 more runs:
  1.26-1.43 s);
- under 12 busy loops: 25.7 s before, 1.68/1.65/1.83 s after (the
  ticket's 57 s was under a heavier release-test load).

Phase split (idle, then loaded): test-side PNG generation 13.2 s, then
16.6 s (a debug PNG encode of incompressible noise); daemon
decode-and-draw 7.0 s, then 8.4 s on two default outputs (mostly
resampling: any mismatched size costs 1.2-1.9 s a side, while a 1600x1000
image on 1600x1000 outputs draws in under 0.1 s). After: generation
0.54 s, then 0.64 s; decode-and-draw on the small output 0.17 s, then
0.16 s. The client kill lands 6 ms (39 ms loaded) after the state file
names the image, while the decode is still in flight (~170 ms), so the
test still exercises the kill-mid-decode path.

Meaning kept by mutation: a daemon that abandons the apply when the
client dies (dropping the image job and choice once the dead client's
waiter is forgotten) fails the test before the speedup (33.8 s) and
after it (21.2 s alone, 21.5 s loaded). `PATIENCE` unchanged; no product
code changed.

## Not in this ticket

The daemon itself: nothing here says it misbehaved. The other load flakes
already filed (`scale-change-decode-flake.md`,
`resolved/scaler-probe-margin-daemon-exit-flake-done.md`).
