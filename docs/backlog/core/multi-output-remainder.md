---
title: "Multi-output remainder: --tty multi-CRTC, placement policy, default binds (milestone 19 phases E–I)"
status: "open"
area: "core"
priority: "high"
blocked: "phase E and the scale/mode surface need real two-connector hardware — do not build blind"
---

# Multi-output remainder: --tty multi-CRTC, placement policy, default binds

Milestone 19 ([plan](../roadmap/19-multi-output.md)) landed phases A–D + F
(render/capture, layer shell, lock, workspaces/output-management/pointer,
cross-output moves). This entry tracks everything left before the README
"Not yet: multi-output is partial" bullet clears. The per-output
scale/mode surface is its own entry
([per-output-scale-mode](./per-output-scale-mode.md)) and stays last.

## E1 — TTY multi-CRTC enumeration + output registration

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

## E2 — TTY per-connector rendering + hotplug add/remove

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
