---
title: "GPU scanout: real-GPU proof, then cursor + overlay planes"
status: "open"
area: "rendering"
priority: "medium"
blocked: "phase 1 needs the user's Asahi machine — no VM or container can answer it"
---

# GPU scanout: real-GPU proof, then cursor + overlay planes

Maps to the README bullet clause-by-clause (`README.md:57-61`): (a)
`gpu-scanout`-build-only, (b) primary-plane-only, (c) never run on a real
GPU — plus the headless/nested read-back footnote, which is **design, not
TODO** (scanout is tty-only: headless has no CRTC, nested presents bytes
to its host; `render/gles.rs:15-24`, `docs/tty.md:207-213`). The bullet
clears when (1) Asahi numbers exist in the roadmap file, (2) cursor +
overlay planes ride KMS with capture still correct; `scoot-gpu` stays a
deliberate opt-in (link-time libgbm) throughout. Measurement methodology
lives in [gpu-vs-cpu-measured](./gpu-vs-cpu-measured.md) — this entry is
the correctness work beside it.

## Phase 1 — real-GPU proof on Asahi (gates everything else)

Runbook `Asahi.md:371-438` (Test 4). `nix build .#scoot-gpu`
(`flake.nix:257-301`) or `cargo build -p scoot --features gpu-scanout`;
alternate pixman/scanout tiers ≥4 rounds same scene; confirm tier from the
log (`scanout="gpu"`) before trusting any number — `--renderer gles`
without the feature silently keeps pixman. Metrics in order: idle-CPU
jiffies, frame cost under damage, RSS, power. Medians + spread, never
best-of. **Correctness before performance**: if it does not come up,
that is the more valuable result — file it in
`docs/roadmap/06-gpu-pipeline.md` next to `:504-515`.

Known hazard — split render/display: AGX owns the render node,
`apple,dcp` owns the CRTCs (`Asahi.md:24-26`, `docs/tty.md:35-52`), but
current code wraps **one** GBM device for allocator + exporter + EGL
(`tty/scanout.rs:100-105,227-234`, `render/scanout.rs:106-120`
"expressible later … **Untested**"). Phase 1 records whether
single-device `DrmCompositor::new` succeeds on `apple,dcp` or the split
construction (already separable per `06-gpu-pipeline.md:511-515`) is
required. Either outcome defines phase 2; failure does not block its
design.

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

Hardware gate: phase-1 Asahi confirmation + a device with usable
cursor/overlay planes; the virtio-gpu VM cannot validate this.

## What done looks like

Asahi numbers recorded, cursor + overlay on KMS planes with capture
correct, `docs/tty.md:207-216,273-277` and `06-gpu-pipeline.md:504-515`
updated, `gpu-vs-cpu-measured.md` closed or re-scoped, `Asahi.md` Test 4
appended with results. `flake.nix:257-301` `scoot-gpu` unchanged unless
Asahi drivers expose link/packaging issues.
