//! `wp_single_pixel_buffer_manager_v1`: solid-color 1x1 buffers with no shm.
//!
//! Some toolkits use this for cheap fills instead of allocating a real pool.
//! The whole implementation is Smithay's at the pinned rev
//! (`SinglePixelBufferState`, created in [`State::new`](super::State::new) and
//! held on [`State`](super::State) only to keep the global alive): buffers
//! report 1x1 through `buffer_dimensions`, skip texture import in
//! `update_surface` (there is nothing to import), and render as a `SolidColor`
//! element, which the pixman backend draws with `draw_solid` -- so a 1x1
//! buffer composited here renders sanely with no flexwm-side import or blit
//! path of its own.
//!
//! ## Edge cases, stated rather than re-derived
//!
//! - **Malformed `create_u32_rgba_buffer` colors are not expressible.** The
//!   spec gives every channel the full `uint` range (`0` to `u32::MAX`,
//!   interpreted as a percentage), so there is no out-of-range value to clamp
//!   or refuse -- the boundary values are pinned by
//!   `created_buffers_carry_their_rgba_values_and_use_no_shm` instead.
//! - **Manager `destroy` leaves its buffers usable.** The spec says the child
//!   objects are unaffected; `destroying_the_manager_leaves_its_buffers_usable`
//!   destroys the manager first and still maps and renders from the buffer.
//! - **Buffer `destroy` while attached is legal.** Wayland lets a client
//!   destroy a `wl_buffer` its surface still names; Smithay reports it through
//!   the [`BufferHandler`](smithay::wayland::buffer::BufferHandler) this
//!   compositor already implements, and
//!   `destroying_an_attached_buffer_keeps_client_and_compositor_alive` pins
//!   the survival property -- no panic, no dead client -- rather than any
//!   particular pixel, since what a destroyed buffer draws is Smithay's call.
//! - **No shm-pool interaction.** These buffers allocate no pool, so
//!   `dispatch.rs`'s pool guards (all `TypeId`-gated to `wl_shm` /
//!   `wl_shm_pool`) never see them: nothing to claim against the per-client
//!   pool budget, and no bypass of a limit that should apply either, since
//!   there is no fd, no mapping and no reservation to bound. Pinned by the
//!   pool-count assertion in the RGBA test.
//! - **No dmabuf interaction.** No dmabuf object is created, named or
//!   imported anywhere on this path, so the `zwp_linux_dmabuf_v1` feedback
//!   (see [`dmabuf`](super::dmabuf)) neither affects nor is affected by it.
//! - **Cost.** Bind-time plus one tiny allocation per buffer
//!   (`SinglePixelBufferUserData`, four `u32`s); nothing runs per frame or
//!   per event, so no benchmark.
//!
//! ## Trust model
//!
//! No client filter, the same deliberate consistency as every other
//! advertisement here: flexwm has no security-context support, so an
//! allow-list would be theatre (see `README.md`'s trust note). This global
//! hands out solid colors, not pixels, so it extends that note rather than
//! widening it.

#[cfg(test)]
mod tests;
