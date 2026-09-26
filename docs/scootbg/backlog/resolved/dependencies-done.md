---
title: "Choosing dependencies (and checking licences)"
status: "resolved"
area: "scootbg"
priority: "research"
blocked: null
---

# Choosing dependencies (and checking licences) — RESOLVED

Decided by measurement on 2026-09-26, before any scootbg code exists.
Throwaway prototypes were built outside the repository, each with its own
target directory, against the workspace's own release profile and
`Cargo.lock`, and run against `scoot --headless`. They were deleted
afterwards. This file is the record: the decisions first, then what they
change in the plan, then the evidence.

The original ticket asked for the choice to be recorded in the crate's
`Cargo.toml` comments as well. That happens in the
[crate PR](../crate-and-daemon.md), which links here.

## Decisions

| Area | Choice | Main alternative | Deciding numbers |
|---|---|---|---|
| Wayland client | `wayland-client` 0.31 + `wayland-protocols` 0.32 (`client`, `staging`) + `wayland-protocols-wlr` 0.3 (`client`), no toolkit | `smithay-client-toolkit` 0.21 | 566 KB vs 607 KB, clean build 30.4 s vs 36.7 s; idle identical (0 wakeups); every crate already in the workspace lock, while SCTK adds 4 |
| Decoding | `zune-jpeg` 0.5 + `png` 0.18 + `image-webp` 0.2 directly, plus a ~30-line EXIF orientation reader | `image` 0.25 (`png`, `jpeg`, `webp`); `zune-png` | +565 KB vs +705 KB. JPEG peak +1.1 MB over the output buffer vs +10.7 MB. PNG 233 ms, +0.6 MB, vs `zune-png`'s 551 ms, +70 MB |
| Scaling | `fast_image_resize` 6 (default features, no `rayon`), `U8x3` | `image::imageops::resize` | Lanczos3 6000×4000 → 3840×2160: 81 ms vs 892 ms; transient +63 MB vs +227 MB. Costs +520 KB and +61 s clean / +69 s incremental release build |
| CLI | hand-rolled, as `scootctl` does | `lexopt`, `pico-args`, `clap` | +12 KB vs +25 / +16 / +299 KB |
| Socket JSON | `serde` + `serde_json` (workspace deps), scootbg's own framing | `miniserde`, `nanoserde`; reusing `scoot-ipc` | +74 KB vs +49 / +53 KB; `scoot-ipc` framing is byte-identical in size but brings no benefit (see §5) |
| State file | hand-written line format | `toml` 1 | +20 KB vs +201 KB (+180 KB even with serde already in) |
| Heap return | glibc: `mallopt(M_MMAP_THRESHOLD, 1 MiB)` and `mallopt(M_ARENA_MAX, 1)` at startup, `malloc_trim(0)` after each set, through `libc` (already in the lock) | `malloc_trim` alone; `mimalloc` | 8 live sets with a decode thread: 192–196 KB of heap left, vs 61.6 MB with `malloc_trim` alone; `mimalloc` kept 148 MB after one set |
| Build profile | the workspace `[profile.release]` unchanged (opt-level 3, fat LTO) | per-package `opt-level = "s"` / `"z"` | combined binary / ms per set: `3` 1.73 MB / 356; `s` 1.72 MB (−0.9%) / 380, but the base grew 17%; `z` 4.09 MB / 1241 |
| Async runtime | none (unchanged) | — | the prototypes' `blocking_dispatch` loop: 0 context switches, 0 CPU ticks over 60 s |

All of it together (Wayland client, all three decoders, the scaler,
serde_json, a state file, the CLI) is a **1,733,480-byte** stripped binary.
It links only `libc`, `libm` (glibc) and `libgcc_s` (197 KB, part of every
Rust binary on `*-linux-gnu`), and it idles at 3.6 MB RSS, 188 KB of it
heap.

Every crate in the chosen tree is MIT-compatible (§8). Nothing comes from
awww/swww, wpaperd or hyprpaper.

## What this changes in the plan

