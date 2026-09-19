//! `wp_alpha_modifier_v1`: a client-controlled whole-surface opacity factor.
//!
//! A client that wants its surface drawn translucent names a `u32` multiplier
//! (`0` transparent, `u32::MAX` opaque) instead of compositing the opacity
//! itself. The whole implementation is Smithay's at the pinned rev
//! (`AlphaModifierState`, created in [`State::new`](super::State::new) and
//! held on [`State`](super::State) only to keep the global alive):
//! `WaylandSurfaceRenderElement::from_surface` multiplies the pending
//! multiplier into every surface-tree element it builds, and the pixman
//! backend draws a sub-1.0 element through a solid-alpha mask -- so the
//! factor reaches the framebuffer on every backend with no flexwm-side
//! render work of its own. Windows, layer surfaces, lock surfaces and client
//! cursor surfaces all build through that same `from_surface`, so all four
//! honour it uniformly.
//!
//! ## Cost
//!
//! Nothing flexwm adds runs per frame or per event: the one cached-state
//! lookup per surface per frame already runs inside Smithay's `from_surface`
//! whether or not this global is advertised, so advertising it adds zero
//! render-path work. No benchmark: there is no before/after to measure.
//!
//! ## Edge cases, stated rather than re-derived
//!
//! - **`0` is fully transparent, and that is the client's choice.** The
//!   element still exists with an alpha of zero; the pixman mask draws
//!   nothing for it. A lock surface that asks for zero gets the blank the
//!   locked path draws behind it -- the locked element list never contains
//!   windows, so no opacity value leaks desktop pixels.
//! - **Destroying the modifier-surface object unsets the factor.**
//!   Double-buffered, exactly like `set_multiplier(u32::MAX)`: the surface
//!   is opaque again after the next commit. Smithay's `destroyed` is a
//!   no-op by design (the graceful `Destroy` request already queued the
//!   unset); pinned by `destroying_the_modifier_surface_restores_opacity`.
//! - **Destroying the manager leaves its surfaces working.** The spec says
//!   the child objects are unaffected, and they are.
//! - **One modifier object per surface.** A second `get_surface` for the
//!   same `wl_surface` is Smithay's `already_constructed` protocol error,
//!   which kills only the offending client.
//! - **No shm-pool or buffer interaction.** No pool, buffer or fd is
//!   created, named or imported anywhere on this path, so neither the
//!   per-client pool budget nor the live-buffer bound moves for it.
//!
//! ## Trust model
//!
//! No client filter, the same deliberate consistency as every other
//! advertisement here: flexwm has no security-context support, so an
//! allow-list would be theatre (see `README.md`'s trust note). A global
//! that only dims a client's own surface widens nothing.

#[cfg(test)]
mod tests;
