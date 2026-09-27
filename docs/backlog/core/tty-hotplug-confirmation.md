---
title: "`--tty` hotplug follow-up: confirm the two unreproduced paths on real hardware"
status: "open"
area: "core"
priority: "low"
blocked: "gh #48 closed 2026-09-27 on unit-test evidence (user decision); live proof of both paths deferred to a rig that can drive them"
---

# `--tty` hotplug follow-up: confirm the two unreproduced paths on real hardware

Filed as gh issue #48 (2026-09-16 — read it for the full shape, not a
plan). PR #51 landed the `--tty` hotplug following (udev monitor,
connector re-choice, re-modeset); see
`../resolved/tty-drm-hotplug-done.md` for what shipped and what it
proved on the QEMU dev VM. This entry tracks exactly what is left: the
issue stayed open because two paths could not be reproduced there.

1. **A new mode list on the same connector** (vfkit host-window
   rescale/move offering a new preferred mode + list). The code path is
   the same re-choice the VM proved for connector *loss*; what wants
   confirmation is the list actually changing under a live session and
   the session following to the new preferred (or `--mode`) size, with
   `wl_output.mode`/`done` reaching clients.
2. **Falling back to a *different* connector** (unplug the one it is on
   with another `Connected` one present). Single-output invariant holds
   — the session must switch connectors, not add one — and must not go
   black.

Both want the vfkit/laptop hardware that filed the issue (or equivalent
two-connector/relayout-capable hardware), with the exact commands,
`wlr-randr` before/after, and screenshot proof recorded — not a
paraphrase. If a path fails on hardware, it becomes its own fix ticket;
if both confirm, close this entry with the evidence and close #48.


## Update 2026-09-25 (Asahi M2 Air, external monitor via the `fairydust` kernel)

Hotplug events on real hardware are received and handled: unplugging and
replugging an external monitor while scoot drove the panel produced two udev
change events and left the session running on `eDP-1` with no disruption
(`Asahi.md` Test 3 results). The two paths this ticket tracks remain
unconfirmed on this machine: the panel can't be unplugged, and the
experimental kernel kept `DP-1` reading `connected` across the unplug. A
machine with two unpluggable outputs (or a kernel that reports DP HPD loss)
is still needed.


## Update 2026-09-25, later: multi-output phase E changes what path 2 means

`--tty` now drives every connected connector (milestone 19 phase E). Path 2
(falling back to a *different* connector) now applies only when **no**
driven connector is still connected. That is `hotplug::heads::replan`'s
`MoveTo`, pinned in its unit tests. With another screen still lit, an unplug
*removes* that screen's output instead (`State::remove_output`, harness
suites `outputs/removal.rs` and friends), and a plug *adds* one. The old
"second display stays dark" behaviour is gone.

**Corrected the same day.** A physical replug at 17:59Z *did* read
`disconnected` (33 samples at 0.5 s in `~/fx/replug/dp-status.log`), and
scoot removed output 2 and then added output 3 on CRTC 68. The morning's
"DP-1 kept reading connected" came from a quicker unplug and is not a
property of the kernel. The multi-output remove/add paths are therefore
confirmed on hardware (see `Asahi.md` Test 3). What this ticket tracks is
still open:
- **path 2 (fallback to a different connector):** it needs the *only* lit
  screen to be unpluggable, and this machine's panel is not;
- **path 1 (a new mode list on the same connector):** it needs vfkit-style
  host rescaling.

## Update 2026-09-26: path 1 attempted on vkms, `edid_override` is inert there

Tried to drive a mode-list change without host rescaling: dev VM kernel
6.18.50, vkms card1/`Virtual-3`, two hand-built 128-byte EDIDs each with
a single 1280x720@60 DTD (74.25 MHz, `di-edid-decode`-clean, valid
checksum; the first fully valid including zeroed chromaticity, the
second with an sRGB block). Both stored fine (128 bytes readable back
from `edid_override`) and both had **zero effect**: the probed list
stayed the 34-mode no-EDID fallback with 1024x768 preferred, across a
fresh `modprobe`, repeated `modetest` probes minutes apart, and
`force` off/on cycles; sysfs `edid` stayed 0 bytes throughout. So on
this kernel the knob does not feed the mode list a `GETCONNECTOR`
probe (or `connector_mode(Reprobe)`) reads, and path 1 still has no
live driver on any rig tried so far. Not a scoot misbehaviour — nothing
reached `replan`, so no fix ticket; the `NewMode` logic stays pinned by
`hotplug/heads.rs: a_new_mode_on_any_head_is_followed_on_that_head_only`
and `hotplug/tests.rs` until vfkit/rescale-capable hardware (or a
kernel where the knob works) can drive it. The Hold/Reconnected halves
of the neighbouring multi-output remainder *were* proven live on the
same rig the same day — see `multi-output-remainder.md` follow-up 1 —
including the runbook (synthetic trigger required, ~2–5 s `force`
latch, card0 sentinel).

## Update 2026-09-27: #48 closed on unit-test evidence; live proofs deferred

**User decision:** close gh #48, accepting the unit tests plus code
inspection as the bar for both paths for now, and "fix later". This entry
stays open, at low priority, as the record of the two live proofs still
owed:
- **Path 1** (`NewMode`): pinned by `hotplug/heads.rs`
  `a_new_mode_on_any_head_is_followed_on_that_head_only` and
  `hotplug/tests.rs`.
- **Path 2** (`MoveTo`): pinned by `hotplug/heads.rs`, e.g.
  `every_screen_unplugged_with_one_connected_keeps_the_primary_and_moves_it`.
  Since phase E it fires only when every driven screen goes away and a
  connected, undriven connector is present in the same event (a dock or
  KVM swap).

Everything else #48 asked for is proven live:
- **Add on plug:** `Asahi.md` Tests 3 and 12–13.
- **Remove, with windows adopted and restored:** Tests 3 and 11–13.
- **Hold and Reconnected** when the only screen goes: on vkms, 2026-09-26
  (`multi-output-remainder.md`).

**Lead for path 1, unconfirmed.** vfkit is not installed on the dev Mac;
both VMs are QEMU. During a probe of the QEMU dev VM (`virtio-gpu-pci`,
Cocoa display, kernel 6.18.50), `card0-Virtual-1`'s list changed twice
with a `HOTPLUG=1` change event each time, at VM clock 02:31:55 and
02:32:37:
- first 1600x1000 preferred, 9 modes → 1440x900 preferred, 8 modes;
- then 1440x900 preferred, 9 modes.

The cause is **not attributed**. The Cocoa window would not resize, and
another worktree was using the VM at the time, so a guest-side change
cannot be ruled out. Next time, on an otherwise idle VM, check whether
QEMU Cocoa's resize, full-screen or display move (with View → Zoom To Fit
on) changes the list. If it does, that is a vfkit-equivalent driver for
path 1 on the existing rig. The probe logs are on the VM under
`~/evidence/newmode/`.

