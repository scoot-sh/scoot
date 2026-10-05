---
title: "How does swaybg show the same wallpaper at 2.0 MB PSS"
status: "open"
area: "scootbg"
priority: "low"
blocked: null
---

# How does swaybg show the same wallpaper at 2.0 MB PSS

Filed 2026-10-05 from PR #455's review (r455, section "The swaybg
lever"). Serves **daily-drive** (memory footprint: scootbg's biggest
resident is its kept on-screen pool, and swaybg suggests a ~10 MB
saving may exist — but only a measurement says whether it is real and
worth the tradeoff).

## The gap

The five-desktop idle benchmark (Asahi M2, `docs/benchmarks.md`) puts
swaybg at 2.0 MB total PSS in the same role where scootbg, wallpaper
up, holds its output-sized floor (~12.3 MB PSS shared with the
compositor here: two output-sized `wl_shm` pools). scootbg-lean's
2.1 MB matched swaybg only because nothing was drawn (its image file
was unreadable — see
`resolved/image-retention-done.md`). So the real gap is ~2 MB
(swaybg) vs ~12 MB (scootbg with the wallpaper up), and nobody has
measured where swaybg's 10 MB goes: a smaller buffer (compositor-side
scaling, at a quality cost?), dropping client buffers after upload
(under which renderer — pixman re-reads client shm on repaint, GLES
releases after texture upload?), or something else entirely.

## What to do

Measure first, redesign never-before-that. Put swaybg under the same
wallpaper (moonrise, `fill`, same outputs) and read its
`smaps_rollup` plus its per-map buffers: buffer sizes, count, and
whether they persist after the compositor's upload. Compare against
scootbg's floor (`lightest.md`: one output-sized buffer per output
size, kept deliberately so another same-size output's buffer can be
made over it, and because a static layer surface's buffer stays
attached). Per r455's reasoning: released spare buffers are already
dropped at once, so what is kept is the pool behind the **on-screen**
buffer — "drop it after release" is a redesign
(detach-while-shown plus a re-commit protocol dance, with flicker and
~13 ms worst-case re-decode costs at 4K), not a drop. If the
measurement shows swaybg's lever is real, cheap and quality-neutral,
file the redesign from it; if it is compositor-side scaling or a
buffer the compositor re-demands on every configure, say so and close
this as a measured don't-build.

## Not in this ticket

Any change to scootbg's kept on-screen pool (that redesign is gated
on this ticket's measurement); scootbg CPU (zero everywhere
measured); the compositor-side shm the wallpaper surface costs
(that is scoot's, not scootbg's).
