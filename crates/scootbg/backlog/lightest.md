---
title: "Lowest resource use of any wallpaper daemon: the release gate"
status: "open"
area: "scootbg"
priority: "high"
blocked: null
---

# Lowest resource use of any wallpaper daemon: the release gate

The headline goal, and a **release gate**: v1 is not released, and a
later PR does not merge, while scootbg is beaten by any competitor on any
row below. Where one cannot be won (see "the floor"), the README says so
with the numbers, rather than dropping the row. Measured on the same
machine and outputs, and published in `crates/scootbg/README.md`:

| Measure | How |
|---|---|
| Binary size | stripped release build, as packaged |
| Idle memory | RSS and PSS (the shm buffer is shared with the compositor, so PSS is the fair number) one minute after setting a wallpaper, for 1× 1080p and 2× 4K |
| Idle wakeups | wakeups and CPU time over 60 s with a static wallpaper (target: zero) |
| Set latency | `set` to committed buffer for a 4K JPEG, and for a colour |
| Peak memory | the high-water mark while decoding and scaling a 6000×4000 JPEG |
| CPU to set | total CPU time (user + sys) for one `set` of that JPEG, and of a colour |
| Startup | `daemon` start to first committed buffer, colour and restored image |
| Disk | installed size, plus any cache it writes |

Compared against `swaybg`, `awww`, `hyprpaper`, `wpaperd` and `wbg` (the
current releases, recorded by version), on scoot `--headless` and
`--tty` and on sway, over at least five runs each, reporting the median.

**The floor.** Every daemon needs the output-sized pixels somewhere: a
3840×2160 `XRGB8888` buffer is ~33 MB, shared with the compositor. That
is the same for all of them (unless one uses a smaller format or lets the
compositor scale, which trades quality). The contest is everything above
the floor, so the table reports PSS both with and without the buffer.
Colours are the exception: single-pixel buffers put the floor near zero,
and scootbg should win that row by orders of magnitude.

Design rules that follow from the goal, each to be checked against the
numbers rather than assumed:

- No async runtime; one thread with a `poll` loop over the Wayland fd and
  the socket, plus a short-lived decode thread that exits when done.
- Colours never touch shared memory (`wp_single_pixel_buffer_v1`).
- `XRGB8888` buffers at exactly the output's device size, nothing larger.
- The decoded source is dropped after scaling, and freed heap is returned
  to the OS (`malloc_trim` or an allocator that does it) so idle RSS falls
  back after a set instead of keeping the decode's high-water mark.
- Only the image-format features actually shipped are compiled in.
- A tiny state file format instead of a general config parser, if that
  saves real bytes (measure `toml` against a hand-written line format).

A regression here is a regression like any other: the benchmark is re-run
for any PR that touches decoding, buffers or the event loop.
