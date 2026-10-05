---
title: "scootbg holds 12 MB anon at idle in some sessions, near zero in others"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-05"
---

# scootbg holds 12 MB anon at idle in some sessions, near zero in others

Filed 2026-10-05 from the five-desktop idle benchmark (Asahi M2,
`docs/benchmarks.md`). Serves **daily-drive** (memory footprint: the
decoded wallpaper is scootbg's biggest resident by far when retained).

## The gap

`smaps` PSS-by-mapping-class on 6 idle scoot sessions (same moonrise
wallpaper, `fill`, one foot, 4+ min after login) is bimodal:

- 3 sessions: anon 12.4 MB, total PSS 14.7 MB
  (`ev/scoot-r1-smaps-scootbg-*.txt`: anon 12370 kB over 11 maps)
- 3 sessions: anon ~0 MB, total PSS 2.1 MB
  (`ev/scoot-r1-smaps-scootbg-*.txt` vs `ev/scoot-c1-*.txt`)

Same binary (`scootbg-0.1.0`), same image, same idle time. The 12 MB is
the decoded wallpaper (2560x1600x4 plus DP-1's 1920x1080x4). Something
decides whether it stays mapped; the trigger is unknown (not
time-since-login: both cohorts sampled ~4 min in; not screenshots:
samples precede them).

Correlation: the compositor shows the same bimodality across the same
two cohorts (75.6-75.9 MB in the r-rounds, 63.6 MB in the c-rounds —
same ~12 MB delta, same binary, same config). Whatever 12 MB mapping
comes and goes, it comes and goes in both processes together. The
compositor's `smaps` split exists only for the fat cohort (shm
12.3 MB there); the lean cohort was never `smaps`-split, so the class
that differs is unproven — capture both next time.

For scale: swaybg holds 2.0 MB total in the same role, hyprpaper
40.5 MB. scootbg-lean already matches swaybg; scootbg-fat is 7x that.

## What to do

Find the retention trigger (serve/apply-config lifecycle? per-output
repaint keeping a reference? a cache with no timeout?), then drop the
decoded image once every output has its frame, keeping at most a
re-decode-on-demand path. Expected saving when fat: ~12 MB PSS per
session, zero CPU cost (no work happens at idle either way). Pin the
behavior with a test that asserts anon PSS after settle (the benchmark's
`smaps.sh` mapping classes), and re-run the benchmark's wallpaper row.

## Not in this ticket

scootbg CPU (zero everywhere measured); the compositor-side shm the
wallpaper surface costs (that is scoot's, not scootbg's).

## Resolution (2026-10-05): no retained copy — the lean cohort never drew the wallpaper

Traced on a real `--tty` login (Asahi M2, eDP-1 2560x1600 + DP-1
1920x1080, current main, moonrise `fill`): with the wallpaper up, scootbg
holds exactly the two output-sized `wl_shm` pools (24.7 MB mapped,
~12.3 MB PSS shared with the compositor) plus ~0.3 MB anon. The decoded
source, the crops and the scaled temporaries are all dropped on the worker
thread before the buffers are allocated (`image::render::render_each`);
settled `Pss_Anon` measures 0.5–0.8 MB (debug). There is no decoded copy
to drop, so the fat state is the documented floor (one output-sized buffer
per output size, `lightest.md`), not a leak.

Two artifacts made it look like one:

- The 12 MB "anon" is misclassified shared memory. The benchmark's
  `smaps.sh` (`~/fx/cmpde-cerval/smaps.sh`, scratch, not in this repo)
  credits each block's PSS to the *next* block's class, so whole memfd
  pools read as `anon-other` (reproduced live: an 8 MB pool reads as
  8 MB anon, 0 shm). Read with a fixed classifier the fat cohort is shm,
  as the floor predicts.
- The lean cohort (2.1 MB, no pools anywhere) never drew anything. The
  session journal shows for all three second-campaign logins:
  `scootbg: cannot restore ".../moonrise.png" for every output:
  Permission denied (os error 13); showing the compositor's own
  background` — the image lived under 700 `/home/steve`, unreadable to
  the test user by the second campaign (the first campaign's logins say
  "applied the [wallpaper] section"). Same binary, same section JSON;
  only file access differed. A missing-file restore was re-enacted
  headless: `shows: null`, PSS 2160 kB (debug), zero shm — the lean
  signature exactly. The compositor's ~12 MB delta is the same pools
  unmapped on both sides while it shows its own background.

No behavior change: making "lean the only state" while showing wallpaper
would mean dropping on-screen buffers, against the measured floor tradeoff
(13 ms saved at 4K per change). Pinned by
`crates/scootbg/tests/retention.rs`
(`no_retained_copy_after_image_settles`: settled `Pss_Anon` < 2 MB with
moonrise fill up — verified to fail with a 12 MB leak injected, pass
without). Before/after measurements (3 sessions each, fixed classifier)
are in the PR report; they are identical by construction.
