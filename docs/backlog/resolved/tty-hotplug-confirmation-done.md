---
title: "`--tty` hotplug follow-up: both unreproduced paths confirmed live — RESOLVED."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# `--tty` hotplug follow-up: both unreproduced paths confirmed live — RESOLVED.

Resolved 2026-09-28 by live measurement on the dev VM (QEMU `virtio-gpu-pci`,
Cocoa display, kernel 6.18.50; card0 `Virtual-1` connected / `Virtual-2`
disconnected, `max_outputs=2`). Both paths #48 left open are now proven on
real hardware paths, with the exact commands, log lines and screenshots
below — not a paraphrase. No code changed: the session ran `/var/cargo-target/debug/scoot`
built from `87bb36469` (tree clean, `md5 cd677c76fde2617e964f9249020859ea`).

## The entry as filed

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

## Update 2026-09-27: #48 closed on unit-test evidence; `MoveTo` live proof next

**User decision:** close gh #48, accepting the unit tests plus code
inspection as the bar for both paths for now, and "fix later". Review then
found `MoveTo` reachable by an ordinary action, with its DRM half never
run (below). The user kept #48 closed but kept this entry at **medium**:
prove `MoveTo` live on the dev VM as soon as it is idle. The two live
proofs still owed:
- **Path 1** (`NewMode`): pinned by `hotplug/heads.rs`
  `a_new_mode_on_any_head_is_followed_on_that_head_only` and
  `hotplug/tests.rs`.
- **Path 2** (`MoveTo`): the *decision* is pinned by `hotplug/heads.rs`,
  e.g.
  `every_screen_unplugged_with_one_connected_keeps_the_primary_and_moves_it`.
  It fires whenever no driven connector is still connected and some other
  connector is (rule 1 of the `heads.rs` module doc). That includes the
  first uevent after a `Hold`, not only a same-event swap, so an ordinary
  case reaches it: a one-monitor desktop, with the monitor unplugged, then
  a monitor plugged into a different port. It also targets any connector
  `add_head` turned down (`MAX_OUTPUTS`, no free CRTC, surface or presenter
  refused), which stays connected and undriven.
  **The DRM half has never run anywhere**, not live and not in a unit test:
  `retarget`, `set_pending`'s connector-change branch, and the
  `switch_crtc` fallback. `hotplug/tests.rs` states `switch_crtc` as
  unverified, and `resolved/tty-drm-hotplug-done.md` records that
  `NewConnector` never ran on the one-connector VM. `retarget` is written
  so a refusal leaves the session running and the next uevent retries.
  The realistic worst case is a black screen on the new monitor with the
  session alive, not a crash.

