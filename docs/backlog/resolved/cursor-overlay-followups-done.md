---
title: "Cursor overlay plane follow-ups from #485's review"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-07"
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
   `renderer_pixman`. Fix it at the next fork rebase (see `dev/forks.md`).
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

## Resolution (2026-10-07, PR #492)

Items 1–3 landed with fail-before tests; item 5's nits and item 4's rebase
note are in the same PR; item 6 did not reproduce (below).

- **Item 1** (`tty/scanout.rs`): new `lit_floor` records `next_flip` at
  every lit reset (build, `adopt_surface`, `reactivate`, `use_mode`); new
  pure `lights_lit` gates `frame_submitted` on completed flip >= floor.
  Test `only_a_flip_at_or_past_the_reset_lights_the_crtc` fails on the old
  body (`is_some()`) and passes on the new one. Live `use_mode` repro not
  run (no safe mode trigger on the box); the `reactivate` half was
  exercised by VT cycles, which instead surfaced a pre-existing DRM-master
  EPERM on return (greeter/logind keeps master; fails before any lit logic
  runs) — reported to the coordinator, box restored.
- **Item 2** (`render/cursor_plane.rs`, `reload.rs`, `tty/mod.rs`):
  `back` skips zero-overlap elements before any build (`overlaps`
  helper); `Cursor::rebuild` clears every output's twin cache via
  `Tty::clear_cursor_overlays`. Tests `a_cursor_on_another_output_...`
  (no upload, no cache entry off-output), `overlap_is_any_visible_pixel`,
  `clearing_drops_stale_images_after_a_rebuild` fail/pass as above. Live
  RSS (release, two outputs, 20 alternating `cursor_size` reloads):
  95888 → 96048 kB.
- **Item 3** (`cursor_rides_overlay` + `render.rs` call site): takes a
  window-candidate flag; single-overlay CRTCs yield to a marked window,
  two-overlay behavior unchanged (pinned by the extended existing test).
  Test `a_marked_window_keeps_the_only_overlay` fails/passes as above.
  `gpu-overlay-window-candidates.md:74` corrected to match.
- **Item 4**: no fork change; `dev/forks.md` records the pixman-only
  `ImportDma` for the next rebase (scoot-side fix impossible).
- **Item 5**: Asahi.md 23 px → 22 px, 20 px tall → 2 px; `render.rs`
  twin comment corrected.
- **Item 6**: not reproduced, no new entry. Fixed build, two 210-burst
  `background_color` reload runs: fds 72 → 77 → 75 (noise, not monotonic),
  KMS plane fb refs 3 → 3; the background path shares no code with #485.
- Live regression figure (release `--tty`, fullscreen mpv, pointer
  visible): 13 and 14 jiffies/10 s, inside the 13–15 band; cursor twin on
  overlay plane 45, mpv on the primary.
