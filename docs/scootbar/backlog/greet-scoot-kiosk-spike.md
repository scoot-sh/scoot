---
title: "Spike: can scoot itself host the greeter (a locked-down kiosk mode), and what it costs"
status: "open"
area: "scootbar"
priority: "low"
blocked: "greet-nixos"
milestone: "M7"
---

# Spike: scoot as the greeter's host

Filed 2026-10-12 from the [scootgreet design](../../../dev/design/scootgreet.md#which-compositor-hosts-it). Serves
**daily-drive** (one compositor in the closure, GPU-free by construction).

## What to do

A throwaway scoot branch with the kiosk flags the design lists (no IPC socket, VT-switch binds
only, root-owned config only, no XWayland, exit with the child, a reduced global list), run as
the greeter's host on the dev VM and the M2. Measure the handover into a scoot user session
(DRM master through logind, `live: true` after; the unmeasured logind path in
`docs/backlog/testing/logind-session-reconnect-test.md`), closure and RSS against cage. A
security read of every global still exposed.

## Acceptance

A recorded go/no-go for [scoot-greeter-mode](../../backlog/core/scoot-greeter-mode.md), with
the flag list that ticket builds.

## Evidence the review needs

The handover transcripts and `scoot msg outputs` after, the global list, the numbers with SHAs.

## Not in this ticket

Shipping any of it.
