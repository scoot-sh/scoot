---
title: "Explicit sync: linux-drm-syncobj-v1 on the GPU scanout tier"
status: "open"
area: "protocols"
priority: "medium"
blocked: null
---

# Explicit sync (`wp_linux_drm_syncobj_manager_v1`)

Filed 2026-09-22 (coordinator, GPU-tier survey). Serves **daily-drive** on
real GPUs: NVIDIA's driver and current Mesa Vulkan WSI prefer (NVIDIA
effectively requires) explicit sync; without it GPU clients rely on implicit
fencing, which some drivers do not provide.

`rg -i syncobj crates/` finds nothing. The pinned Smithay rev carries the
protocol (`src/wayland/drm_syncobj/`) and the DRM compositor's support for
acquire/release points (`src/backend/drm/compositor/mod.rs`,
`src/backend/renderer/utils/wayland.rs`) — verify the exact API there.

## What to do

- Advertise the global only where it can be honoured: on the `--tty`
  GPU-scanout tier, when the DRM device supports syncobj timelines with
  eventfd (Smithay has a probe for this — find it at the pinned rev). Never on
  pixman / headless / nested, and never on a device that fails the probe — a
  client that binds it and gets unhonoured points is worse off than one that
  never saw it.
- Wait on acquire points before a buffer is used (Smithay's blocker
  pattern), signal release points when the buffer is done — including on the
  capture path and on a frame that goes direct.
- Protocol errors for malformed requests are Smithay's; check what
  happens to a surface's pending points when the session pauses (VT switch)
  and on client disconnect mid-wait — no hang, no leaked eventfd sources.

## Evidence

Hardware-dependent: virtio-gpu may not expose timeline syncobj. Probe on the
dev VM first and report; if unsupported, the global must simply not appear
there (verify that live) and the positive path needs a unit/harness pin plus
an `Asahi.md` runbook entry rather than a claim.
