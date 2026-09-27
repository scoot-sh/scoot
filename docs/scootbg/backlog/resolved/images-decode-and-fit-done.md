---
title: "Decoding images and fitting them to an output"
status: "resolved"
area: "scootbg"
priority: null
blocked: null
---

# Decoding images and fitting them to an output — RESOLVED

Resolved 2026-09-27, together with the image half of
[the CLI ticket](cli-and-ipc-done.md). What landed, where it departs from
the plan, what was verified where (and what could not be), and the
measurements are in [Resolution](#resolution) at the end; the original
ticket follows unchanged.

`scootbg set <path> [--output NAME] [--mode fill|fit|stretch|center|tile]
[--fill COLOR] [--filter ...]`.

- Formats for v1: PNG, JPEG, WebP (still). Others behind cargo features
  later (see `more-formats.md`).
- Decode on a worker thread, never on the Wayland event loop, so a slow
  or huge file cannot stall frame handling for the other outputs.
- An output removed while its image decodes: the result carries the
  output's `OutputId` (never reused) and looks it up again when it lands;
  not found means dropped, no panic
  ([the hook](resolved/outputs-and-layer-surfaces-done.md#for-the-next-tickets)).
- Guard against decompression bombs: refuse images past a pixel budget
  (configurable, default well above 8K×8K) before allocating, and report a
  truncated or corrupt file as an error reply, never a panic.
- Validate every dimension before scaling, not only before decoding:
  - the source and target are both non-zero;
  - the fill-crop rectangle lies inside the source;
  - every `width × height × channels` product is computed with
    `checked_mul`.

  `pic-scale-safe` is `forbid(unsafe_code)`, so a bad size there is a
  panic, not corruption. A panic still aborts the daemon, so it is
  stopped before the call.
- Honour EXIF orientation for JPEG (and WebP, which carries EXIF too).
  `zune-jpeg` hands over raw EXIF, so the orientation tag is read by a
  small bounds-checked parser
  ([decided](resolved/dependencies-done.md#2-decoding)). Apply the
  orientation inside the pass that packs the scaled RGB into the XRGB8888
  shm buffer, by reading through the rotated index:
  - scale the unrotated source to the rotated target size, with width
    and height swapped for the 90° cases and the crop rotated to match;
  - then rotation needs no buffer at all.

  Rotating the decoded source first, as the `image` crate does, measured
  +60 MB peak and +200 ms on 6000×4000. The packing-pass version is
  designed, not yet measured.
- Fit modes: `fill` (cover, crop centred; the default), `fit` (contain,
  letterbox in `--fill`), `stretch`, `center` (no scaling), `tile`.
- Scale once per output size and scale factor with a good filter
  (Lanczos3 or CatmullRom default; a `--filter` choice), with
  `pic-scale-safe` (`#![forbid(unsafe_code)]`) behind one function, so a
  swap stays contained.
  - Crop to the fill rectangle in place first: rows are a sub-slice, and
    columns are compacted row by row with `copy_within`. It takes no crop
    rectangle, so the crop is integer (≤ 0.5 px from exact).
  - Scale RGB to RGB, then pack into the shm buffer in one pass.
  - Measured on 6000×4000 → 3840×2160 Lanczos3: 174 ms with +25 MB
    transient (the output only). `fast_image_resize` took 80 ms / +63 MB
    and `image` 892 ms / +227 MB.
  - Pipeline peak 131.2 MB.
  - The ~100 ms per 4K change over fir is the risk the competitor run
    checks ([decided](resolved/dependencies-done.md#3b-round-two-safe-scalers-and-the-choice)).
- Do not scale four-channel pixels straight into the shm buffer. It links
  a scaler's alpha paths (+2.3 MB with fir) and decodes 4 bytes per
  pixel, for no CPU gain
  ([measured](resolved/dependencies-done.md#6b-round-two-pure-rust)).
- The same image on several outputs of different sizes is decoded once and
  scaled once per distinct target size.
- Drop the decoded source once every output needing it is drawn, unless a
  pending hotplug or scale change needs it again (then re-decode rather
  than hold tens of MB forever).

Color management is out of scope for v1: images are treated as sRGB and
written as 8-bit.

## Resolution

### What landed

Code at `67a0c84` (commits `0af2e5a` and `67a0c84`); the docs came after,
in a docs-only commit.

- **The command** (`src/cli.rs`): `scootbg set PATH [--output NAME]
  [--mode fill|fit|stretch|center|tile] [--fill '#rrggbb'] [--filter
  lanczos3|catmull-rom|bilinear|nearest]`, each flag also as
  `--flag=VALUE`. An argument starting with `#` is a color, anything else
  a path (so `./#name.png` for a file named with `#`), made absolute by
  the CLI with `std::path::absolute` (no symlink resolution, no file
  system access; `.` components dropped, `..` kept). A path that is not
  UTF-8 is a usage error saying the protocol cannot carry it. `--mode`,
  `--fill` and `--filter` with a color are a usage error (exit 2), as are
  unknown values.
- **The protocol** (`src/protocol.rs`, additive to protocol 1): `set`
  takes `color` *or* `image` (absolute path; a relative one is refused),
  with optional `mode`, `fill`, `filter` (defaults `fill`, `#000000`,
  `lanczos3`), refused with a color. `query`'s `shows` for an image is
  `{"image":"/abs/path","mode":"fill","fill":"#rrggbb","filter":"lanczos3"}`.
- **The pipeline** (`src/image/`, no Wayland, no daemon state, every step
  unit-tested in its own `tests.rs`):
  - `decode`: format sniffed from the first 12 bytes; `zune-jpeg`, `png`,
    `image-webp` directly into packed RGB; the header's size checked
    against `MAX_PIXELS` = 2^28 (16384×16384) before the pixel buffer is
    reserved, and that reserved fallibly (`try_reserve_exact`), so a bomb
    or an allocation the system refuses is an error reply. Grey, grey +
    alpha and RGBA become RGB in place (narrowing forwards, widening
    backwards: no second buffer), alpha flattened over the fill color.
    16-bit PNG channels are stripped to 8, palettes expanded, animated PNG
    and WebP give their first frame. The file is opened `O_NONBLOCK |
    O_CLOEXEC | O_NOCTTY` and refused unless a regular file, so a FIFO
    or a device cannot hang the worker.
  - `exif`: the orientation tag (0x0112) from IFD0, both byte orders,
    `Exif\0\0` prefix optional, every read bounds-checked with
    `checked_add`/`get`; anything malformed is "no orientation". Read for
    JPEG (APP1), WebP (EXIF chunk, capped at 1 MiB) and PNG (`eXIf`).
  - `orientation`: the eight EXIF orientations as display↔stored maps, a
    rectangle map, and a `Walk` (start index and signed steps) for reading
    the stored image in displayed order.
  - `fit`: per mode, the displayed crop, the scaled size and the
    placement, integer, crop and scaled size rounded to nearest and at
    least one pixel; `u64` products, so no size overflows.
  - `scale`: the one call into `pic-scale-safe`, RGB to RGB, after
    checking both sizes are non-zero, every byte count with `checked_mul`
    (and within `isize`), and the slice length; a same-size call is
    refused (the caller packs the source instead of paying for the copy
    the scaler would make).
  - `pack`: into the `XRGB8888` shm buffer, reading through the rotated
    index: orientations 1–4 as contiguous runs (reversed for mirrors),
    5–8 in 32×32 blocks; every size checked first, every access through
    `get`. Letterbox bars in the fill color; `tile` copies the first
    repeat by doubling `copy_within`s.
  - `render`: a size `wl_shm` cannot take is refused first; then, when
    scaling, crop in place (row sub-slice, columns compacted with
    `copy_within`) and `shrink_to_fit` (an `mremap` in place), scale in
    the stored orientation to the stored target size, **drop the source,
    then** allocate the shm buffer and pack. `center`, `tile` and a
    same-size image are packed straight from the source: no crop, no
    copy.
- **Off the loop** (`src/daemon/worker.rs`): a thread per job
  (`scootbg-decode`), ending with it, so an idle daemon has one thread.
  The result goes down an `mpsc` channel, then the thread writes an
  `eventfd` the poll loop watches (rustix, no new `unsafe`: `scootbg`
  stays `#![forbid(unsafe_code)]`). A guard sends a result on unwind, so
  a panic in a build that unwinds cannot leave the job waited for forever
  (the release profile aborts). One decode per job; each distinct buffer
  size rendered once (the source borrowed for all but the last, which
  takes it by value); other outputs of that size get a copy.
- **The queue** (`src/jobs.rs`, pure): a *trial* per image `set`, drawn
  for the targeted outputs configured when it was asked; a *render* when
  an image already chosen is needed at a size nobody drew it at (an
  output plugged in, reconfigured, a new scale), merged per image while it
  waits and never asked twice for a target a job already covers. The
  worker takes the **newest** job first. Trials queued are capped at 32
  (`MAX_TRIALS`, the same bound as the waiting replies); one more is
  refused, nothing changed.
- **Landing** (`src/daemon/images.rs`): results carry `OutputId`s and are
  looked up again; a buffer for an output gone, no longer wanting the
  image, or no longer that size, is dropped. A failed trial is an error
  reply naming the path and the reason, and nothing changes. A trial that
  newer choices have covered is answered `ok` and changes nothing. A
  winning trial is recorded, the outputs it now applies to are stamped
  with its generation and drawn, and its reply waits for them through the
  existing `waiters.rs` (commits plus a `wl_display.sync`).
- **Newest wins** (`src/choices.rs`): every choice carries its request's
  generation, and `Choices::set` records an older one only where nothing
  newer has chosen (a newer every-output choice supersedes everything
  older; a newer named one only that name). A color landing after an
  image was asked therefore stays, and so does an image asked after a
  color whatever order they finish in. When a choice is recorded, queued
  trials it covers are answered `ok` without being decoded.
- **Drawing** (`src/daemon/canvas.rs`): an image is a full-size shm
  buffer (the configured surface size times `wl_output`'s integer scale,
  with `set_buffer_scale`, the size the full-size color path uses) on
  every path, opaque region the whole surface. Buffer scale, viewport
  destination and opaque region are each tracked and sent only when they
  change, so a surface can go from an image to a 1×1 color and back
  (`set_buffer_scale(1)` with the 1×1 buffer) correctly. A released buffer
  holding an image is dropped unless it is on screen (nothing could reuse
  it but a new render), so after a change each output holds one image
  buffer, not two. A kept image buffer is attached again to a surface the
  compositor closed and scootbg re-created, without a decode.
- **CI**: the integration job runs `tests/image.rs` too, with scoot and
  sway required.

### Departures from the plan, and why

- **The pixel budget is a constant, not configurable.** The ticket said
  "configurable". scootbg has no config file yet
  ([config-and-rotation.md](../config-and-rotation.md)), and an
  environment variable or flag with no home would be a knob added ahead
  of its surface. `MAX_PIXELS` is one constant in `image/decode.rs`;
  whichever ticket adds scootbg's own configuration can expose it.
- **A thread per job, not a long-lived worker.** It keeps an idle daemon
  at one thread and returns each job's thread-local malloc state with the
  thread; starting one costs tens of microseconds against hundreds of
  milliseconds of decoding.
- **The source is never kept**, even while a hotplug or scale change is
  pending: every later size decodes the file again (the ticket allowed
  this: "re-decode rather than hold tens of MB").
- **Superseded requests answer `ok`**, not an error and not a separate
  marker (the choice the coordinating session left open): `ok` means "what
  shows reflects this request or a newer one". A request superseded before
  it started is never decoded, so a missing file in it goes unreported;
  the newer request's own reply is the one that describes the screen.
- **Newest first** instead of first come, first served: a burst of sets
  shows the last one after one decode, and the older ones are answered as
  superseded. If the newest fails, the next newest runs, so the newest
  request that *can* be shown wins.
- **EXIF orientation for PNG too** (its `eXIf` chunk), since the same
  reader serves it at no cost.
- **`--filter` has four choices** (Lanczos3 default, Catmull-Rom,
  bilinear, nearest): one runtime enum into the same scaler call.
- **Truncation needs a check of our own for JPEG** (below).

### Found along the way

- **Smithay applies a new buffer scale only with a new buffer.** scoot's
  pinned fork reads `set_buffer_scale`/`set_buffer_transform` in
  `RendererSurfaceState::update_buffer` only on `NewBuffer`, so a commit
  that changes only the scale keeps the old one (the protocol says it
  applies at the commit). A scale and mode that change together keep an
  image's buffer size, and the screenshot showed the buffer's top-left
  quarter blown up. scootbg now attaches the buffer on screen again
  whenever it sends a new scale without a new buffer; the scale test
  fails without it. The full-size color path had the same latent bug,
  invisible with a flat color. Filed for the compositor as
  [buffer-scale-needs-a-new-buffer.md](../../../backlog/core/buffer-scale-needs-a-new-buffer.md).
- **`zune-jpeg` decodes a JPEG cut inside its last row of MCUs
  "successfully"**, zero-padded, even in strict mode: it checks for
  running out of data only at the start of each MCU row
  (`mcu.rs`, `stream.overread_by`). The unit test cutting the fixture at
  every length found cuts 302–311 of 312 bytes decoding. The decoder now
  gets the reader by reference, and afterwards a 64-byte window around
  where it stopped must hold the EOI marker (`FF D9`, which entropy-coded
  data cannot contain). Data after the EOI (a phone's appended
  motion-photo video, trailers) still decodes; tested.
- **A PNG missing only its `IEND` chunk decodes**, and its picture is
  whole: allowed, and the test pins that such a cut is byte-identical to
  the whole file's pixels.
- **`render` validated the buffer size only when allocating it**, after
  the scaler had allocated the scaled image at that size (4.8 GB for a
  40000×40000 target). The `wl_shm` geometry is now checked first; a
  test drives every mode at that size and returns at once.
- **Rotated rendering differs from upright by up to ~22 levels at hard
  edges** (Lanczos ringing clamped between the scaler's two passes, which
  swap order for the 90° cases); on smooth content by at most 2. The test
  compares smooth content, and checks hard-edged quadrants away from the
  edges.
- **The hotplug-mid-decode race does happen mid-decode** on this machine
  in a debug build: the outputs changed 12 ms after the request, the reply
  came 4.9 s later (10.6 s with a 2400×1600 image, which was then made
  smaller to keep well inside the harness's 20 s patience under load).

### Verified where

All on a Claude Code web container (x86_64, 4 CPUs), no dev VM.

- **Unit tests** (215 in the binary): the EXIF reader (all eight
  orientations in both byte orders, with and without the `Exif\0\0`
  prefix, every truncation, malicious offsets and counts, 512 random
  blocks); orientation maps against the specification's table, the walk
  and the rectangle map over every sub-rectangle; fit geometry for every
  mode including zero, one-pixel and extreme aspects up to `u32::MAX`;
  the scaler over every 1–5 × 1–5 shape both ways and extreme aspects,
  every filter; decoding every PNG color type and depth, grey, palette,
  alpha over the fill, progressive and grey JPEG, lossless WebP RGB/RGBA,
  EXIF in all three formats, every truncation of four files, 1,200 random
  corruptions, bombs from the header (PNG 100000², JPEG 65535²), FIFOs,
  directories, `/dev/null`; packing in all eight orientations against the
  map, regions, bars, tiling; rendering every mode × orientation against
  upright; the job queue (newest first, superseding, merging, bounds);
  newest-wins choices; the worker's one decode for several targets, a
  failing target alone, the eventfd wake-up and the thread ending.
- **End to end** (`tests/image.rs`, 6 tests): every fit mode on scoot by
  screenshot (`center` and `tile` exact to the pixel, `nearest` exactly
  four colors); EXIF orientations 1, 3, 6 and 8 on a JPEG and 6 on a
  WebP, shown upright; every failure (missing, directory, text, truncated,
  bomb, relative path) an error that leaves both outputs' pixels and
  `query` unchanged; newest wins across image-then-color, color-then-
  image, a burst of six and a failing newest; outputs of different sizes
  on sway (1920×1080 and 1024×768), an output plugged in later, and one
  unplugged and one plugged in mid-decode; a scale change keeping the
  buffer size (no new decode, trace-counted) and a fractional one
  (2134×1334, a new decode), both by screenshot; one thread and no
  wakeups after each.
- **Commands** at `67a0c84`:
  `SCOOTBG_REQUIRE_SCOOT=1 SCOOTBG_REQUIRE_SWAY=1 SCOOTBG_TEST_SWAY=… devenv shell -- soft-egl cargo nextest run -p scootbg -p scootbg-mem`
  (266 passed, 1 skipped: the `#[ignore]`d benchmark);
  `cargo test -p scootbg -p scootbg-mem` with the same variables (all
  pass); `cargo clippy -p scootbg -p scootbg-mem --all-targets -- -D
  warnings` and `cargo fmt --check -p scootbg -p scootbg-mem` clean;
  `RUSTFLAGS="-D warnings" cargo build --release -p scootbg` clean;
  `cargo nextest run --workspace` (same variables) 2,321 passed, 25
  skipped (the compositor's 24 `#[ignore]`s and the benchmark);
  `cargo test -p scootbg -p scootbg-mem` 215 + 9 + 14 + 2 + 6 + 3 + 14 + 3
  passed, 1 ignored. **Stress** at `67a0c84`: `cargo nextest run -p
  scootbg -p scootbg-mem --stress-count 10 -j 8`, every one of the 266
  tests in all 10 iterations (323.5 s). An earlier run of the `color` and
  `image` binaries ×10 (342 s) passed too. actionlint on `ci.yml`: only
  the SC2174 at line 180 that `main` has.
- **Bug-bash by hand** (release, `67a0c84`'s binary, sha256 prefix
  `c8e9bb3f`): a CMYK JPEG, a 16-bit PNG, an interlaced (Adam7) PNG, a
  palette PNG, a half-transparent PNG (50% red over `#203040` shows
  (144,24,32), the exact blend), a path with a space, a non-ASCII letter
  and a `#`, and a symlink, all shown; 200 clients each sending an image
  `set` and hanging up at once: a waiting client is refused "too many
  images are waiting" (nothing changed), the backlog drains after one
  decode (newest first; the rest superseded), the state is the newest
  accepted request, heap back to 388 kB; a 12000×12000 JPEG: 898 ms,
  peak 438.7 MB, heap back to 388 kB; `kill` while an image decodes: the
  waiting `set` says the daemon closed the connection (exit 1), `kill`
  exits 0, the socket is removed; on sway with every output unplugged, an
  image `set` is validated and recorded (exit 0) and an output plugged in
  afterwards shows it.
- **Nix**: `nix build .#scootbg --option sandbox true` builds from a
  clean tree with `67a0c84`'s code (the docs commit on top), no warnings,
  1,480,416 B (the package strips with its own flags), linking
  `libgcc_s`, `libm` and `libc` only. `nix flake check` was not run here;
  CI runs it.

### Measurements

Release (`cargo build --release -p scootbg`), code at `67a0c84`; the test
images are the dependencies record's recipe regenerated (ImageMagick
fractal plasma plus Gaussian noise, 6000×4000, JPEG quality 92 4:2:0,
7,925,275 B; PNG 44,547,501 B; WebP q90 7,050,508 B; and the JPEG with an
orientation-6 APP1 segment spliced in, 7,925,311 B).

**In-process stages**, `fill` onto 3840×2160, 5 runs each (the
`#[ignore]`d `image::bench::pipeline`, `cargo test --release -p scootbg
-- --ignored --nocapture image::bench`; peak is the test process's
`VmHWM`, reset before each run, with the stages run one by one, then
`render()` whole):

| File | decode, ms | scale, ms | pack, ms | total, ms | peak, kB | `render()` whole, ms |
|---|---|---|---|---|---|---|
| JPEG | 231.9, 230.5, 233.9, 238.4, 244.6 | 160.6, 163.2, 161.6, 167.9, 160.7 | 23.3, 24.3, 22.7, 22.7, 24.1 | 426.7, 428.9, 428.8, 439.5, 440.2 | 88,932, then 88,884 ×4 | 428.5, 430.1, 438.9, 427.4, 432.9 |
| JPEG, orientation 6 | 236.6, 229.8, 229.9, 227.7, 225.2 | 124.7, 111.7, 111.6, 108.0, 108.5 | 63.2, 44.3, 43.7, 41.6, 42.1 | 434.2, 394.9, 393.5, 385.5, 384.6 | 76,020 ×5 | 410.6, 417.8, 392.8, 384.3, 387.7 |
| PNG | 230.9, 236.5, 232.0, 261.9, 237.9 | 162.3, 167.9, 161.4, 182.3, 194.9 | 22.7, 24.1, 22.7, 23.9, 33.0 | 426.8, 439.5, 426.9, 480.0, 479.7 | 89,012 ×5 | 416.7, 436.8, 425.0, 426.4, 451.2 |
| WebP | 1092.2, 1034.0, 1046.7, 1051.6, 1089.1 | 154.0, 152.8, 159.2, 166.9, 166.4 | 21.8, 22.2, 22.1, 22.6, 23.3 | 1278.4, 1219.6, 1238.9, 1252.2, 1289.9 | 120,328 ×5 | 1291.1, 1220.6, 1232.3, 1278.7, 1256.2 |

Crop in place 6.0–8.4 ms each; the shm allocation 0.0–0.1 ms. The test
process's `RssAnon` settled at 748 kB and stayed there across all 20 runs.

**End to end**: the release daemon on `scoot --headless --width 3840
--height 2160 --outputs 1` (debug scoot), `set` over the socket from a
Python client, `fill`, 3 rounds of the four files in turn; peak is the
daemon's `VmHWM` reset before each `set` (`/proc/PID/clear_refs`); CPU is
`utime + stime` of the whole process (worker threads included) across the
`set`; "after" is read once the daemon has been idle a second:

| File | request to reply, ms | CPU, ms | peak, kB |
|---|---|---|---|
| JPEG | 467.5, 444.8, 415.8 | 430, 440, 420 | 87,868 (the first set, nothing on screen yet), 120,464, 120,572 |
| JPEG, orientation 6 | 387.8, 387.7, 457.4 | 380, 380, 460 | 107,516, 107,728, 107,668 |
| PNG | 428.0, 443.2, 414.3 | 430, 440, 400 | 120,320, 120,576, 120,544 |
| WebP | 1313.1, 1300.1, 1335.6 | 1310, 1290, 1290 | 151,540, 151,636, 151,580 |

After every set: 1 thread, 9 fds (the 8 of a color daemon plus the
buffer's memfd), `RssShmem` 32,400 kB (one 3840×2160 buffer: the previous
one is dropped on release), `RssAnon` 372–652 kB, `[heap]` Rss 36 kB, RSS
36,476–36,804 kB, PSS 18,476–18,804 kB (the buffer is shared with the
compositor). Then 30 s idle: 0 context switches, 0 ms CPU.

**Against the dependencies record's predictions:**

- Scale 174 ms predicted: 152.8–194.9 ms measured (medians: JPEG 161.6,
  PNG 167.9, WebP 159.2).
- Decode: JPEG 234.8 ms predicted, 230.5–244.6 measured; PNG 233.4,
  230.9–261.9; WebP 1,302, 1,034–1,092.
- Per set 444 ms (J sequence, §3b): 426.7–480.0 ms in-process,
  414–468 ms request to reply.
- Pipeline peak 131.2 MB predicted: **88.9 MB** in-process (source cropped
  and shrunk, plus the scaled image: 60.8 + 24.9 MB over the process's
  base), **120.5 MB** in the daemon with the previous wallpaper's 32.4 MB
  buffer still mapped (and 87.9 MB for a first set). The in-place crop's
  `shrink_to_fit` returns the 11 MB cropped off before the scaler
  allocates; the prototype's accounting was not re-run to attribute the
  rest of the difference.
- Orientation in the packing pass, "designed, not measured": packing
  took 41.6–44.3 ms (one run 63.2) against 22.7–24.3 ms upright, **about
  +20 ms**, and **no extra memory** (the peak is lower, since the stored
  crop is narrower), against +60 MB and +200 ms for rotating the source
  first.

**Binary and idle cost** (release, stripped): 783,072 B at `2cbff25`
(before) → **1,500,008 B** (+716,936) at `67a0c84`; the round-two pipeline
prototype was 1,229,656 B over a 565,968 B base (+663,688). It now links
`libm.so.6` (the scaler's `sinf`), as §9 predicted; no `libc` crate
(`cargo tree -p scootbg -e normal --prefix none | grep -c '^libc '` → 0).
Idle with a color on two 1600×1000 outputs, 3 rounds interleaved, 10 s
windows:

| | RSS, kB | PSS, kB | `[heap]`, kB | threads | fds | context switches, CPU ticks |
|---|---|---|---|---|---|---|
| before | 2,684, 2,656, 2,672 | 1,233, 1,205, 1,224 | 36 | 1 | 7 | 0, 0 |
| after | 3,540, 3,664, 3,552 | 1,841, 1,978, 1,812 | 36 | 1 | 8 | 0, 0 |

The +0.9 MB is file-backed code, not heap (per mapping, `/proc/PID/smaps`):
scootbg's text +416 kB and read-only data +176 kB (a larger binary faults
more in around what a color touches) and `libm` +388 kB (shared with
every other process that maps it); the extra fd is the worker's eventfd.

### Not verified, and why

- **`--tty`, the dev VM, a GPU**: not reachable from this container.
  Nothing here is backend-specific, but the gap before the first frame of
  a restored 4K JPEG at login is ticket 9/10's to measure there.
- **Competitors**: lightest.md's run; scootbg's own numbers are above.
- **Coverage-guided fuzzing** of decode + scale: not done; the
  deterministic truncation and corruption loops are unit tests
  ([testing.md](../testing.md) keeps the `cargo fuzz` item).
- **An allocation failure inside the scaler**: `pic-scale-safe` allocates
  its output infallibly, so a refusal there aborts the daemon. The
  decoded buffer is reserved fallibly and the target size is checked
  against `wl_shm`'s limit first, but a legitimate huge target on a box
  that refuses the memory is not handled. Not reachable under Linux's
  default overcommit short of the OOM killer, which ends the process
  either way.
- **The kept image buffer on a re-created surface** is by reading: no
  compositor here closes the background surface of an output it keeps
  (the same gap as ticket 3's `closed` path).
- **A stalled image draw** (both buffers held when the render lands):
  neither compositor holds the older buffer; by reading, as for colors.
- **macOS `cargo check`**: no Darwin toolchain here; CI runs it.

### For the next tickets

- [hidpi-fractional-scale.md](../hidpi-fractional-scale.md): the buffer
  size is decided in one place, `daemon::change::image_dims` (surface
  size times the integer scale), and every image draw is the worker's
  full-size render, so a `wp_fractional_scale_v1` size is a change there
  plus a viewport destination. A scale change re-renders only when the
  buffer size changes; when it does not, the buffer is attached again for
  Smithay's sake.
- [memory-and-idle.md](../memory-and-idle.md): two outputs of one size
  get two buffers with the same pixels (one `wl_buffer` on both would save
  32.4 MB per extra 4K output); a rendered image waiting on a stalled draw
  is one more buffer per output at most.
- [restore-state.md](../restore-state.md): an image choice is its path,
  mode, fill and filter (`crate::wallpaper::Image`); restoring one is a
  trial like any `set`, so a file gone since is an error on stderr and the
  output shows nothing rather than something stale. Paths are UTF-8 only
  (the protocol is JSON).
- [lightest.md](../lightest.md): the Set row's CPU is the scaler's (§3b);
  the WebP decode is 1 s of it for WebP.
- [animated-images.md](../animated-images.md): animated PNG and WebP show
  their first frame today.
