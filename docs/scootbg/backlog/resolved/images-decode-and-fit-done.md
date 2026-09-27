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

Code at `3a8bf74`: `0af2e5a` and `67a0c84`, then `3a8bf74`, the fixes from
the review of PR #281 ([below](#review-of-pr-281)); the docs came after
each, in docs-only commits.

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
    allocated, and a JPEG's length against the least data its size needs
    (one bit per 8×8 block, `jpeg_min_len`). The pixel buffer is zeroed
    memory that is fallible and committed only as it is written
    (`scootbg_mem::zeroed_bytes`), so the allocator refusing it is an
    error reply, and a file claiming more than it holds costs what it
    holds. The decoders' own working memory is not fallible (see
    [Not verified](#not-verified-and-why)). Grey, grey +
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
  The result goes down an `mpsc` channel with its job's ticket, then the
  thread writes an `eventfd` the poll loop watches (rustix, no new
  `unsafe`: `scootbg` stays `#![forbid(unsafe_code)]`); the loop takes
  only the awaited ticket's result and discards anything else. A guard,
  made inside the thread, sends a result on unwind, so a panic in a build
  that unwinds cannot leave the job waited for forever (the release
  profile aborts); a thread that cannot be started sends nothing, and its
  error is `start`'s return value alone. One decode per job; each distinct buffer
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
  newer choices have covered changes nothing, and its reply waits at its
  own generation, as a superseded color's does, so its `ok` comes once
  what replaced it is on screen. A
  winning trial is recorded, the outputs it now applies to are stamped
  with its generation and drawn, and its reply waits for them through the
  existing `waiters.rs` (commits plus a `wl_display.sync`).
- **Newest wins** (`src/choices.rs`): every choice carries its request's
  generation, and `Choices::set` records an older one only where nothing
  newer has chosen (a newer every-output choice supersedes everything
  older; a newer named one only that name). A color landing after an
  image was asked therefore stays, and so does an image asked after a
  color whatever order they finish in. When a choice is recorded, queued
  trials it covers leave the queue without being decoded, their replies
  waiting as above.
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
  compositor closed and scootbg re-created, without a decode. A rendered
  image waiting to go on screen is dropped as soon as anything else is
  drawn or wanted there. A buffer attached again only to carry a new
  scale is marked held until its next release, like any attach.
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
  shows reflects this request or a newer one", and, as for a color, it
  comes once the newer choice is on screen (at first it came at once; the
  review asked for this). A request superseded before it started is never
  decoded, so a missing file in it goes unreported.
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

### Review of PR #281

One blocking finding and six more; all fixed in `3a8bf74`, each with a
test that fails without its fix where the code can be tested (checked by
reverting the fix and running it):

1. **Blocking: a thread that could not be started landed twice.** The
   result guard was built outside the thread and moved into the spawn
   closure; when `spawn` failed, std dropped the closure, the guard sent
   "the decoding thread failed" and woke the loop, and `pump` also landed
   the job as the spawn error. One wakeup takes one message, so two
   refusals in one loop turn left a stale "failed" behind, which the next
   job took as its own, and every later result landed one job late.
   **Reproduced** here as the reviewer did, in a v1 pids cgroup
   (`pids.max` 1 on the daemon, two image `set`s at once, the limit
   lifted, two more one by one), 3 rounds on each binary:
   - `67a0c84`, 3 of 3: both refused ("cannot start a decoding thread"),
     then blue: exit 1, "the decoding thread failed"; then yellow: exit
     0, `query` says `cg-yellow.png`, the screen shows (0, 0, 255), blue;
   - `3a8bf74`, 3 of 3: both refused, then blue: exit 0, screen (0, 0,
     255), `query` blue; yellow: exit 0, screen (255, 255, 0), `query`
     yellow.

   **Fix:** the guard is made inside the thread, so a spawn that never
   ran sends nothing; every result carries its job's ticket and the loop
   takes only the awaited one (anything else is discarded); `spawn` is a
   seam (`Worker::with_spawn`). Tests:
   `a_thread_that_cannot_start_leaves_no_result_behind` (fails with the
   guard moved back outside: "a refused spawn wakes nothing"),
   `a_result_for_another_job_is_discarded`.
2. **A few hundred bytes committed 0.8–1.06 GB.** The decoded buffer was
   reserved and then filled with zeros, touching every page before a
   pixel was read. It is now `scootbg_mem::zeroed_bytes`: zeroed memory
   from the allocator (a fresh mapping for a large block), committed only
   as written, and fallible. The decoders' own large buffers were zeroed
   allocations already, so they are lazy too. Measured on every PNG color
   type, Adam7, 16-bit, progressive JPEG, lossless (opaque and alpha) and
   lossy WebP ([the table below](#measurements)): from 0.79–1.06 GB and
   0.5–0.7 s to within 72 kB of the RSS before, under 1 ms. **Also
   found:** a *baseline* JPEG of 312 bytes claiming 16384×16384 was
   accepted and shown (2.2 s, 791 MB, flat grey): `zune-jpeg` feeds zeros
   past an early end-of-image marker, so every pixel was really written
   and lazy memory could not help. A JPEG must now hold at least one bit
   per 8×8 block of its full-resolution component (`jpeg_min_len`: every
   block codes its DC coefficient with a Huffman code of one bit or
   more), checked from the header. Tests:
   `zeroed_bytes_are_committed_only_as_written` (`scootbg-mem`),
   `the_jpeg_size_bound_is_one_bit_per_block`, the bomb test's in-budget
   JPEG, and end to end
   `a_file_that_claims_a_large_size_costs_what_it_holds` (six such files
   against the daemon, peak under 16 MB above; with the old buffer it
   fails at "rgb.png (4160 bytes): peak 792396 kB from 5996 kB", without
   the JPEG bound it fails on the baseline JPEG, the debug daemon taking
   past the client's 30 s to decode 805 MB of grey). The module docs and
   [Not verified](#not-verified-and-why) now say exactly which
   allocations are fallible. Also noted: `image-webp` computes a lossless
   width as `(1 + field) & 0x3FFF`, so 16384, the format's maximum, reads
   as 0 and is refused as 0×0; harmless, not ours to fix.
3. **Stale numbers** in `docs/scootbg/README.md` and `lightest.md`: every
   copy now carries the `3a8bf74` measurements.
4. **A superseded image answered `ok` at once** while a superseded color
   waits for the newer choice to be on screen. It now pushes a waiter at
   its own generation, like a color (`set --help`, both READMEs say so).
   Test: `a_superseded_image_request_waits_like_a_color` (fails with the
   immediate reply); `the_newest_set_wins` now screenshots the moment a
   superseded request answers. (End to end the old and new orders cannot
   be told apart here: the newer choice is committed in the same loop
   turn, and the old reply went through a sync too.)
5. **`Canvas::ready` outlived its use**: a rendered image is now dropped
   on any draw of something else, when an unchanged or kept buffer is
   shown, and at every `reconcile` where the output no longer wants that
   image (which covers a surface not configured yet).
6. **The scale-only re-attach did not mark the buffer held**; it now goes
   through the same `attached()` as any attach, so a later draw cannot
   `Reuse` and fill it while the compositor may be reading it.
7. **EXIF orientation for PNG** was applied but not documented: `set
   --help` and the README now say JPEG, WebP and PNG `eXIf`.

Also: three leftover processes from the bug-bash (two headless scoots and
a scootbg daemon, started by the scratch `run.sh`) were stopped and their
runtime directories removed.

**Verified** at `3a8bf74` (logs in the scratch record, each headed with
the SHA and a clean `crates/`):

- `cargo fmt --check -p scootbg -p scootbg-mem`: exit 0;
- `cargo clippy -p scootbg -p scootbg-mem --all-targets -- -D warnings`:
  exit 0;
- `RUSTFLAGS="-D warnings" cargo build --release -p scootbg`: exit 0,
  1,500,008 B;
- `SCOOTBG_REQUIRE_SCOOT=1 SCOOTBG_REQUIRE_SWAY=1 SCOOTBG_TEST_SWAY=…
  soft-egl cargo nextest run -p scootbg -p scootbg-mem`: 275 passed,
  1 skipped;
- `cargo test -p scootbg -p scootbg-mem` (same variables): 219 + 9 + 14
  + 2 + 7 + 3 + 17 + 4 passed, 1 ignored;
- the same nextest with `--stress-count 10 -j 8`: 10 of 10 iterations
  (331.3 s);
- `cargo nextest run --workspace`: 2,330 passed, 25 skipped.

### Measurements

Release (`cargo build --release -p scootbg`), code at `3a8bf74` (after the
review's fixes; the first round, at `67a0c84`, was within a few percent
on every row but two, noted below). The test images are the dependencies
record's recipe regenerated (ImageMagick fractal plasma plus Gaussian
noise, 6000×4000, JPEG quality 92 4:2:0, 7,925,275 B; PNG 44,547,501 B;
WebP q90 7,050,508 B; and the JPEG with an orientation-6 APP1 segment
spliced in, 7,925,311 B).

**In-process stages**, `fill` onto 3840×2160, 5 runs each (the
`#[ignore]`d `image::bench::pipeline`, `cargo test --release -p scootbg
-- --ignored --nocapture image::bench`; peak is the test process's
`VmHWM`, reset before each run, with the stages run one by one, then
`render()` whole):

| File | decode, ms | scale, ms | pack, ms | total, ms | peak, kB | `render()` whole, ms |
|---|---|---|---|---|---|---|
| JPEG | 227.1, 222.2, 216.0, 225.3, 226.4 | 156.3, 154.0, 206.4, 158.8, 157.6 | 22.7, 22.3, 30.6, 22.6, 22.5 | 416.7, 409.2, 466.2, 417.7, 417.5 | 88,864, then 88,816 ×4 | 412.0, 409.2, 407.5, 415.7, 407.3 |
| JPEG, orientation 6 | 220.6, 218.7, 217.1, 221.5, 217.6 | 103.8, 105.6, 121.0, 106.4, 105.5 | 40.7, 42.0, 43.0, 42.3, 43.6 | 373.2, 374.4, 390.1, 378.6, 374.7 | 75,952 ×5 | 378.8, 375.7, 387.2, 380.5, 364.3 |
| PNG | 217.2, 216.2, 221.6, 214.9, 221.2 | 164.1, 156.2, 155.2, 154.6, 158.4 | 23.3, 21.2, 22.5, 22.0, 20.9 | 415.4, 405.5, 410.6, 402.9, 411.3 | 89,072 ×5 | 399.5, 406.0, 400.3, 410.4, 419.4 |
| WebP | 1019.9, 1006.6, 1006.4, 1021.8, 1090.8 | 159.7, 148.5, 155.8, 158.1, 153.2 | 21.5, 20.7, 22.2, 22.4, 22.3 | 1211.8, 1186.2, 1194.9, 1213.1, 1278.0 | 110,728 ×5 | 1194.2, 1188.4, 1214.7, 1251.4, 1231.0 |

Crop in place 5.9–8.1 ms each; the shm allocation 0.0 ms. The test
process's `RssAnon` settled at 748 kB and stayed there across all 20
runs. Against `67a0c84` (same method, same files): decode 5–15 ms faster
(the pixel buffer is no longer written with zeros before the decoder
writes it), and WebP's peak 110,728 kB, down from 120,328 (the frame
buffer is sized for RGB or RGBA, whichever is larger, and the part the
decoder never writes is no longer made resident).

**End to end**: the release daemon on `scoot --headless --width 3840
--height 2160 --outputs 1` (debug scoot), `set` over the socket from a
Python client, `fill`, 3 rounds of the four files in turn; peak is the
daemon's `VmHWM` reset before each `set` (`/proc/PID/clear_refs`); CPU is
`utime + stime` of the whole process (worker threads included) across the
`set`; "after" is read once the daemon has been idle a second:

| File | request to reply, ms | CPU, ms | peak, kB |
|---|---|---|---|
| JPEG | 433.6, 402.6, 397.0 | 420, 400, 390 | 87,992 (the first set, nothing on screen yet), 120,608, 120,536 |
| JPEG, orientation 6 | 431.5, 446.8, 386.4 | 430, 450, 380 | 107,644, 107,700, 107,804 |
| PNG | 417.6, 408.1, 409.2 | 420, 400, 410 | 120,444, 120,580, 120,556 |
| WebP | 1289.4, 1247.5, 1218.0 | 1280, 1240, 1210 | 141,964, 141,952, 142,092 (151,540–151,636 at `67a0c84`) |

After every set: 1 thread, 9 fds (the 8 of a color daemon plus the
buffer's memfd), `RssShmem` 32,400 kB (one 3840×2160 buffer: the previous
one is dropped on release), `RssAnon` 372–568 kB, `[heap]` Rss 36 kB, RSS
36,540–36,716 kB, PSS 19,267–19,443 kB (the buffer is shared with the
compositor). Then 30 s idle: 0 context switches, 0 ms CPU.

**Files that claim more than they hold** (the review's finding 2): each
claims 16384×16384 (16383×16383 for WebP, the largest `image-webp` reads)
and holds a few hundred bytes; the daemon's `VmHWM` after each `set`, reset
before it, against its RSS before (release, one 1600×1000 output, one
daemon per binary, files in the order listed):

| File (bytes) | `67a0c84`: reply, ms, peak kB | `3a8bf74`: reply, ms, peak kB (RSS before) |
|---|---|---|
| PNG RGB (79) | error, 587.2, 789,992 | error, 0.8, 3,760 (3,440) |
| PNG RGBA (79) | error, 676.8, 1,052,356 | error, 0.5, 3,764 (3,760) |
| PNG grey (79) | error, 482.3, 790,276 | error, 0.5, 3,768 (3,764) |
| PNG Adam7 (79) | error, 482.3, 790,280 | error, 0.7, 3,816 (3,768) |
| PNG 16-bit (79) | error, 487.5, 790,384 | error, 0.7, 3,816 (3,816) |
| JPEG baseline (312) | **ok (shown: flat grey)**, 2,213.8, 791,152 | error, 0.5, 3,880 (3,816) |
| JPEG progressive (546) | error, 481.8, 796,768 | error, 0.6, 3,880 (3,880) |
| WebP lossless, opaque (942) | error, 488.9, 796,640 | error, 0.7, 3,952 (3,880) |
| WebP lossless, alpha (938) | error, 639.5, 1,058,764 | error, 0.9, 3,952 (3,952) |
| WebP lossy (176) | error, 498.1, 796,744 | error, 0.8, 3,952 (3,952) |

The files are made by `make.py` in the scratch record (PNG: IHDR plus an
unfinished zlib stream of 4 KiB of zeros; JPEG: the test fixtures with
the frame header's size rewritten; WebP: small real files with the
header's size rewritten); `tests/image.rs` builds the same kinds in code
and asserts a peak under 16 MB above the RSS before.

**Against the dependencies record's predictions:**

- Scale 174 ms predicted: 103.8–206.4 ms measured (medians: JPEG 157.6,
  PNG 156.2, WebP 155.8; orientation 6 scales a narrower stored crop,
  105.6).
- Decode: JPEG 234.8 ms predicted, 216.0–227.1 measured; PNG 233.4,
  214.9–221.6; WebP 1,302, 1,006–1,091.
- Per set 444 ms (J sequence, §3b): 402.9–466.2 ms in-process,
  397.0–433.6 ms request to reply.
- Pipeline peak 131.2 MB predicted: **88.8 MB** in-process (source cropped
  and shrunk, plus the scaled image: 60.8 + 24.9 MB over the process's
  base), **120.5–120.6 MB** in the daemon with the previous wallpaper's
  32.4 MB buffer still mapped (and 88.0 MB for a first set). The in-place
  crop's `shrink_to_fit` returns the 11 MB cropped off before the scaler
  allocates; the prototype's accounting was not re-run to attribute the
  rest of the difference.
- Orientation in the packing pass, "designed, not measured": packing
  took 40.7–43.6 ms against 20.9–30.6 ms upright, **about +20 ms**, and
  **no extra memory** (the peak is lower, since the stored crop is
  narrower), against +60 MB and +200 ms for rotating the source first.

**Binary and idle cost** (release, stripped): 783,072 B at `2cbff25`
(before) → **1,500,008 B** (+716,936) at `67a0c84` and again at `3a8bf74`
(sizes come in 4 KiB steps); the round-two pipeline prototype was
1,229,656 B over a 565,968 B base (+663,688). It now links `libm.so.6`
(the scaler's `sinf`), as §9 predicted; no `libc` crate (`cargo tree -p
scootbg -e normal --prefix none | grep -c '^libc '` → 0). Idle with a
color on two 1600×1000 outputs, 3 rounds interleaved, 10 s windows, two
batches (PSS depends on what else maps the same libraries at the time,
so it is compared within a batch):

| | RSS, kB | PSS, kB | `[heap]`, kB | threads | fds | context switches, CPU ticks |
|---|---|---|---|---|---|---|
| before (`2cbff25`) | 2,684, 2,656, 2,672 | 1,233, 1,205, 1,224 | 36 | 1 | 7 | 0, 0 |
| `67a0c84` | 3,540, 3,664, 3,552 | 1,841, 1,978, 1,812 | 36 | 1 | 8 | 0, 0 |
| before (`2cbff25`), second batch | 2,656, 2,664, 2,676 | 1,747, 1,755, 1,767 | 36 | 1 | 7 | 0, 0 |
| `3a8bf74`, second batch | 3,536, 3,536, 3,564 | 2,461, 2,469, 2,509 | 36 | 1 | 8 | 0, 0 |

The +0.9 MB RSS (+0.6 to +0.7 MB PSS) is file-backed code, not heap (per
mapping, `/proc/PID/smaps`, at `67a0c84`): scootbg's text +416 kB and
read-only data +176 kB (a larger binary faults more in around what a
color touches) and `libm` +388 kB (shared with every other process that
maps it); the extra fd is the worker's eventfd.

### Not verified, and why

- **`--tty`, the dev VM, a GPU**: not reachable from this container.
  Nothing here is backend-specific, but the gap before the first frame of
  a restored 4K JPEG at login is ticket 9/10's to measure there.
- **Competitors**: lightest.md's run; scootbg's own numbers are above.
- **Coverage-guided fuzzing** of decode + scale: not done; the
  deterministic truncation and corruption loops are unit tests
  ([testing.md](../testing.md) keeps the `cargo fuzz` item).
- **Infallible allocations outside our buffer.** Only the decoded pixel
  buffer is fallible. These are plain `Vec`s, and the allocator refusing
  one aborts the daemon:
  - `zune-jpeg`: its per-MCU-row buffers and upsampler scratch (width-
    sized), and for a progressive JPEG its coefficients for the whole
    image (2 bytes per sample per component: up to 1.6 GB at the budget);
  - `png`: its row and unfiltering buffers (width-sized, and bounded by
    its own 64 MiB limit);
  - `image-webp`: the RGBA frame it decodes an opaque lossless image into
    before copying out RGB (4 bytes a pixel), and a lossy image's YUV
    planes (1.5 bytes a pixel) and alpha plane;
  - `pic-scale-safe`: its output (the target's size in RGB), after the
    target is checked against `wl_shm`'s limit;
  - scootbg's own crop copy for a second size (`try_reserve_exact`, so
    that one is fallible) and the copies for same-size outputs (an shm
    buffer each, fallible).

  The large ones are zeroed allocations (`vec![0; n]`), committed only
  as written, so a lying file does not make them resident (the table
  above). What is not handled is a *legitimate* image near the budget on
  a machine that refuses the memory; under Linux's default overcommit
  that is the OOM killer's decision, which ends the process either way.
- **Fixes 5 and 6 of the review** (a rendered image dropped as soon as it
  is moot; a buffer re-attached for a scale change marked held) are by
  reading: the canvas works on live Wayland objects and has no unit
  tests, and neither compositor here stalls an image draw or lets the
  re-attached buffer be reused while held long enough to observe. The
  scale test (`a_new_scale_redraws_at_the_real_pixel_size`) runs the
  re-attach path and still passes.
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
