---
title: "Buffers, memory and zero idle cost"
status: "open"
area: "scootbg"
priority: "high"
blocked: null
---

# Buffers, memory and zero idle cost

The resource budget is the feature. Targets to measure and publish in
`docs/scootbg/README.md` once the static path exists:

- **Idle:** no wakeups at all with a static wallpaper (no timers, no frame
  callbacks requested). Verify with `perf stat` / wakeup counts over a
  minute, the way `docs/benchmarks.md` measures the compositor.
- **Buffers:** one `XRGB8888` buffer per output in a `memfd` pool, opaque
  region set, reused in place on a same-size redraw. A 4K output is ~33 MB;
  two 4K outputs showing one image share nothing but the source decode.
- **RSS:** record resident memory for 1× 1080p, 1× 4K and 2× 4K, after the
  source has been dropped.
- **Startup:** time from `scootbg daemon` to the first committed buffer
  with a restored 4K JPEG.

No `release` event handling tricks until measured: a static wallpaper
commits once, so double buffering only matters for transitions.

**A reply can be held up** (from [solid-color-done.md](resolved/solid-color-done.md)):
an every-output `set` or `clear` waits for every output whose surface is
live, so an output whose surface never gets its `configure`, or a stalled
shm draw whose held buffer is never released, holds up the replies to
that request and later every-output ones until it resolves. Bounded by the
client's 30 s timeout (and the waiting lists by the connection limit), and
not reached on scoot or sway, whose surfaces configure within a round trip
and which release a buffer once the next is committed. If a compositor is
found that does, a per-output bound on the wait (or leaving outputs that
have never configured out of it) belongs here.

**From [ticket 6](resolved/images-decode-and-fit-done.md#measurements):**
after an image `set` on a 3840×2160 output (release, 12 sets of 6000×4000
JPEG, PNG and WebP): one 32.4 MB shm buffer per output (the old one is
dropped on release: a released buffer holding an image is never kept as
a spare), heap 372–568 kB anonymous, one thread, 0 wakeups over 30 s.
A rendered image no longer wanted (the choice changed before it could be
shown) is dropped at once.
Left for here:

- Two outputs of the same size showing one image get two buffers with
  the same pixels (the scale is done once, the second is a copy). One
  `wl_buffer` attached to both would save a buffer per extra output
  (32.4 MB at 4K); it needs release tracking per attach.
- A rendered image waiting for a held buffer's release (a stalled draw)
  is kept, one per output at most, beside the two slots.
- Each shm buffer keeps its memfd open (from ticket 4): +1 fd per image
  buffer.
