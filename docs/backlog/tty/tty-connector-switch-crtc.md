---
title: "`--tty` hotplug can only switch to a connector its current CRTC can drive."
status: "open"
area: "tty"
priority: "low"
---

# `--tty` hotplug can only switch to a connector its current CRTC can drive.

Split out of PR #51 (issue #48, DRM hotplug) rather than grown into it, the
same way PR #46/#47/#49 split their own leftovers — so what shipped and what
did not are two separate records.

`tty/hotplug.rs` moves the session onto a different connector by asking the
*existing* `DrmSurface` to change its connector set
(`DrmSurface::set_connectors`, via `set_pending`). That surface is bound to
one CRTC, chosen once at startup by `tty/mod.rs`'s `create_surface`. So the
fall-back only works when the new connector can be routed to that CRTC.

Where it bites: a display controller whose encoders are wired to specific
CRTCs — common on ARM SoCs, and the same family of hardware as the
split-GPU case `--gpu` exists for — can refuse. On ordinary PC graphics,
where `possible_crtcs` is usually permissive, it does not.

What the refusal looks like today (both are handled, neither is silent):

- The atomic path answers `Err(TestFailed)`, so `set_pending` tries the
  other order and then gives up with `could not move the surface onto the
  new connector in either order; staying on the current one`.
- The legacy path answers `Ok(())` and simply does not apply the change
  (`surface/legacy.rs` at the pinned rev assigns the new set only when every
  `check_connector` passed, with no `else`). `set_pending`'s `move_connector`
  reads `pending_connectors()` back specifically to catch that, and logs
  `the surface accepted the new connector without applying it`.

So the failure mode is "stays on the connector that just went away", which
on a real unplug is a black screen — exactly what #48 set out to fix, just
on hardware where the one-CRTC assumption does not hold.

## What fixing it needs

Recreating the surface on a different CRTC, which means dropping the old one
first: `DrmDevice::create_surface` takes a `PlaneClaim` from the device's
`plane_claim_storage`, and the live surface holds the claim for its own
primary plane, so a second surface on the same CRTC cannot be built while
the first exists. `Tty::surface` would have to become droppable mid-operation
(an `Option<DrmSurface>`, or an equivalent take-and-replace), which touches
every site that reads it: `gamma_size`, `set_gamma_ramp`, `present`,
`on_vblank`, `reactivate` and `hotplug.rs`'s own paths. That is a real
refactor of the backend's core invariant ("there is always a surface") for a
case no hardware here can reproduce, which is why it was not done inside #48.

The CRTC's gamma size is per-CRTC (`Tty::gamma_size`, read once at startup
into `zwlr_gamma_control_v1`), so a CRTC change also has to re-read it and
tell `state.gamma_control`.

## What a user loses meanwhile

Nothing they had before #48 — this is a limit on a fall-back that did not
exist at all until then. On hardware where the CRTC is permissive, which is
most of it, the fall-back works; `README.md`'s `--tty` hotplug section
states the limit so "my laptop went black when I unplugged the external
monitor" has somewhere to land.
