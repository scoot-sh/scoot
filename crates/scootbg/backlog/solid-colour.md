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
- scoot's `docs/tty.md` notes that a single-pixel solid wallpaper counts as
  the background for its direct-scanout checks. Keep that true: verify on
  scoot's `--tty` GPU tier that a fullscreen opaque window still scans out
  directly with a solid-colour scootbg underneath.

This is also the cheapest way to prove the daemon, the socket and output
handling end to end before any image code exists.
