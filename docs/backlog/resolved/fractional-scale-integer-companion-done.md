---
title: "A `wp_fractional_scale_v1` client was sent the fractional `preferred_scale` but never the integer `wl_surface.preferred_buffer_scale` (a v6 event) — DONE; the protocol gap is fixed and regression-tested"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# The integer `preferred_buffer_scale` companion was never sent — DONE

**Scope, stated up front:** this fixes a real, independently verified protocol
gap (a v6 client received the fractional value but not its integer companion).
It is **not** a confirmed fix for the Ghostty-at-`1.5` symptom that surfaced it
— review found GTK4 ignores this event while a fractional object exists. See
`../protocols/ghostty-fails-at-1-5.md`, which stays open for that symptom.

Reported by the user on their Asahi Linux (M2) laptop, 2026-09-14, right after
`[output] scale` (PR #30) landed:

- `[output] scale = 2.0` → Ghostty loads and renders correctly.
- `[output] scale = 1.5` → **Ghostty does not load.**
- `foot` works at both `1.5` and `2.0`.

## Diagnosis (verified against the pinned Smithay rev and the tree)

Output scaling created the `wl_compositor` global with
`CompositorState::new::<Self>` — **version 5** — at
`crates/flexwm/src/compositor/state.rs`. The event in question,
`wl_surface.preferred_buffer_scale`, is a **version 6** event.

In the pinned Smithay rev (`~/.cargo/git/checkouts/smithay-*/0ff0098/`):

- `compositor::send_surface_state` (`src/wayland/compositor/mod.rs:411`)
  early-returns when `surface.version() < 6` (`:412`). It caches the last
  `(scale, transform)` per surface in `SuggestedSurfaceState` and only emits on
  a change.
- `CompositorState::new_v6` exists (`:710`) but nothing in flexwm used it, and
  **`send_surface_state` has no caller anywhere in Smithay** (verified: the only
  hits under `src/` are its own definition and doc references).
- flexwm's `CompositorHandler::commit` called `on_commit_buffer_handler` but
  never `send_surface_state`, and `new_surface` was the trait default (a no-op).

Consequence: a client that opts into `wp_fractional_scale_v1` was sent the exact
fractional `preferred_scale` but no integer `preferred_buffer_scale`. That is a
genuine protocol gap regardless of which client tripped over it: the two are
separate objects and one does not imply the other. (The original theory — that
foot is an integer-only client and Ghostty a fractional one, and that this
difference explains the symptom — was wrong: live capture showed foot binds
`wp_fractional_scale_v1` and `wp_viewporter` too. foot simply tolerates the
missing companion; the client-side need for it is not established, see the open
entry.)

## Fix

1. `state.rs` now builds the global with `CompositorState::new_v6::<Self>`, so
   clients may bind `wl_compositor` v6 and receive the event.
2. `CompositorHandler::new_surface` calls a new
   `output_scale::send_preferred_buffer_scale`, which forwards to Smithay's
   `send_surface_state` with `integer_scale = ceil(output_scale)` (the same
   integer `wl_output.scale` advertises, cached on `State`) and
   `Transform::Normal` (what `headless::set_mode` uses). Only `new_surface`
   calls it: Smithay runs that hook for every surface `create_surface` makes,
   and the scale is fixed, so a per-commit call would just be a no-op cache hit
   on the hot path.
3. The fractional `preferred_scale` path is unchanged; a client that opts into
   it now receives **both** events, which is the intended, protocol-correct
   behavior — each covers clients that speak only one.

## Smithay caching detail found while testing

`SuggestedSurfaceState::default()` is `scale: 1` (`compositor/tree.rs:544`), not
0. So at exactly `scale = 1.0`, `send_surface_state` emits **nothing** — the
cached default already equals the value — and the client keeps the implicit
integer default of 1. That is correct and byte-identical to the behavior before
this event existed; the regression test pins it as `None` (with the effective
integer asserted as 1). At `1.5`/`2.0` the value differs from the cached default
and is emitted.

## Verification

- `crates/flexwm/src/compositor/output_scale/tests.rs`: a real `wayland-client`
  bound to `wl_compositor` v6 creates a surface, binds `wp_fractional_scale_v1`,
  commits three times, and asserts it receives `preferred_scale` (fractional)
  **and** `preferred_buffer_scale` (integer `ceil`) — at 1.0, 1.5 and 2.0. The
  event **count** checks Smithay's cache (three commits, at most one event).
  Red/green confirmed: neutralizing `send_preferred_buffer_scale` makes the
  `1.5`/`2.0` cases fail (`preferred_buffer_scale: None`).
- A pure `integer_scale` test asserts `ceil` agrees with Smithay's own
  `Scale::integer_scale()` for the same configured value.
- Separate-process live probe on the dev VM (a throwaway `wayland-client`
  example binding `wl_compositor` 6..=6, since removed) against `--headless` at
  `[output] scale` 1.5/2.0/1.0: `preferred_scale` `1.5`/`2.0`/`1.0` and
  `preferred_buffer_scale` `2`/`2`/`None` (implicit 1) respectively.
- Real independent client on the dev VM (`WAYLAND_DEBUG=1 foot` against the
  rebuilt binary, 1.5): `foot` binds `wl_compositor` v6 and is sent
  `wl_surface.preferred_buffer_scale(2)` on its surfaces; `wayland-info`
  reports `wl_output` `scale: 2`. Against the pre-fix (v5) binary `foot`
  bound v5 and was sent nothing, matching the diagnosis.
- Dev VM: full `cargo test -p flexwm` (389 pass), `cargo test --workspace`,
  `clippy -D warnings`, `fmt --check`, and `MODE=--headless`/`MODE=--nested`
  smoke tests (all pass) plus `MODE=--tty`, which fails only the pre-existing
  background-pixel assertion documented in
  `docs/backlog/resolved/tty-background-not-painted-done.md` (rings still
  pass) — no new failure. (That assertion has since been resolved: it was
  the cursor, not the background; the path here is updated to the record
  rather than left dangling at the moved file.)
- Hot path: 20 000 commits through the real dispatch loop at scales 1.0 and
  1.5, with and without the send call, are within run-to-run noise
  (~34–46 ms both ways on the dev VM), consistent with a non-allocating
  per-surface-cached lookup.

**Real hardware did not confirm the symptom is fixed, and review suggests it
may not be.** Ghostty is not installed on the dev VM and the dev VM's virtual
display cannot stand in for the reporter's Asahi M2 panel. Worse, GTK4's own
`surface_preferred_buffer_scale` (`gdk/wayland/gdksurface-wayland.c`) returns
early whenever a `wp_fractional_scale_v1` object exists — which it always does
now — so a GTK4/Ghostty client likely ignores the event this fix adds. The
confirmed part is that a v6 client now actually receives the integer
`preferred_buffer_scale`, closing the diagnosed protocol gap. The Ghostty `1.5`
failure itself remains open: `../protocols/ghostty-fails-at-1-5.md`.
