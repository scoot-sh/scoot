---
title: "scootbg holds 12 MB anon at idle in some sessions, near zero in others"
status: "open"
area: "scootbg"
priority: "medium"
blocked: null
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
