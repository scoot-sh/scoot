---
title: "Choosing dependencies (and checking licences)"
status: "resolved"
area: "scootbg"
priority: null
blocked: null
---

# Choosing dependencies (and checking licences) — RESOLVED

Decided by measurement on 2026-09-26, before any scootbg code exists, in
two rounds:

- **Round one** picked the crates. It also found that glibc keeps a
  decode's heap unless tuned, and proposed `mallopt` + `malloc_trim`
  through `libc` to fix that.
- **Round two** followed the user's rule that scootbg calls no C and
  trades neither speed nor safety for the other. It replaced the glibc
  tuning with a pure-Rust allocator wrapper, audited the tree for C and
  `unsafe`, and changed the scaler to `pic-scale-safe`. The user made the
  scaler choice after both were measured.

Throwaway prototypes were built outside the repository, each with its own
target directory, against the workspace's own release profile and
`Cargo.lock`, and run against `scoot --headless`. They were deleted
afterwards. This file is the record: the decisions first, then what they
change in the plan, then the evidence. Where round two overrides round
one, the round-one text stays as evidence and is marked **superseded**.

The original ticket asked for the choice to be recorded in the crate's
`Cargo.toml` comments as well. That happens in the
[crate PR](../crate-and-daemon.md), which links here.

## Decisions

| Area | Choice | Main alternative | Deciding numbers |
|---|---|---|---|
| Wayland client | `wayland-client` 0.31 + `wayland-protocols` 0.32 (`client`, `staging`) + `wayland-protocols-wlr` 0.3 (`client`), no toolkit | `smithay-client-toolkit` 0.21 | 566 KB vs 607 KB, clean build 30.4 s vs 36.7 s; idle identical (0 wakeups); every crate already in the workspace lock, while SCTK adds 4 |
| Decoding | `zune-jpeg` 0.5 + `png` 0.18 + `image-webp` 0.2 directly, plus a ~30-line EXIF orientation reader | `image` 0.25 (`png`, `jpeg`, `webp`); `zune-png`; `jpeg-decoder` | +565 KB vs +705 KB. JPEG peak +1.1 MB over the output buffer vs +10.7 MB (and +35.6 MB for `jpeg-decoder`). PNG 233 ms, +0.6 MB, vs `zune-png`'s 551 ms, +70 MB |
| **Scaling** | **`pic-scale-safe` 0.1 (`#![forbid(unsafe_code)]`)** | `fast_image_resize` 6 (~650 `unsafe` sites, unfuzzed); `image::imageops::resize`; `resize` | 6000×4000 → 3840×2160 Lanczos3: 174 ms vs fir's 80 ms and `image`'s 892 ms. Transient: output only (+25 MB) vs +63 MB and +227 MB. Pipeline binary 1,229,656 vs 1,868,632 B; clean / incremental build 36.5 / 15.0 s vs 56.4 / 34.6 s. **Costs +40% CPU per JPEG/PNG change** (§3b) |
| **Heap return** | **a ~50-line `#[global_allocator]` wrapper: blocks ≥ 128 KiB are their own `rustix` mapping (`mmap`/`munmap`/`mremap`), smaller ones go to `System`** | glibc `mallopt` + `malloc_trim` through `libc` (**superseded**); caller-owned mappings for every image buffer; `mimalloc` | 8 live sets, JPEG/PNG and WebP, main thread and decode thread: heap ≤ 0.65 MB with `pic-scale-safe` (≤ 0.51 MB with fir) with no tuning and no C calls, at the same CPU as `mallopt` + trim (≤ 0.24 MB), for +37 KB (§6b) |
| Peak memory | the source is dropped before the shm buffer is allocated, and no scaler keeps a source-width × target-height intermediate | fir's one-shot resize (internal 39 MB buffer) | measured pipeline peak **131.2 MB** with `pic-scale-safe`, vs 145.1 MB for fir split into two passes and 169.8 MB for fir one-shot (§3b, §6b) |
| Output format | scale RGB (3 bytes per pixel), then one packing pass into the XRGB8888 shm buffer | scaling 4-channel pixels straight into the shm buffer | the straight path links fir's U8x4 and alpha code: 4,113,240 vs 1,835,864 B at equal CPU, and 181 vs 146 MB peak (§6b). **Rejected** |
| CLI | hand-rolled, as `scootctl` does | `lexopt`, `pico-args`, `clap` | +12 KB vs +25 / +16 / +299 KB |
| Socket JSON | `serde` + `serde_json` (workspace deps), scootbg's own framing | `miniserde`, `nanoserde`; reusing `scoot-ipc` | +74 KB vs +49 / +53 KB; `scoot-ipc` framing is byte-identical in size but brings no benefit (see §5) |
| State file | hand-written line format | `toml` 1 | +20 KB vs +201 KB (+180 KB even with serde already in) |
| `unsafe` in scootbg | `#![forbid(unsafe_code)]` in `scootbg`; the only `unsafe` lives in a small crate, `scootbg-mem` (the allocator wrapper and the shm mapping), each block with a written safety argument (§11) | `unsafe` spread through the daemon | — |
| Build profile | the workspace `[profile.release]` unchanged (opt-level 3, fat LTO) | per-package `opt-level = "s"` / `"z"` | combined binary / ms per set: `3` 1.73 MB / 356; `s` 1.72 MB (−0.9%) / 380, but the base grew 17%; `z` 4.09 MB / 1241 |
| Async runtime | none (unchanged) | — | the prototypes' `blocking_dispatch` loop: 0 context switches, 0 CPU ticks over 60 s |

**No C in scootbg's own dependency tree.** The `libc` crate is not
compiled, no build script compiles or links C, and `rustix` uses its
`linux_raw` backend (raw syscalls). The C that remains is std's:

- glibc (`libc.so.6`) for small allocations, threads, file I/O and TLS;
- `libm` (`sinf`, from `pic-scale-safe`'s filter weights);
- `libgcc_s`, the unwinder.

See §9.

Every crate in the chosen tree is MIT-compatible (§8). Nothing comes from
awww/swww, wpaperd or hyprpaper.

**Two binaries were measured, not one combined build:**

- **Round one's full combination** (Wayland client, decoders, fir,
  serde_json, a state file, the CLI): 1,733,480 bytes. It idled at 3.6 MB
  RSS, 188 KB of it heap.
- **Round two's pipeline binary** (Wayland client, decoders,
  `pic-scale-safe`, the allocator wrapper): 1,229,656 bytes.

The combination with `pic-scale-safe` was not rebuilt as one binary.
Scaler aside, the parts are the same, so it should land near 1.1–1.2 MB,
but that is an estimate, not a measurement.

## What this changes in the plan

- **[lightest.md](../lightest.md): heap goes back to the kernel by
  construction, not by tuning.**
  - glibc keeps a decode's heap: a decode thread's arena held 61.6 MB
    after eight live sets, even with `malloc_trim(0)` after each (§6a).
  - Round one's fix was two `mallopt` calls plus a trim, through `libc`.
    That is **superseded** by the allocator wrapper (§6b), which needs no
    C call and no tuning and also covers dependencies' internal buffers.
