---
title: "Cursor overlay plane follow-ups from #485's review"
status: "open"
area: "core"
priority: "medium"
blocked: null
---

# Cursor overlay plane follow-ups from #485's review

Filed 2026-10-06 after #485 merged; the review came back CLEAN. Serves
**daily-drive** (battery and memory on Asahi). Nothing here can crash,
hang or lose work. Wherever one of these bites, the cursor is composited,
as it was before #485. The full review is in the #485 thread.

## Items

1. **Stale flip after a mode change sets the lit gate.**
   - Where: `tty/scanout.rs`, set at `frame_submitted` (:1069) and cleared in
     `use_mode` (:1222).
   - `use_mode` drains no pending flip, so the old flip's vblank can set the
     gate again before the modeset frame. DCP then fails the overlay test and
     Smithay caches the failure. The cursor stays composited, and a
     fullscreen window loses direct scanout until the pointer moves.
   - Fix: record the flip number at each reset, and set the gate only for a
     completed flip at or past it.
   - Established from the source only; reproduce with a DP-1 mode change
     during fullscreen mpv.
2. **Off-output cursor twins.**
   - Where: `render/cursor_plane.rs:202` builds `image()` before the `fits()`
     check at :221.
   - Every output's frame allocates, uploads and caches a dma-buf twin for a
     cursor that isn't on it.
   - The cache is bounded by count (64 per output), not bytes. At a 256 px
     cursor that is about 768 KiB per entry, so up to about 144 MiB idle
     across 3 monitors.
   - Fix: skip off-output elements, or call `fits` first. Clear the cache when
     `Cursor::rebuild` replaces the image set.
3. **One-overlay CRTCs.** The cursor takes the only overlay ahead of a window
   candidate, so a tiled video that rode the overlay before now composites.
   The M2's current kernel has two overlays per CRTC, so it isn't affected.
   Gate on `overlay_planes >= 2`, or on no window candidate being marked.
   Correct `gpu-overlay-window-candidates.md:74` to match.
4. **The fork's unused import without pixman.** In fork
   `drm/compositor/mod.rs:162`, `ImportDma` is used only under
   `renderer_pixman`. Fix it at the next fork rebase (see `docs/forks.md`).
5. **Doc nits.**
   - `Asahi.md:2997` and `:3000`, the edge table: 23 px should be 22 px, and
     "20 px tall" should be 2 px (hotspot (3,1)).
   - The `render.rs` comment above `planes.back(...)`: the twin is padded
     (32x32 minimum) and does not have the cursor's own geometry.
6. **Lead, not attributed to #485.** On the GLES tier, 210
   `background_color` reloads grew scoot's fd count by 6 and the KMS
   framebuffers from 7 to 9. Compare against a build before #485, then file
   on its own if it's real.

## Evidence that each item is fixed

- A test or a live repro that fails before the fix, for items 1–3.
- For item 2, scoot's RSS before and after a burst of `cursor_size` reloads
  on two outputs.
