---
title: "scoot: a locked-down greeter-host mode (no IPC, no binds but VT switch, exit with child)"
status: "open"
area: "core"
priority: "low"
blocked: "greet-scoot-kiosk-spike"
---

# scoot: a greeter-host mode

Filed 2026-10-12 from the [scootgreet design](../../../dev/design/scootgreet.md#what-scoot-would-need). Serves
**daily-drive**, opt-in only. Reached only if cage fails the GPU-free check
(see [greet-scoot-kiosk-spike](../../scootbar/backlog/greet-scoot-kiosk-spike.md)).

## The gap

Each of these is right for a session and wrong for a login screen:

- scoot always creates its IPC socket (`crates/scoot/src/compositor/mod.rs:449`);
- it ships 45 default binds, including a terminal, with no no-binds mode (`config.rs:1750-1762`);
- it reads the user's config dir;
- it does not exit when its `--` child exits (`site/src/content/docs/scoot/index.md:104-108`).

## What to do

The flag set the spike settles:

- no IPC socket;
- only the always-enforced VT-switch binds;
- config from an explicit root-owned path;
- XWayland refused;
- exit with the child's status;
- a reduced global list.

It is exposed to NixOS as `programs.scoot.greeter.host = "scoot"`, with `"cage"` the default.

## Acceptance

- Tests that each restriction holds: no socket file, a default bind does nothing, XWayland is
  refused, and the exit status is propagated.
- The M2 handover, as the spike measured it.
- An independent security review of the global list.

## Not in this ticket

Making it the default host.