- **[images-decode-and-fit.md](../images-decode-and-fit.md): scale with
  `pic-scale-safe`, then pack.**
  - Crop to the fill rectangle in place: rows are a sub-slice, and
    columns are compacted row by row.
  - Scale RGB to RGB, then write the XRGB8888 shm buffer in one packing
    pass. **Apply EXIF orientation inside that packing loop** by reading
    the scaled image through the rotated index, so rotation costs no
    buffer at all. The `image` crate's way (rotate the decoded source
    first) measured +60 MB peak and +200 ms on 6000×4000. The
    packing-loop version is designed, not measured.
  - Validate dimensions before allocating or scaling.
- **[crate-and-daemon.md](../crate-and-daemon.md):**
  - Two crates: `scootbg` (`#![forbid(unsafe_code)]`) and `scootbg-mem`
    (the only `unsafe`).
  - No `scoot-ipc` reuse. It pulls no compositor types, but it would
    share only ~25 lines of generic framing and would put
    `crates/scoot-ipc/` on scootbg's CI path (§5).
  - Weigh the release binary with `cargo build --release -p scootbg`,
    never `--workspace`, which unifies features.
- **[testing.md](../testing.md): fuzz decode + scale ourselves.**
  `pic-scale-safe` has no upstream fuzzing. Its `forbid(unsafe_code)`
  turns a bug into a panic rather than memory corruption, and a panic
  aborts the daemon.
- **The CPU-per-change gate row is the known risk of `pic-scale-safe`.**
  It spends ~100 ms more than fir per 4K change (a change only; idle is
  untouched). lightest.md's competitor run is where that is checked.
  scootbg keeps the scaler behind one function, so a swap stays contained
  if it ever loses that row.

## Setup

- Machine: a Claude Code web container, Intel Xeon @ 2.10 GHz, 4 vCPUs
  (AVX2 and AVX-512 present), 16 GB. rustc 1.97.1, glibc 2.42 (the devenv
  shell's), ImageMagick from the devenv shell. **Timings are from a shared
  VM and noisy.** Memory figures and binary sizes are close to exact; times
  within ~10% of each other are ties.
- Compositor: `target/debug/scoot --headless --outputs 2` from `main` at
  `48a8c21`, two 1600×1000 outputs, pixman renderer, `WAYLAND_DISPLAY=wayland-1`.
- Profile: a copy of the workspace's `[profile.release]` (`lto = "fat"`,
  `codegen-units = 1`, `panic = "abort"`, `strip = true`, `opt-level = 3`),
  the workspace `Cargo.lock` copied in, and the same `[patch.crates-io]`
  for `wayland-backend`, so versions and sources match what a workspace
  member would get.
- Prototypes: **A** (plain `wayland-client`, 179 lines), **B** (the same
  through SCTK), and **full**: A's Wayland code plus decode, scale, CLI,
  JSON, state and allocator variants behind cargo features, so every
  size delta is over the same base. The base with no features built
  byte-identical to A (565,968 bytes).
- Test images, from ImageMagick: `big.jpg` is 6000×4000, fractal plasma
  plus Gaussian noise so it has a photo's entropy, quality 92, 4:2:0,
  9,493,408 bytes. From it: `big.png` (48,978,449 bytes), `big.webp`
  (q90, 8,639,796 bytes), and `big-orient6.jpg`, the same JPEG with a
  hand-built EXIF APP1 segment for orientation 6 (ImageMagick reads it
  back as `RightTop`). The first attempt had no noise and came out at
  1.9 MB, unrealistically compressible, so it was discarded.
- Memory in-process from `/proc/self/status`. `VmHWM` is reset with
  `echo 5 > /proc/self/clear_refs` before the measured step, so a peak
  belongs to that step alone. `RssAnon` is the heap, `RssShmem` the
  stand-in `memfd` buffer.
- **Round two** used the same machine, profile and lock, with
  regenerated test images: the same recipe, but new random noise, so
  `big.jpg` is 9,626,713 bytes this time. Every round-one variant it is
  compared against was therefore re-run in the same interleaved batch;
  no round-two figure is compared with a round-one number. Process CPU
  per set is `clock_gettime(ProcessCPUTime)` through rustix, and PSS is
  from `/proc/self/smaps_rollup`.
- Stripped sizes seem to come in 4 KiB steps, so two builds with the same
  byte count are not necessarily the same code.
- Decode, scale and single-set pipeline figures are medians of 5 runs.
  Compile times, idle windows and the repeated-set heap sequences ran 3
  times, and a few checks ran once; each table says which. Raw values are
  listed throughout.

## 1. Wayland client stack

Both prototypes bind `wl_compositor`, `wl_shm`, `zwlr_layer_shell_v1`,
`wp_viewporter`, `wp_single_pixel_buffer_manager_v1` and
`wp_fractional_scale_manager_v1`. They create one `background` layer
surface per `wl_output` (anchored to all edges, exclusive zone −1, no
keyboard, empty input region), attach a single-pixel buffer scaled by the
viewport on configure, and then sit in `blocking_dispatch`.

**They work on scoot.** Screenshot both outputs
(`scootctl screenshot --output N --no-cursor`) and sample pixels (10,10),
(800,500) and (1590,990) with `magick … -format %[pixel:p{x,y}]`:

```
a output 1: 1600x1000 px(10,10)=srgba(30,128,64,1) px(800,500)=srgba(30,128,64,1) px(1590,990)=srgba(30,128,64,1)
a output 2: 1600x1000 px(10,10)=srgba(30,128,64,1) px(800,500)=srgba(30,128,64,1) px(1590,990)=srgba(30,128,64,1)
b output 1: 1600x1000 px(10,10)=srgba(48,80,192,1) px(800,500)=srgba(48,80,192,1) px(1590,990)=srgba(48,80,192,1)
b output 2: 1600x1000 px(10,10)=srgba(48,80,192,1) px(800,500)=srgba(48,80,192,1) px(1590,990)=srgba(48,80,192,1)
```

(`#1e8040` and `#3050c0` requested, exact on every sample.)

| | A: wayland-client | B: SCTK 0.21.1 |
|---|---|---|
| Stripped binary | 565,968 B | 606,928 B (+40,960) |
| `ldd` beyond glibc | `libgcc_s` | `libgcc_s` |
| Clean release build | 30.93 / 29.95 / 30.38 s → **30.4 s** | 37.24 / 36.65 / 36.70 s → **36.7 s** |
| RSS / PSS after 3 s, 3 runs | 2500 / 1306, 2500 / 1306, 2496 / 1305 KB | 2540 / 1346, 2544 / 1350, 2544 / 1350 KB |
| Heap (`RssAnon`) | 164 KB | 172 KB |
| Voluntary + involuntary context switches, 30 s window, 3 runs | 0, 0, 0 | 0, 0, 0 |
| CPU ticks (utime + stime), 30 s, 3 runs | 0, 0, 0 | 0, 0, 0 |
| 60 s window, 1 run | 0 switches, 0 ticks, RSS 2500 KB | 0 switches, 0 ticks, RSS 2532 KB |
| Crates not already in the workspace lock | none | `smithay-client-toolkit`, `wayland-csd-frame`, `wayland-cursor`, `wayland-protocols-experimental` |

**Decision: plain `wayland-client`.** Idle is a tie, but plain is 41 KB
and 6 s lighter. SCTK's saving is the registry and output bookkeeping,
and prototype A's whole client, registry and hotplug included, is 179
lines. Tracking the `wl_output` name and scale events is the one other
small piece scootbg needs. SCTK also compiles cursor, CSD-frame and
experimental-protocol code that a wallpaper daemon never uses.

