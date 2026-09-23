---
title: "GPU scanout: widen the framebuffer exporter so client buffers can go direct"
status: "open"
area: "core"
priority: "high"
blocked: null
---

# GPU scanout: widen the framebuffer exporter so client buffers can go direct

Filed 2026-09-22 (coordinator, GPU-tier survey). Serves **daily-drive**
(zero-copy fullscreen video/games on a real GPU) and is the precondition for
[scanout candidates](./gpu-scanout-candidates.md).

## What is missing

`ALLOW_SCANOUT` has been passed since PR #218, and the capture fix that makes
it safe landed with it. But the flag is inert: `tty/scanout.rs`'s `build`
constructs `GbmFramebufferExporter::new(.., NodeFilter::None)`, which rejects
every client dmabuf in `can_add_framebuffer` before any hardware is touched,
and shm/solid elements never produce an exportable buffer. So no frame this
tree produces can take the primary plane directly. `README.md` ("No window
leaves the primary plane yet ... the framebuffer exporter admits no client
buffers") and `docs/tty.md` say so.

## What to do

1. Widen the exporter to admit client dmabufs allocated on the device that
   will scan them out — `NodeFilter` naming the scanout device's own node
   (and, on the Asahi split topology, whatever node the renderer's
   `main_device` advertises to clients — verify what `NodeFilter` variants the
   pinned Smithay rev has and what each compares against, in
   `src/backend/drm/exporter/gbm.rs`, before choosing).
2. **Carry the live force-path verification** the resolved ticket recorded as
   owed (`docs/backlog/resolved/gpu-scanout-planes-done.md`, caveat 1): the
   `ensure_scanout_capture_current` force and the loud refusal have never
   fired. Widening the exporter is exactly what makes them reachable, so this
   change must show them firing — a harness/unit pin at minimum, and live on
   hardware that actually goes direct if any is reachable.
3. Consider caveat 2 while there: `capture_pixels_for` renders twice on the
   stale path; once direct exists, an early return may be worth it (measure).

## Hardware reality

On the dev VM's virtio-gpu (llvmpipe, no render node for clients to allocate
on) it is not obvious anything can produce a client dmabuf that the exporter
would accept — see `CLAUDE.md`'s PR #130 note on `udmabuf` provenance. Go
and measure before either claiming live proof or declaring it unreachable. If
direct scanout cannot be exercised on the VM, the change still has to prove
(by pin + pinned-source trace) that a direct frame cannot produce a stale
capture, and the README must say where it has and has not been seen go
direct. Asahi runs are user-driven (`Asahi.md`); if a re-run is wanted, add
it to that runbook rather than claiming it.

## Out of scope

Marking window elements `ScanoutCandidate` (next ticket). Overlay-plane
assignment of windows. `ALLOW_PRIMARY_PLANE_SCANOUT_ANY`.
