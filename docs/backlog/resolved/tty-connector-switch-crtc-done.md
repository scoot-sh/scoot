---
title: "`--tty` hotplug can switch to a connector its current CRTC cannot drive — RESOLVED."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# `--tty` hotplug can switch to a connector its current CRTC cannot drive — RESOLVED.

## The entry as filed

`docs/backlog/tty/tty-connector-switch-crtc.md` (LOW):

> `tty/hotplug.rs` moves the session onto a different connector by asking the
> *existing* `DrmSurface` to change its connector set
> (`DrmSurface::set_connectors`, via `set_pending`). That surface is bound to
> one CRTC, chosen once at startup by `tty/mod.rs`'s `create_surface`. So the
> fall-back only works when the new connector can be routed to that CRTC.
> [ARM SoCs with encoders wired to specific CRTCs] can refuse. ... The failure
> mode is "stays on the connector that just went away", which on a real unplug
> is a black screen.
>
> What fixing it needs: recreating the surface on a different CRTC, which
> means dropping the old one first ... `Tty::surface` would have to become
> droppable mid-operation (an `Option<DrmSurface>`, or an equivalent
> take-and-replace), which touches every site that reads it ... That is a real
> refactor of the backend's core invariant ("there is always a surface") for a
> case no hardware here can reproduce.

## Resolution (2026-09-18)

`retarget`'s `set_pending` failure now falls through to `Tty::switch_crtc`
(`hotplug.rs`), which rebuilds the `DrmSurface` on a different CRTC that can
drive the new connector — and the ticket's anticipated `Option<DrmSurface>`
refactor proved unnecessary. Total failure still degrades to today's
behaviour (log + stay put + retry on the next uevent), never a panic, never a
missing surface.

### Verify-first: what the pinned rev actually says

All re-verified in source at `0ff0098`, not relayed from the ticket:

- `DrmDevice::create_surface` claims the target CRTC's *primary plane* via
  `plane_claim_storage.claim(plane, crtc)` (`device/mod.rs`). Claims are keyed
  by plane handle: same `(plane, crtc)` re-claims the live claim, a different
  CRTC on the same plane returns `None`. Dropping the surface drops the claim
  (`PlaneClaimInner::drop` → `storage.remove`). So a replacement on a
  *different* CRTC builds fine while the old surface lives (each CRTC owns its
  primary plane), and only a same-CRTC rebuild would need drop-first.
- `set_pending` already proved the current CRTC refuses this target, so
  rebuilding there is pointless anyway: only *other* CRTCs are candidates.
  Build-first-swap-on-success therefore has no window with no surface, and
  `Tty::surface` stays a plain `DrmSurface` — every reader (`present`,
  `on_vblank`, `reactivate`, `gamma_size`, `set_gamma_ramp`) is untouched.
- Neither constructor validates routability: atomic records the pending state
  blindly (`atomic.rs` `new`), legacy assigns it (`legacy.rs` `new`). A fresh
  surface must be probed, not trusted — each candidate goes through the same
  two setters `set_pending` uses (connectors, then mode). The target's own
  values still exercise both checks: atomic `TEST_ONLY`-commits them,
  legacy runs `check_connector` (encoder + `possible_crtcs`).
- Legacy limit, stated not papered over: a fresh legacy surface's pending set
  already names the target (the constructor put it there), so
  `move_connector`'s readback cannot catch the silent non-apply the way it
  does for the live surface. A legacy candidate that fails its encoder check
  still probes green; the refusal surfaces at the first real commit through
  `present`'s existing failure arm (warn + bounded retry + quiet), which ties
  the old behaviour since the previous connector is gone either way. Atomic
  validates honestly (`TestFailed`).
- Dropping a failed *candidate* runs Smithay's surface `Drop`, which clears
  that candidate CRTC's state. It cannot touch the live path (the clear
  addresses the candidate's own CRTC and its current connectors, disjoint
  from the old CRTC/connector by construction — one `CRTC_ID` per
  connector), and the candidate CRTC is idle in the common single-display
  case. At worst it drops firmware/console content on a display flexwm never
  drove, plus the probe's `TEST_ONLY` commits — all on the cable-move path.