**The `wayland-backend` fork.** The root `[patch.crates-io]` applies to
every workspace member, so scootbg would build against
`scoot-sh/wayland-rs` at `70f81e00`. The fork is 0.3.17 plus two commits.
`git diff 72f7fe0d 70f81e00 --stat` touches only
`wayland-backend/src/rs/server_impl/client.rs` (+96) and
`wayland-backend/src/rs/socket.rs` (+10), and the socket change is two
`pub(crate)` helpers (`queued_fds`, `drop_queued_fds`) called only from
the server side. The client path is upstream 0.3.17 unchanged. `wayland-sys`
comes from the fork too, byte-identical. Neither prototype links
`libwayland-client` (the pure-Rust backend is the default). The Nix
`outputHashes` entry `wayland-backend-0.3.17` already covers both
crates, so a scootbg package from the same lock needs no new plumbing.

**Versions line up.** Prototype A resolved `wayland-client` 0.31.15,
`wayland-protocols` 0.32.13, `wayland-protocols-wlr` 0.3.12,
`wayland-scanner` 0.31.11 and `rustix` 1.1.4, all already in the
workspace `Cargo.lock`. No duplicates, nothing new.

## 2. Decoding

Each variant decodes to packed RGB8. That buffer, 70,313 KB for
6000×4000, is what every decoder has to produce, so the useful figure is
the peak above it: `peak − rss_before − output`. 5 runs each:

| File | Decoder | Time, ms (runs → median) | Peak above the output, KB (median) |
|---|---|---|---|
| big.jpg | `image` (zune-jpeg inside) | 237.9, 228.0, 237.4, 225.8, 239.5 → **237.4** | 10656, 10736, 10732, 10796, 10860 → **10,736** |
| big.jpg | `zune-jpeg` direct | 239.5, 243.0, 234.8, 233.6, 228.4 → **234.8** | 1148, 1276, 1332, 1212, 1276 → **1,276** |
| big.jpg | direct set (rebuilt) | 237.2, 245.0, 243.2, 243.9, 234.7 → **243.2** | 1148, 1148, 1152, 1148, 1148 → **1,148** |
| big-orient6.jpg | `image` + `apply_orientation` | 419.5, 450.4, 418.2, 463.6, 435.3 → **435.3** | peak 143,712–143,776 KB total (a second full buffer) |
| big-orient6.jpg | `zune-jpeg`, orientation read only | 282.6, 236.0, 272.0, 229.6, 230.6 → **236.0** | peak 74,080–74,144 KB total; reads orientation 6 |
| big.png | `image` (`png` inside) | 232.3, 257.3, 235.3, 233.6, 232.6 → **233.6** | 656, 712, 580, 656, 568 → **656** |
| big.png | `zune-png` | 583.0, 546.5, 551.2, 550.8, 564.6 → **551.2** | 70552, 70668, 70716, 70716, 70724 → **70,716** |
| big.png | `png` direct | 241.4, 242.8, 233.4, 229.7, 225.1 → **233.4** | 576, 380, 444, 576, 576 → **576** |
| big.webp | `image` (`image-webp` inside) | 1291.5, 1298.5, 1307.6, 1343.0, 1305.9 → **1305.9** | 38440, 38440, 38380, 38312, 38376 → **38,380** |
| big.webp | `image-webp` direct | 1323.1, 1261.7, 1261.2, 1302.4, 1312.7 → **1302.4** | 38360, 38232, 38296, 38244, 38360 → **38,296** |

Binary size over the 565,968-byte base:

| Decoders | Binary | Delta |
|---|---|---|
| `image` with `png`, `jpeg`, `webp` | 1,270,640 | +704,672 |
| `zune-jpeg` + `zune-png` + `image-webp` | 1,102,664 | +536,696 |
| **`zune-jpeg` + `png` + `image-webp`** | 1,131,336 | **+565,368** |

What the numbers say:

- `image` 0.25 already uses `zune-jpeg`, `png` and `image-webp`. Using
  them directly gives the same decoders for 139 KB less, without the
  wrapper's costs. Its JPEG decoder `read_to_end`s the whole file first
  (`codecs/jpeg/decoder.rs`): +9.5 MB of peak here, the file's size.
  Its orientation handling copies the whole image.
- `zune-png` is slower than `png` and holds the whole inflated stream
  beside the output (+70 MB). `png` streams, so it wins on both counts.
- WebP is the same code both ways. The +38 MB is lossy WebP's YUV planes,
  a property of `image-webp`. No lighter pure-Rust WebP decoder was
  looked for; `more-formats.md` is the place if that row ever matters.
- **EXIF orientation:** `zune-jpeg` exposes the raw EXIF (`exif()`) but
  does not interpret it. The prototype's reader handles both byte orders,
  finds tag `0x0112` in IFD0 and bounds-checks every read. It is about 30
  lines and correctly returned 6 for `big-orient6.jpg` and 1 elsewhere.
  `image-webp` exposes EXIF too (`exif_metadata()`), so WebP gets
  orientation from the same reader for free.

## 3. Scaling

`fill` from 6000×4000 to 3840×2160: centre-crop to 6000×3375, then scale.
The decode is not timed. Peak is measured above the RSS with the source
already resident. 5 runs:

| Scaler | Filter | Time, ms (runs → median) | Transient above the source, KB (median) |
|---|---|---|---|
| `image::imageops::resize` | Lanczos3 | 909.9, 892.2, 899.1, 810.0, 845.8 → **892.2** | **226,936** |
| `fast_image_resize` | Lanczos3 | 80.4, 81.1, 87.3, 83.6, 80.1 → **81.1** | **62,980** |
| `image::imageops::resize` | CatmullRom | 868.1, 699.5, 679.6, 698.8, 707.3 → **699.5** | **226,752** |
| `fast_image_resize` | CatmullRom | 71.4, 70.7, 73.5, 73.9, 88.6 → **73.5** | **62,756** |

- `image`'s resize builds an `Rgba32F` intermediate of source width ×
  target height (`vertical_sample` in `imageops/sample.rs`): 6000 × 2160 ×
  16 bytes = 207 MB for an RGB8 image. fir's transient is the 24.3 MB
  output plus a u8 intermediate.
- fir picks its SIMD path at runtime: `Resizer::cpu_extensions()`
  reported `Avx2` here. There is no build-time CPU flag, so one binary
  serves every x86-64 machine. Without the `rayon` feature it is
  single-threaded, as is `image`'s resize.
- Size: fir added 524,304 bytes over the direct decoders (1,655,640 in
  the rebuild used for the typed-API comparison; the first build was
  1,651,544, +520,208), and `image`'s resize 45,072 (1,176,408). The other size levers tried:
  - fir's `only_u8x4` feature made it *larger* (1,803,096).
  - The typed API (`resize_typed::<U8x3>`) came out byte-for-byte the
    same size (1,655,640 against 1,655,640, different hashes).
  - So the half-megabyte is the RGB kernels themselves (scalar, SSE4.1,
    AVX2, AVX-512), not pixel types left alive by dynamic dispatch.
- Compile time, clean release, 3 runs:
  - all decoders + fir + serde_json + CLI: 118.5, 117.1, 116.0 s
    (median **117.1**)
  - the same with `image`'s resize instead of fir: 55.9, 53.9, 56.7 s
    (median **55.9**)
  - all of it through `image` (decode and resize): 66.0, 65.0, 63.8 s
    (median **65.0**)

  Touching `main.rs` and rebuilding release: 93.2, 95.3, 92.1 s with fir
  against 24.3, 24.9, 24.3 s without. Fat LTO re-codegens fir on every
  release link.
