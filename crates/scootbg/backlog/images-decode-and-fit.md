---
title: "Decoding images and fitting them to an output"
status: "open"
area: "scootbg"
priority: "high"
blocked: "dependency choices in dependencies.md come first"
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
- Honour EXIF orientation for JPEG.
- Fit modes: `fill` (cover, crop centred; the default), `fit` (contain,
  letterbox in `--fill`), `stretch`, `center` (no scaling), `tile`.
- Scale once per output size and scale factor with a good filter
  (Lanczos3 or CatmullRom default; a `--filter` choice). Measure the scaling
  library against the alternatives on a 4K target before picking one.
- The same image on several outputs of different sizes is decoded once and
  scaled once per distinct target size.
- Drop the decoded source once every output needing it is drawn, unless a
  pending hotplug or scale change needs it again (then re-decode rather
  than hold tens of MB forever).

Colour management is out of scope for v1: images are treated as sRGB and
written as 8-bit.
