---
title: "Explicit sync: linux-drm-syncobj-v1 on the GPU scanout tier — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Explicit sync (`wp_linux_drm_syncobj_manager_v1`) — RESOLVED

RESOLVED 2026-09-23 (PR #PRNUM, branch `linux-drm-syncobj`). On the GPU
scanout tier, where a DRM device passes Smithay's syncobj-eventfd probe,
scoot offers `wp_linux_drm_syncobj_manager_v1` v1, waits on acquire points
before a commit applies and signals release points only once it is done
with each buffer. Everywhere else the global does not exist. One thing is
left open, because it cannot be fixed on the scoot side at the pinned rev:
[Smithay leaks a syncobj handle per timeline import](../protocols/syncobj-handle-leak.md).

## What landed

- **Where the global exists** (`drm_syncobj.rs`, `tty::init`): only when the
  session came up on the GPU scanout tier. The display device is probed
  first; if its driver cannot import timelines, `/dev/dri/renderD128` is
  tried next (a split render/display machine like Apple Silicon, where only
  the GPU's driver is expected to have syncobjs; a syncobj works on any
  DRM device that supports them). The first device that passes is the
  import device. Never on pixman, headless or nested, and never where no
  device passes.
- **Acquire points** (`drm_syncobj/acquire.rs`): a pre-commit hook,
  installed per surface only while the global exists. An already-signalled
  point costs one query ioctl and nothing else. An unsignalled one gets
  Smithay's eventfd blocker as a calloop source. Only commits Smithay will
  accept are waited on (a new dma-buf, both points, ordered on a shared
  timeline); a malformed one gets Smithay's protocol error and no eventfd.
  Beyond anvil's pattern:
  - a surface destroyed mid-wait, including every surface of a client that
    disconnects, removes its sources at once and releases its blockers
    (`AcquireBlocker`), so a point that never signals leaks neither an
    eventfd nor a dead transaction in the client's queue;
  - outstanding waits are capped per client (64, and 16 while the fd table
    is pressured). Past the cap the client is disconnected with
    `wl_display.error` `no_memory`, posted on the display object. A bare
    `Client::kill` sends no error event at all; the test that asserts the
    error text caught that.
- **Review-driven change (before the PR):** the release-early paths that
  cannot show the frame (a refused queue, a session pause, a reactivation,
  a CRTC rebuild) release without waiting on the render fence. A GPU hang
  cannot then block the pause and activate handlers, which are 05b's
  recovery path. The errored completion (the frame just shown) and the
  in-flight cap still wait. The wire tests share `cargo test`'s process
  with suites that count dma-buf mappings and read the fd table, so they
  hold both of those suites' locks.
- **Release points.** Smithay signals a release point when the last
  reference to the buffer drops. On the composited path that happens when
  the surface replaces its buffer, while a queued frame may still be
  sampling it on the GPU: scoot passes the render fence to KMS as
  `IN_FENCE_FD` and does not wait on it. The scanout presenter now holds a
  clone of every explicit buffer a composited frame used
  (`drm_syncobj/release_hold.rs`). The clone is kept until that frame's
  flip completes, after waiting on its render `SyncPoint`, which is free
  wherever flips are fenced. It is released early, without waiting, when
  the frame will never be shown: a refused queue, a session pause, a
  reactivation or a CRTC rebuild. An errored completion, where the frame
  that just flipped is on screen but its number is lost, releases
  everything after the wait. At most two frames are held, a pending one
  and a queued one; a third waits out the oldest. Explicit buffers are
  told apart by `wl_buffer`, which is all a render element exposes
  (`ExplicitBuffers`, pruned on buffer destroy). Buffers without sync
  points keep their existing release timing. A primary-direct buffer is
  already held by `DrmCompositor` until the frame after it is on screen,
  so its release follows scanout. Captures read the swapchain slot or read
  their pixels back synchronously, so they need nothing extra.
- **Live timelines are capped per client** at 128 (32 under fd pressure),
  claimed in `dispatch.rs` before delegation and released by the
  destruction hook. Each timeline keeps the client's fd open in scoot. Past
  the cap the import is refused with `invalid_timeline`. `fd_pressure.rs`'s
  per-connection arithmetic now includes both new bounds: one connection
  at every cap holds about 833 fds.
- **Documented, not patched:** re-committing the same `wl_buffer` keeps the
  first commit's `Buffer`, and its points, in Smithay
  (`RendererSurfaceState::update_buffer`). So a buffer switched from
  explicit to implicit sync without an intervening attach is classified
  implicit while still carrying the old release point.

## Evidence (summary; commands, SHAs and raw paths are in the PR)

- **Probe, dev VM** (virtio-gpu, kernel 6.18): `DRM_CAP_SYNCOBJ` = 1 and
  `DRM_CAP_SYNCOBJ_TIMELINE` = 1 on `card0` and `renderD128`. Smithay's probe
  answers `ENOENT`, meaning supported, and a timeline signal reached an
  eventfd. The global is offered on the display device.
- **Harness** (`drm_syncobj/tests.rs`, over the wire against a real render
  node, udmabuf buffers and real syncobj ioctls, 19 tests): absent unless
  enabled, the device ladder, a held commit applying on signal, the
  already-signalled fast path creating nothing, per-surface ordering, a
  never-signalled point stalling only its own surface, destroy and
  disconnect mid-wait removing the calloop sources (`LoopHandle::update`
  refuses the tokens), both bounds killing only the offender with the
  documented errors, Smithay's malformed-commit error with no wait, release
  on replacement and on surface destroy, and the explicit set's pruning.
  `release_hold/tests.rs` pins the hold (10 tests). Full suite: 1566 passed
  under `gpu-scanout` nextest before the ladder commit; the final gate is
  in the PR.
- **Live, dev VM `--tty --renderer gles`**, with a test client rendering
  into card0 dumb buffers and signalling its own timelines in place of a
  GPU:
  - The global shows on the GPU tier only (`wayland-info`, plus the client's
    registry). It is absent on dumb `--tty`, headless pixman and headless
    gles.
  - A held commit: the old shade stayed on screen until the signal, the new
    one appeared after it, and the old buffer's release came 0.14 ms after
    the signal. The same held across a VT switch, with the signal and the
    release both happening while switched away.
  - A never-signalling client beside a normal one: the normal client's
    frame-callback mean was 21.5 ms, against 21.7 ms alone. IPC latency was
    unchanged. SIGKILLing the stalled client returned the compositor's
    eventfds and syncobj fds to baseline.
  - Floods: disconnected after the 65th blocked commit (`wl_display` error
    2) and after the 129th import (`invalid_timeline`). fds were back to
    baseline afterwards, and a normal client that followed was unaffected.
  - A continuously drawing explicit client, three buffers, each release
    waited on before reuse: 0 reuse timeouts tiled and fullscreen. Every
    fullscreen frame went primary-direct, with `zero_copy` 390/390. A
    buffer replaced while on the primary plane was released about 20 ms
    later, when its replacement's flip completed, not at the replacement.
    The composited hold engaged on every composited frame and released at
    each flip.
  - The handle leak through scoot: about 79-87 bytes of kernel slab per
    import-and-destroy cycle (two runs), all returned when scoot exited.
    Abandoned waits (a surface destroyed mid-wait) cost about 200 bytes
    each. That memory outlived the client and was returned only when scoot
    exited: the same leak, filed with it.
  - Real clients on the VM: `vkcube --wsi wayland` (lavapipe) and
    `es2gears_wayland` (llvmpipe) draw through `wl_shm`, never bind the
    global, and run unaffected.
  - The compositor's own EGL display on the GBM device has
    `EGL_ANDROID_native_fence_sync`, so a composited frame's render fence
    is real on the VM, not a `glFinish`. The acquire path needs no EGL
    fence at all: a commit applies only after its blocker releases, so
    `DrmCompositor` is right to hand KMS an already-signalled fence.
  - Compositor CPU, before (`a30cd58`) and after, over 10 s windows:
    unchanged within noise for a fullscreen direct client (15-17 against
    16-18 jiffies) and a composited one (452-456 against 452-459). An
    explicit client costs the same as an implicit one.
- **Not verified: a real Vulkan or NVIDIA client, and real GPU
  hardware.** Nothing on the dev VM can allocate a dma-buf from a GL or
  Vulkan driver. That check is [`Asahi.md`](../../../Asahi.md) Test 7.

Original entry below, kept verbatim.

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