- Output quality: the two Lanczos3 results compare at PSNR 39.5 dB
  (ImageMagick `compare -metric PSNR`), mean absolute error 0.83%, and a
  largest single-channel difference of 17/255. That overstates the
  kernels' difference, because the `image` path had to crop at an integer
  row (312) and fir at 312.5, a half-pixel shift on a noise image. The
  outputs were not inspected by eye, and this was not investigated
  further, as the ticket allowed.

**Round one chose `fast_image_resize` (superseded by §3b).** Peak
memory while decoding and scaling a 6000×4000 JPEG, and CPU per set, are
both release-gate rows. With `image`'s resize the full pipeline peaked at
300.6 MB and 1153 ms; with fir it was 136.7 MB and 353 ms (§6a table).
`image`'s resize stays rejected on those numbers. The choice between fir
and a safe scaler is §3b.

## 3b. Round two: safe scalers, and the choice

fir has the most `unsafe` of any image crate in the tree (~650 sites,
§10) and no fuzzing upstream. Two safe or near-safe scalers were measured
against it, each in an isolated build (Wayland base, `zune-jpeg`, one
scaler). The fill crop is on integer rows, because not every candidate
takes a crop rectangle. 5 runs:

| Scaler | `unsafe` | Lanczos3 ms, runs → median | Transient above the source, KB | Binary | Clean / incremental release build, s (3 runs) |
|---|---|---|---|---|---|
| `fast_image_resize` 6.1 | ~650 (§10) | 79.7, 76.7, 81.7, 79.4, 80.2 → **79.7** | 62,980 | 1,290,976 | 56.0, 56.4, 57.2 / 34.6, 35.4, 33.9 → **56.4 / 34.6** |
| **`pic-scale-safe` 0.1.12** | **0, `#![forbid(unsafe_code)]`** | 169.6, 174.7, 177.0, 168.1, 174.3 → **174.3** | **25,004** (the output only) | **836,320** | 36.5, 37.0, 36.1 / 15.0, 15.7, 15.0 → **36.5 / 15.0** |
| `resize` 0.8.9 (+ `rgb`, ~75 `unsafe`) | ~76 | 423.6, 444.9, 408.8, 405.2, 410.6 → **410.6** | 176,272 | 828,144 | — |

Details:

- **Quality.**
  - Against ImageMagick's Lanczos on the same integer crop: fir
    50.9 dB PSNR, `pic-scale-safe` 50.8 dB, `resize` 51.5 dB.
  - `pic-scale-safe` against fir: 60.4 dB. No visible difference is
    expected; the images were not inspected by eye.
- **`pic-scale-safe`'s API.** It takes a whole image and returns a new
  `Vec<u8>`, with no crop rectangle. The pipeline therefore crops in place
  first: rows are a sub-slice, and columns are compacted row by row with
  `copy_within`, where the write index never passes the read index. The
  crop is integer, a ≤ 0.5 px shift against fir's fractional crop. It
  uses fixed-point (`i32`) arithmetic for 8-bit input.
- **Licence:** BSD-3-Clause OR Apache-2.0, MIT-compatible. Its only
  dependency is `num-traits`.
- **Maintenance and fuzzing.** Upstream last committed on 2026-08-26
  (`b50e020`, `awxkee/pic-scale-safe`). It has no fuzz targets and is not
  in OSS-Fuzz.
- **End to end** (under the §6b allocator wrapper, 8 live sets, 3 runs):

  | Sequence, thread | `pic-scale-safe` CPU, ms | fir (two-pass) CPU, ms | `pic-scale-safe` peak | fir peak | heap max, pic / fir |
  |---|---|---|---|---|---|
  | J, main | 2203, 2157, 2103 → **2157** | 1568, 1510, 1519 → **1519** | **131.2 MB** | 145.1 MB | 552 / 432 KB |
  | J, thread | 2137, 2144, 2179 → **2144** | 1560, 1519, 1532 → **1532** | 131.3 MB | 145.0 MB | 648 / 420 KB |
  | W, main | 4571, 4620, 4564 → **4571** | 3998, 3966, 3986 → **3986** | 141.7 MB | 145.0 MB | 420 / 404 KB |
  | W, thread | 4592, 4622, 4712 → **4622** | 4118, 4038, 4047 → **4047** | 141.7 MB | 145.1 MB | 600 / 408 KB |

  Per set: the 6000×4000 JPEG takes 444 ms against 340 ms, and the
  2560×1440 upscale 160–195 ms against ~100 ms. The pipeline binaries
  (Wayland, decoders, scaler, allocator wrapper) are 1,229,656 bytes
  against 1,868,632. Sequences J and W are defined in §6b.

**Decision: `pic-scale-safe`**, the user's call after the measurements.

- **What it wins:** zero `unsafe` meets the "speed and safety, not
  traded off" rule. It also wins peak memory (−14 MB), binary size
  (−639 KB) and build time (−20 s clean, −20 s per incremental release
  build).
- **What it costs:** ~100 ms more CPU per 4K change: +40% on the J
  sequence, +14% on the decode-heavy W sequence. That is paid only on a
  change; idle is unaffected. CPU per set is a release-gate row, so
  lightest.md's competitor run checks it.
- **How a swap stays contained:** the scaler sits behind one function.
  If that row is ever lost, fir (with the two-pass split in §6b) is the
  measured fallback.

## 4. CLI parsing

`scootctl` hand-rolls its parser (`crates/scootctl/src/cli.rs`; its only
dependencies are `scoot-ipc` and `serde_json`). The prototype parsed the
planned grammar four ways:

```
set TARGET [--output NAME] [--mode fill|fit|stretch|center|tile] [--fill COLOR] | query | clear [--output NAME] | daemon
```

All four produced the same `Debug` for
`set ~/p/a.jpg --output DP-1 --mode fit --fill '#101014'`, `clear --output DP-1`
and `query`, and all four rejected `--mode bogus` with exit 2.

| Parser | Binary | Delta over base |
|---|---|---|
| hand-rolled | 578,256 | **+12,288** |
| `pico-args` 0.5 | 582,352 | +16,384 |
| `lexopt` 0.3 | 590,544 | +24,576 |
| `clap` 4.6 (derive, default features) | 864,976 | +299,008 |

**Decision: hand-rolled, like `scootctl`.** It is the lightest, and it
keeps the two clients consistent. `clap` would add 299 KB, 17% of the
combined binary.

## 5. Serialization: control socket and state file

The same flat request/response shapes in every variant, each checked to
decode a `set` request and emit the same two-output `query` reply:

| Socket JSON | Binary | Delta over base |
|---|---|---|
| `serde` + `serde_json` | 639,712 | **+73,744** |
| through `scoot_ipc::encode`/`decode` | 639,712 | +73,744 (identical) |
| `nanoserde` 0.2 (json) | 619,216 | +53,248 |
| `miniserde` 0.1 | 615,120 | +49,152 |

| State file (round-trip checked, including a tab and `%` in a path) | Binary | Delta |
|---|---|---|
| `toml` 1.1 (serde) | 766,672 | +200,704 |
| hand-written line format | 586,448 | **+20,480** |
| serde_json + `toml` | 832,224 | +266,256 |
| serde_json + line format | 652,000 | +86,032 |

**Decisions.**

