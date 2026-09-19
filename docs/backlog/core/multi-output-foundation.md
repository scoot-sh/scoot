---
title: "Multi-output foundation: make `State.output` a collection, and give `--headless` an `--outputs N`"
status: "open"
area: "core"
priority: "high"
blocked: "start once milestone 6 stage 4 lands — both change `State::new`, and a collection refactor merged against a constructor reordering is the bad case"
---

# Multi-output foundation: make `State.output` a collection, and give `--headless` an `--outputs N`

Split out of [multi-output](./multi-output.md) on 2026-09-19, when asking
"can we test multi-monitor headless today?" produced a sharper answer than
expected: **no, and the blocker is not any backend.**

`State` holds `pub output: Option<Output>` (`state.rs:206`) — *singular*.
One slot, not a collection that happens to hold one. `OUTPUT_ID` is a
`const` (`headless.rs:65`), not a lookup. There is nowhere to put a second
output on `--headless`, `--nested` or `--tty` alike.

## Why this is its own item, ahead of the rest

Because it is what makes the rest cheap. With two *virtual* outputs on
`--headless`, almost everything in the parent item becomes testable on the
dev VM with no second monitor in the building:

- layer-shell zones per output — and per `headless.rs`'s `OUTPUT_ID` doc,
  the four sites do **not** all change the same way (`layer_destroyed` and
  `commit_layer_surface` want the surface's own output, `layer_hit` the one
  under the pointer, `refresh_layer_zone` a zone per output);
- the session-lock rule that `locked` must wait for **every** output's
  blanked frame, which is the security-relevant one and is currently true
  only because there is one output;
- `ext-workspace` groups per output (`ext_workspace.rs:68`);
- focus rules across outputs, including whether focus follows the pointer.

Only the `--tty` multi-CRTC half stays hardware-bound — and once this
lands, it stops blocking anything else.

## Scope

**In:** `State.output` becomes a collection with a stable id→`Output`
lookup; `OUTPUT_ID` stops being a const; `--headless --outputs N` (and the
matching geometry — two outputs need positions, not just sizes); the core's
`OutputId` plumbing where it currently assumes one.

**Out:** making any *protocol* correct across two outputs. That is the
parent item, and it is deliberately the next thing rather than this thing —
a collection that nothing yet uses correctly is a reviewable, testable
change; a collection plus four protocols is not.

## Blast radius, measured

`OUTPUT_ID` is referenced in **9 files**; the singular `output` field is
read in at least a dozen more, including `ext_workspace.rs`,
`foreign_toplevel_management.rs`, `input.rs`, `input_method.rs`,
`handlers.rs` and `headless.rs` (7 sites there alone). Most are mechanical.
The ones that are not are exactly the ones `OUTPUT_ID`'s own doc enumerates
— read it first; it is more precise than a fresh survey.

## Two decisions to make deliberately

1. **Does `scoot-core` model the collection, or does the compositor?** The
   core is platform-independent by fixed decision and a macOS adapter is
   meant to reuse it, which argues for the core. It currently takes a single
   `OutputId` throughout, which makes this the largest single edit in the
   item.
2. **What does a scrolling strip mean across two outputs?** niri gives each
   output its own strip. Probably right, but it should be a decision rather
   than whatever falls out of the refactor.

## Sequencing note

Start after milestone 6 stage 4 merges. Both touch `State::new` — stage 4
because `dmabuf::advertise` runs inside it before any renderer exists, this
because the output field is constructed there — and merging a collection
refactor against a constructor reordering is the case this project has
already been bitten by twice with concurrent implementers.
