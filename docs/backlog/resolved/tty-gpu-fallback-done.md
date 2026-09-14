---
title: "`--tty` can pick a GPU that can't drive a display at all, and gives up outright instead of trying another one \u2014 DONE as item 17, PR #27; built and verified on the dev VM, and still awaiting confirmation on the Apple Silicon machine it was reported from (which no dev-VM test can stand in for \u2014 see item 17)."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# `--tty` can pick a GPU that can't drive a display at all, and gives up outright instead of trying another one — DONE as item 17, PR #27; built and verified on the dev VM, and still awaiting confirmation on the Apple Silicon machine it was reported from (which no dev-VM test can stand in for — see item 17).

~~`--tty` can pick a GPU that can't drive a display at all, and gives up
outright instead of trying another one~~ — DONE as item 17, PR #27;
built and verified on the dev VM, and still awaiting confirmation on the
Apple Silicon machine it was reported from (which no dev-VM test can
stand in for — see item 17). Real, reported from the user's own Apple
Silicon (Asahi Linux, M2) laptop, 2026-09-13. `flexwm --tty` there failed
immediately:

```
WARN smithay::backend::drm::device::fd: Unable to become drm master, assuming unprivileged mode
INFO smithay::backend::drm::device: DrmDevice initializing
INFO smithay::backend::drm::device::fd: Dropping device: Some("/dev/dri/card1") (Operation not supported (os error 95))
flexwm: DRM access error: Error loading resource handles on device `Some("/dev/dri/card1")`
```

**Root cause, traced to Smithay's own heuristic, not flexwm's code**:
`tty/mod.rs`'s `run` calls `smithay::backend::udev::primary_gpu(&seat_name)`
and trusts whatever it returns, with no fallback. That function's own
priority order (`backend/udev.rs`, pinned rev): (1) a PCI parent device
with `boot_vga=1`, (2) the first device (of those sorted alphabetically)
that has a DRM *render* node, (3) alphabetically first otherwise. Apple
Silicon has no PCI GPU and no legacy VGA BIOS concept, so (1) never
matches anywhere on this class of hardware. (2) then wins, and on a
split GPU/display-controller SoC like this — the 3D GPU (`asahi`/AGX,
which does have a render node) is a *separate* DRM device from the
actual display controller (`apple,dcp`, which owns the CRTCs/connectors
but likely has no render node at all) — the heuristic picks the
render-capable compute GPU over the device that can actually drive a
screen. `ENOTSUP` loading resource handles is exactly what a render-only
DRM node with no KMS/mode-setting pipeline would return. flexwm has no
`--gpu`/override flag and no fallback to `smithay::backend::udev::
all_gpus()` if the chosen device doesn't work, so this is a hard failure
rather than a recoverable one — though it **did** fail safely: a clean
error message, process exit, back to the shell, nothing stranded.

**Confirms the hardware itself isn't the blocker**: the user's default
compositor on this same machine before trying flexwm was niri, which
they report drives the display fine (GPU-accelerated) — so whatever niri
does to pick or use the display device works on this hardware; flexwm's
gap is Smithay's PC-centric heuristic, not something unfixable about
Asahi Linux.

**Fix direction**: (a) if `primary_gpu()`'s chosen device fails to open
or load resources, fall back to trying each device from `all_gpus()` in
turn rather than giving up immediately — likely the higher-value fix,
since it would make `--tty` "just work" on this class of hardware with
no user action needed; (b) add a manual override (a `--gpu PATH` flag
and/or an env var) so a user can point flexwm at the right
`/dev/dri/cardN` directly, both as an immediate workaround and as a
documented escape hatch if the automatic fallback ever guesses wrong.
Probably want both, not just one.

**Verification will need real hardware, not just the dev VM**: the dev
VM's `virtio-gpu` device is a single unified render+display device, so
this split-topology bug can't be reproduced or verified there — the fix
can be built and unit-tested for logic correctness in the usual way, but
confirming it actually resolves `--tty` on Apple Silicon needs the same
real machine that reported this, or equivalent split-GPU/display-
controller hardware.
