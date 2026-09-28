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

**The gate does not pass; its one failing class is waived for v1 (user,
2026-09-28).** After the cheap levers, awww still holds 1.0–1.5 MiB less
idle memory above the floor: 9 gated rows on each compositor. That
memory is mostly clean program code the kernel can reclaim (scootbg's
own, and `libc`'s): file-backed pages it can drop under pressure and read
back on demand. The rest is anonymous memory: with an image about 0.24 MiB
more, nearly all of it the decode thread's malloc arena. Every other row is a win
or a tie. The user waived that class, and only that class, so v1 is not
held for it; the rule above is otherwise unchanged, and a new loss on any
other row would still hold v1. The one lever that would close the gap is
a separate daemon binary, which has real packaging and process costs; it
is post-v1 ([idle-code-pages.md](idle-code-pages.md)). After v1, every PR
that touches decoding, buffers or the event loop still re-runs the
benchmark and must not regress beyond the margin.

## What is measured

On the same machine and outputs, published in `docs/scootbg/README.md`:

| Row | How |
|---|---|
| Size | the stripped binary plus its non-libc shared-library closure (`ldd`), so a small C binary linking cairo and gdk-pixbuf is weighed with them |
| Idle memory | RSS and PSS one minute after the wallpaper is up, for 1× 1080p and 2× 4K, reported both with and without the output-sized buffer (see the floor) |
| Idle wakeups | wakeups and CPU time over 60 s with a static wallpaper (target: zero) |
| Peak memory | the high-water mark while decoding and scaling a 6000×4000 JPEG, as the summed PSS of the daemon's processes (a client that decodes, as awww's does, included; shared pages counted once) |
| Set | CPU time and latency for one live change to that JPEG, and to a color |
| Startup | daemon start to the committed buffer that puts the wallpaper up, for a color and an image: the **last** buffer commit before the daemon goes quiet, so a placeholder committed first (wpaperd commits one 187–256 ms after start, its image at 914–1,027 ms) does not count as the wallpaper. Every other daemon commits once, so first and last are the same commit. (Written "first committed buffer" until the first run showed the placeholder.) |
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

## Measured: the first comparison (2026-09-27) — the gate does not pass

The benchmark exists and has run: `scripts/scootbg-bench/bench.py`, all
rows, 5 rounds, on headless scoot and on headless sway, with the table,
the method, the versions and every raw run in
[the comparison](../README.md#against-the-other-daemons). swaybg 1.2.2,
awww 0.12.1, wbg 1.3.0 and wpaperd 1.3.0 ran on both compositors.
hyprpaper 0.8.4 ran on neither: on scoot it needs a DRM device, and this
machine has none; on sway it binds `xdg_wm_base` v6 where sway has v5.
`--tty` was not reachable.

**The first run (2026-09-27) did not pass.** What it left:

- **Idle memory above the floor, against awww: 9 losses on scoot.**
  - RSS above the floor: 1.1–1.6 MiB more than awww's (medians 3.85–4.49
    against 2.72–2.89 MiB).
  - PSS above the floor: 1.0–1.5 MiB more (2.07–2.47 against 0.96–1.05
    MiB).
  - Both with a color and with an image, on 1× 1080p and 2× 4K.
  - The same bytes make the 1080p image's total with the floor 10.3
    against 8.9 MiB.
  - They are mostly scootbg's own code: 0.86 MiB more of its `.text`
    and read-only data is resident, clean pages. The rest is `libc` code
    (0.31 MiB then, about 0.25 MiB after the `getpid` lever) and a little
    anonymous memory. The daemon runs in a binary
    whose `.text` is 1.23 MiB, holding the decoders and the CLI as well,
    while `awww-daemon`'s `.text` is 0.36 MiB.
  - Plan: [idle-code-pages.md](idle-code-pages.md).
- The results on sway: [below](#on-sway).

### Re-run of the idle rows (2026-09-28): still not passed, waived for v1

After the attribution and the cheap levers of
[idle-code-pages.md](idle-code-pages.md#levers-tried-2026-09-28), the idle
rows ran again, 5 rounds, on headless scoot and sway, with `35f3a13`
([scoot](../bench/2026-09-28-idle-scoot/table.md),
[sway](../bench/2026-09-28-idle-sway/table.md)). The same 9 losses to awww
on each; `compare` against the 2026-09-27 runs finds no regression.

- **Kept:** one lever, `getpid` through `rustix`, 64 KiB of libc code
  (inside the noise).
- **Gap left, scoot**: RSS above the floor 1.06–1.10 MiB with a color,
  1.45–1.52 MiB with an image; PSS 0.98–1.01 and 1.41–1.42 MiB.
- **Gap left, sway**: RSS 1.09–1.10 and 1.41–1.47 MiB, PSS 0.98–0.99
  and 1.35–1.44 MiB.
- **Why**: the daemon runs 277 KiB of the 1,248 KiB of functions in its
  binary, and the kernel's fault-around makes 1,048–1,200 KiB of it
  resident, the decoders and std's backtrace code included. Removing that
  code on stable is not possible for the backtrace (std's default panic
  hook links it into every binary), and costs the image pipeline 20% for
  `opt-level = "s"`.
- **Decided (user, 2026-09-28)**: the class is waived for v1, and the
  separate daemon binary, which a prototype showed ties awww on the color
  rows, is post-v1
  ([the later option](idle-code-pages.md#later-option-a-separate-daemon-binary)).
  The page-out fallback stays the user's call, not a quiet fix.

Won or tied everywhere else. The points where it is closest, or where the
rule decided:

- **The JPEG at start-up against wbg: a tie.** wbg's medians are 4.7% and
  7.0% lower (447 against 469 ms; 413 against 444 ms of CPU), inside the
  combined spread (64 and 53 ms). wbg uses a C decoder and a bilinear
  filter; with the same filter, scootbg takes 394 ms. If later runs make
  this a loss, it is the next item: zune-jpeg against libjpeg-turbo, and
  Lanczos3 against bilinear, the second being a quality choice for the
  user.
- **Size and peak**, the rows this ticket expected to need fixes, are
  clear wins, so neither was touched:
  - Size is 1,866,680 B against awww's 9,080,536 B, the smallest of the
    others. Ticket 10's 106 KB were not trimmed.
  - Peak at start-up, as PSS (the gated figure, 2026-09-28 runs), is
    84.7 MiB against wbg's 106.0, the lowest of the others (as RSS, 88.9
    against 107.4). A reduced-size JPEG decode is not available in zune-jpeg
    0.5.15, and would not apply to a 1.56× fill.
- **Idle wakeups and CPU**: 0 for scootbg, awww, swaybg and wbg (ties);
  wpaperd wakes 125–126 times a minute on 1× 1080p and 160–161 on 2×
  4K on scoot, 126–127 and 165–168 on sway.
- **Disk**: scootbg's nix closure carries `gcc-15.3.0-lib` (9.8 MiB), for
  one library of 193 KiB in `gcc-15.3.0-libgcc`. It still wins (11.6 MiB
  against awww's 31.8 MiB), but this is the cheapest byte to take off
  when Disk matters.

### On sway

Headless sway 1.12 gives the same verdicts
([table](../bench/2026-09-27-sway/table.md)): the same 9 idle losses to
awww (RSS above the floor 3.87–4.45 against 2.66–2.90 MiB, PSS 2.06–2.39
against 0.90–1.04 MiB, the 1080p image's total 10.3 against 8.9 MiB),
and the JPEG at start-up against wbg again a tie (475 against 490 ms).
