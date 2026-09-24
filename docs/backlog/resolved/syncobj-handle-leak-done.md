---
title: "Explicit sync: Smithay leaks one kernel syncobj handle per timeline import — RESOLVED (in scoot, via a Smithay fork)"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Smithay leaks one kernel syncobj handle per timeline import — RESOLVED in scoot

RESOLVED 2026-09-24 (PR #233, the same PR that added explicit sync), **in
scoot only**. scoot now builds against a scoot-sh fork of Smithay
(`github.com/scoot-sh/smithay`, rev `43f50eb2`, branch
`scoot/syncobj-timeline-drop`). The fork is upstream's pinned `0ff00983`
plus exactly one commit, the `Drop` below. **Upstream is still open.** As
of 2026-09-24 it is not fixed on Smithay `master` (head `79bbed5e1` has no
such `Drop`) or in any release (v0.7.0 predates `0ff00983`). The user will
file the upstream PR. Getting scoot back onto upstream is
[`core/smithay-fork-repin.md`](../core/smithay-fork-repin.md).

Filed 2026-09-23 by the [explicit-sync work](./linux-drm-syncobj-done.md),
which found it while looking for leaked handles.

## What was wrong

`wp_linux_drm_syncobj_manager_v1.import_timeline` becomes
`DrmTimeline::new` → `DrmTimelineDeviceSpecific::import` →
`fd_to_syncobj`, which creates a handle on the import device's DRM file.
Upstream, nothing ever destroys that handle. `DrmTimelineInner` has no
`Drop`, and only `invalidate()` (from `DrmSyncobjState::into_global`)
calls `destroy_syncobj`. So every import keeps a handle, and with it the
client's syncobj and its fence state, until scoot's DRM file closes when
scoot exits.

There is a second path on the same leak: waits abandoned by destroying the
surface. scoot removes its own eventfd source the moment a surface with a
pending wait is destroyed. But the kernel keeps the
`DRM_IOCTL_SYNCOBJ_EVENTFD` registration, with its eventfd context, on the
client's syncobj until the point signals or the syncobj is freed, and the
leaked handle kept the syncobj alive past the client's exit.

Review of PR #233 measured the import path at **~24 MB/s of kernel slab**
under a hostile import-and-destroy loop, unaccounted to any process and
persisting until scoot exits.

## Why it could not be fixed in scoot alone

- **Destroying the handle ourselves:** the handle number is private to
  Smithay, and guessing it risks destroying a live handle another timeline
  uses.
- **Rotating the import device** (`close_device` + `update_device`, then
  dropping the old fd): Smithay migrates only the timelines in
  `known_timelines`, and it drops a timeline from that list when the
  client destroys the timeline object, even while points on it are still
  in flight. Those points keep only a weak reference to the old device, so
  after a rotation their release points could never be signalled. The
  protocol says destroying a timeline does not unset its points, so this
  would be a real client harm.
- **Capping imports per connection over its lifetime:** that would
  eventually kill a legitimate long-lived Vulkan client that resizes a lot,
  since Mesa's WSI imports two timelines per swapchain image and recreates
  the swapchain on resize. It would also do nothing about reconnect loops.

## The fix (the fork's one commit)

```rust
impl Drop for DrmTimelineDeviceSpecific {
    fn drop(&mut self) {
        if let Some(device) = self.device.upgrade() {
            let _ = device.destroy_syncobj(self.syncobj);
        }
    }
}
```

It is safe with `invalidate()`, which destroys the handle itself and
clears `device` first, and with `update_device()`, where replacing the ctx
destroys the old handle on the old device.

## Measured (dev VM, kernel 6.18, virtio-gpu, `--tty --renderer gles`, release builds)

Raw output is in `~/evidence/sync/` on the dev VM.

**Before** means scoot `3e719cc` on upstream Smithay `0ff00983`. **After**
means scoot `01f79a9`, built from the committed repin with the real git
dependency on the fork.

| scenario | before | after (repinned build) |
|---|---|---|
| 50000 import-and-destroy cycles (`g-repin-*.out`) | +4332 kB slab (~88 B per cycle), returned only when scoot exited | **+0 kB** |
| 20000 surfaces each committed with an unsignalled acquire point, then destroyed (`g2-repin-*.out`) | +3708 kB while the client lived, **+3668 kB after it exited**, returned only when scoot exited | +3608 kB while the client lived, **+84 kB** after it exited |

While the client lives, the abandoned waits' memory (about 200 B each) is
the client's own syncobj registrations, the same thing it could create
without scoot. What the fork removes is that memory outliving the client.

Earlier runs agree. Before: 4296 kB for the import loop (`g.out`, at
`3e719cc`), and 3904/3908 kB alive/exited for the abandoned waits
(`g2.out`). With the same patch applied as a local `[patch]`: 180 kB and
3780/100 kB (`g-patched.out`, `g2-patched.out`).

Without scoot, a C loop doing exactly what `DrmTimeline::new` does, 200000
imports into a second DRM file, leaks 83-88 B of slab per fresh syncobj and
4-10 B per repeated one. All of it is returned when that file closes
(`c-probes/leak.out`).

The repinned build still behaves: a held commit applied on signal, and the
old buffer was released 0.44 ms after it (`b-01f79a9.out`). The full gate
is in the PR.

## Original entry

Smithay upstream leaked one kernel handle per timeline import and kept
abandoned waits alive past their client. The fix is a `Drop` in Smithay:
carry a patched fork, or fix it upstream and bump the rev. The user chose
the fork, and filing upstream is theirs.
