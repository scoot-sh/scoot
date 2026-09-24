---
title: "Explicit sync: Smithay leaks one kernel syncobj handle per timeline import"
status: "open"
area: "protocols"
priority: "medium"
blocked: "needs a Smithay change (a Drop for the imported timeline): a patched fork, or an upstream fix and a rev bump"
---

# Smithay leaks one kernel syncobj handle per timeline import

Filed 2026-09-23 by the [explicit-sync work](../resolved/linux-drm-syncobj-done.md),
which found it while looking for leaked handles. It serves **daily-drive**:
a client-triggerable resource leak on the GPU tier.

## What is wrong

`wp_linux_drm_syncobj_manager_v1.import_timeline` becomes
`DrmTimeline::new` → `DrmTimelineDeviceSpecific::import` →
`fd_to_syncobj`, which creates a handle on the import device's DRM file.
Nothing ever destroys that handle. `DrmTimelineInner` has no `Drop`, and
only `invalidate()` (from `DrmSyncobjState::into_global`) calls
`destroy_syncobj` (`src/wayland/drm_syncobj/sync_point.rs` at the pinned
`0ff0098`; unchanged on Smithay's `master` as of 2026-09-23). So every
import keeps a handle, and with it the client's syncobj and its fence
state, until scoot's DRM file closes when scoot exits.

## Measured (dev VM, kernel 6.18, virtio-gpu)

- **Without scoot**, a C loop doing exactly what `DrmTimeline::new` does,
  200000 imports into a second DRM file: 3.7 bytes of slab per import of one
  syncobj imported repeatedly, 83 bytes per import of a fresh syncobj. It
  was all returned when that file closed.
- **Through scoot** (`--tty --renderer gles`, a client importing a fresh
  syncobj and destroying the timeline 50000 times): slab grew by 3904 kB,
  about 79 bytes per cycle. It was all returned when scoot exited. The
  per-client live-timeline bound (128) never trips, because each timeline is
  destroyed before the next is imported.

A legitimate session leaks a little. Mesa's Vulkan WSI imports two timelines
per swapchain image and recreates the swapchain on every resize, so a busy
day is kilobytes to a few megabytes. A hostile local client leaks at wire
speed, and the kernel memory is not returned until the compositor exits.

## Why scoot cannot fix it alone

- **Destroying the handle ourselves**: the handle number is private to
  Smithay, and guessing it risks destroying a live handle another timeline
  uses.
- **Rotating the import device** (`close_device` + `update_device`, then
  dropping the old fd): Smithay migrates only the timelines in
  `known_timelines`, and it drops a timeline from that list when the client
  destroys the timeline *object*, even while points on it are still in
  flight. Those points keep only a weak reference to the old device, so
  after a rotation their release points could never be signalled. The
  protocol says destroying a timeline does not unset its points, so this is
  a real client harm.
- **Capping imports per connection over its lifetime** would eventually kill
  a legitimate long-lived Vulkan client that resizes a lot. It also does
  nothing about reconnect loops.

## What to do

Add a `Drop` for `DrmTimelineInner` (or `DrmTimelineDeviceSpecific`) that
calls `destroy_syncobj` on the device, if it is still alive. The timeline
fd then closes with it. Upstream it to Smithay. Until a rev carries it,
decide whether to carry it as a patched fork (`[patch]` in the workspace
`Cargo.toml`), which departs from the "pinned to one Smithay rev" rule and
is the coordinator's call. The alternative is to leave explicit sync
offered with this documented. Re-measure with the loop above either way;
the evidence recipe is in the explicit-sync PR.
