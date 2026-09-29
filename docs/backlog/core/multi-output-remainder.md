---
title: "Multi-output remainder: --tty multi-CRTC, placement policy, default binds (milestone 19 phases E–I)"
status: "open"
area: "core"
priority: "high"
blocked: "E1/E2 landed and ran on hardware (a physical DP-1 replug on the Asahi M2 Air, 2026-09-25; GPU-tier runtime add and multi-head mode change proven there 2026-09-26, Asahi.md Tests 11-12); #48 MoveTo proven live on the dev VM 2026-09-28 (Virtual-1 force-off / Virtual-2 force-on, switch_crtc to CRTC 44 — see tty-hotplug-confirmation-done.md); the scale/mode surface landed 2026-09-29 (per-output-scale-mode-done.md); left: position and a live mode change (output-position-and-live-mode)"
---

# Multi-output remainder: --tty multi-CRTC, placement policy, default binds

Milestone 19 ([plan](../roadmap/19-multi-output.md)) landed phases A–D + F
(render/capture, layer shell, lock, workspaces/output-management/pointer,
cross-output moves). This entry tracks everything left before the README
"Not yet: multi-output is partial" bullet clears. The per-output
scale/mode surface was its own entry
([per-output-scale-mode](../resolved/per-output-scale-mode-done.md)) and
landed last (2026-09-29); what it left out -- position, a live mode change
-- is [output-position-and-live-mode](./output-position-and-live-mode.md).

## E1 — TTY multi-CRTC enumeration + output registration — DONE 2026-09-25 (`b782b06`)

Landed as specified, with two corrections found in the code. First,
CRTCs are matched from `possible_crtcs` (`tty/crtcs.rs`), not probed with
`create_surface`, which refuses only a claimed primary plane. Second, the
"two-connector fake `ControlDevice`" pin is impossible as written: `drm`
0.14.1's `connector::Info` has only `pub(crate)` fields and no constructor
(`drm-0.14.1/src/control/connector.rs:51-61`). The seam is pure functions
over handles and sizes instead: `gpu::search_all`, `crtcs::assign` and
`hotplug::heads::replan`. See `docs/roadmap/19-multi-output.md` phase E for
the record.

`tty/gpu.rs` picks the first `Connected` connector
(`find_connector_and_mode`, `:491-516`) into a singular `OpenGpu`
(`:71-85`); hotplug switches rather than adds (`tty/hotplug.rs:25-43`).

- Generalize `search` (`tty/gpu.rs:561-570`) from first-wins to collecting
  all `Connected`-with-modes triples, preserving kernel order. Keep the
  single-output wrapper so the one-connector path stays byte-identical.
- Startup: one output per triple via the `headless::init_named` pattern
  (`headless.rs:153-208`), real `wl_output` names from `connector_mode`
  (`tty/gpu.rs:621-624`), `OutputId` via `outputs.add`, per-output render
  target (Phase A shape). Side-by-side logical tiling from each
  connector's mode size. `--mode` applies per connector independently.
- Pins: two-connector fake `ControlDevice` unit tests (`tty/gpu.rs`
  tests, `:707+` pattern); harness multi-CRTC bind test.

## E2 — TTY per-connector rendering + hotplug add/remove — DONE 2026-09-25 (`2bd7d47`)

Landed: per-head presenters, a render loop keyed by `OutputId`, per-output
session-lock vblank waits (a security fix; see the milestone record), gamma
per CRTC, and absolute pointer/tablet mapping over the union. Hotplug adds
and removes outputs (`hotplug::heads::replan`, `State::remove_output`),
with the last output never removed. Live on the Asahi M2 Air, including a
physical DP-1 unplug and replug (17:59Z): output 2 removed, output 3 added
on CRTC 68 with a modeset. The kernel reported the disconnect that time,
unlike a quicker unplug that morning.

**Follow-ups closed here (why this file stays open is item 3 below plus the scale/mode surface):**

