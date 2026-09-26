---
title: "Multi-output remainder: --tty multi-CRTC, placement policy, default binds (milestone 19 phases E–I)"
status: "open"
area: "core"
priority: "high"
blocked: "E1/E2 landed and ran on hardware (a physical DP-1 replug on the Asahi M2 Air, 2026-09-25); left: GPU-tier runtime add and #48 MoveTo on hardware, reconnect restore (its own ticket), the scale/mode surface (its own entry)"
---

# Multi-output remainder: --tty multi-CRTC, placement policy, default binds

Milestone 19 ([plan](../roadmap/19-multi-output.md)) landed phases A–D + F
(render/capture, layer shell, lock, workspaces/output-management/pointer,
cross-output moves). This entry tracks everything left before the README
"Not yet: multi-output is partial" bullet clears. The per-output
scale/mode surface is its own entry
([per-output-scale-mode](./per-output-scale-mode.md)) and stays last.

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

**Follow-ups left here (why this file stays open):**

1. **The rest of the hotplug paths on hardware.** Unplug and replug are
   confirmed on the dumb tier. Still unexecuted: a GPU-tier runtime add
   (fresh `DrmCompositor` and EGL context beside a live one), the #48
   `MoveTo` fallback (needs the only lit screen to be pullable), and a mode
   change with several heads.

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
2. **Reconnect restore and output ids on replug.** Now its own ticket,
   [output-reconnect-restore](../resolved/output-reconnect-restore-done.md)
   (RESOLVED 2026-09-25, PR #249): match a
   returning monitor by connector identity, give its windows and workspaces
   back, and have the default output-2 binds follow the connector, not the
   id. **Subsumed by that ticket's implementation** (positional default binds
   for the first/second screen plus workspace restore on identity match) —
   this follow-up needs no separate work.
3. **Per-output render scheduling.** A render walks every output. With
   damage on one screen the other costs a no-damage pass, about 1 pp of CPU
   measured. This is an optimisation, not a correctness gap.

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

`shell.rs:32` files `WindowOpened` with `world.outputs().first()`;
`output_of_window` falls back to primary
(`foreign_toplevel_management.rs:317-331`). Recommended: **pointer's
output** (matches milestone focus doctrine `:21-25`), else `primary_id()`.
Update the fallback comment and `outputs.rs:133-142` docs. Pins:
`shell/tests`, `foreign_toplevel_management/tests/outputs.rs` template,
IPC `output` field assertion.

## H — Default binds for FocusOutput / MoveFocusedWindowToOutput — LANDED 2026-09-21 (PR #208)

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

E2 done (one-connector clause gone) + G done (first-output clause gone) +
H done (no-defaults clause gone) + scale/mode entry done → the bullet
leaves the README. Also refresh the stale body of `multi-output.md`
(`:50-113` describes A–D as future) and the stale `cli.rs:181-193`
"one render target" comment in passing.
