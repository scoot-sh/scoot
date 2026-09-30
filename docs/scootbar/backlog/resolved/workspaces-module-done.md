---
title: "Workspaces module: numbers, the active one marked, click to switch"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M2"
resolved: "2026-09-29"
---

# Workspaces module — RESOLVED

Resolved 2026-09-29 (PR #334, squash-merge `646293e`). What landed:

Filed 2026-09-29. Serves **daily-drive**; the headline of the second milestone,
and the first daily-usable bar.

An `ext-workspace-v1` client that lists each output's workspaces, marks the
active one (a rounded pill, the look the bar sets), redraws on `done`, and
switches with `activate` then `commit` on a click. It is the one module with
a custom draw hook, because it is per-output and shapes the bar's look.

## Facts from `docs/protocols.md` to design around

- One group per output; each bar shows its own output's group.
- Handles are positions, not identities, and scoot sends no `id`: redraw from
  what the last `done` said, never remember a handle. Sort by `coordinates`
  (not name: `"10"` sorts before `"2"`).
- A workspace adopted from an unplugged monitor is named `"2 DP-1"`; show the
  number, and decide how (or whether) to mark adopted ones.
- The list grows and shrinks with the trailing empty workspace, renumbering
  what follows.
- The only state bit scoot sends is `active`. Occupied and urgent need
  scoot-side work (below); the first version needs neither.
- `activate` on a non-focused output is dropped today.

## Pointer in this milestone

Click-to-switch needs pointer input, but the general mechanism (module trait
`on_input`, the config keys, actions) is a later milestone
([pointer-and-interactions](pointer-and-interactions.md)) and this one must ship
without it. So this module carries its **own minimal hit test**: it knows its pill
rects, takes a `wl_pointer` button press over one, and sends `activate` then
`commit`. No config, no scroll, no hover. The later entry generalizes that code
rather than replacing the behavior.

## What the research says about this module

Workspaces are the module that breaks most often in other bars: the Waybar
Hyprland workspace module alone has some 150 issues, several with dozens of
comments, most of them compositor-version churn (research report, 2026-09-29;
issue numbers in it). Two consequences:

- **Bind only to `ext-workspace-v1`**, no compositor-specific path, and test this
  module hardest: rapid switching, a compositor restart, `finished`, output
  hotplug, and both scoot and sway.
- **The most-requested feature is persistent workspaces** (Waybar #1629: 44
  comments, 47 hearts): always show slots 1..N, even empty. In scoot the set is
  dynamic (empty workspaces are dropped, and `focus-workspace-index` past the end
  does nothing, `docs/ipc.md`), so a bar that drew N slots would show ones it
  cannot switch to, which is a lie. **Default: show what scoot reports.** Fixed
  slots need a scoot concept first
  ([persistent workspaces](../../backlog/core/persistent-workspaces.md)); the bar
  should not synthesize them.

## Scoot-side follow-ups (filed, built when this module needs them)

- [workspace snapshot event](../../backlog/ipc/workspace-snapshot-event.md):
  dim empty workspaces
- [output-targeted workspace switch](../../backlog/ipc/workspace-switch-targeted-output.md):
  click on a non-focused monitor's bar
- [`urgent` state bit](../../backlog/protocols/ext-workspace-urgent-state.md)

Each is optional: the module degrades to "all shown alike" and "clicks on the
focused output only" without them, and works unmodified on other compositors.

## Tests

Headless scoot with two outputs: the pill follows a switch, a click switches,
a workspace appearing and disappearing redraws, hotplug rebuilds, a manager
that finishes mid-batch is not drawn half-updated. Check that a `done`-less
batch never triggers a redraw and that a window-churn flood costs no more than
one redraw per `done`.

## Done when

Time and workspaces on every output, verified by screenshot, at real scale.

## Resolution

- **The module** (`crates/scootbar/src/modules/workspaces/`, feature
  `workspaces`): an `ext-workspace-v1` client behind three defaulted
  `Module` hooks that earlier modules keep untouched — `on_dispatch`
  (state arriving on the connection fd), `on_click` (a press in the
  module's span), `custom_draw` (the active pill). Per-output numbers
  sorted by coordinates, active marked, redrawn from the last `done`
  only; handles never retained (positions, not identities); adopted
  names (`"2 DP-1"`) shown by number. Click sends `activate` then
  `commit` through its own minimal hit test over the pill rects; no
  config, no scroll, no hover — [pointer-and-interactions](../pointer-and-interactions.md)
  generalizes it later.
- **The pill is rectangular**, an accent fill with the number in the
  bar's background: the canvas has no rounded shape until
  [appearance](appearance-done.md) brings one (with its snapshots).
  Default layout unchanged (clock center); `--left workspaces` places it.
- **Review finding, fixed before merge**: a click landing on a pill
  whose workspace was `Removed` since the last `done` would `activate`
  a destroyed handle — a protocol error killing the bar. `on_click`
  now requires the handle to still be staged (commit `7bd7c76`).
- **Evidence** (at the merge): `cargo nextest run -p scootbar`
  206/206; clippy `-D warnings` clean on default, no-default,
  clock-alone and workspaces-alone builds; `fmt --check` clean;
  headless scoot ×2 outputs — pill follows a switch, click switches,
  appear/disappear redraw, hotplug rebuild, `finished`-mid-batch and
  `done`-less batches never redraw, one redraw per `done` under churn;
  screenshots at real scale showing time and workspaces on every output.
- The three scoot-side follow-ups stay open and optional (snapshot
  event, output-targeted switch, `urgent` bit); without them the module
  degrades as the ticket describes.