1. **The rest of the hotplug paths on hardware — all proven, nothing open.**
   Unplug and replug were confirmed on the dumb tier at E2 merge. The three
   paths still unexecuted then are all proven live now (updates below), and
   the full matrix is recorded in
   [`tty-hotplug-confirmation-done.md`](../resolved/tty-hotplug-confirmation-done.md)
   (RESOLVED 2026-09-28).

   **Update 2026-09-26 (dev VM, vkms `Virtual-3`, dumb tier): the
   Hold/Reconnected halves are now proven live; `MoveTo`, the GPU-tier
   add and the multi-head mode change stay open.** `scoot --tty --gpu
   /dev/dri/card1` drove the single virtual head at its preferred
   1024x768 with a foot window; each forced change was paired with a
   synthetic `udevadm trigger --subsystem-match=drm --action=change`
   (vkms `force` writes emit no uevent of their own — `udevadm monitor`
   shows only `SYNTH_UUID` events — and the synthetic `drm_minor` change
   for card1 reaches `reconfigure`: card1 is seat-tagged, no "not in
   udev's list" warning at startup). Binary built from `187da3c` with no
   code changes; full log at `/tmp/vkms2.log` on the VM, screenshots at
   `/tmp/v2-{before,hold,reconnected}.png` (hold and reconnected shots
   byte-identical to before via `cmp`).
   - **Hold:** `force=off` + trigger → `Device changed: #57857`
     (226:1, card1) then `drm: nothing is connected to this device any
     more; holding the last frame`. Output kept (`Virtual-3` 1024x768,
     never removed), session answered `version`/`outputs`/`windows`,
     screenshot identical. A second trigger while still disconnected
     stayed silent (no duplicate WARN, modeset count unchanged) — the
     once-per-disconnection arm.
   - **Reconnected:** `force=on` + trigger → `drm: a display is
     connected again; forcing a modeset`, then `drm: modeset (full
     commit)` 21 ms later with zero commit-fail/ERROR lines. Output
     unchanged, screenshot identical, `wlr-randr` still shows 1024x768
     preferred+current. A further trigger plans `Keep` (`hotplug changed
     nothing`), proving `nothing_connected` cleared.
   - **Sentinels:** card0 (the QEMU window) never forced — `Virtual-1`
     connected / `Virtual-2` disconnected with identical modes before and
     after; session survived all cycles; seat released, vkms left loaded
     pristine (`force=unspecified`, empty override) for the follow-ups.
   - **Rig note for the next run:** a `force` write latches after ~2–5 s
     — early `modetest`/sysfs reads show the *old* state while scoot's
     slightly later re-probe already sees the new one. Poll `modetest`
     to the intended state (up to ~15 s) *before* triggering; never
     trust the first probe. `force` cannot be written back to
     `unspecified` (`Invalid argument`) and a stored `edid_override`
     cannot be cleared — `rmmod`+`modprobe vkms` is the reset (connector
     name stayed `Virtual-3` across reloads here).
   **Update 2026-09-26 (Asahi M2 Air, physical hands, `main` `87e1935`):
   the GPU-tier runtime add and the multi-head mode change are proven;
   only `MoveTo` stays open.** Full record: `Asahi.md` Test 12 (the
   dumb-tier mode change and virtual-pull remove/restore are Test 11).
   Powering on a sleeping DP-1 under `--tty --renderer gles` added a
   GPU-scanout head (second EGL context beside the live one) 50 ms after
   the hotplug event. A real cable pull adopted its window, and the replug
   restored it under a fresh id 3 with an identical rect. Every remove
   returned fds to exactly 51, so nothing leaks per head. Two 3-fd
   differences between like states are unexplained; the table is in Test 12.
   `--mode 1280x720` drove DP-1 at
   1280x720 beside eDP-1's native mode, and a replug in that session came
   back at 1280x720. `MoveTo` still needs the only lit screen to be
   pullable, and the Asahi panel is not.
   **Update 2026-09-28 (dev VM card0, dumb tier): `MoveTo` is proven live;
   nothing in this follow-up stays open on its account.**
   `Virtual-1 force=off` + synthetic trigger → Hold (last frame held,
   `hold.png` byte-identical); `Virtual-2 force=on` + trigger → `MoveTo`
   with the full DRM half (`set_pending` refused on CRTC 37, `switch_crtc`
   to CRTC 44, modeset committed, session never black). Full record:
   [`tty-hotplug-confirmation-done.md`](../resolved/tty-hotplug-confirmation-done.md).
   Card0 rig deltas vs the vkms runbook above: `force` writes emit a
   natural `change` event of their own, but it produced no session log line
   — keep pairing every write with a synthetic trigger; sysfs `status` and
   `modetest` stay stale for minutes after a write (scoot's re-probe is the
   authoritative read); `echo unspecified > force` fails with
   `Invalid argument`, so restore means explicit `on`/`off`.
2. **Reconnect restore and output ids on replug.** Now its own ticket,
   [output-reconnect-restore](../resolved/output-reconnect-restore-done.md)
   (RESOLVED 2026-09-25, PR #249): match a
   returning monitor by connector identity, give its windows and workspaces
   back, and have the default output-2 binds follow the connector, not the
   id. **Subsumed by that ticket's implementation** (positional default binds
   for the first/second screen plus workspace restore on identity match) —
   this follow-up needs no separate work.
3. **Per-output render scheduling — NOT PLANNED (optimisation, not a correctness gap).**
   A render walks every output: with damage on one screen the other costs a
   no-damage pass plus a frame-callback round, measured at ~1 pp of CPU with
   the second screen idle (milestone 19 Phase E decided-record,
   `../roadmap/19-multi-output.md:399-404`; Asahi jiffies 36.20% eDP-1-only
   vs 35.20/35.67% single-output for the same workload). Per-output
   scheduling would save only that pass and stays parked until a measurement
   shows it matters — do not re-file it as a correctness gap. Adjacent and
   already resolved, not this item:
   [`arrange-per-output-per-frame`](../resolved/arrange-per-output-per-frame-done.md)
   (RESOLVED 2026-09-26, PR #267: one arrange per frame, hoisted out of the
   per-output walk).

- One `DrmSurface`/CRTC per driven connector; render loop walks all TTY
  outputs like headless Phase A.
- Hotplug plug of a second connector = `OutputAdded`, not
  reselect-stay-put; unplug of a driven connector = per-output teardown +
  `OutputRemoved` (new — `outputs.rs` has `add` only today; core already
  handles `OutputRemoved`, `scoot-core/src/world/events.rs:17`, focus
  fixup `:92-121`). Windows on a removed output follow core removal
  semantics (`world/tests/outputs.rs:45`).
- `wl_output` rename limitation (`hotplug.rs:35-43`) becomes per-output
  create/destroy. Absolute-pointer/tablet mapping (`tty/mod.rs:1354-1420`)
  stops using `primary()` (mirror Phase D union-clamp, `input.rs`
  `clamp_to_output_union`). Gamma per CRTC.
- Pins: plug-adds, unplug-removes, reselect-stays-put regression
  (`hotplug.rs` tests, `:45-46`; existing `reselect` contract
  `gpu.rs:523-536` preserved for the *first* plug).

## G — New-window placement policy — LANDED 2026-09-21 (PR #208)

Landed as specified: new windows file under the pointer's output
(`shell.rs:62-71`, `pointer_output().and_then(...).or_else(primary_id())`), the milestone focus
doctrine (`../roadmap/19-multi-output.md:21-25`); default output binds exist
alongside (item H). What follows is the original spec, kept as the design
record.

`shell.rs:32` files `WindowOpened` with `world.outputs().first()`;
`output_of_window` falls back to primary
(`foreign_toplevel_management.rs:317-331`). Recommended: **pointer's
output** (matches milestone focus doctrine `:21-25`), else `primary_id()`.
Update the fallback comment and `outputs.rs:133-142` docs. Pins:
`shell/tests`, `foreign_toplevel_management/tests/outputs.rs` template,
IPC `output` field assertion.

## H — Default binds for FocusOutput / MoveFocusedWindowToOutput — LANDED 2026-09-21 (PR #208)

Landed: `Super+comma`/`Super+period` (+ Shift for moves) name the
first/second screen and resolve positionally at dispatch (ids are never
reused; stability rule in `docs/configuration.md#moving-across-outputs`).
What follows is the original spec, kept as the design record.

Wire + grammar exist (`config.rs:758-759`, `cli.rs:43`, documented manual
binds `configuration.md:443-462`); `Keybindings::default`
(`keybindings.rs:103-214`, 36 entries) has neither. Candidates:
`Super+comma/period` + Shift (already the documented example) or the
`Super+Ctrl` tier. Decide explicitly like the workspace-index decision.
Updates: defaults table, `default_config_toml` emission, 36-count
assertion (`config.rs:2737`), docs tables, `--print-default-config`.
Ids 3+ stay manual (`MAX_OUTPUTS=8`, `cli.rs:68-81`).

## Ordering and verification

E1 → E2 → G → H; scale/mode surface after E (hardware truth first).
Harness throughout: `--headless --outputs 2` (per-output screenshots vs
`grim -o headless-2`, both `wl_output`s, both heads); single-output
byte-identical. Asahi runbook: extend `Asahi.md` Test 3 — boot `--tty`
with two connectors, `scoot msg outputs` names both, unplug-driven →
fallback, plug-second-while-running → added and session stays on panel.

## What done looks like

E2, G and H are done (above). The `cli.rs:181-193` "one render target"
comment this section used to cite is already current — it describes
per-output render targets (verified 2026-09-28, no edit needed) — and the
stale `multi-output.md:50-113` body (A–D as future) is superseded by this
triage (`../resolved/multi-output-superseded.md`, milestone 19
authoritative). The [scale/mode surface](../resolved/per-output-scale-mode-done.md)
landed 2026-09-29; left: [position and a live mode
change](./output-position-and-live-mode.md), and the README bullet is
narrowed to **Monitor placement** accordingly.
When it lands, the README "Not yet" bullet — already narrowed to
**Per-monitor settings** ("Multiple monitors work, but they share one
scale and resolution…", `README.md:174-175`, retitled past one-output in
`../resolved/readme-rereview-done.md`) — clears.
