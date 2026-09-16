---
title: "`--tty` follows DRM hotplug and host display reconfiguration — RESOLVED."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# `--tty` follows DRM hotplug and host display reconfiguration — RESOLVED.

## The entry as filed

Issue #48, filed 2026-09-16 from hands-on `--tty` testing under Apple's
vfkit, with kernel-level evidence attached. `--tty` read the connector's
modes once in `gpu::probe`, mode-set that once, and ignored whatever the
connector said afterwards — `tty/mod.rs`'s module doc listed DRM hotplug as
out of scope. Two real cases, structurally the same event:

1. **Host display reconfiguration.** Apple's Virtualization framework sets
   `automaticallyReconfiguresDisplay`, so moving the VM window between a 2x
   and a 1x screen, resizing it or going full-screen gives virtio-gpu a new
   host size; the guest drops a hotplug event and offers a new mode list and
   preferred mode. flexwm kept scanning out the old framebuffer, so the host
   scaled it or showed scrollbars. `--mode WxH` was the workaround, and only
   worked while the pinned size stayed in the list.
2. **Real hotplug.** Plug a monitor into a laptop running `--tty` on
   `eDP-1`: nothing happened. Unplug the one it was on: black screen until
   restart.

Scoped at filing time: single output stays the invariant — on hotplug,
re-select one connector/mode the way startup does; do not build multi-output.

## Resolution (2026-09-16, PR #51)

`compositor/tty/hotplug.rs` registers a `smithay::backend::udev::UdevBackend`
as a calloop source — a module this crate already depended on for
`all_gpus`/`primary_gpu` but had never used as an event source — and
re-runs the same single-connector choice on every `change` uevent for its own
device (matched on `DrmDevice::device_id`, so the seat's other DRM nodes are
ignored).

- `gpu::reselect` prefers the connector already being driven while it is
  still `Connected`, and widens to the rest only when it is gone — so a
  second monitor does not move the session off the panel the user is looking
  at. `--mode WxH` is honoured on every re-probe, not just the first.
- `plan` is the pure decision (unchanged / new mode / new connector /
  nothing connected), unit-tested, comparing **sizes rather than `Mode`s**:
  a re-probe can move the `PREFERRED` bit without changing the picture, and
  `Mode`'s `PartialEq` is a byte compare, so `==` would force a
  screen-blanking modeset for nothing.
- Nothing connected holds the last frame and says so once per disconnection
  rather than going black silently; a display coming back forces a modeset
  even at an unchanged mode.
- A hotplug that arrives while VT-switched away cannot be acted on (no DRM
  master), so the reactivation path re-probes on the way back.
- `State::resize_output` now ends in `apply()` and returns whether it
  succeeded; its own comment had already said a dynamically-resizing backend
  would need the first.

### The part that was nearly shipped broken

The first implementation queried connectors with `get_connector(conn,
false)` — which is `force_probe`. In `drm-ffi` 0.9.1 that sends
`count_modes: 1`, and the kernel's `drm_mode_getconnector` only calls the
driver's `fill_modes()` when `count_modes` is `0`. So the re-probe read the
kernel's *cached* mode list; and because the in-kernel fbdev client stops
refreshing that cache once a userspace DRM master exists, the cache is
exactly what goes stale while flexwm is running. `plan` would have concluded
nothing changed, forever, with no error — the silent class of failure this
project's standards single out.

It survived the first round of live testing because the *test's own trigger*
hid it: writing to `/sys/class/drm/cardN-*/status` makes the kernel call
`fill_modes()` as a side effect, so the cache was refreshed before flexwm
ever read it. Caught in review, and then pinned with an A/B that removes
that confound — `udevadm trigger` alone, no sysfs write, KMS debug on:

| binary | kernel probe lines |
| --- | --- |
| no flexwm running (control) | 0 |
| `get_connector(conn, false)` | 0 |
| `get_connector(conn, force_probe)` | 1 (`drm_helper_probe_single_connector_modes` → `probed modes:`) |

The fix is a `Freshness` enum rather than a bare `bool`, spelled out at both
call sites, because "what does `false` mean here" is precisely what went
wrong. It carries the cost with it: a forced probe re-reads EDID over DDC on
real HDMI/DP hardware — tens of milliseconds, more on a marginal link —
synchronously on the calloop thread, on every `change` uevent and every
VT-switch-back. wlroots pays the same for the same reason. `reselect`
therefore probes as few connectors as it can, and never re-probes the
current one twice.

## What is still not covered

- **`Plan::NewMode` and `Plan::NewConnector` have never run on real
  hardware.** Nothing in the QEMU/virtio-gpu dev VM can change a connector's
  mode list at runtime (EDID override is ignored by the driver, the mode
  list is stable across forced re-probes, and the host size is a fixed point
  that follows whatever flexwm mode-sets), and it has one connector. Both
  need confirmation on the hardware that filed the issue: the vfkit window
  resize, and a two-connector laptop unplug. Issue #48 is therefore
  referenced, not closed, by PR #51.
- **Connector switching is limited to the CRTC chosen at startup** — see
  [`../tty/tty-connector-switch-crtc.md`](../tty/tty-connector-switch-crtc.md).
- Multi-GPU, multi-output, DPMS and key repeat remain out of scope for this
  backend, unchanged.
