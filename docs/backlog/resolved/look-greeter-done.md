---
title: "The greeter's look: greeter.look rendered at NixOS build time"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-16"
---

# The greeter's look: greeter.look rendered at NixOS build time

Filed 2026-10-12, child of [`live-looks`](../core/live-looks.md). Serves
**daily-drive**.

## The gap

The greeter takes the look's CSS, wallpaper, dark setting and font at NixOS
build time (`nix/modules/nixos.nix:2420-2477`). A user's live swap must never
reach it.

## What to do

- `programs.scoot.greeter.look` (default: the declared `look`), rendered at
  system build time into the `greet.toml` fragment (for scootgreet, per the
  shell-apps plan) and ReGreet's CSS today.
- Keep the `frame.background` card-scope rule (`theming.md:108-125`) and the
  Stylix regreet assertion (`nixos.nix:2314`).
- Later, not here: a non-Nix `sudo scoot look apply --greeter`.

## Evidence to record

A greeter screenshot per shipped look on the dev VM.

## Resolution (2026-10-16, PR #638)

`programs.scoot.greeter.look` (null follows the declared `look`) resolves
at system build time into ReGreet's CSS, backdrop, dark setting and sans;
a custom catalog look themes from its manifest down the `extends` chain.
Every render carries the `greet.toml` fragment for scootgreet (thirteen
roles, UI sans, opaque), and a canary plus write-inventory test proves a
live swap never reaches the login screen. The `frame.background`
card-scope rule and the Stylix regreet assertion are kept. Screenshots
are still owed: the dev VM was down through the session, so no greeter
screenshot ran; the Nix-side gates (module suite, all-systems eval, both
Linux drvPaths) are green.

## Not in this ticket

scootgreet itself.

The design this implements against: [live looks](../../../dev/research/live-looks-design.md).
