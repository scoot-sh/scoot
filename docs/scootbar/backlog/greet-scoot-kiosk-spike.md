---
title: "Spike: can scoot itself host the greeter (a locked-down kiosk mode), only if cage fails GPU-free"
status: "open"
area: "scootbar"
priority: "low"
blocked: "cage failing the GPU-free check in greet-baseline (a human removes this clause only then)"
milestone: "M7"
---

# Spike: scoot as the greeter's host

Filed 2026-10-12 from the [scootgreet design](../../../dev/design/scootgreet.md#which-compositor-hosts-it). Serves
**daily-drive**. **This runs only if `greet-baseline` shows cage cannot draw on a GPU-less box.**
Whether to add a compositor attack surface at the login screen at all is the maintainer's call.

## What to do

A throwaway scoot branch with the kiosk flags the design lists:

- no IPC socket;
- VT-switch binds only;
- root-owned config only;
- no XWayland;
- exit with the child;
- a reduced global list.

Run it as the greeter's host on the dev VM and the M2, and measure:

- the handover into a scoot user session: DRM master through logind, `live: true` afterwards,
  on the unmeasured logind path in `docs/backlog/testing/logind-session-reconnect-test.md`;
- closure and RSS against cage.

Also do a security read of every global still exposed.

## Acceptance

A recorded go/no-go for [scoot-greeter-mode](../../backlog/core/scoot-greeter-mode.md), with the
flag list it builds.
