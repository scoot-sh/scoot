---
title: "The greeter's look: greeter.look on NixOS, an explicit root step elsewhere"
status: "open"
area: "packaging"
priority: "low"
blocked: "look-nix-catalog"
---

# The greeter's look: greeter.look on NixOS, an explicit root step elsewhere

Filed 2026-10-12, child of [`live-looks`](../core/live-looks.md). Serves
**daily-drive**.

## The gap

The greeter takes the look's CSS, wallpaper, dark setting and font at
NixOS build time (`nix/modules/nixos.nix:2420-2477`), and has no look on a
non-Nix box. A user's live swap must never reach it: it runs before any
session and is root-owned.

## What to do

- `programs.scoot.greeter.look` (default: the declared `look`), rendered at
  system build time from the same look directory.
- Non-Nix: an explicit `sudo scoot look apply --greeter NAME` that writes
  the greeter's CSS and background once; never a side effect of a user's
  swap.
- Keep the `frame.background` card-scope rule (`theming.md:108-125`) and the
  Stylix regreet assertion (`nixos.nix:2314`); check the inferred conflict
  of the look's `mkDefault` background with Stylix's regreet target.

## Evidence to record

A greeter screenshot per shipped look on the dev VM.

## Not in this ticket

A custom greeter (scootgreet is planned separately).

The design doc this implements against: [live looks](../../../dev/research/live-looks-design.md).
