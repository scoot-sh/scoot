---
title: "A layer surface that commits but never attaches a buffer holds its exclusive zone — RESOLVED."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# A layer surface that commits but never attaches a buffer holds its exclusive zone — RESOLVED.

## Resolution (2026-09-18)

Decide + pin, no behavior change. Verified first in-harness: a `Top` bar
with `exclusive_zone = 30` that commits but never attaches reserves the full
30px (`usable` shrinks, nothing drawn) — the existing
`a_bar_reserves_its_zone_from_its_initial_commit_not_its_first_buffer`
re-run green before anything was touched. The ticket's question — reserve on
configure, on first commit, or on first buffer? — is answered **on first
commit**, and stays that way, for three protocol-grounded reasons:

1. **The protocol requires the buffer-less state.** `get_layer_surface`
   mandates "the client must perform an initial commit without any buffer
   attached", and only "then" may it attach a buffer "to map the surface".
   Every healthy bar passes through committed-but-buffer-less for a frame or
   two; reserving from the initial commit is what keeps windows from visibly
   jumping when the first buffer lands.
2. **The zone is double-buffered state applied at commit.** "Layer surface
   state (layer, size, anchor, exclusive zone, margin, interactivity) is
   double-buffered, and will be applied at the time `wl_surface.commit`."
   From the wire's perspective the zone *is* in effect from the initial
   commit. The spec's only hedge — "the compositor's use of this information
   is implementation-dependent" — permits either answer; it mandates
   neither.
3. **The fix would mean reimplementing `arrange`.** Re-verified against the
   pinned Smithay rev (`0ff0098`,
   `src/desktop/wayland/layer.rs::LayerMap::arrange`): it iterates every
   surface in the map and reads committed `cached_state`, with no
   mapped/buffer check anywhere. Filtering on mapped-ness means carrying a
   local fork of the anchor/margin/`-1`/implied-edge/exclusive-first
   geometry in the core layout path — high risk for an edge with no observed
   real client, exactly the coupling `layer_shell.rs`'s module doc refuses
   ("flexwm does not reimplement any of that").

Two deliberate asymmetries, stated so neither looks accidental:

- **Focus already draws the line the layout doesn't.** `layer_focus`
  returns `Never` while `last_acked.is_none()`: a buffer-less surface holds
  no keyboard "while nothing of it is on screen". Different cost calculus,
  not an oversight — a wrong focus routes keystrokes (password-class), an
  early zone costs a 30px strip for a frame or two (cosmetic, self-heals).
- **No timeout, bounded lifetime instead.** A hung client holds the space
  until it disconnects — but so does a mapped-but-frozen bar, and no layer
  of this compositor times out a client. What is bounded is the lifetime,
  and all three edges are now pinned (below), including the destroy path's
  PR #34 lineage (`layer_destroyed` unmaps + refreshes, so a surface
  destroyed between its initial commit and its first buffer leaves nothing).

### Tests

Three new in `compositor/layer_shell/tests/layout.rs` (plus a new
`Step::SetLayerExclusiveZone` in the harness), each confirmed fail-first by
temporarily neutering the refresh it pins and watching it go red:

- `a_bar_that_never_draws_holds_its_zone_until_it_disconnects` — held from
  the initial commit with nothing ever drawn, released on disconnect. Fails
  with `layer_destroyed`'s `refresh_layer_zone` neutered (round 2).
- `destroying_a_bar_that_never_drew_returns_its_space` — explicit destroy of
  a never-mapped bar returns the space immediately. Same round-2 red.
- `a_mapped_bar_may_drop_its_zone_after_its_first_buffer` — a mapped bar
  committing `zone: 0` gives the space back while still drawing (bar over
  the window that moved back underneath it). Fails with
  `commit_layer_surface`'s refresh neutered (round 1), passes under round 2
  — proving it pins the commit path, not the destroy one.

Round 1 (commit refresh neutered) additionally reddened the pre-existing
zone tests that share the path; round 2 left every commit-path test green.
No compositor code changed; no hot path touched, so no benchmark. No README
change: the exclusive-zones bullet ("gives that space back the moment it
exits or its client dies") stays accurate, and a pin documents nothing new
a bar author must do.

Original entry, left as written:

A layer surface that commits but never attaches a buffer holds its
exclusive zone (item 14, deliberate, documented in
`a_bar_reserves_its_zone_from_its_initial_commit_not_its_first_buffer`).
Smithay's `LayerMap::arrange` arranges every surface mapped into the map,
buffer or not, so the reservation starts at the initial (buffer-less)
commit the protocol requires. For a healthy bar that window is a frame or
two; for a client that commits and then hangs, the space stays reserved
until it disconnects. Fixing it means filtering on mapped-ness while
computing the zone, which today would mean reimplementing `arrange`
locally — worth doing only if a real client is seen to hit it, or if
upstream grows the distinction.
