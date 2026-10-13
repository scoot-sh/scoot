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
**daily-drive** (the login screen draws the session's look, at a fraction of ReGreet's
stack) and **computer use** (a login a VM test or agent drives with one command).

The design is [dev/design/scootgreet.md](../../../dev/design/scootgreet.md); what it shares with the other shell apps is
in [shell-apps.md](../../../dev/design/shell-apps.md). The name is a recommendation (Decisions needed 2).

## Children, in order

1. [greet-baseline](greet-baseline.md): measure ReGreet + cage; verify the three facts the
   no-stranding design rests on.
2. [greet-greetd-client](greet-greetd-client.md): the protocol, the conversation, the secret's
   path, `scootgreet login`.
3. [greet-ui](greet-ui.md): the screen; the go/no-go gate.
4. [greet-nixos](greet-nixos.md): the module and the agreety fallback; opt-in.
5. [greet-packaging-iso](greet-packaging-iso.md), then [greet-default-flip](greet-default-flip.md).

Later and opt-in: [greet-scoot-kiosk-spike](greet-scoot-kiosk-spike.md) then
[scoot-greeter-mode](../../backlog/core/scoot-greeter-mode.md) (scoot as the host instead of
cage).

## Done when

The children are resolved, or the gate in `greet-ui` says no and ReGreet stays, recorded here.
