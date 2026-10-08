---
title: "Multi-output policy: which outputs get a bar, per-output overrides, shared state"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M3"
resolved: "2026-09-30"
---

# Multi-output policy

## Resolution (2026-09-30)

Landed: `outputs = "all" | [connector names]` (and `--outputs`),
`[output."NAME"]` overrides for `edge`, `layer`, `exclusive`, `height`,
`margin` and the `left`/`center`/`right` lists, applied when an output
settles (its name is known then) and on every reload. Reference:
[cli.md](../../cli.md#outputs). Modules are started once from the union of
every output's lists and each output's scene shows its own members, so a
second output adds a surface and its buffers, not fds (measured: 8 fds with
one output and with two, `tests/outputs.rs`); the hidden/visible state
machine gained a second, independent reason for a surface to be absent
(unselected), so hide, show and reload compose per output. Verified on
headless scoot with two outputs (`tests/outputs.rs`: different module sets,
heights and edges from one config; the list; reload moving the bar; hide and
show keeping to the list; the fd count) and on a headless sway
(`tests/hotplug.rs`: the list decides which plugged output gets a bar, and a
storm of plugs and unplugs leaves every module still ticking, with no fd or
buffer leak).

Not built, and why: a per-output **`font-size`** (the em is the same logical
size everywhere; each bar still draws at its own output's real device
pixels, so it scales with the output; a per-output size needs the style
threaded per output, filed as
[multi-output-font-size](multi-output-font-size-done.md)); the per-output
**workspace switch** waits on scoot's
[output-targeted switch](../../backlog/ipc/workspace-switch-targeted-output.md)
(open): each bar already shows its own output's workspaces, but only the
focused output's bar can switch, documented in
[cli.md](../../cli.md#workspaces). scoot has no runtime output hotplug on
its headless backend, so the hotplug storm runs on sway, as
`tests/hotplug.rs` already did.

Filed 2026-09-29. Serves **daily-drive** (a second monitor).

The skeleton gives every output a bar. Real setups want to choose.

## Facts (`docs/protocols.md`, layer shell)

- A surface must name its output (`get_layer_surface` with a `wl_output`); one
  that names none lands on the compositor's choice, the first output.
- Each output keeps its own exclusive zones, and a bar reserves only its own
  output's edge.
- Workspaces are per output (one group each); volume, battery and the clock are
  the same everywhere.

## What to decide and build

- **Which outputs**: `outputs = "all"` (default) or a list of connector names
  (`DP-1`); an output that appears later matching the list gets a bar, one
  that leaves loses it.
- **Per-output overrides**: height, edge, module lists, scale-dependent font
  size. Example: the tray and battery only on the laptop panel.
- **Shared vs per-output modules**: one module instance whose model is shared
  and whose `view(output)` may differ (workspaces do, most do not). Data
  sources (netlink, audio) are opened once, never per output; measure that a
  second monitor adds one surface's buffers, not a second set of fds.
- **"Primary"**: scoot has no such concept for clients beyond the first output;
  do not invent one in the bar. If a module should appear "once, on one output",
  name the output in config.
- Different scales per output: each bar draws at its own real device pixels
  (see [skeleton](resolved/skeleton-layer-surface-done.md)); fonts scale with it.
- A per-output workspace switch depends on scoot's
  [output-targeted switch](../../backlog/ipc/workspace-switch-targeted-output.md).

## What other bars get wrong

Crashes on output disconnect, modules that stop updating after hotplug, and
wake-from-DPMS segfaults are the multi-monitor complaints that recur (Waybar #2808,
#4823, #1019). The rule that prevents them is already in the design: **per-output
surface lifecycle is separate from shared data sources**, so removing an output
destroys a surface and nothing else. Pin it with a hotplug storm test that
re-checks every module still updates afterwards.

## Done when

Two headless outputs show different module sets from one config, hotplug
adds and removes bars per the list, and a second output adds no new
data-source fds.
