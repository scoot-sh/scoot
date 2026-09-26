---
title: "Decoding images and fitting them to an output"
status: "open"
area: "scootbg"
priority: "high"
blocked: null
---

# Decoding images and fitting them to an output

`scootbg set <path> [--output NAME] [--mode fill|fit|stretch|center|tile]
[--fill COLOUR] [--filter ...]`.

- Formats for v1: PNG, JPEG, WebP (still). Others behind cargo features
  later (see `more-formats.md`).
- Decode on a worker thread, never on the Wayland event loop, so a slow
  or huge file cannot stall frame handling for the other outputs.
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

Colour management is out of scope for v1: images are treated as sRGB and
written as 8-bit.
