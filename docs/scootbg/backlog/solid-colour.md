---
title: "Solid colours through single-pixel buffers"
status: "open"
area: "scootbg"
priority: "high"
blocked: null
---

# Solid colours through single-pixel buffers

`scootbg set '#rrggbb'` sets a whole output to one colour.

- With `wp_single_pixel_buffer_manager_v1` and `wp_viewporter`: one
  single-pixel buffer, viewport destination set to the surface size. No
  shared memory at all.
- Without them: a 1×1 `wl_shm` buffer and `wp_viewporter`, or, with no
  viewporter either, a full-size buffer filled once.
- Opaque region set to the whole surface.
- scoot's direct-scanout path (`docs/tty.md`) treats a covering, fully
  opaque, **black** single-pixel wallpaper as the background, so a
  fullscreen window with an *alpha* format still scans out directly over
  it (`a_black_single_pixel_wallpaper_hands_the_primary_to_the_window_above_it`).
  An opaque-format window scans out over any wallpaper. Only the
  single-pixel path keeps the alpha case: the `wl_shm` fallback or a
  full-size buffer blocks it. Verify on the `--tty` GPU tier with a black
  `scootbg set '#000000'` and an alpha-format fullscreen client.

This is also the cheapest way to prove the daemon, the socket and output
handling end to end before any image code exists.
