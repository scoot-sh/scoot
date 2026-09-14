---
item: "3"
title: "Real tty/DRM backend"
status: "done"
area: "backend"
pr: 5
commit: "0a90dc9"
---

# Real tty/DRM backend

~~Real tty/DRM backend~~ — DONE, merged to `main` at `0a90dc9`, PR #5.
`--tty`: libseat session + raw `DrmSurface`/dumb-buffer scanout (no GBM —
this Smithay rev has no `Bind<DumbBuffer>` for pixman, so presenting is a
memcpy, confirmed necessary not just simplest) + libinput. Mirrors
`nested::Host`'s shape (`Tty::present` same byte-slice signature). VT
switching added as `keybindings::Bound::ChangeVt` (kept out of
`flexwm_core::Action` — session concern, not layout, keeps the core
platform-independent). Two independent review rounds; second found 3 real
issues, all fixed: (a) `reactivate()` used to skip `libinput.resume()` on
a failed `drm.activate`, which could strand the keyboard dead alongside a
dead display with no recovery path — fixed so every recovery step runs
independently; (b) `DrmEvent::Error` didn't free the pending buffer slot
the way `VBlank` did, leaking it forever after repeated occurrences —
fixed via a shared `flip_settled()`; (c) `build.rs`'s `pkg-config`
build-dependency was wrongly gated behind `cfg(target_os = "linux")`,
which broke `cargo check`/`cargo build` on the Mac dev host (build-deps
compile for the *host*, not the target) — fixed by ungating it. Verified
on the dev VM's real `virtio-gpu` KMS device: real scanout, VT-switch
round trip (pause/reactivate + forced full modeset on the way back), idle
CPU ~0, a real `/dev/uinput`-injected keystroke proven to reach a client.
The "real scanout" and VT-switch claims here were the ones later called
into question by Smithay's `unprivileged mode` warning; both were
re-confirmed directly on 2026-09-13 (the CRTC's plane really does scan out
a flexwm-allocated framebuffer, and master really is dropped and
reacquired across a VT switch) — see the resolved DRM-master entry in the
Backlog.
Out of scope, stated: cursor rendering, DRM hotplug, multi-GPU/output,
DPMS, output scale, key-repeat.
