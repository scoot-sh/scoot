---
title: "Slide animation follow-up: spring easing"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-25"
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

## Resolution (2026-10-25, PR forthcoming)

Landed as `feat(scoot): optional spring easing for the window slide`.
`Easing` gained `Spring`, parsed from `[animation] easing = "spring"`
(default stays `"ease-out"`); the curve is a 65-entry baked table
(underdamped spring, damping ratio 0.6: ~9% overshoot mid-flight, pinned
endpoints) so the frame path pays one lookup and a lerp per window per
frame — no `exp`/`sin` (the analytic form measured ~3x the table, 3.89ns
vs 1.33ns per placement in flight on the CPU renderer, release). Clamped
duration still bounds the settle; a late frame still snaps and the
compositor still goes idle. `slide_position` is saturating now so an
overshooting `i32::MAX` window clamps instead of wrapping. Same
retarget-from-visual and damage discipline on both axes, still no client
re-commit; in-flight slides keep their curve across reloads.
`scoot-core` untouched. 9 new tests in `animation/tests.rs` cover the pin
list on the spring (shape, snap/idle, retarget, cancel, reload, extremes,
IPC-target). Measured on the CPU renderer (headless, pixman, release,
same VM): ease-out 0.90ns, linear 0.90ns, spring 1.33ns per placement in
flight (+~0.4ns vs shipped ease-out; shipped `slide_position` remeasured
at 0.81ns, so the saturating arithmetic costs nothing measurable).
`cargo test -p scoot`: 2801 passed, 1 failed — the failure is the
pre-existing `xwayland refused_cost` fd-count flake, which fails
identically on pristine `origin/main` on the same VM; `cargo nextest run
--workspace --no-fail-fast`: 6462 passed, 142 failed, all 142 in
`scootbar` (`/tmp/opencode` scratch collision) plus that same xwayland
test, every one reproduced on pristine `origin/main`. Smoke test
`scripts/smoke-test.sh`: rc=0 (49 oks).