- **Socket: `serde` + `serde_json`.** `miniserde` would save 25 KB, but
  it supports only unit enum variants, not the internally tagged enums
  `scoot-ipc` uses for versioned replies. Both serde crates are already
  workspace dependencies.
- **State: a hand-written line format.** `toml` costs 180 KB even with
  serde already present, which is exactly what lightest.md's rule
  anticipated. The prototype's format is a `v1 <hash>` header, then one
  line per output with five tab-separated fields, `%`, tab and newline
  percent-escaped, and `-` for no fill.
- **If `config-and-rotation.md`'s `config.toml` ever ships**, scootbg
  will parse TOML anyway, and `toml` for the state file then costs only
  the difference. Revisit the hand format then.
- **Note for restore-state.md:** paths need not be UTF-8. Neither `toml`
  nor this format stores a non-UTF-8 path as it stands; percent-escaping
  raw bytes would.

**Can scootbg reuse `scoot-ipc`'s framing without compositor types? Yes,
but it shouldn't.**

- `scoot-ipc`'s dependencies are `base64`, `serde`, `serde_json` and an
  optional `scoot-core` behind the `core` feature, off by default. Nothing
  Wayland or compositor-side comes in.
- The reusable part is `codec.rs`: `encode`, `decode`, `write_message`
  and `read_message_buffered`, generic over serde, about 25 lines of
  code. It costs nothing in the binary, as the identical 639,712 shows.
- What does not transfer: `socket_path()` (`SCOOT_SOCKET`, `scoot.sock`),
  `Client` (typed to scoot's `Request`/`Response`) and `PROTOCOL_VERSION`.
- So reuse would couple scootbg's build and CI to `crates/scoot-ipc/`
  (the planned split lists it under the scoot jobs) in exchange for 25
  lines. Keep scootbg's protocol self-contained and follow the same
  pattern, reused line buffer included.

## 6. Returning heap to the OS

### 6a. Round one: glibc tuning (superseded by §6b)

This is the evidence for *why* heap return needs a deliberate design. The
fix it proposed (glibc `mallopt` + `malloc_trim` through the `libc`
crate) is **superseded**, because scootbg calls no C.

The pipeline is decode, scale (fir, Lanczos3) and pack into a
3840×2160 XRGB8888 `memfd` mapping (the stand-in `wl_shm` buffer, 32,400 KB
of `RssShmem`), then drop everything else.

**One set in a fresh process** (`big.jpg`, 5 runs; heap figures identical
across runs):

| Allocator, thread | Total ms (median) | Peak KB | Heap after drop | After 300 ms | After `malloc_trim(0)` |
|---|---|---|---|---|---|
| glibc, main thread | 353 (348–372) | 136,724 | 580 | 580 | 220 |
| glibc, decode thread | 377 (354–400) | 136,608 | 568 | 568 | 568 |
| `mimalloc` 0.1.52, main | 447 (325–896) | 184,008 | 147,736 | 147,736 | 147,736 |
| `mimalloc`, decode thread | 316 (303–504) | 186,116 | 149,768 | 149,768 | 149,768 |
| glibc, `image` resize, main | 1153 (1145–1298) | 300,576 | 344 | 344 | 212 |

A fresh process flatters glibc. Every big buffer is `mmap`ed and unmapped
on free, so almost nothing is retained. **A long-lived daemon is
different.** The first large free raises glibc's mmap threshold (up to
32 MiB), and later buffers under it come from the heap and stay there. So
the prototype ran eight live sets in one process, keeping only the latest
shm buffer: `big.jpg`, `small.jpg` (2560×1440), `mid43.jpg` (4000×3000),
`wide.jpg` (5000×2800), `big.png`, `small.jpg`, `mid.jpg` (3000×2000),
`small.jpg`. Heap (`RssAnon`, KB) after each set. Rows marked (3) ran
three times with identical results (±4 KB); rows marked (1) ran once:

| Setting | After sets 1–8 | Trim at the end |
|---|---|---|
| main thread, no trim (3) | 584, 644, 25120, 25120, 25332, 52028, **61588, 61588** | 232 |
| main thread, trim after each set (3) | 220, 220, 220, 220, 232, 232, 232, 232 | 232 |
| decode thread, no trim (3) | 568, 52012, 60260, 60260, 60260, 60260, **79244, 79244** | 60260 |
| decode thread, trim after each set (3) | 568, 52012, 340, 25104, 25200, 52012, **61572, 61572** | 61572 |
| `mmap_threshold` = 1 MiB, decode thread, trim (1) | 900, 712, 804, 804, 328, 712, 712, 712 | 712 |
| `arena_max` = 1, decode thread, trim (1) | 196 in every column | 196 |
| **both, decode thread, trim** (`GLIBC_TUNABLES`) (1) | **192 in every column** | 192 |
| **both through `mallopt()` in-process**, decode thread, trim (3) | **192–196 in every column** | 192–196 |
| both through `mallopt()`, decode thread, no trim (3) | 892, 324, 800, 800, 324, 708, 708, 708 | 196 |

Peak was 169.5–169.9 MB in every row but one (the PNG decode plus fir,
with the previous shm buffer still mapped). The exception is the decode
thread without trim or tunables, at 204.5–204.7 MB, the retained arena on
top. The `GLIBC_TUNABLES` rows also ran once each on the main thread:

- with the 1 MiB threshold, 908–972 KB without trim and 220–228 KB with
  it, the same as the `mallopt()` rows;
- with `arena_max` = 1 alone and no trim, the same 61.6 MB as the
  default, because the arena count is not the main thread's problem.

`mimalloc` held 148 MB after a single set, so it was not put through the
repeated test.

**What round one proposed (superseded):** at startup,
`mallopt(M_MMAP_THRESHOLD, 1 MiB)` and `mallopt(M_ARENA_MAX, 1)`, then
`malloc_trim(0)` after each set, all through `libc`.

Review of PR #266 pointed out that the table above already shows
`M_ARENA_MAX = 1` plus the per-set trim is sufficient on its own: 196 KB
in every column, without the threshold call. That is moot now that §6b
replaces all three calls. `mimalloc` stays rejected: 148 MB retained, a C
build, and +149 KB.

### 6b. Round two: pure Rust

Two sequences of 8 live sets each. Each set decodes, crops for `fill`,
scales to 3840×2160 and fills a new XRGB8888 `memfd` buffer, keeping only
the latest buffer mapped, as a daemon does:

- **J:** big.jpg, small.jpg, mid43.jpg, wide.jpg, big.png, small.jpg,
  mid.jpg, small.jpg (the §6a sequence).
- **W:** big.webp, small.webp, big.jpg, small.webp, mid.jpg, small.webp,
  big.webp, small.webp. Lossy WebP allocates its YUV planes inside
  `image-webp`, out of scootbg's reach.

Each ran on the main thread and with a decode thread per set, 3 runs,
interleaved in one batch. Heap (`RssAnon`) was read after each set; the
tables give the worst across all runs.

**First try: caller-owned mappings for every image buffer.**

- **Setup.** The decoders write into anonymous `rustix` mappings
  (`zune-jpeg` `decode_into`, `png` `next_frame`, `image-webp`
  `read_image`), released with `munmap`.
