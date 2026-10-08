---
title: "How does swaybg show the same wallpaper at 2.0 MB PSS"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-08"
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

## Resolution (2026-10-08): measured don't-build — the lever is real, and it saves nothing on scoot

No code changed. The lever is **the client drops its pool and buffer
right after commit**, confirmed in swaybg 1.2.2's source and in
`smaps`: not a smaller buffer, not compositor-side scaling. Under a
pixman compositor the pool persists in the compositor's mapping at
full size, so adopting it would only move PSS from client to
compositor while costing a re-decode on every re-commit. Don't build;
revisit only if scoot ever gains a renderer tier that uploads client
shm to GPU textures.

### The mechanism (source, swaybg 1.2.2)

- `main.c render_frame`: attaches, damages, commits, then
  `wl_buffer_destroy(buf)` **immediately** — no wait for `release`.
- `main.c draw_buffer`: renders into a `pool_buffer`, then detaches
  the `wl_buffer` (`buffer.buffer = NULL`) and `destroy_buffer`s the
  rest, which `munmap`s the client's mapping (`pool-buffer.c`).
  Steady state keeps zero client shm and one `wl_buffer` protocol
  object per output.
- The decoded image (`cairo_surface_t`) is destroyed right after the
  last output renders from it (`main` loop). Nothing decoded is kept.
- `get_buffer_size` renders the full output size times scale (or the
  fractional-scale rounding under a viewport): no smaller buffer, no
  compositor-upscale quality tradeoff. One buffer per output — same-size
  outputs do NOT share (the bench's 2× 4K floor, 63.3 MiB = two pools,
  corroborates; scootbg's shared floor is half that there).
- Colors use the single-pixel buffer path, same as scootbg.

So under niri/GLES the 2.0 MB is: code and Wayland objects only, the
pixels living in a GPU texture no process's PSS counts. The open
question was what happens under a compositor that re-reads shm.

### The measurement (Asahi M2, headless scoot, pixman)

Same compositor, same image, `fill`, 1× 1920×1080 scale 1, 3 rounds
per side with a fresh compositor and its own `XDG_RUNTIME_DIR` each
round. scoot 0.1.0 (`ec5ffa2c…`, log says `renderer=pixman`), swaybg
1.2.2 (`ed34b840…`), scootbg 0.1.0 (`10b9a0e6…`). Image: a 3840×2160
PNG (`8f983ae4…`, moonrise is not in the nix store and images are
never committed — the mechanism is image-independent; the same file
both sides). Load 0.3–4.9 across snapshots, recorded per snapshot
(memory numbers, not timings).

Pixel proof every round (lean-cohort lesson: never trust a Resident
number without one): baseline mean `srgba(20,20,25,1)`; swaybg shots
`srgba(38.0019%,48.8822%,53.345%,1)` 3/3, scootbg shots
`srgba(38.1433%,49.0236%,53.4869%,1)` 3/3 — the same picture up both
sides (means agree within the Lanczos3-vs-cairo scaler difference).

Client steady state, 3/3 rounds:

| | swaybg | scootbg |
|---|---|---|
| shm maps | **0** | **1**, 8112 kB `/memfd:scootbg-wallpaper (deleted)` |
| RSS | 8576–8608 kB, all file (8208–8224) + 368 anon | 11920–11952 kB (8112 shm + ~370 anon + ~3450 file) |
| PSS | 5780–6177 kB | 6271–6649 kB (of it 4056 shm, shared) |
| fds / threads | 4 / 1 | 8 / 1 |

Compositor steady state (fresh RSS 20288–20320, 0 shm):

| | after 1 screenshot, no wallpaper | + wallpaper up |
|---|---|---|
| RSS | 28848–28880 (+~8560, all anon: first-frame cost) | swaybg 37440–37472, scootbg 37392–37408 |
| shm maps | 0 | exactly 1, 8112 kB: `/dev/shm/swaybg-* (deleted)` resp. `/memfd:scootbg-wallpaper (deleted)` |
| shm PSS | 0 | swaybg 8112 (sole mapper), scootbg 4056 (shared with the client) |

The floor is **identical**: one 8112 kB output-sized pool in exactly
one mapping set, 3/3 rounds each side. Above the floor, swaybg's
client holds 5.8–6.2 MB of mostly file-backed code (cairo,
gdk-pixbuf) where scootbg holds 2.2–2.6 MB — the already-waived
idle-code-pages gap, not this ticket.

### Why not to build it

On scoot/pixman the pool must stay mapped by the compositor, which
re-reads it on repaint: destroying the client's copy moves ~4056 kB
PSS from client to compositor (sole mapper then pays full PSS) and
the system total does not move. The price is real: a full re-decode
(hundreds of ms) plus ~13 ms worst-case realloc at 4K on every
re-commit (configure, rescale, re-show), the detach-while-shown dance
with its flicker risk, and losing the in-place reuse plus the
release tracking Smithay's re-attach behavior needs (see
`resolved/memory-and-idle-done.md`). swaybg's destroy-without-release
was safe on every compositor tried here (screenshots correct every
round), but safe is not free.

If scoot ever gains a GLES renderer tier, the tradeoff flips: the
compositor would upload to a texture and the shm could go away
entirely, saving the full pool. The numbers to beat are above; until
then this stays a don't-build. No follow-up entry: there is no GLES
tier ticket to block on, and the redesign is not cheap even there.
