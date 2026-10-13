---
title: "A style-only reload keeps every module running and repaints"
status: "open"
area: "scootbar"
priority: "medium"
blocked: null
---

# A style-only reload keeps every module running and repaints

Filed 2026-10-12, split out of [`look-layer-scootbar`](look-layer-scootbar.md)
for the [`live-looks`](../../backlog/core/live-looks.md) epic, and useful on
its own. Serves **daily-drive**.

## The gap

Every reload restarts every module except an unchanged `exec`
(`crates/scootbar/src/modules/mod.rs:615-661`), so a colors-only reload tears
down and rebuilds every D-Bus client, and it closes any open popup
(`daemon/surfaces.rs:260-266`).

## What to do

A reload whose only differences are style (`[colors]`, the `[bar]` style
keys, the font) keeps every module and its state and repaints; only a
layout or module-table change restarts modules.

## Evidence to record

Before/after for a colors-only reload: wall time, CPU, allocations, D-Bus
reconnects (zero after); a module's subscription survives; the resource
ratchet unchanged.

## Not in this ticket

The look layer.

The design this implements against: [live looks](../../../dev/research/live-looks-design.md).