- **fir needed a split.** Its one-shot resize keeps a source-width ×
  target-height `Vec` inside the resizer (39 MB for 6000×4000 RGB). Two
  single-axis calls into a caller-owned intermediate, vertical then
  horizontal (the order fir uses for u8 itself), write straight to the
  destination:
  - `size_of_internal_buffers()` asserted 0;
  - the output was byte-identical to the one-shot call on `big.jpg`,
    `small.jpg` and `big.png`;
  - it costs +180 KB of binary, because fir's vertical kernel is then
    instantiated for both branches.
- **Result.** Heap (KB) after each set, no tuning, J on the main thread:
  552, 696, 884, 1072, 1072, 704, 720, 720. Over 40 sets it stayed under
  1,052 KB. On W it was 3036, 6820, 6832, 6860, 6860, 6860, 828, 6804: the
  WebP planes still come from the global allocator, where glibc's raised
  mmap threshold parks them. So mappings for our own buffers are not
  enough, because dependencies allocate too.

**Straight into the shm buffer as 4-channel pixels: rejected.** Scaling
RGBA/BGRA straight into the XRGB8888 buffer saves the final packing copy.
But it links fir's U8x4 and alpha code, including a 512 KB
`RECIP_ALPHA16` table, even with `use_alpha(false)` at runtime:

- binary 4,113,240 against 1,835,864 bytes for RGB plus one packing pass;
- the same CPU (J main 1508 against 1551 ms, a tie);
- peak 181.5 against 145.6 MB, because the decoded source is 4 bytes per
  pixel.

zune-jpeg's BGRA output also takes its scalar path; only RGB and RGBA are
vectorised. All three JPEG layouts gave byte-identical output.

**The fix that covers everything: a large-allocation global allocator.**
It is `#[global_allocator]`, ~50 lines:

- `alloc`, `alloc_zeroed`: blocks of 128 KiB or more with alignment
  ≤ one page become their own private anonymous mapping (`rustix`
  `mmap_anonymous`); everything else goes to `System`.
- `dealloc`: `munmap` for the big ones.
- `realloc`: `mremap(MAYMOVE)` when both sizes are big, and allocate,
  copy, free when a block crosses the threshold.

glibc then never sees a large block, so its mmap threshold never rises,
and dependencies' internal buffers (fir's, `image-webp`'s) go back to the
kernel on free like our own. CPU totals are the 8 sets, main / thread,
median of 3 runs. Heap is the worst after any set across all runs.

| Variant | J CPU, ms | J heap max, KB | J peak, MB | W CPU, ms | W heap max, KB | W peak, MB | Binary |
|---|---|---|---|---|---|---|---|
| fir one-shot, glibc default (J from the first batch) | 1474 / 1488 | 61,616 / 79,252 | 169.6 / 204.8 | — | 78,992 (main) | 185.6 (main) | 1,663,832 |
| fir one-shot + `mallopt` + trim (§6a, superseded) | 1533 / 1631 | 236 / 204 | 169.8 | 4091 / 4146 | 244 / 212 | 169.8 | 1,663,832 |
| **fir one-shot + allocator wrapper** | 1539 / 1637 | **456 / 508** | 169.4 | 4085 / 4094 | **416 / 404** | 169.4 | 1,700,696 (+37 KB) |
| fir two-pass into mappings, no tuning | 1551 / 1537 | 1072 / 988 | 145.8 | 4054 / 4087 | 6860 / 6860 | 159.1 | 1,835,864 |
| fir two-pass + wrapper, plain `Vec` scratch | 1616 / 1527 | 428 / 420 | 145.1 | 4043 / 4063 | 404 / 408 | 145.1 | 1,868,632 |
| **`pic-scale-safe` + wrapper (the choice)** | 2157 / 2144 | 552 / 648 | **131.2** | 4571 / 4622 | 420 / 600 | 141.7 | **1,229,656** |

Raw per-run CPU for the chosen and fir rows is in §3b. The same rows
from the first batch agree within the noise.

Findings:

- **Heap.** The wrapper keeps heap under 0.65 MB on both sequences and
  both thread models, with no C call and no tuning. That is ~0.2–0.4 MB
  above `mallopt` + trim, at the same CPU (1539 against 1533 ms, J main).
- **RSS/PSS.** After the last set, every tuned or wrapped variant sits at
  36.3–37.0 MB RSS and 35.4–36.1 MB PSS. That is 32.4 MB of shm buffer
  plus the binary and heap. Untuned fir sits at 97.9 / 97.0 MB (J main)
  and 115.7 / 114.8 MB (J thread).
- **Peak.** The lower peak comes from never holding a source-width ×
  target-height intermediate, and from dropping the source before the
  shm buffer is allocated. It does not come from mapping buffers by hand:
  the plain-`Vec` scratch row matches the mapped one.
  - fir needs the two-pass split to get it (145.1 MB).
  - `pic-scale-safe` already allocates only its output (+25.0 MB), so it
    needs no split and peaks lower still: 131.2 MB.
- **Why 128 KiB.** It is glibc's own default mmap threshold. The wrapper
  pins it so it can no longer rise.
- **What the wrapper costs.** Each large allocation is an `mmap` syscall
  and fresh zeroed pages, which glibc's mmap path already paid.

## 7. Build profile

The workspace profile can only be overridden per package, through
`[profile.release.package.<name>]` in the root manifest. Overriding
`opt-level` for the prototype and every dependency (`"*"`) gave:

| opt-level | Base binary | Combined binary | Pipeline ms (3 runs) |
|---|---|---|---|
| 3 (workspace) | 565,968 | 1,733,480 | 356, 367, 337 |
| `"s"` | 662,776 | 1,717,256 | 380, 377, 387 |
| `"z"` | 684,048 | 4,089,016 | 1218, 1241, 1254 |

`std` stays precompiled at opt-level 3 and inlining decisions shift under
LTO, so the "small" levels barely help and can hurt:

- `"s"` saved 16 KB (0.9%) on the combined binary, grew the base by 17%,
  and ran the pipeline ~7% slower, which is within this machine's noise
  but not in `"s"`'s favour.
- `"z"` grew the base by 21%, more than doubled the combined binary, and
  tripled the pipeline time.

The workspace profile stays as it is. For reference, a hello-world
under it is 324,304 bytes, so the Wayland client itself is ~242 KB.

## 8. Licences

For the final set (Wayland client, direct decoders, `pic-scale-safe`,
the allocator wrapper, serde_json, the line-format state file and the
hand-rolled CLI), from
`cargo tree -e normal --features <the chosen set> -f "{p} | {l}"`:

