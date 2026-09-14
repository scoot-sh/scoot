---
title: "Output scaling: fractional-scale-v1 + viewporter + a config scale key — DONE"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Output scaling: fractional-scale-v1 + viewporter + a config scale key — DONE

~~Output scaling: fractional-scale-v1 + viewporter + a config scale key, and
the coordinate-space split it forces~~ — DONE. `[output] scale` sets the
output scale; `headless::set_mode` applies it as `Scale::Integer(1)` at
exactly 1.0 (byte-identical to before) or `Scale::Fractional(f)` otherwise,
which makes Smithay advertise `ceil(f)` on `wl_output`. `wp_fractional_scale_v1`
(the exact value, per surface) and `wp_viewporter` (so a client can submit a
fractionally-scaled buffer) are both advertised. The coordinate-space split
landed as scoped: the core is told the *logical* output rectangle (the same
`ceil(physical / scale)` Smithay's `Space` computes), and the handful of
scale-1.0 assumptions were fixed — decorations (`Decorations::elements` now
takes the scale and converts ring geometry), the cursor
(`element_location`/`element`, including the client-surface path), the input
clamp (`pointer_move_relative` clamps against the logical extent, not the
physical one), and the session-lock configure size (logical). `--nested`
forces scale 1.0 with a warning. IPC's `OutputSnapshot` carries `scale`
(additive, serde-defaulted to 1.0, so it did **not** need a `PROTOCOL_VERSION`
bump — the planned `usable`-rect/focus-workspace bundle is untouched).

**Follow-up (2026-09-14):** this landed the fractional path but created the
`wl_compositor` global at version 5, so clients never received the integer
`wl_surface.preferred_buffer_scale` (a v6 event) that accompanies the
fractional `preferred_scale`. That gap is fixed in
[`fractional-scale-integer-companion-done.md`](fractional-scale-integer-companion-done.md)
(global now v6; `send_surface_state` called from `new_surface`). Note the
reported symptom that surfaced it — Ghostty failing to load at
`[output] scale = 1.5` — is **not** confirmed fixed by this, and review found
GTK4 ignores that event while a fractional object exists; it is tracked
separately and still open in
[`../protocols/ghostty-fails-at-1-5.md`](../protocols/ghostty-fails-at-1-5.md).

The original diagnosis is kept below for history.

---

# Output scaling: fractional-scale-v1 + viewporter + a config scale key, and the coordinate-space split it forces

Reported by the user on their Asahi Linux (M2) laptop, 2026-09-14: flexwm
loads and works, but text is much smaller than under niri on the same
hardware. Root cause is not a client problem — flexwm advertises output
scale 1.0 unconditionally.

**Verified cause.** `headless.rs`'s `set_mode` calls
`output.change_current_state(Some(mode), Some(Transform::Normal), None, ..)` —
the `None` third argument is the scale slot, so the `Output` stays at
Smithay's default `Scale::Integer(1)`. The render path *reads*
`output.current_scale().fractional_scale()` and threads it through windows,
layer surfaces and lock surfaces already, but nothing ever sets it. `--tty`
lists "output scale" as explicitly out of scope (`tty/mod.rs`).

**Why niri differs.** niri advertises the real scale, so clients render at
2x/1.5x; flexwm tells every client the display is 1x, so everything renders
at native pixels and looks tiny on a HiDPI panel.

**Scope chosen (user decision, 2026-09-14): everything in one pass, integer
and fractional together, config under `[output] scale`.**

## What makes this bigger than a config key

Smithay has a complete `fractional_scale` helper and a `viewporter` helper
(the same shape as `session_lock`/`layer_shell`, no hand-written dispatch),
and the render path is already scale-parameterized. The real work is that
flexwm currently treats the framebuffer's *physical* size as the core's
*logical* size everywhere, and several auxiliary paths hardcode scale 1.0.
Those fail silently — no crash, just a ring that doesn't line up and a
pointer that drifts or leaves the screen.

The exact enumeration (with cites) is in the implementer's scoping report;
the load-bearing items:

1. `change_current_state(.., Some(scale), ..)` in `set_mode`; `Scale::Fractional`
   already advertises `ceil()` to `wl_output.scale` automatically.
2. `FractionalScaleHandler::new_fractional_scale` + per-surface
   `set_preferred_scale` on map/commit (anvil is the reference).
3. `ViewporterState` stored on `State` (no handler; render path already honors
   it). Effectively required for GTK/Qt fractional.
4. **The coordinate-space split** — report *logical* size (`physical / scale`)
   to `flexwm_core` in `OutputAdded`/`OutputChanged`, so core layout and
   Smithay's logical `Space` geometry agree.
5. Decorations/focus ring and cursor are hardcoded scale 1.0 — both misplace
   at scale != 1.
6. Input: `pointer_move_relative` clamps against the *physical*
   `current_mode().size` while the pointer lives in logical space (pointer can
   be driven off the real desktop); `element_location` returns logical as
   physical.
7. Session-lock configure size uses physical size.
8. IPC/screenshot: screenshot pixels are physical while `msg windows` rects
   are logical; a scale field is needed for an agent to convert.

## Non-goals

- Dynamic scale changes at runtime (startup-only is acceptable v1, but the
  code must not pretend otherwise).
- `--nested` scale (document as scale-1-only, as it already is).
- Multi-output.

## Verification

Real hardware is the user's Asahi machine (the dev VM's `virtio-gpu` can't
stand in for the HiDPI panel). Dev VM covers: protocol negotiation (a real
client receives the expected `preferred_scale`/`wl_output.scale`), pixel
placement at a scaled buffer, input-clamp bounds, and the full test suite.
