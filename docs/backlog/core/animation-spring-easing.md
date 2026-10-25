---
title: "Slide animation follow-up: spring easing"
status: "open"
area: "core"
priority: "low"
blocked: null
---

# Slide animation follow-up: spring easing

Filed 2026-10-25. Serves **daily-drive** (a spring option for users who prefer niri-style motion).

## The gap

The slide offers `ease-out` (cubic) and `linear` only
(`animation.rs::Easing`). niri and Hyprland animate with springs, which
overshoot and settle; some users prefer that feel.

## What to do

Add an optional spring easing behind `[animation] easing` (new spelling,
clamped duration still bounds the settle; no per-frame transcendental
unless measured cheap, else a small precomputed table). Keep the
closed-form ease-out as the default; a spring must still finish
immediately when the compositor falls behind and go fully idle at the
end. Compare frame cost against ease-out on the CPU renderer. Pin: the
same retarget/cancel and reload tests as the shipped curves.

## Not in this ticket

Open/close fade, workspace-switch slide, vertical slides (siblings).