| Crate | Licence |
|---|---|
| adler2 2.0.1 | 0BSD OR MIT OR Apache-2.0 |
| bitflags 2.13.2 | MIT OR Apache-2.0 |
| byteorder-lite 0.1.0 | Unlicense OR MIT |
| cfg-if 1.0.4 | MIT OR Apache-2.0 |
| crc32fast 1.5.1 | MIT OR Apache-2.0 |
| downcast-rs 1.2.1 | MIT/Apache-2.0 |
| fdeflate 0.3.7 | MIT OR Apache-2.0 |
| flate2 1.1.10 | MIT OR Apache-2.0 |
| image-webp 0.2.4 | MIT OR Apache-2.0 |
| itoa 1.0.18 | MIT OR Apache-2.0 |
| linux-raw-sys 0.12.1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| memchr 2.8.3 | Unlicense OR MIT |
| miniz_oxide 0.8.9 and 0.9.1 | MIT OR Zlib OR Apache-2.0 |
| num-traits 0.2.19 | MIT OR Apache-2.0 |
| **pic-scale-safe 0.1.12** | **BSD-3-Clause OR Apache-2.0** |
| png 0.18.1 | MIT OR Apache-2.0 |
| proc-macro2 1.0.107, quote 1.0.47, syn 3.0.5 | MIT OR Apache-2.0 |
| quick-error 2.0.1 | MIT/Apache-2.0 |
| quick-xml 0.41.0 (via wayland-scanner, build time) | MIT |
| rustix 1.1.4 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| serde, serde_core, serde_derive 1.0.229; serde_json 1.0.151 | MIT OR Apache-2.0 |
| simd-adler32 0.3.10 | MIT |
| smallvec 1.16.1 | MIT OR Apache-2.0 |
| unicode-ident 1.0.24 (proc-macro dep) | (MIT OR Apache-2.0) AND Unicode-3.0 |
| wayland-backend 0.3.17 (scoot-sh fork), wayland-sys 0.31.11 (fork) | MIT |
| wayland-client 0.31.15, wayland-protocols 0.32.13, wayland-protocols-wlr 0.3.12, wayland-scanner 0.31.11 | MIT |
| zmij 1.0.23 | MIT |
| zune-core 0.5.3, zune-jpeg 0.5.15 | MIT OR Apache-2.0 OR Zlib |
| build scripts only: autocfg 1.5.1, cc 1.4.5, find-msvc-tools 0.1.12, pkg-config 0.3.34, shlex 2.0.1 | MIT OR Apache-2.0 |

**All permissive, all MIT-compatible.**

- **`pic-scale-safe`** is BSD-3-Clause OR Apache-2.0. Either option
  obliges a binary distribution to carry its notice, as MIT does for
  scootbg's own. Package notices should list it.
- **Unicode-3.0** is permissive and reaches only a compile-time
  proc-macro.
- There is no GPL, LGPL or MPL anywhere.
- **The one duplicate**, `miniz_oxide` 0.8 and 0.9, is inside `png` 0.18
  (directly, and through `flate2`). Both are already in the workspace
  lock for the compositor's own PNG encoding.
- **Dropped with fir:** `fast_image_resize`, `document-features`, `litrs`
  and `thiserror`. Round one's `libc` entry is gone with §6a.

The crates new to the workspace lock are `pic-scale-safe`, `num-traits`,
`autocfg`, `zune-jpeg`, `zune-core`, `image-webp`, `quick-error` and
`byteorder-lite`. The rejected SCTK tree was also all MIT / Apache-2.0 /
Zlib / Unlicense (34 crates). Nothing was taken from awww/swww, wpaperd or
hyprpaper; none was consulted.

## 9. What C remains

From a clean release build of the chosen set:

| Question | Answer | How it was checked |
|---|---|---|
| Is the `libc` crate compiled? | **No.** It appears only for other targets (`rustix` → `errno` where `linux_raw` is unavailable). | `cargo tree -e normal,build -i libc` prints nothing for the host; `cargo tree --target all -i libc` shows the `errno` path; not in the build's `Compiling` list |
| Which backend does `rustix` use? | `linux_raw`: raw syscalls, no libc. | its build script output: `cargo:rustc-cfg=linux_raw` |
| `-sys` crates | `linux-raw-sys` (pure-Rust constants and structs) and `wayland-sys`, built with no features, which links nothing (its build script output is empty) | `cargo tree`, `target/release/build/*/output` |
| Build scripts compiling C | **None.** `cc` and `pkg-config` are compiled, as unconditional build-dependencies of `wayland-backend` and `wayland-sys`, but never invoked: `wayland-backend/build.rs` calls `cc` only under its `log` feature, and `wayland-sys/build.rs` probes pkg-config only under `client`/`cursor`/`egl`/`server`. All are off. | the two `build.rs` files at the fork rev; no `.o` or `.a` anywhere in the build tree |
| Dynamic libraries | `libc.so.6`, `libm.so.6`, `libgcc_s.so.1` | `ldd` |
| What they are used for | std's own glibc use (malloc/free for small blocks, pthreads, file I/O, env, TLS destructors, `getrandom`); `libm`: `sinf` in the `pic-scale-safe` build (fir's needed `sin`, `sincos`, `exp`), in filter-weight setup, not per pixel; `libgcc_s`: `_Unwind_*` for std's backtraces | `nm -D --undefined-only`: 78 symbols in the `pic-scale-safe` build, none of them `mallopt` or `malloc_trim` |

The C that remains is the Rust standard library's, on
`x86_64-unknown-linux-gnu`.

**musl was not measured**: this toolchain ships no musl std. What it
would change:

- The binary becomes static, with no shared-library closure for the size
  row.
- It still contains C, statically: musl, and the unwinder.
- The allocator wrapper keeps large blocks off musl's malloc as well, so
  the heap results should carry over. That is unmeasured.

## 10. `unsafe` in the tree

The counts are approximate. `cargo-geiger` 0.13.0 failed to compile
here, and `cargo-audit`/`cargo-deny` are not installed, so each crate's
`src/` was grepped for `unsafe {`, `unsafe fn` and `unsafe impl`, with
comments stripped and `tests/`, `benches/`, `examples/` and `fuzz/`
excluded. Where a crate carries code this build does not compile, the
compiled part is given separately.

Fuzzing was checked by cloning each upstream repository (blob-less, head
commits as of 2026-09-26) for fuzz targets and fuzz CI, and against
OSS-Fuzz's `projects/` index. Advisories were checked against a clone of
`rustsec/advisory-db` at `e211151` (2026-09-25), for every crate name in
the tree.

| Crate | ≈ `unsafe` sites | What for | Upstream fuzzing | RustSec |
|---|---|---|---|---|
| zune-jpeg 0.5.15 | 79, 76 of them in SIMD files (IDCT, upsampler, colour convert, AVX2/NEON) | SIMD | `fuzz/` targets and per-format fuzz CI (`etemesi254/zune-image`); also OSS-Fuzz through the `image-rs` project's JPEG fuzzer | none |
| png 0.18.1, fdeflate, miniz_oxide, adler2, image-webp, byteorder-lite | **0** (`#![forbid(unsafe_code)]`) | — | png: OSS-Fuzz (`image-png`) + cifuzz; fdeflate: 8 targets; miniz_oxide: OSS-Fuzz; image-webp: 2 targets + OSS-Fuzz through `image-rs` | none |
| **pic-scale-safe 0.1.12** | **0** (`#![forbid(unsafe_code)]`) | — | **none** (no targets, not in OSS-Fuzz) | none |
| crc32fast / simd-adler32 | 15 / 36 | SIMD | 1 target / 5 targets + fuzz CI | none |
| flate2 1.1.10 | 11 compiled (25 more in the C-zlib backend, not built) | buffer handling | OSS-Fuzz + cifuzz | none |
| wayland-backend 0.3.17 (fork) | 10 in the compiled pure-Rust `rs/` backend (200 in the libwayland FFI `sys/` backend, not built) | fd passing, socket buffers | **none** | none |
| wayland-client, wayland-protocols(-wlr) | 1 / 0 / 0 | — | none | none |
| rustix 1.1.4 | 681 in `backend/linux_raw` (192 of them in `arch/`) + 384 in the public API layer (469 more in the libc backend, not built) | raw syscalls: this is why no C is needed | none in-repo | none |
| linux-raw-sys 0.12.1 | many, all in generated bindings for every architecture | type and constant definitions | — | none |
| memchr / smallvec / serde_json / itoa / zmij | 333 / 70 / 13 / 13 / 70 | SIMD / raw buffers / number formatting | memchr: 8 targets; smallvec: fuzz CI; serde_json: OSS-Fuzz | smallvec: 5 advisories, all fixed by 1.6.1 (we lock 1.16.1) |
| quick-xml 0.41.0 (build time) | — | — | — | RUSTSEC-2026-0194 and -0195, both patched in 0.41.0, the locked version |
| shlex 2.0.1 (build time) | — | — | — | RUSTSEC-2024-0006, patched ≥ 1.3.0 |
| *for comparison:* fast_image_resize 6.1.0 | ~653: 121 AVX2, 121 SSE4.1, 140 NEON, 93 wasm32, 178 generic; 334 raw-pointer operations | SIMD + raw pointers | **none** | none |
| *for comparison:* jpeg-decoder 0.3.2 | 16, SIMD only (`deny(unsafe_code)` otherwise; `platform_independent` forbids it) | SIMD | not checked | none |

