---
title: "apply_config_killed_mid_wait_changes_it_all_the_same took 57 s under load"
status: "open"
area: "scootbg"
priority: "low"
blocked: null
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

## Not in this ticket

The daemon itself: nothing here says it misbehaved. The other load flakes
already filed (`scale-change-decode-flake.md`,
`resolved/scaler-probe-margin-daemon-exit-flake-done.md`).
