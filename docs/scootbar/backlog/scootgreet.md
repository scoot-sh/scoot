---
title: "`scootgreet`: the login greeter (epic)"
status: "open"
area: "scootbar"
priority: "low"
blocked: "greet-baseline, greet-greetd-client, greet-ui, greet-nixos, greet-packaging-iso, greet-default-flip"
milestone: "M7"
---

# `scootgreet`: the login greeter

Filed 2026-10-12 at the user's request ("And scootgreeter or scootgreet"). Serves
**daily-drive** and **computer use**. The login screen would draw the session's look without
ReGreet's 1,018.5 MiB of ReGreet and cage, and a VM test or agent could drive the login with one
command.

The design is [dev/design/scootgreet.md](../../../dev/design/scootgreet.md), and the parts it shares with the
other shell apps are in [shell-apps.md](../../../dev/design/shell-apps.md). The name is a recommendation
(Decisions needed 2).

## Children, in order

1. [greet-baseline](greet-baseline.md): measure ReGreet and cage. This can start now.
2. [greet-prototype](greet-prototype.md): the go/no-go gate against a real greetd, out of the
   workspace.
3. [greet-greetd-client](greet-greetd-client.md): the cancel discipline, the supervisor and the
   agreety fallback, the locked secret path, and `scootgreet login`.
4. [greet-ui](greet-ui.md), then [greet-users-sessions](greet-users-sessions.md) and
   [greet-power](greet-power.md).
5. [greet-nixos](greet-nixos.md), [greet-packaging-iso](greet-packaging-iso.md),
   [greet-default-flip](greet-default-flip.md).

Only if cage fails the GPU-free check: [greet-scoot-kiosk-spike](greet-scoot-kiosk-spike.md),
then [scoot-greeter-mode](../../backlog/core/scoot-greeter-mode.md).

## Theming

Theming is static, through the live-looks contract (PR #578 §6.4): the greeter's look is set by
the system build, never by a user's swap.

## Done when

The children are resolved, or `greet-prototype` says no and ReGreet stays, recorded here.
