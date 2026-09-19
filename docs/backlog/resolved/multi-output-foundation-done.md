---
title: "Multi-output foundation: make `State.output` a collection, and give `--headless` an `--outputs N`"
status: "resolved"
area: "core"
priority: "high"
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

## Resolved 2026-09-19

`State.output: Option<Output>` is now `State.outputs: Outputs`
(`crates/scoot/src/compositor/outputs.rs`), an id-keyed collection;
`OUTPUT_ID` is gone, replaced by `Outputs::primary` as the one named place
the single-output assumption lives; and `--headless --outputs N` (1-8) builds
N outputs left to right, each with its own `wl_output` global, its own
logical position and its own scrolling strip in the core.

**The two decisions the entry asked for, answered:**

1. **The core already modelled the collection — the compositor did not.**
   The entry expected the core's single `OutputId` to be "the largest single
   edit in the item"; it turned out to be no edit at all. `World` already
   holds `outputs: Vec<Output>`, upserts them by `OutputId`, gives each its
   own workspaces and usable area, tracks `focused_output`, and stamps every
   `Placement` with the output it belongs to (`world/mod.rs`, `events.rs`,
   `arrange.rs`). What was singular lived entirely on the compositor side.
   So the new collection is the *binding* from a core id to a
   `wl_output`-backed Smithay `Output`, which is Wayland and belongs there —
   a macOS Accessibility adapter would keep its own id-to-`NSScreen` map
   against the same core.
2. **One scrolling strip per output, niri-style** — and the core's tree had
   already committed to it: workspaces hang off an output, and `view_x`
   hangs off a workspace. Written down in `outputs.rs`'s module doc as three
   consequences: a window is on exactly one output, moving a column across
   outputs is a tree move rather than a scroll, and one output's reserved
   edges shrink that output's usable area only.

**One step past mechanical, deliberately.** `commit_layer_surface` and
`layer_destroyed` now find the surface's *own* output rather than the
primary one. With a real second output the old lookup was two client-facing
faults, not an incompleteness: a commit on a surface the primary's map does
not hold reads as "not a layer surface", so no initial configure is ever sent
and the client waits forever; and the destruction never unmaps it, leaving a
dead surface arranged on a map `render()`'s `cleanup()` does not walk.

**Refused rather than answered wrong.** One output is composited, so
`screenshot --output N` for any other output is now an error instead of the
primary output's pixels under another output's name. `screencopy` and
`gamma_control` already compared against the single output and keep doing so
against the primary one.

Everything else stays single-output *in behaviour* and goes through
`Outputs::primary`, which is what [multi-output](../core/multi-output.md)
picks up: layer-shell zones per output, `layer_hit`, `layer_keyboard_focus`,
`render()`'s per-output pass, session lock (including `locked` waiting for
every output's blanked frame), `ext-workspace` groups, output-management
heads and the pointer clamp.
