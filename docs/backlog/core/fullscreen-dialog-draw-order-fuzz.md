---
title: "Fuzz seeds past the historical set break the fullscreen dialog draw-order invariant"
status: "open"
area: "core"
priority: "medium"
blocked: "none — repro below; needs minimizing before anyone can say whether production can reach it"
---

# Fuzz seeds past the historical set break the fullscreen dialog draw-order invariant

Filed 2026-09-25 from the `output-reconnect-restore` implementation, which
extended the invariant fuzz driver (`scoot-core/src/world/tests/invariants.rs`)
with the two new positional output actions and immediately went red.

## What is wrong

`assert_invariants` requires a visible floating dialog that descends from the
covering fullscreen window to be drawn *above* it in arrangement order
(`invariants.rs`, the `{placement:?} under its parent` assert). A perturbed
fuzz stream breaks it:

- On the implementation branch: seed 16, step 658 (`ToggleFullscreen`),
  `WindowId(310)` floating and visible under its parent.
- On pristine `main` (`e87d8fc`) with *only* the driver range widened
  (`below(24)` → `below(25)`, arm 24 firing the same `CloseFocused` the
  fallback already produced — no product code touched): seed 17, step 1234,
  `WindowId(1013)` floating and visible under its parent.

The second repro exonerates the reconnect work: the hole is pre-existing, and
any perturbation of the modulo RNG stream (which reshuffles every draw past
the first widened one) can wander into it. The historical 24 seeds never did.

## Why medium, not high

The invariant guards a hang-like symptom (a modal dialog hidden under the
fullscreen parent it blocks), which would be high if production could reach
it. What is proven is only that the fuzz state exists, with adversarial
ingredients (wild ids, `usize::MAX` indices, random parent links) — nobody
has minimized the seed-17 sequence to a realistic one yet. Per the project's
rule, an alarming test name is a reason to go and measure, not a verdict.

## What done looks like

- Minimize the seed-17 repro to the shortest realistic sequence (real ids,
  no wild indices) or show it needs adversarial input.
- If realistic: fix the draw order (`floating_order.rs` / `arrange.rs`) and
  keep the fix's regression test.
- Either way: re-extend the fuzz driver with the positional output actions
  (`FocusOutputIndex`, `MoveFocusedWindowToOutputIndex` — the arms the
  reconnect branch had to revert to stay green), widen the range, and confirm
  seeds 1..=24 pass.
