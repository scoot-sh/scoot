---
title: "scoot: put an output-less layer surface on the pointer's output, as a new window"
status: "open"
area: "core"
priority: "medium"
blocked: null
---

# scoot: default output for layer surfaces

Filed 2026-10-12 from the [scootlaunch design](../../../dev/design/scootlaunch.md#keyboard-first-ux). Serves **daily-drive**.
A launcher or notification should open where the user is looking. It helps **today's** mako and
fuzzel, neither of which sets an output (`nix/modules/notifications-home.nix`,
`nix/modules/fuzzel-theme.nix`), so it is unblocked and independent of the shell apps.

## The gap

A layer surface that names no output lands on the primary output
(`crates/scoot/src/compositor/layer_shell.rs:228-239`), while a new window opens on the pointer's
output (`site/src/content/docs/scoot/protocols.md:1017-1021`). wlr-layer-shell says a null output
lets the compositor decide, generally "the one that the user most recently interacted with".

## What to do

Resolve a null output to **the output under the pointer** at the surface's initial configure, as
windows do. This is the design's recommendation
([Decisions needed (a)](../../../dev/design/shell-apps.md#decisions-needed)); the keyboard-focus output was the
alternative, and gives nothing on an empty workspace. Update the protocols page, which today says
"the first output, the compositor's choice".

## Acceptance

- A two-output headless test: with the pointer on output 2, an output-less layer surface maps on
  output 2.
- The existing layer-shell tests are otherwise unchanged.
- mako and fuzzel checked on the M2 with two monitors.

## Not in this ticket

Moving a mapped surface when the pointer moves later.
