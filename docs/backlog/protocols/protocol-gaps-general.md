---
title: "Smaller/general-client-compatibility protocol gaps, lower urgency, bundled here as one entry since none has design work done and none is blocking anything else on this list."
status: "open"
area: "protocols"
priority: "low"
blocked: null
---

# Smaller/general-client-compatibility protocol gaps, lower urgency, bundled here as one entry since none has design work done and none is blocking anything else on this list.

Smaller/general-client-compatibility protocol gaps, lower urgency,
bundled here as one entry since none has design work done and none is
blocking anything else on this list. User request, 2026-09-13,
recorded so they don't get lost rather than because any is scheduled:
- `wp_presentation` (presentation-time) — precise frame-timing
  feedback, mainly useful for smooth video/animation clients.
- `wp_viewporter` — ~~lets a client crop/scale its own buffer; some
  clients assume this exists~~ — **DONE**, implemented with output scaling
  (`docs/backlog/resolved/output-scaling-done.md`); the render path reads
  each surface's viewport destination.
- `single-pixel-buffer-v1` — a trivial protocol for a client to get
  a solid-color 1x1 buffer without allocating a real one; some toolkits
  use it for cheap fills.
- `relative-pointer-unstable-v1` — raw, unaccelerated pointer deltas;
  pairs with the `pointer_constraints` support already present, and
  games/3D apps expect both together, not just pointer lock/confinement
  alone.
- `fractional-scale-v1` — ~~crisp non-integer output scaling. Not
  urgent while flexwm has exactly one output and no real scale
  configuration story yet, but relevant once multi-output/HiDPI does.~~
  **DONE**, implemented with `[output] scale`
  (`docs/backlog/resolved/output-scaling-done.md`). Its integer companion
  `wl_surface.preferred_buffer_scale` (a v6 `wl_compositor` event) landed as a
  follow-up; see
  `docs/backlog/resolved/fractional-scale-integer-companion-done.md`. (The
  Ghostty-at-`1.5` symptom that surfaced it is a separate, still-open question:
  `docs/backlog/protocols/ghostty-fails-at-1-5.md`.)
- `text-input-v3`/`input-method-v2` — ~~IME support for non-Latin
  script input, and on-screen keyboards. A real gap for non-US-keyboard
  daily use; unrelated to item 14's `flexwm msg type`/`msg key` work,
  which is about agent-driven synthetic input, not live IME composition
  from a real input method.~~ — **DONE**, both halves implemented together
  with the other three protocols `foot` warned about
  (`docs/backlog/resolved/foot-protocol-warnings-done.md`); the compositor's
  own part is the IME popup, see `compositor/input_method.rs`. The statement
  above still holds: this is unrelated to `msg type`/`msg key`, which remain
  the agent-driven synthetic-input path.
