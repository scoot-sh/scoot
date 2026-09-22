---
title: "GPU scanout: cursor + overlay planes (phase 1 landed — the real-GPU proof is in)"
status: "open"
area: "rendering"
priority: "medium"
blocked: null
---

# GPU scanout: cursor + overlay planes (phase 1 landed — the real-GPU proof is in)

Maps to the README bullet clause-by-clause (`README.md:57-65`): (a)
`gpu-scanout`-build-only, (b) primary-plane-only — plus the headless/nested
read-back footnote, which is **design, not TODO** (scanout is tty-only:
headless has no CRTC, nested presents bytes to its host;
`render/gles.rs:15-24`, `docs/tty.md:207-213`). A third clause, "never run
on a real GPU", was the reason this entry had a phase 1 at all; the
2026-09-21 Asahi run retired it and the README no longer carries it. The
bullet now clears on one thing alone: cursor + overlay planes riding KMS
with capture still correct. `scoot-gpu` stays a deliberate opt-in
(link-time libgbm) throughout. Measurement methodology and the numbers live
in [gpu-vs-cpu-measured](../resolved/gpu-vs-cpu-measured-done.md) — this
entry is the correctness work beside it.

## Phase 1 — real-GPU proof on Asahi — **DONE 2026-09-21**

Ran on the user's Apple M2 (`apple,t8112`) under Asahi Linux, `eDP-1` at
`2560x1600@60`. Evidence: `Asahi.md` Test 4's results section and
`docs/roadmap/06-gpu-pipeline.md`'s "Evidence (Apple M2 / AGX under Asahi
Linux, 2026-09-21)"; re-runnable as `scripts/asahi-test4.sh`.

- **It comes up**: `drm: driving this device path=/dev/dri/card2
  connector=eDP-1 width=2560 height=1600 scanout="gpu"`, mode set atomically
  on `crtc::Handle(45)`/`plane::Handle(35)`.
- **The known hazard was not one.** Single-device `DrmCompositor::new`
  succeeds on `apple,dcp` even though AGX owns the render node: **one GBM
  device serving allocator + exporter + EGL is enough**, so the split
  construction reserved at `06-gpu-pipeline.md` is *not* required and
  `render/scanout.rs`'s "expressible later … **Untested**" note needs no
  follow-up. That is the cheapest of the two outcomes this phase was written
  to distinguish.
- **The frames are right**: pinned-scene captures identical to the dumb tier
  across all 4.096M pixels except an 18×34 box at the cursor (max channel
  delta 3/255); same tier across rounds is `AE = 0`.
- **Performance**: 4.2–5.1x less compositor CPU under damage, ~0.2 W less
  power, +7–16 MB RSS, zero idle CPU on both tiers. Numbers, spreads and
  caveats in the roadmap file.

VM baseline for comparison (`06-gpu-pipeline.md:406-502`): KMS plumbing
proven on virtio-gpu; scanout ~1.5x dumb-tier CPU there vs 17–32x for
offscreen GLES — read-back was the dominant cost.

## Phase 2 — overlay + cursor planes (after phase 1 green)

Current locks (`tty/scanout.rs:218-247,539-560`): `planes: Some(primary
only)`, `gbm: None` (cursor plane disabled), `FrameFlags::empty()` (not
`ALLOW_SCANOUT`), `COLOR_FORMATS = [Argb8888, Xrgb8888]`. Order:
**cursor → overlay → direct scanout**, because each widens the capture
contract.

1. **Cursor plane first.** `gbm: Some(...)`, real `cursor_size` for the
   `(64,64)` placeholder (`:242-244`), populate `Planes.cursor`.
   Smallest KMS delta; capture (`render/scanout.rs:21-47`) unaffected.
2. **Overlay planes.** Populate `Planes.overlay` from
   `surface.planes()`; per-CRTC enumeration + fallback where a CRTC lacks
   usable overlays.
3. **`ALLOW_SCANOUT` last.** Deferred at `tty/scanout.rs:207-215` and
   `06-gpu-pipeline.md:343-349` for a reason: the frame may then *not* be
   in the swapchain buffer, so `note_frame`/`frame` capture
   (`render/scanout.rs:92-103,228-293`) silently returns the wrong buffer.
   Lands with a capture fix in the same change, never a flag flip.

Hardware gate: phase 1 is now confirmed, so what remains is a device with
usable cursor/overlay planes; the virtio-gpu VM cannot validate this. The
same Asahi M2 is the candidate -- whether `apple,dcp` exposes usable cursor
and overlay planes at all is the first thing phase 2 has to establish, and
`scripts/asahi-test4.sh` is the harness to extend for it.


## What done looks like

Done for phase 1 (2026-09-21): Asahi numbers recorded in
`06-gpu-pipeline.md`, `Asahi.md` Test 4 appended with results,
`gpu-vs-cpu-measured.md` resolved to
`../resolved/gpu-vs-cpu-measured-done.md`, `docs/tty.md`, `docs/nix.md`,
`README.md` and `ROADMAP.md` no longer say "never run on a real GPU", and
`scoot-gpu` needed no packaging change (the Asahi drivers exposed none).

Still open, and all that is left here: cursor + overlay planes on KMS with
capture still correct, and the README bullet losing its
primary-plane-only clause.