**No advisory applies at the locked versions.**

The input-facing decoders are either `forbid(unsafe_code)` (png,
image-webp, and their inflate crates) or SIMD-only and fuzzed
(zune-jpeg). zune-jpeg's alternative, `jpeg-decoder`, decoded in 265 ms
(367 ms fully safe) against 240 ms, with +35.6 MB peak against +1.1 MB,
for 29 KB less binary. zune-jpeg stays.

The weakest spot left is wayland-rs: no fuzzing, though only ~10
`unsafe` sites are compiled. There is no maintained safe alternative.

## 11. scootbg's own `unsafe`: two modules in one small crate

`scootbg` itself is `#![forbid(unsafe_code)]`. All of its `unsafe` lives
in **`crates/scootbg-mem`**: pure Rust, `publish = false`, with
`#![deny(unsafe_op_in_unsafe_fn)]` and clippy's
`undocumented_unsafe_blocks` lint as an error, so every block carries a
`// SAFETY:` comment.

Why a separate crate rather than one `#[allow]`ed module:

- `forbid` cannot be overridden inside the crate that declares it.
- A separate crate also makes the `unsafe` surface one directory a
  reviewer reads end to end.

Why not name it `-sys`: by Cargo convention that suffix means bindings
to a native library, and this crate exists precisely so that there is
none. The name says what it owns, memory.

**a) `alloc.rs`, the large-allocation global allocator (~50 lines).**
Its `unsafe` is the `unsafe impl GlobalAlloc` and, inside it:

- `rustix::mm::mmap_anonymous` in `alloc` and `alloc_zeroed`;
- `munmap` in `dealloc`;
- `mremap(MAYMOVE)` in `realloc`, when both sizes are big;
- `copy_nonoverlapping` when a `realloc` crosses the threshold;
- the delegations to `System`.

Safety argument:

- **Routing is a pure function of the `Layout`**: size ≥ 128 KiB and
  align ≤ the page size. `GlobalAlloc` guarantees that `dealloc` and
  `realloc` receive the layout the block was allocated with, so a block
  always returns to the path that made it. On the mapping path, `(ptr,
  size)` is exactly one whole mapping of ours.
- **Private anonymous mappings alias nothing and are page-aligned**,
  which covers every alignment routed to them. Larger alignments stay
  with `System`.
- **Anonymous pages are zero-filled**, which is what `alloc_zeroed`
  promises.
- **Failure returns null**, `GlobalAlloc`'s out-of-memory signal. Nothing
  panics inside the allocator.
- **Page size:** use `rustix::param::page_size()`, not the prototype's
  hard-coded 4096.

**b) `shm.rs`, the `wl_shm` buffer.** It creates a `memfd`, sizes it,
maps it `MAP_SHARED`, and hands out `&mut [u8]`. Its `unsafe`:

- `rustix::mm::mmap`;
- `slice::from_raw_parts_mut`;
- `munmap` in `Drop`;
- `unsafe impl Send`.

Safety argument:

- `len = stride × height` comes from `checked_mul` and is non-zero.
- The slice borrows `&mut self`, so there is one writer in this process.
- `Drop` unmaps exactly the `(ptr, len)` that `mmap` returned.
- `Send` without `Sync`: a decode thread fills the buffer and hands it
  over.

**Seal the memfd, which the prototype did not do.** Create it with
`MFD_ALLOW_SEALING`, and after `ftruncate` add
`F_SEAL_SHRINK | F_SEAL_GROW | F_SEAL_SEAL` (`rustix::fs::fcntl_add_seals`).
The compositor receives this fd. If anything truncated the file, our next
write would fault with SIGBUS and kill the daemon; with the seals, the
kernel refuses the truncation instead.

State one more caveat plainly: another process (the compositor) maps the
same pages. scootbg only writes them and never reads them back, the same
practice as SCTK and Smithay.

Everything else needs no `unsafe`:

- decoding into a `Vec`;
- cropping and compacting in place (`copy_within`);
- scaling (`pic-scale-safe`);
- packing (and rotating) into the shm slice;
- the socket, the state file and the Wayland client.

## Incidental finding: a 1×1 `wl_shm` buffer upscaled on scoot

The solid-colour fallback (a 1×1 XRGB8888 `wl_shm` buffer, the viewport
scaling it to the output) does **not** render as a flat colour on
`scoot --headless` with the pixman renderer. Setting `PROTO_NO_SPB=1` with
`#c03020` requested, the samples are:

- centre (800,500): `srgba(187,46,31,0.976)`
- corner (10,10): `srgba(48,12,8,0.251)`
- along the middle row: 0.49 alpha at the left edge, 0.98 at the centre

It is a bilinear fade toward transparent at every edge, consistent with
sampling outside a 1-pixel texture with no edge repeat. The single-pixel
buffer path is exact. scootbg on scoot always has
`wp_single_pixel_buffer_manager_v1`, so this reaches scootbg only on a
compositor without it. It is a compositor-side bug for any client that
upscales a tiny `wl_shm` buffer. It is tracked as a compositor item,
[shm-viewport-upscale-edge-fade.md](../../../backlog/core/shm-viewport-upscale-edge-fade.md)
(widened in review to every upscaled surface), and noted in
[solid-colour.md](../solid-colour.md), whose fallback it affects.

## Not measured, and why

- **Competitors.** No competitor was run. That is lightest.md's first
  step, and it needs their builds on the dev VM.
- **The dev VM.** It was not reachable from this environment. Nothing
  here needed `--tty` hardware.
- **`perf stat` wakeups.** `perf` was not available. Idle was measured
  from `/proc/<pid>/status` context-switch deltas and `/proc/<pid>/stat`
  CPU ticks, both flat at zero.
- **musl.** This toolchain ships no musl std (§9).
- **The full binary with `pic-scale-safe`.** Only the pipeline binary
  (1,229,656 bytes) and round one's fir combination (1,733,480) were
  built; the new combination is estimated in the summary.
- **EXIF rotation folded into the pack.** It is designed, not measured.
  It costs no buffer by construction, but its CPU was not timed.
- **`cargo-geiger`, `cargo-audit`, `cargo-deny`.** The first failed to
  compile, and the other two are not installed. The `unsafe` counts are
  grep-based and the advisory check was done by hand against the
  advisory-db (§10).
- **A lighter WebP decoder.** Noted above as the next thing to try if its
  row ever matters.
- **Debug build times.** Only release was timed. Debug skips LTO.
