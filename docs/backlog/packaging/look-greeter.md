---
title: "The greeter's look: greeter.look rendered at NixOS build time"
status: "open"
area: "packaging"
priority: "low"
blocked: "look-nix-catalog"
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

## Not in this ticket

scootgreet itself.

The design this implements against: [live looks](../../../dev/research/live-looks-design.md).
