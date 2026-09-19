---
title: "Multi-output: more than one monitor at a time"
status: "open"
area: "core"
priority: "high"
blocked: "sequenced behind the `scootctl` split and milestone 6 (user, 2026-09-19) — not a technical block"
---

# Multi-output: more than one monitor at a time

Requested 2026-09-19. This is the biggest user-facing gap scoot has, and
`README.md`'s "Not yet" list leads with it: *"One output. Plug in a second
monitor and it stays dark."*

It is almost certainly **milestone-sized rather than backlog-sized** — it
touches the core, all three backends, three protocols and the lock screen,
and several already-resolved tickets were closed as *single-output pins*
that this work has to revisit. Filed here because that is where it was
asked for; promoting it to a numbered milestone in `ROADMAP.md` when it is
picked up would be reasonable.

## The foundation has landed; this is the rest

`State` used to hold `pub output: Option<Output>` — *singular*, one slot,
with `OUTPUT_ID` as a `const` rather than a lookup — so there was nowhere to
put a second output on any backend. That was split out as its own item and
resolved on 2026-09-19
([multi-output-foundation-done](../resolved/multi-output-foundation-done.md)):
`State.outputs` is an id-keyed collection, and `--headless --outputs N`
(1-8) creates N virtual outputs side by side, each with its own `wl_output`,
its own logical position and its own scrolling strip in the core.

So almost everything below is now testable on the dev VM with no second
monitor anywhere: layer-shell zones per output, the session-lock rule that
`locked` must wait for *every* output's blanked frame, `ext-workspace` groups
per output, focus rules across outputs. Only the `--tty` multi-CRTC half
stays hardware-bound.

What remains in *this* entry is making the protocols correct across those
outputs — plus the render loop, which the foundation deliberately left
single: one framebuffer is composited, the primary output's.

## The scope is already written down

`crates/scoot/src/compositor/outputs.rs`'s doc on `Outputs::primary`
enumerates the sites — every caller of it is one — and it is more precise
than a fresh survey would be. Read it first. In summary, and **not all of
them change the same way**:

- **Layer shell.** `layer_destroyed` and `commit_layer_surface` already
  resolve the surface's *own* output (the foundation did those two: without
  them a layer surface on a second output never gets its initial configure
  and is never unmapped). What is left: `layer_hit` wants the output under
  the pointer; `refresh_layer_zone` needs a zone per output rather than one;
  and `layer_keyboard_focus` plus `render()`'s frame-callback/cleanup pass
  have to walk more than one map.
- **Session lock**, whose four per-output sites are recorded in
  `docs/backlog/resolved/session-lock-per-output-done.md` — resolved **as
  single-output pins, not as multi-output**. `new_surface` falls back to the
  single output; `configure_all` sizes every surface to that one output;
  confirmation treats the first blanked frame as "presented on all outputs",
  which is true if and only if there is one; and the locked render path
  composites every surface onto that output at its origin with the keyboard
  on the first of them. Per-output, the fallback goes away, each output gets
  its own size, **`locked` must wait for every output's blanked frame**, and
  the focus rule needs one surface per output. That middle one is
  security-relevant: confirming the lock before every screen has blanked is
  the bug the vblank-confirmation work exists to prevent.
- **`ext-workspace-v1`** (`ext_workspace.rs:68`) pins a single workspace
  group to the primary output; multi-output has to make that a group per
  output.
- **The render loop.** `State::render` draws the primary output's
  framebuffer and nothing else, because `State.backend` is one `Backend`.
  Compositing a second output means a render target per output, which also
  decides what `screenshot --output N` (refused today) and `screencopy` of a
  non-primary output answer.
- **`wlr-output-management` reconfiguration.** The read half shipped; the
  `apply`/`test` half was deliberately deferred *because nothing a
  configuration could ask for existed yet*
  (`docs/backlog/resolved/output-management-reconfiguration-done.md`). This
  is what makes it real.
- **`--tty`.** One connector is chosen at startup and hotplug *switches*
  rather than *adds* (`tty/gpu.rs`, `tty/hotplug.rs`). Driving two
  connectors at once is a different shape from following one.

## Questions to settle before implementing

1. **Does `scoot-core` model outputs, or does the compositor?** The core is
   platform-independent by fixed decision and a macOS adapter is supposed to
   be able to reuse it, so "a workspace set per output" probably belongs in
   the core — but it currently takes a single `OutputId` throughout, and
   changing that is the largest single edit in this item.
2. **What does a scrolling strip mean across two monitors?** niri gives each
   output its own strip. That is the obvious answer and probably the right
   one, but it should be a decision rather than a default that emerges from
   the implementation.
3. **Does focus follow the pointer across outputs, or is it explicit?**
   Affects the keyboard-focus rules that currently assume one surface set.

## Staging, if it stays a backlog item

The lock-screen and layer-shell halves are separable from the `--tty`
multi-CRTC half: `--headless` and `--nested` can grow a second virtual
output long before real hardware drives two connectors, and that is where
the core and protocol work can be tested cheaply. Doing hardware first would
put the riskiest part in front of the part that defines the interfaces.
