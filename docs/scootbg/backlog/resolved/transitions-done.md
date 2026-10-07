---
title: "Transitions between wallpapers (milestone 2)"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-07"
---

# Transitions between wallpapers (milestone 2)

Fade, wipe (with an angle), grow/outer from a point, and `none`, with a
duration and an easing curve.

- CPU only, into `wl_shm`: per frame, blend old and new scaled buffers
  into a third. Pace by frame callbacks, and use `wp_presentation` where
  offered to drop frames rather than fall behind.
- Damage only what changed each frame (a wipe's moving band), so the
  compositor redraws the least.
- A new request mid-transition starts from what is on screen now; no
  queueing of stale transitions.
- A per-frame time budget, measured on the dev VM under pixman at 4K. If a
  transition cannot hold it, it degrades (fewer steps), never stutters the
  compositor.
- Once finished, free the extra buffers and return to zero idle cost.

Benchmarks before and after are required: this is the first hot path in
scootbg.

## Resolved 2026-10-07 (PR #501)

Landed as designed, on `feat/scootbg-transitions`, verified on the Asahi
M2 (the dev VM is down; headless scoot + scootbg there, pixman):

- Kinds `none`/`fade`/`wipe`/`grow`, `duration-ms` (0–60000), easing
  (`linear`, `ease-in`, `ease-out`, `ease-in-out`, `smooth`), wipe
  `angle`, grow `position`, on `set`, in `[wallpaper]` per table, and on
  the wire (protocol stays 1).
- Frames blend shared endpoints into full-size buffers (a second frame
  buffer allocated lazily where the compositor releases-on-replace),
  damaged to what changed, paced by frame callbacks + `wp_presentation`
  feedback where offered + a 60 Hz timer; progress from the clock (drops
  frames), 8 ms budget with skip-ahead degrade; restart snapshots the
  screen; completion frees everything (0 switches in 30 s, no retained
  buffers).
- Per-frame blend, release, medians of 30 frames ×3: 1080p fade 5.4 /
  wipe 3.0 / grow 2.4 ms; 4K fade 21.6 / wipe 12.1 / grow 10.0 ms.
  Release size +131,072 B file / +51,172 B `.text` vs the parent build,
  reported for the maintainer (ratchet: waived nothing).
- Tests: pure math (endpoints, easing, angles, damage covers every
  changed pixel), CLI/protocol/section/scoot strictness, 9 headless
  integration tests (mid-flight query + screenshots per kind, restart,
  section reloads, refusals). New tests fail on the base binary
  (`unexpected argument --transition`, exit 2).
- Docs: `site/scootbg/transitions` (+ option table, symptoms, frame
  strip from `scripts/scootbg-transition-strip.sh`), `[wallpaper]` rows,
  CLI/troubleshooting, README protocol + measurements.
- Evidence: PR #501 CI; raw logs/numbers in the PR body and
  `scripts/scootbg-transition-bench.sh`.