Everything else #48 asked for is proven live:
- **Add on plug:** `Asahi.md` Tests 3 and 12–13.
- **Remove, with windows adopted:** Tests 3 and 11–13. **Restored on
  replug:** Tests 11–13 (in Test 3 the returned monitor came back empty,
  the reconnect gap PR #249 later fixed).
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

**Lead for path 2, untried.** The dev VM's card0 now has two connectors
(`Virtual-1` connected, `Virtual-2` disconnected; `max_outputs=2`), unlike
the one-connector VM of PR #51. Forcing `Virtual-1` off and `Virtual-2` on
through debugfs `force`, each paired with a synthetic
`udevadm trigger --subsystem-match=drm --action=change` (the vkms runbook
in `multi-output-remainder.md`), may drive `MoveTo`, including its DRM
half, live on the existing rig. Run it only on an otherwise idle VM.

## Resolution 2026-09-28: both paths confirmed live on the dev VM

One `--tty` session (dumb tier, `foot -a proofwin` as the single client),
driven entirely from the idle dev VM over `ssh -p 2222 dev@localhost` plus
QEMU Cocoa window operations from the Mac side. Seat check first: no scoot
processes, load 0.02, logind sessions idle 8h, last `~/evidence` activity
~4h earlier; active VT `tty1` throughout (`/sys/class/tty/tty0/active`;
`fgconsole` has no console fd over ssh). All evidence on the VM under
`~/evidence/hotplug-confirm-20260928/` (`poll.log`: 0.5 s sysfs
status+modes samples; `udev.log`: `stdbuf -o0 udevadm monitor
--subsystem-match=drm`; `scoot-tty.log`: the session; six IPC screenshots).

### Path 1 (`NewMode`) — CONFIRMED, both directions

The unattributed lead is attributed: **QEMU Cocoa resize and fullscreen
with View → Zoom To Fit ON change card0-Virtual-1's mode list with a
`change` event.** Zoom To Fit was OFF (no `AXMenuItemMarkChar`), and a
resize with it off changed nothing (guest list steady, no uevent). After
enabling it (`click menu item "Zoom To Fit"`, mark `✓`):

- 08:02:35 UTC: window 800x528 → 893x586 (aspect-locked). KERNEL+UDEV
  `change` on card0 at 29975.059; sysfs list flipped `1600x1000`-first/26 →
  `1440x900`-first/8 between poll samples 08:02:36.14 and 08:02:36.65;
  modetest preferred 1600x1000 → 1786x1116 (= window content 893x558 @2x).
  This is the prior probe's 02:31:55 signature exactly — someone resized the
  QEMU window with Zoom To Fit on.
- A windowed resize to a non-16:10 size is refused by QEMU (window stayed
  893x586, no event); Enter Fullscreen instead drove the live proof.

Live, session up on Virtual-1 at 1786x1116 (`before.png`, foot `proofwin`
open, `wlr-randr`: `1786x1116 preferred, current`):

- 08:09:24 fullscreen on → `Device changed: #57856` 08:09:25.49 →
  `drm: display reconfigured; mode-setting onto it connector=Virtual-1
  width=3456 height=2170` → `drm: modeset (full commit)` 41 ms later.
  IPC `outputs`: 3456x2170; foot re-laid-out to 1710x2146;
  `after-fullscreen.png` 3456x2170 (viewed: terminal, ring, cursor).
  `wlr-randr` then listed 1786x1116 *and* 3456x2170 preferred+current —
  `wl_output.mode`/`done` reached clients.
- 08:10:15 fullscreen off → `Device changed` 08:10:16.84 →
  `reconfigured; mode-setting onto it connector=Virtual-1 width=1778
  height=1116` → `modeset (full commit)`. modetest preferred 1778x1116;
  `wlr-randr` three modes, 1778x1116 preferred+current;
  `after-exit-fullscreen.png` 1778x1116.

### Path 2 (`MoveTo`, incl. the never-run DRM half) — CONFIRMED

Rig notes for the next run (virtio-gpu card0 differs from the vkms
runbook): both `force` files exist but `echo unspecified > force` fails
with `Invalid argument` (same as vkms — restore means explicit `on`/`off`,
functionally pristine here); and **sysfs `status`/`modetest` reads go stale
for minutes after a force write** (V1 read `connected` for 30 polls after
`force=off` latched; V2 read `disconnected` for 3+ min after `force=on`) —
scoot's uevent-driven re-probe is the authoritative read, never the first
sysfs poll. A `force` write on virtio-gpu emits a natural `change` event of
its own (30435.45 after the V1-off write), but that one produced **no
session log line at all** (seen by `udevadm`, never `Device changed`;
mechanism unconfirmed) — the synthetic trigger pairing stays required.

- 08:11:00 `echo off > .../Virtual-1/force` (V2 still off); trigger
  08:13:26 → two `Device changed` (#57984 renderD128, #57856 card0) →
  `drm: nothing is connected to this device any more; holding the last
  frame. The session keeps running -- plug a display back in and scoot
  mode-sets onto it.` Session answered `version`; `hold.png` 1778x1116,
  `cmp`-identical to `after-exit-fullscreen.png`.
- 08:14:08 `echo on > .../Virtual-2/force`; trigger 08:17:36 →
  `Device changed` x2 → `retarget`'s `set_pending` connector-change branch
  refused on the live CRTC 37 (`drm: could not move the surface onto the
  new connector in either order; staying on the current one`) → the
  `switch_crtc` fallback built on CRTC 44 and probed green (`create_surface
  crtc=44 mode=1024x768 ... connectors=[45]`) →
  `drm: display reconfigured onto a different crtc; mode-setting onto it
  connector=Virtual-2 crtc=crtc::Handle(44) width=1024 height=768` →
  `drm: modeset (full commit)` 20 ms later (`Setting new mode: "1024x768"`).
- Post-move health: session alive; single output kept (id 1, still named
  `Virtual-1` — the documented stale-name behaviour), now 1024x768; foot
  still output 1 / workspace 0 / focused, resized to 494x744 (windows
  followed); `wlr-randr` four modes with 1024x768 preferred+current;
  `after-moveto.png` 1024x768 (viewed: terminal, ring, cursor — never
  black). No crash, no client kill: the realistic worst case from the
  ticket (black screen, session alive) did not occur — both tiers of the
  DRM half ran and the modeset committed.

Restore (bonus designed-path coverage): V1 `force=on` + trigger 08:20:39
→ `driving a newly connected display` / `added an output for it
connector=Virtual-1 output=2` on CRTC 37 at 1776x1116 (phase-E add), then a
natural event 1 s later → `NewMode` on the V2 head to 5120x2160 (forced-on
default list head). V2 `force=off` + trigger 08:21:08 → `this connector
went away` / `removing its output output=1`; foot `adopted=true` on output
2 at 870x1092; `final.png` 1776x1116 (viewed). Session stopped after;
nothing running, no DRM clients, VT still `tty1`, connectors back to
connected/disconnected, forces explicit `on`/`off`. Mac side restored:
window 834,184 800x528, Zoom To Fit off; guest list back to
1600x1000-first/9.

Nothing stays open: path 1's `NewMode` and path 2's `MoveTo` (decision in
`hotplug/heads.rs`, DRM half in `retarget`/`set_pending`/`switch_crtc`)
have all run live. The `hotplug/tests.rs` `switch_crtc`-unverified note and
`resolved/tty-drm-hotplug-done.md`'s `NewConnector`-never-ran record are
superseded by this run (a runtime add also ran live at 08:20:39).