- **[lightest.md](../lightest.md): `malloc_trim` alone is not enough with
  a decode thread.** A thread's glibc arena kept 61.6 MB after eight live
  sets even with `malloc_trim(0)` after each one. glibc's dynamic mmap
  threshold also means a *main-thread* daemon without trim keeps up to
  61.6 MB after a few sets, although a fresh process shows almost nothing
  retained. The fix is two `mallopt` calls at startup (§6).
- **[images-decode-and-fit.md](../images-decode-and-fit.md): apply EXIF
  orientation after scaling, not before.** Rotating the decoded source
  the way `image` does (`apply_orientation`) copies all 72 MB and raised
  peak memory from 83.6 MB to 143.8 MB and decode time from 237 ms to
  435 ms. Scaling the unrotated source to the rotated target (swap the
  target's width and height, rotate the crop) and rotating the 25 MB
  result costs one output-sized buffer instead.
- **[crate-and-daemon.md](../crate-and-daemon.md): do not reuse
  `scoot-ipc`.** It pulls no compositor types (`scoot-core` sits behind
  its off-by-default `core` feature), but all it would share is four
  generic serde functions, about 25 lines. Its socket path, `Client` and
  `PROTOCOL_VERSION` are scoot's own. Depending on it would put
  `crates/scoot-ipc/` on scootbg's CI path for no bytes saved (§5).
- **Weigh the release binary with `cargo build --release -p scootbg`.** A
  `--workspace` build unifies features on shared crates
  (`wayland-protocols` gets `server`, for instance). LTO should drop what
  is unused, but the gate's number should come from the package build,
  which is also what a Nix package runs.
- **`fast_image_resize` is the one heavy dependency.** It is 30% of the
  combined binary and 61 s of a 117 s clean release build. Its fat-LTO
  codegen also recurs on every release rebuild: touching one file costs
  93 s with it and 24 s without. It stays because two gated rows, peak
  memory and CPU per set, need it (§3). If the size row is ever the one
  scootbg loses, the fallback to measure is a hand-written separable u8
  convolution for RGB only. It was not built or measured here.

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
- Size: fir added 520,208 bytes over the direct decoders (1,651,544) and
  `image`'s resize 45,072 (1,176,408). The other size levers tried:
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

**Decision: `fast_image_resize`.** Peak memory while decoding and scaling
a 6000×4000 JPEG, and CPU per set, are both release-gate rows. With
`image`'s resize the full pipeline peaks at **300.6 MB and 1153 ms**;
with fir it is **136.7 MB and 353 ms** (§6 table). Those rows decide it,
and the size and build-time cost is paid knowingly (see "What this
changes").

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

**Decision:**

```rust
// At daemon startup (glibc only: cfg(target_env = "gnu")). Each returns 1 on success.
libc::mallopt(libc::M_MMAP_THRESHOLD, 1 << 20); // fixed: also stops the dynamic raise
libc::mallopt(libc::M_ARENA_MAX, 1);            // the decode thread shares the main arena
// After each set, once the source and scratch buffers are dropped:
libc::malloc_trim(0);
```

- `libc` 0.2.189 is already in the workspace lock.
- Setting the threshold explicitly is what turns glibc's dynamic
  adjustment off (mallopt(3)). A single arena costs nothing here: only
  one decode thread ever runs, and it does not contend with the idle loop.
- `mimalloc` is out: 148 MB retained, +149 KB of binary, and a C build.
- musl was not measured. Its allocator returns memory differently, so
  the calls are cfg'd to glibc and a musl build would need its own
  measurement.

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

`cargo tree -e normal --features <the chosen set> -f "{p} | {l}"`:

| Crate | Licence |
|---|---|
| adler2 0.2.1 | 0BSD OR MIT OR Apache-2.0 |
| bitflags 2.13.2 | MIT OR Apache-2.0 |
| byteorder-lite 0.1.0 | Unlicense OR MIT |
| cfg-if 1.0.4 | MIT OR Apache-2.0 |
| crc32fast 1.5.1 | MIT OR Apache-2.0 |
| document-features 0.2.12 (proc-macro) | MIT OR Apache-2.0 |
| downcast-rs 1.2.1 | MIT/Apache-2.0 |
| fast_image_resize 6.1.0 | MIT OR Apache-2.0 |
| fdeflate 0.3.7 | MIT OR Apache-2.0 |
| flate2 1.1.10 | MIT OR Apache-2.0 |
| image-webp 0.2.4 | MIT OR Apache-2.0 |
| itoa 1.0.18 | MIT OR Apache-2.0 |
| linux-raw-sys 0.12.1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| litrs 1.0.0 | MIT OR Apache-2.0 |
| memchr 2.8.3 | Unlicense OR MIT |
| miniz_oxide 0.8.9 and 0.9.1 | MIT OR Zlib OR Apache-2.0 |
| num-traits 0.2.19 | MIT OR Apache-2.0 |
| png 0.18.1 | MIT OR Apache-2.0 |
| proc-macro2 1.0.107, quote 1.0.47, syn 3.0.5 | MIT OR Apache-2.0 |
| quick-error 2.0.1 | MIT/Apache-2.0 |
| quick-xml 0.41.0 (via wayland-scanner, build time) | MIT |
| rustix 1.1.4 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| serde, serde_core, serde_derive 1.0.229; serde_json 1.0.151 | MIT OR Apache-2.0 |
| simd-adler32 0.3.10 | MIT |
| smallvec 1.16.1 | MIT OR Apache-2.0 |
| thiserror, thiserror-impl 2.0.20 | MIT OR Apache-2.0 |
| unicode-ident 1.0.24 (proc-macro dep) | (MIT OR Apache-2.0) AND Unicode-3.0 |
| wayland-backend 0.3.17 (scoot-sh fork), wayland-sys 0.31.11 (fork) | MIT |
| wayland-client 0.31.15, wayland-protocols 0.32.13, wayland-protocols-wlr 0.3.12, wayland-scanner 0.31.11 | MIT |
| zmij 1.0.23 | MIT |
| zune-core 0.5.3, zune-jpeg 0.5.15 | MIT OR Apache-2.0 OR Zlib |
| build scripts only: autocfg 1.5.1, cc 1.4.5, find-msvc-tools 0.1.12, pkg-config 0.3.34, shlex 2.0.1 | MIT OR Apache-2.0 |
| `libc` 0.2.189 (for §6) | MIT OR Apache-2.0 |

**All permissive, all MIT-compatible.** Unicode-3.0 is permissive and
reaches only a compile-time proc-macro. There is no GPL, LGPL or MPL
anywhere. The one duplicate, `miniz_oxide` 0.8 and 0.9, is inside `png`
0.18 (directly, and through `flate2`) and already in the workspace lock
for the compositor's own PNG encoding.

The crates new to the workspace lock are `fast_image_resize`,
`document-features`, `litrs`, `num-traits`, `autocfg`, `zune-jpeg`,
`zune-core`, `image-webp`, `quick-error` and `byteorder-lite`. The
rejected SCTK tree was also all MIT / Apache-2.0 / Zlib / Unlicense (34
crates). Nothing was taken from awww/swww, wpaperd or hyprpaper; none was
consulted.

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
upscales a tiny `wl_shm` buffer, and was reported to the coordinating
session rather than fixed here. It is noted in
[solid-colour.md](../solid-colour.md), whose fallback it affects.

## Not measured, and why

- **Competitors.** No competitor was run. That is lightest.md's first
  step, and it needs their builds on the dev VM.
- **The dev VM.** It was not reachable from this environment. Nothing
  here needed `--tty` hardware.
- **`perf stat` wakeups.** `perf` was not available. Idle was measured
  from `/proc/<pid>/status` context-switch deltas and `/proc/<pid>/stat`
  CPU ticks, both flat at zero.
- **musl's allocator**, a hand-written scaler, and a lighter WebP decoder,
  each noted above as the next thing to try if its row ever matters.
- **Debug build times.** Only release was timed. Debug skips LTO, so
  fir's recurring 69 s should not apply to the development loop.
