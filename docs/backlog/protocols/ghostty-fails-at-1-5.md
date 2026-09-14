---
title: "Ghostty fails to load at `[output] scale = 1.5` (works at 2.0); the integer `wl_surface.preferred_buffer_scale` was missing and is now sent, but that does not explain the symptom"
status: "open"
area: "protocols"
priority: "high"
blocked: "needs the reporter's Asahi machine to confirm or refute the fix"
---

# Ghostty fails to load at `[output] scale = 1.5` (works at 2.0)

Reported by the user on their Asahi Linux (M2) laptop, 2026-09-14, right after
`[output] scale` (PR #30) landed:

- `[output] scale = 2.0` → Ghostty loads and renders correctly.
- `[output] scale = 1.5` → **Ghostty does not load.**
- `foot` works at both `1.5` and `2.0`.

## What was found and fixed (a real, separate protocol gap)

Output scaling created the `wl_compositor` global with `CompositorState::new`
(version 5). `wl_surface.preferred_buffer_scale` is a **v6** event, and
Smithay's `send_surface_state` (`wayland/compositor/mod.rs:411` in the pinned
rev) early-returns for `version() < 6` and has no caller anywhere in Smithay.
So a client that opts into `wp_fractional_scale_v1` received the exact
fractional `preferred_scale` but never the integer companion event.

That gap is real and is now closed (PR #31): `CompositorState::new_v6` plus a
`send_surface_state` call from `CompositorHandler::new_surface`, so a v6
client receives **both** `preferred_scale` (`1.5`) and
`preferred_buffer_scale` (`ceil(1.5) = 2`). Regression-tested on the dev VM
with a real client, red/green.

## Why this entry is still open, not resolved

**The fix does not explain the reported symptom, and review found evidence it
may not fix it.** GTK4's own source
(`gdk/wayland/gdksurface-wayland.c`, `surface_preferred_buffer_scale`) returns
early and ignores the event whenever a `wp_fractional_scale_v1` object exists —
which it always does now that flexwm advertises that global. So a GTK4/Ghostty
client likely discards the very event this PR adds.

What that means:

- The integer companion was worth sending for protocol completeness (it is
  what wlroots does), and that part is correct and tested.
- It is **not established** that sending it makes Ghostty load at `1.5`. The
  root cause of the reported failure is therefore **unknown** as of this entry.

## What to do next

The only way to settle it is the reporter's hardware:

1. Pull `main` (after #31) and retry `[output] scale = 1.5` with Ghostty. If it
   now loads, the integer companion mattered despite the GTK4 source; if it
   still fails, the cause is elsewhere.
2. If it still fails, gather Ghostty's own stderr/`WAYLAND_DEBUG=1` output at
   1.5 specifically, and compare against foot's at the same scale. Candidates
   worth checking next: whether Ghostty's fractional buffer sizing trips the
   viewport destination path (a buffer the compositor then rejects or draws
   zero-sized), or whether it needs `wp_viewporter` on a surface flexwm does
   not yet wire, or a specific `wl_output` version it does not get.
3. A local reproduction is preferable to guessing: Ghostty is a GTK4 client,
   so a minimal GTK4 app (or even `gtk4-demo`/`gtk4-widget-factory`) run under
   `--headless` at scale 1.5 on the dev VM may reproduce the same class of
   failure and give a debuggable stack without the reporter's panel.

## Related

- `docs/backlog/resolved/output-scaling-done.md` — the scaling feature itself.
- `docs/backlog/resolved/fractional-scale-integer-companion-done.md` —
  the integer-companion mechanism, fixed and tested; kept there for the
  diagnosis even though this symptom it was thought to explain did not
  resolve.
