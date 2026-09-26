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
- Honour EXIF orientation for JPEG (and WebP, which carries EXIF too).
  Apply it after scaling, on the output-sized result: rotating the
  decoded source first copies all of it (measured +60 MB peak and +200 ms
  on 6000×4000). `zune-jpeg` hands over raw EXIF, so the orientation tag
  is read by a small bounds-checked parser
  ([decided](resolved/dependencies-done.md#2-decoding)).
- Fit modes: `fill` (cover, crop centred; the default), `fit` (contain,
  letterbox in `--fill`), `stretch`, `center` (no scaling), `tile`.
- Scale once per output size and scale factor with a good filter
  (Lanczos3 or CatmullRom default; a `--filter` choice). Measured:
  `fast_image_resize` does 6000×4000 → 3840×2160 Lanczos3 in 81 ms with
  +63 MB transient, against `image`'s 892 ms and +227 MB
  ([decided](resolved/dependencies-done.md#3-scaling)).
- The same image on several outputs of different sizes is decoded once and
  scaled once per distinct target size.
- Drop the decoded source once every output needing it is drawn, unless a
  pending hotplug or scale change needs it again (then re-decode rather
  than hold tens of MB forever).

Colour management is out of scope for v1: images are treated as sRGB and
written as 8-bit.