- The successful swap's drop of the *old* surface clears the old CRTC, whose
  connector is gone; the full commit `invalidate_scanout` arms brings the new
  CRTC up. `on_vblank`'s CRTC check ignores stale vblanks for the old CRTC;
  `invalidate_scanout` retires any in-flight flip exactly like the existing
  hotplug/reactivate paths (PR #107's retry semantics unchanged) — plus a
  `PresentRetries` reset: a new CRTC is new device state, so an inherited
  refusal streak must not answer Quiet on the new hardware (review finding,
  PR #112).

### What changed

- `hotplug.rs`: `retarget` falls through to `switch_crtc` when `set_pending`
  fails (buffers allocation moves along: installed on success, dropped on
  failure). `switch_crtc` tries every other CRTC in `drm.crtcs()` order,
  probes each candidate, swaps the surface/connector/buffers/size on the
  first proven one, invalidates scanout, and returns the new
  `Reconfigured::SwitchedCrtc { width, height, size_changed, gamma_size }`
  (gamma re-read from the new CRTC). Total failure logs
  `no other crtc on this device can drive the new connector` and returns
  `Nothing`, keeping `nothing_connected` set so the next uevent retries.
- `Reconfigured::finish`: the new arm runs `gamma_control.crtc_changed` and
  then the `Resized`/`Render` half by `size_changed`. The `Resized` body moved
  verbatim into `apply_resize`, shared by both arms; `Nothing`/`Render` are
  untouched, and `session_event`'s `Nothing → Render` mapping passes the new
  variant through.
- `gamma_control.rs`: new `crtc_changed(size)` — re-records the LUT length
  and fails the live control (the transfer shape) on every switch, so the
  client re-reads `gamma_size` and re-pushes. Review (PR #112) corrected an
  earlier same-length exception: nothing carries the old ramp to the new
  CRTC, so keeping the control showed un-warmed white until the client's
  next periodic set. `set_size` keeps its init-only contract.
- `README.md`: the `--tty` hotplug bullet no longer carries the one-CRTC
  limit (it describes the CRTC move and the stay-put-and-retry fallback), and
  the gamma section records the re-read + fail-on-switch rule.

### Tests

Four harness tests in `hotplug/tests.rs` (shared `test_support::Harness`,
real client binding `wl_output` + the gamma manager) pin `finish` for the new
outcome: size-changing switch resizes `wl_output` current-and-preferred like
`Resized` and reports the new gamma length; same-size switch requests a
render without resizing; a changed length fails the live control (and a later
control learns the new length); an unchanged length leaves it alone. The
surface rebuild itself needs a multi-CRTC device and stays unverified (below).

Fail-first, both directions (dev VM, `cargo test -p flexwm --bin flexwm
switched_crtc`): with the new `finish` arm neutered to nothing, 3 fail and
the same-gamma one passes (it pins the no-op path); with `crtc_changed`'s
early return neutered, exactly the same-gamma test fails. Restored green.

### Bug bash

- The ticket's `Option<DrmSurface>` refactor was the first design; the
  claim-per-CRTC finding above killed it before any code was written — less
  code, no invariant change, no new panic path.
- The candidate-drop side effect (above) was found by tracing `Drop`, not by
  testing: nothing in the suite can construct a second CRTC.
- `switch_crtc` with zero other CRTCs (the dev VM) returns `Nothing` without
  touching anything — the loop simply doesn't run.
- `size_changed` with `buffers: None` is unreachable (`retarget` returns
  before `set_pending` when allocation fails); the `if let Some` handles both
  shapes regardless.

### Live evidence (dev VM, QEMU/virtio-gpu, one CRTC — `drm_info` shows only
`CRTC 0`, so the switch itself cannot fire here; this is regression evidence)

Against commit `f1746cc` + working tree (`fix/tty-connector-switch-crtc`,
verified via `git -C /mnt/flexwm status` on the 9p mount):

- Session starts: `drm: driving this device path=/dev/dri/card0
  connector=Virtual-1 width=1280 height=720` (atomic, CRTC 37), `drm: crtc
  gamma size size=256`.
- Screenshot before/after a `chvt 2` / `chvt 1` cycle: `09a91ae837c2118f5b0ed6dac8bab4b6`
  both times (19665 bytes) — pause → `session paused`, activate →
  `drm: modeset (full commit)`, byte-identical repaint.
- Synthetic hotplug (`sudo udevadm trigger --subsystem-match=drm
  --action=change`): udev `Device changed` arrives, `reconfigure` runs,
  `drm: hotplug changed nothing this backend is driving` (debug build), no
  modeset, pixels identical, and `different crtc` appears 0 times in the log
  (the new path correctly never fires on one CRTC).
- Quit twice: 0 × `Failed to restore previous state`, 0 × permission errors,
  seatd `Removed client`, no stray processes (PR #111 stays quiet).
- `scripts/smoke-test.sh` (headless, `SMOKE_PREFIX=/tmp/smoke-crtc`): all
  `ok` lines, exit 0 on re-run.

### Benchmark

No hot-path benchmark: the new code runs only when a cable moves (one small
`Vec` + `TEST_ONLY` commits per candidate CRTC), never per-frame or
per-event. The render/present paths are untouched.

### Not verified live (stated plainly)

- The actual CRTC switch: no multi-CRTC or encoder-restricted hardware here
  (single virtio-gpu CRTC; the ticket's own premise). Needs the Asahi/ARM
  hardware class the ticket names — or QEMU with a second CRTC if one can be
  conjured.
- The legacy (non-atomic) path in its entirety, including the blind-probe
  limit above: the dev VM is atomic.
- A lock or capture held across a CRTC switch; gamma re-read against a real
  second CRTC with a different LUT length (harness pins the bookkeeping, not
  the ioctl).
- The swapped-but-unroutable legacy corner: after such a swap
  `Tty::connector` names the new connector, so a later identical probe plans
  `Unchanged` instead of retrying the switch. Ties the old behaviour's pixels
  (black either way — the old connector is gone) but retries less loudly;
  filed nowhere separately by design (legacy + wired encoders + this corner is
  beyond what any hardware here can reach).

No config option, keybinding, CLI flag or IPC surface comes with any of this:
a hotplug that now survives a CRTC boundary is invisible except in the log.
