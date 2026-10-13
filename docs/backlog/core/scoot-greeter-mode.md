---
title: "scoot: a locked-down greeter-host mode (no IPC, no binds but VT switch, exit with child)"
status: "open"
area: "core"
priority: "low"
blocked: "greet-scoot-kiosk-spike"
---

# scoot: a greeter-host mode

Filed 2026-10-12 from the [scootgreet design](../../../dev/design/scootgreet.md#what-scoot-would-need). Serves
**daily-drive**, opt-in only.

## The gap

Today scoot always creates its IPC socket (`crates/scoot/src/compositor/mod.rs:449`), ships 45
default binds including a terminal with no no-binds mode (`config.rs:1750-1762`), reads the
user's config dir, and does not exit when its `--` child exits
(`site/src/content/docs/scoot/index.md:104-108`). Each is right for a session and wrong for a
login screen.

## What to do

The flag set the [spike](../../scootbar/backlog/greet-scoot-kiosk-spike.md) settles: no IPC
socket, only the always-enforced VT-switch binds, config from an explicit root-owned path,
XWayland refused, exit with the child's status, a reduced global list. Exposed to NixOS as
`programs.scoot.greeter.host = "scoot"` (default `"cage"`).

## Acceptance

Tests that each restriction holds (no socket file, a default bind does nothing, XWayland
refused, exit status propagated); the M2 handover as the spike measured it; an independent
security review of the global list.

## Not in this ticket

Making it the default host.
