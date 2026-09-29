---
title: "Multi-output policy: which outputs get a bar, per-output overrides, shared state"
status: "open"
area: "scootbar"
priority: "medium"
blocked: "skeleton-layer-surface"
---

# Multi-output policy

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
  (see [skeleton](skeleton-layer-surface.md)); fonts scale with it.
- A per-output workspace switch depends on scoot's
  [output-targeted switch](../../backlog/ipc/workspace-switch-targeted-output.md).

## Done when

Two headless outputs show different module sets from one config, hotplug
adds and removes bars per the list, and a second output adds no new
data-source fds.
