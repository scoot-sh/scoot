---
title: "scoot: put an output-less layer surface on the pointer's output, as a new window"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-27"
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

## Resolution

Landed 2026-10-27: `new_layer_surface` resolves a null output to the
output under the pointer (`State::pointer_output`), falling back to the
primary when the pointer names no output — the same rule `open_window`
files new windows under. The protocol leaves a null output as "the
compositor decides, generally the one the user most recently interacted
with"; the pointer's output is this compositor's standing reading of
that (milestone-19 focus doctrine), now applied to layer surfaces too.
Removing the chosen output still closes the surface (`remove_output`
sends `closed`), unchanged. Cold path (once per surface creation): one
geometry scan, `Output` clones only, no allocation.

Evidence: three new headless two-output tests in
`layer_shell/tests/multi_output.rs` drive a real `wayland-client`
layer surface with a null output — maps on output 2 with the pointer
there (fail-first: reserved output 1 before the fix), falls back to the
primary with the pointer over no output, and is closed with its zone
released when output 2 is removed. The pre-existing
`a_layer_surface_without_a_requested_output_lands_on_the_primary` still
passes unchanged (the pointer starts on the primary). Full cycle on the
dev VM: `nextest -p scoot` 2880 passed / 42 skipped, `cargo test -p
scoot` 2880 passed / 0 failed, `clippy -D warnings` and `fmt --check`
clean. User-visible docs updated (`protocols.md`, the scootlaunch panel
troubleshooting). Not re-driven here (stated, not papered over): the
mako/fuzzel check on two-monitor M2 hardware — no such hardware in this
environment.
