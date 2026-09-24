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
  200000 imports into a second DRM file, run twice: 3.7-10 bytes of slab
  per import of one syncobj imported repeatedly, 83-88 bytes per import of
  a fresh syncobj. It was all returned when that file closed.
- **Through scoot** (`--tty --renderer gles`, a client importing a fresh
  syncobj and destroying the timeline 50000 times, run twice): slab grew by
  3904 and 4296 kB, about 79-87 bytes per cycle. It was all returned when scoot exited. The
  per-client live-timeline bound (128) never trips, because each timeline is
  destroyed before the next is imported.

- **A second path rides on the same leak: waits abandoned by destroying the
  surface.** scoot removes its own eventfd source the moment a surface
  with a pending wait is destroyed, and its fds stay flat. But the kernel
  keeps the `DRM_IOCTL_SYNCOBJ_EVENTFD` registration, with its eventfd
  context, on the client's syncobj until that point signals or the syncobj
  is freed, and the leaked handle keeps the syncobj alive after the client
  exits. Measured through scoot, 20000 surfaces each committed with an
  unsignalled acquire point and then destroyed: slab grew by 3904 kB while
  the client lived (about 200 bytes per abandoned wait). It was still
  there after the client exited, and returned only when scoot exited.
  While the client lives, that memory is no different from registering
  eventfds on its own syncobj, which any client can do without scoot. What
  the leak adds is that it outlives the client. The same `Drop` fixes it:
  once scoot's handle goes, the client's exit frees the syncobj and its
  registrations.

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

## The fix, verified against scoot

A `Drop` for `DrmTimelineDeviceSpecific` that destroys the handle on the
device if the device is still alive. It is safe with `invalidate()`, which
destroys the handle itself and clears `device` first, and with
`update_device()`, where replacing the ctx destroys the old handle on the
old device. Built as a `[patch]` of the pinned checkout into scoot at
`3e719cc` (release, own target dir, since deleted; binary
`scoot-3e719cc-smithay-drop-patch` in the dev VM's `~/evidence/sync/bin/`,
patch at `~/evidence/sync/smithay-drop.patch`):

- the 50000-cycle import-and-destroy loop: slab +180 kB (about 3 bytes per
  cycle, noise), against +3904 to +4296 kB unpatched;
- 20000 abandoned waits: +3780 kB while the client lived, as expected, and
  +100 kB once it exited, against +3908 kB unpatched;
- the held-commit and continuous-client scenarios unchanged (releases
  signalled, 0 reuse timeouts, every fullscreen frame direct).

```rust
impl Drop for DrmTimelineDeviceSpecific {
    fn drop(&mut self) {
        if let Some(device) = self.device.upgrade() {
            let _ = device.destroy_syncobj(self.syncobj);
        }
    }
}
```

## What to do

Upstream the `Drop` above to Smithay. Until a rev carries it,
decide whether to carry it as a patched fork (`[patch]` in the workspace
`Cargo.toml`), which departs from the "pinned to one Smithay rev" rule and
is the coordinator's call. The alternative is to leave explicit sync
offered with this documented. Re-measure with the loop above either way;
the evidence recipe is in the explicit-sync PR.
