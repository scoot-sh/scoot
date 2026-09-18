---
item: "17"
title: "DRM device-selection fallback + --gpu"
status: "done"
area: "backend"
pr: 27
commit: "6bfd29e"
---

# DRM device-selection fallback + --gpu

**The fault is in the heuristic, not the hardware** — niri drives the
same machine's display. `tty/mod.rs` called
`smithay::backend::udev::primary_gpu` and trusted the answer, and that
function ranks (1) a PCI parent with `boot_vga=1`, (2) the
alphabetically first device with a DRM *render* node, (3)
alphabetically first. Apple Silicon has no PCI GPU and no VGA BIOS, so
(1) never matches; (2) then picks `asahi`/AGX, the 3D GPU, over
`apple,dcp`, which is a *separate* DRM device and the one that owns the
CRTCs and connectors. `ENOTSUP` from `resource_handles` is what a
device with no mode-setting pipeline returns.

**Both halves of the Backlog entry's fix direction shipped.** A new
`tty/gpu.rs` builds a candidate list — `primary_gpu()`'s pick first, so
ordinary hardware keeps today's answer and opens exactly one device,
then every other device on the seat in `all_gpus()`'s sorted order —
and `init` walks it until one works, reporting every device it tried
and what each said if none does. `--gpu PATH` replaces the search
outright (one candidate, no fallback), as both the immediate workaround
and the documented escape hatch. It is ignored, with a warning, outside
`--tty`.

**Two decisions worth the words.** (a) *A candidate is checked on a
borrowed fd, before anything owns it.* `Session::close` — which is how
a rejected device goes back to libseat instead of leaving seatd holding
it open for the process's life — needs the `OwnedFd`, and Smithay's
`DeviceFd` is an `Arc<OwnedFd>` with no way back out. So `gpu::probe`
reads the KMS resources and the connector list through a tiny
`Probe(BorrowedFd)` (three empty trait impls) first. It also stops a
hopeless candidate from ever constructing a `DrmDeviceFd`, whose
"Unable to become drm master" logging would otherwise fire once per
device and read like the cause. The three steps *after* that
(`DrmDevice::new`, surface, buffers) still fall through to the next
candidate; they just close by dropping. (b) *No env-var counterpart.*
`FLEXWM_SOCKET` exists because child processes must inherit the socket
path; nothing inherits `--gpu`, and `--config` — the closest analogue —
has no env var either.

**What the dev VM can and cannot prove.** Its `virtio-gpu` is a single
unified render+display device, so the split topology this fixes cannot
exist there. Verified there instead: the working `--tty` path is
unchanged and still opens exactly one device (`drm: driving this device
path=/dev/dri/card0`, no `device unusable` warning — the loop never
iterates); `--gpu /dev/dri/card0` behaves identically to the automatic
path; `--gpu` pointed at a render node, a nonexistent path, `/dev/null`,
a directory and the empty string each exit 1 with a clear message
naming the device, no hang and no panic; a plain `--tty` still starts
after six consecutive failed starts, so a rejected device doesn't leak
the VT-bound seat. The fallback *iterating* is covered by unit tests
against `first_usable`, which is why the loop takes its opener as a
parameter. Whether this actually fixes Asahi Linux needed that machine, and it does:
confirmed 2026-09-18 on the reporter's Apple M2 -- the search rejects the
`asahi` render node and drives `apple-drm`/`eDP-1` unattended. See
`Asahi.md` and `docs/backlog/resolved/tty-gpu-config-key-done.md`.

**One real fix and five accuracy fixes from review**, all on the same
branch. The fix: `candidates` `?`-ed *both* udev calls, so a machine
whose primary device was found and would have worked could fail to
start because the fallback's own `all_gpus` enumeration hiccuped —
contradicting this item's "no behavior change on ordinary hardware".
It now degrades to "primary only, with a warning"; only a failure with
no primary to fall back to is still fatal (`gpu::assemble`, unit-tested
both ways). The rest: the all-candidates-failed error no longer
recommends `--gpu` when *every* candidate was refused at
`Session::open` (a busy seat, which is the likeliest real failure —
naming a device cannot help), which needed the per-candidate reason to
become a typed `gpu::Rejection` rather than a bare string; the
probe-rejection message dropped its "a device that only computes …"
trailing clause, which was asserted for every cause (an evdev path
typo included); `cli.rs`'s `gpu` doc said "silently ignored" where the
code warns; `OpenGpu`'s doc claimed the probe used a *different* file
description when it borrows the same one (and the same-fd property is
load-bearing — libseat keys its device table by raw fd); and two
README wordings ("the certain workaround", a dangling clause).

**Found while bug-bashing, not caused by this change:
`MODE=--tty scripts/smoke-test.sh` fails its background-pixel check**
(`the background pixel at (3,3) is rgb(0,0,0), expected #123456`).
Reproduced identically against a binary built from `main` at `868dd83`
on the same VM, with the assertion lines diffing clean between the two
— see the Backlog entry.
