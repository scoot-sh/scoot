//! `wp_content_type_manager_v1`: a client telling the compositor what kind of
//! pixels a surface holds (`none`, `photo`, `video`, `game`).
//!
//! The whole implementation is Smithay's at the pinned rev
//! (`ContentTypeState`, created in [`State::new`](super::State::new) and held
//! on [`State`](super::State) only to keep the global alive). The value is a
//! pure hint -- the protocol's own words -- for compositors that can use it
//! (adaptive-sync timing, overlay-plane choice, HDR handling). This one has
//! no such consumer: a CPU/pixman-only renderer with no adaptive-sync or GPU
//! compositing story, exactly the standing the niche-gaps ticket filed it
//! under. So nothing reads the committed value back, and advertising the
//! global changes no pixel: a hinted surface renders byte-identical to an
//! unhinted one, which `every_content_type_renders_unchanged` pins rather
//! than assuming.
//!
//! Advertising a global nothing reads is honest here, and only here,
//! because the protocol promises the client nothing back: there is no
//! compositor-to-client event whose absence would be a lie, and no client
//! abandons a working client-side path for it the way cursor-shape clients
//! abandon uploaded cursors (see the `foot` record). Should a future GPU
//! tier grow a consumer, the stored value is already where it looks.
//!
//! ## Cost
//!
//! Nothing runs per frame or per event on this path -- not even Smithay
//! reads the cached state back -- so no benchmark.
//!
//! ## Edge cases, stated rather than re-derived
//!
//! - **Every type is accepted, including `none`.** There is no
//!   out-of-range value: the enum is fixed, and an unknown discriminant
//!   never parses at the wire level.
//! - **Destroying the type object is `set_content_type(none)`.**
//!   Double-buffered, like every other state on this protocol.
//! - **Destroying the manager leaves its type objects working.** The spec
//!   says the child objects are unaffected, and they are.
//! - **One type object per surface.** A second `get_surface` for the same
//!   `wl_surface` is Smithay's `already_constructed` protocol error, which
//!   kills only the offending client.
//! - **No shm-pool or buffer interaction.** No pool, buffer or fd is
//!   created, named or imported anywhere on this path, so neither the
//!   per-client pool budget nor the live-buffer bound moves for it.
//!
//! ## Trust model
//!
//! No client filter, the same deliberate consistency as every other
//! advertisement here: flexwm has no security-context support, so an
//! allow-list would be theatre (see `README.md`'s trust note). A global
//! that only labels a client's own surface widens nothing.

#[cfg(test)]
mod tests;
