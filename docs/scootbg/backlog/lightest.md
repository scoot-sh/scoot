---
title: "Lowest resource use of any wallpaper daemon: the release gate"
status: "open"
area: "scootbg"
priority: "high"
blocked: null
---

# Lowest resource use of any wallpaper daemon: the release gate

The headline goal, and the **v1 release gate**: v1 is not released while
any competitor beats scootbg, beyond the noise margin, on any row that
applies to both. After v1, every PR that touches decoding, buffers or the
event loop re-runs the benchmark and must not regress beyond the margin
against the last published numbers.

## What is measured

On the same machine and outputs, published in `docs/scootbg/README.md`:

| Row | How |
|---|---|
| Size | the stripped binary plus its non-libc shared-library closure (`ldd`), so a small C binary linking cairo and gdk-pixbuf is weighed with them |
| Idle memory | RSS and PSS one minute after the wallpaper is up, for 1× 1080p and 2× 4K, reported both with and without the output-sized buffer (see the floor) |
| Idle wakeups | wakeups and CPU time over 60 s with a static wallpaper (target: zero) |
| Peak memory | the high-water mark while decoding and scaling a 6000×4000 JPEG |
| Set | CPU time and latency for one live change to that JPEG, and to a color |
| Startup | daemon start to first committed buffer, for a color and an image |
| Restore | startup with the last wallpaper restored |
| Disk | installed size, plus any cache it writes |

## Fair comparison

- **Competitors:** `swaybg`, `awww`, `hyprpaper`, `wpaperd`, `wbg`, at
  their current releases, recorded by version.
- **A row only applies where both sides can do the thing.** `swaybg` and
  `wbg` have no live change and no restore, so Set and Restore are "n/a"
  for them, never counted as a win. `awww` runs with its transition off
  (`--transition-type none`) so Set compares the same work.
- **The first step is checking every competitor actually runs** on scoot
  `--headless` and `--tty` and on sway, and recording any that does not.
- **Noise margin.** Each measurement runs at least five times; the median
  is reported with its spread. "Beaten" means the competitor's median is
  better by more than the larger of 5% and the two sides' combined
  spread. Anything inside that is a tie, and a tie does not block.

## The floor

Every daemon needs the output-sized pixels somewhere: a 3840×2160
`XRGB8888` buffer is ~33 MB, shared with the compositor. That is the same
for all of them, unless one uses a smaller format or lets the compositor
scale, which trades quality, and the table says when one does. The
contest is everything above the floor. Colors are the exception:
single-pixel buffers put the floor near zero, and scootbg should win that
row by orders of magnitude.

## Design rules that follow

Each checked against the numbers, not assumed:

- No async runtime; one thread with a `poll` loop over the Wayland fd and
  the socket, plus a short-lived decode thread that exits when done.
- Colors never touch shared memory (`wp_single_pixel_buffer_manager_v1`).
- `XRGB8888` buffers at exactly the output's device size, nothing larger.
- The decoded source is dropped before the output buffer is allocated.
  No scaler may hold a source-width × target-height intermediate.
- Freed heap goes back to the OS, so idle memory falls back after a
  change instead of keeping the decode's high-water mark. That happens by
  construction, not by tuning. A pure-Rust `#[global_allocator]` wrapper
  (in `scootbg-mem`) gives every block of 128 KiB or more its own `rustix`
  mapping, released with `munmap` on free; smaller blocks go to `System`.
  - Untreated, glibc kept 61.6 MB (main thread) and 79.2 MB (decode
    thread) after eight live sets.
  - With the wrapper, heap stayed at or under 0.65 MB across JPEG, PNG
    and WebP sets, on both thread models, with no tuning. CPU matched
    glibc `mallopt` + `malloc_trim` (measured with `fast_image_resize`),
    which it replaces, because scootbg calls no C. It costs +32 KB with
    pic-scale-safe (+37 KB with `fast_image_resize`).
  - The chosen pipeline peaked at 131.2 MB for a 6000×4000 JPEG against
    169.8 MB for the round-one design.
  - `mimalloc` kept 148 MB.

  ([evidence](resolved/dependencies-done.md#6b-round-two-pure-rust))
- Only the image-format features actually shipped are compiled in.
- No C beyond what std links (glibc, `libm`, `libgcc_s`): no `libc` crate,
  no `-sys` crate that links anything, no build script that compiles C
  ([audit](resolved/dependencies-done.md#9-what-c-remains)).
- A tiny state file format instead of a general config parser: measured,
  `toml` costs +180 KB over a hand-written line format's +20 KB, so the
  line format it is ([evidence](resolved/dependencies-done.md#5-serialization-control-socket-and-state-file)).

**From [ticket 6](resolved/images-decode-and-fit-done.md#measurements),
scootbg's own side of three rows** (not yet against competitors), release,
on `scoot --headless` 3840×2160: **Peak memory** for a set of the
6000×4000 JPEG 120.5–120.6 MB with the previous wallpaper's buffer still
mapped (88.0 MB for a first set); **Set** 397.0–433.6 ms request to reply,
390–420 ms of CPU; **Size** 1,500,008 B, now linking `libm` (the scaler's
`sinf`) as predicted. The scaler's cost against fir is the known risk on the Set row
(dependencies-done.md §3b).

**From [ticket 8](resolved/memory-and-idle-done.md#measurements),
scootbg's own side of the idle and startup rows**, same setup: **Idle
wakeups** 0 in 60 s with a color or an image (perf and `/proc`), one
thread; **Idle memory** with the image 12.3–12.4 MB RSS / 7.1–7.3 MB PSS
on 1× 1080p and 36.7–36.8 MB / 19.4–19.6 MB on **2× 4K, where both
outputs share one buffer's pixels**: the floor above is paid once for
all outputs of one size showing one image, so a daemon with a buffer per
output holds it twice (scootbg did until ticket 8: 69.2 MB RSS); with a
color 3.6–3.7 MB / 2.0–2.1 MB (4.0 MB / 2.8–2.9 MB since ticket 9, once
a save has touched 340–370 kB more of clean code pages: see
[its record](resolved/restore-state-done.md#idle-and-memory)). **Startup** 1.8–2.5 ms to the first
answer, 3.0–4.1 ms to a color on screen; an image `set` at start-up
decoded twice (642–720 ms on 4K) until
[ticket 9](resolved/restore-state-done.md#measurements): once now,
450–476 ms, and 451–497 ms to a restored 4K JPEG on screen.
