//! How many live `wl_buffer`s one Wayland client may hold at once.
//!
//! This bounds buffer *objects*. The fds and mappings buffers keep are
//! bounded by the per-client fd ledger (`client_fds.rs`), which counts each
//! pool's fd and each dma-buf plane's fd from the request that handed it
//! over until it really closes -- after the buffer object dies, too, while a
//! surface still has the buffer committed. This cap used to be the fd bound
//! for the shape `create_pool` / `create_buffer` / `destroy_pool` in a loop,
//! which keeps one fd and mapping per iteration with the live-pool count back
//! at zero (a destroyed pool's fd and mapping live while any buffer made from
//! it does -- the protocol mandates it, and a buffer's user data holds an
//! `Arc<Pool>` owning both; see
//! `docs/backlog/resolved/shm-pool-cap-misses-retained-fds-done.md`). Every
//! iteration of that loop keeps a buffer alive, so this cap caught it at the
//! 513th buffer; the ledger now catches it at the 513th pool, and also the
//! shape this cap could not see at all, where the client destroys the buffer
//! too and a surface keeps it (`docs/backlog/resolved/buffer-fds-past-their-object-done.md`).
//! Both caps stay: this one bounds live buffer objects (and the Smithay and
//! renderer state each carries), `shm_pools.rs` live pool objects and their
//! address-space envelope, and the ledger fds.
//!
//! **One thing this cap does not bound, and the reason is worth carrying
//! here rather than only in `dmabuf.rs`:** an imported dmabuf's `mmap` lives
//! in the renderer's own cache, which outlives the `wl_buffer` that carried
//! it. The live count returns to zero the moment the buffer object dies, so
//! import/destroy in a loop is the same shape as the pool bypass above -- a
//! retained mapping with the cap back at zero -- and no buffer count can
//! catch it. What catches it is `dmabuf.rs`'s `schedule_cache_drain`, hung
//! off the same destruction hook that releases this count. Delete neither
//! half without the other, and see that module's cache section for why a
//! rendered frame is not a substitute.
//!
//! ## The number
//!
//! [`MAX_BUFFERS_PER_CLIENT`] is 512. The floor is measured, not guessed:
//! one `foot` window holds exactly **2 live buffers** steady -- double-
//! buffered, reused across typing and resizes rather than churned, zero
//! destroys in the whole session (`WAYLAND_DEBUG=1` wire log, dev VM,
//! 2026-09-17; 2 pools, 2 `create_buffer`, max concurrent 2). The shapes
//! that multiply it are multi-window apps and queued video frames -- a
//! 20-window browser at triple buffering lands near 60 -- so 512 is ~8x
//! the heaviest reasoned legitimate use and 256x the measured single-
//! window floor. The margin errs generous on purpose: tripping this
//! disconnects the client, so a miscount must not kill a heavy-but-
//! legitimate session -- the same death-penalty sizing the capture-frame
//! cap (16 for a legitimate 1) already uses.
//!
//! What 512 bounds per connection: 512 live buffer objects. Not fds: a shm
//! buffer shares its pool's fd, a dma-buf buffer's planes are counted as they
//! arrive, a single-pixel buffer has none, and a destroyed buffer a surface
//! still keeps holds its fds with no object left to count. All of those are
//! the fd ledger's (`client_fds.rs`: 512 fds of every kind per client). Not
//! the renderer-side dmabuf mapping either, which outlives the object
//! (above).
//!
//! Planes added to a params object that has not become a buffer yet are
//! not buffers at all, and are bounded separately
//! (`dmabuf/pending_planes.rs`), besides being counted in the ledger.
//!
//! ## How it is counted
//!
//! Keyed by [`ClientId`](smithay::reexports::wayland_server::backend::ClientId),
//! in `State` beside `ShmPools` -- not in `ClientState`: `Client::get_data`
//! hands out only `&Data`, so per-request mutation there would need a lock,
//! while a `State`-side map keyed the same way needs none. Own counter, not
//! a second column in `ShmPools`: the two refuse different requests with
//! different errors, and count different quantities.
//!
//! Uniformly across every interface that creates a `wl_buffer`, because the
//! release side cannot tell them apart and a selective count would drift
//! fail-open (see below):
//!
//! - `wl_shm_pool.create_buffer` -- the bypass shape; claims in
//!   `dispatch.rs` before delegation.
//! - `zwp_linux_buffer_params_v1.create_immed` -- claims the same way. The
//!   buffer object is created before the import is even attempted: Smithay
//!   inits it first (`data_init.init` before `dmabuf_imported`), so it
//!   exists whether the import succeeds or the `failed()` answer posts
//!   `InvalidWlBuffer`, killing the client and leaving the object for
//!   disconnect cleanup (verified in source at the pinned rev, not
//!   assumed).
//! - `zwp_linux_buffer_params_v1.create` -- the asynchronous sibling, and it
//!   claims too. This doc used to say it never would, on the premise that
//!   scoot answered every import `failed` and so created no object on that
//!   path: "*If a future renderer ever calls `successful()` on a `create`
//!   notifier, that path starts creating buffers and must claim here too.*"
//!   That future is here -- `dmabuf.rs` imports dmabufs into the pixman
//!   renderer, and `ImportNotifier::successful` on a `Falliable` notifier
//!   mints a real, fd-retaining `wl_buffer`. Uncounted, it would be the one
//!   factory outside this cap entirely.
//!
//!   It is also the one creation whose *refusal* leaves the client alive:
//!   `failed()` on a `Falliable` notifier is the protocol's soft answer --
//!   no object, no kill. So [`WlBuffers::forget_buffer`] is called from
//!   `dmabuf.rs`'s `refuse_import` before it answers; otherwise a client
//!   repeatedly offering buffers the renderer cannot map (multi-plane,
//!   non-`LINEAR`) would ratchet its own count to the cap and lock itself
//!   out of creating buffers at all. That release is written to be correct
//!   on the `create_immed` path too, where it simply lands once more on a
//!   client already dying: `forget_buffer` saturates, and the entry is a
//!   dead one either way.
//! - `wp_single_pixel_buffer_manager_v1.create_u32_rgba_buffer` -- always
//!   succeeds, so always pairs. These hold no fd, no mapping and no
//!   reservation, and counting them spends budget on a shape that costs
//!   nothing -- but *not* counting them while counting everything else is
//!   worse: see the release paragraph.
//!
//! Released in `dispatch.rs`'s destruction hook for every destroyed
//! `wl_buffer`, which is also what drains a disconnect (whose cleanup
//! destroys every object) and a protocol-error kill. Release is
//! `saturating_sub`, because a destroy the count never saw would be a
//! bookkeeping bug, not a client one.
//!
//! The pairing is exact for every *live* client, by a mechanism rather than
//! by validation: Smithay initialises the buffer or kills the client, never
//! neither -- every error path in the three creation handlers posts a
//! protocol error (synchronous kill) and returns before `data_init.init`.
//! A creation refused by this guard is never counted (over-cap returns
//! before claiming); a creation Smithay itself refuses still claims -- see
//! the phantom paragraph. So every buffer a live client holds was claimed
//! exactly once, and every destroyed one releases exactly once. The one
//! exception is a creation Smithay refuses: claimed, never initialised,
//! `UninitObjectData::destroyed` never reaching the hook -- exactly one
//! phantom unit on an entry whose client is already dead (no failed
//! creation survives: the kill is synchronous, and already-buffered further
//! requests never dispatch). Dead entries cost one small map entry per
//! killing connection -- noise next to the connection itself -- and never
//! touch a live client's budget. Deliberately no validation replication to
//! avoid the phantom: duplicating upstream's parameter checks here would
//! couple this guard to Smithay's handler logic and drift fail-open on a
//! rev bump; over-counting a dead client is the safe direction.
//!
//! Why the release cannot be kind-selective (and therefore why single-pixel
//! buffers are counted): the blanket `destroyed` hook sees only that *a*
//! `wl_buffer` died -- the user data kind (`ShmBufferUserData` vs `Dmabuf`
//! vs `SinglePixelBufferUserData`) is not observable there, and the new
//! buffer's id is sealed inside `New<WlBuffer>` (no accessor, no `Deref` --
//! wayland-server 0.31.14 `src/dispatch.rs:136-146`), so exact-id tracking
//! is unbuildable at the pinned rev. With scalar counting, releasing for
//! unclaimed kinds would let destroys of cheap buffers drain units claimed
//! by retaining ones (create 5 shm buffers, create and destroy 5
//! single-pixel ones, hold 5 retaining buffers against a count of zero) --
//! a bypass of this very cap. Uniform counting has no such drift: every
//! initialised buffer of every kind claims once and releases once.
//!
//! ## Refusal form
//!
//! A protocol error on the creating object, killing only the offending
//! client -- a silent ignore would leave the uninitialized object that
//! panics the compositor (the argument `dispatch.rs` already makes for the
//! pool caps):
//!
//! - `wl_shm_pool.create_buffer` past the cap: `wl_shm::Error::InvalidStride`
//!   on the pool -- the same code on the same object Smithay's own
//!   bad-parameter refusals and the pool-count refusal use.
//! - `create_immed` past the cap:
//!   `zwp_linux_buffer_params_v1::Error::InvalidWlBuffer` on the params --
//!   the code Smithay's own immed-import failure posts.
//! - `create_u32_rgba_buffer` past the cap: the interface defines no errors
//!   at all (verified against the protocol XML -- no `<enum name="error">`),
//!   so the refusal carries code 0 with an explicit message. Only a client
//!   already holding 512 live buffers ever sees it, i.e. abuse by
//!   construction; the kill is the message.
//!
//! ## Maintenance hazard
//!
//! Any new `wl_buffer` factory Smithay grows (or this compositor starts
//! delegating) must hook both halves: claim on its creating request in
//! `dispatch.rs`, release through the existing `wl_buffer` destruction hook.
//! Re-check the four creation handlers' error-before-init shape on every
//! Smithay bump: the exactness argument above leans on it. The mechanical
//! guard on that shape is
//! `dmabuf/tests.rs::an_import_through_create_immed_is_not_a_client_kill`,
//! which drives an *accepted* `create_immed` and asserts the count is exactly
//! **one**, then destroys the buffer and asserts it is back to **zero**. The
//! discriminating half is the second one, not the first: the `1` is claimed by
//! `dispatch.rs`'s guard on the request itself and would be there whether or
//! not Smithay ever initialised the object, but the count can only *return* to
//! zero if a real server-side `wl_buffer` existed for the destruction hook to
//! fire on. So the pair still guards the shape; a rev that moved
//! `data_init.init` to after the import would keep passing, which is correct,
//! because that ordering is harmless -- what must not change silently is
//! whether an object is created at all. It cannot be guarded on a
//! *refused* creation any more: a refusal that leaves the client alive now
//! releases its own unit, so the count lands on zero whether or not the object
//! was initialised (the dispatch-side test that used to claim otherwise says
//! so in its own doc now).
//!
//! The same goes for anything that starts *refusing* a creation while leaving
//! the client alive -- that needs an explicit release, as the async dmabuf
//! `create` above does.
//!
//! ## Per connection, plus a compositor-wide ceiling
//!
//! Wayland connections are unbounded, so N connections hold up to 512N
//! buffers. The compositor-wide fd ceiling (`fd_pressure`) is enforced where
//! fds arrive, against each client's fds in the ledger, not here: creating a
//! buffer hands scoot no fd. (It used to be enforced here too, on a
//! 128-buffer grace, when this count was the fd proxy; see `fd_pressure`.)

use std::collections::HashMap;

use smithay::reexports::wayland_server::Client;
use smithay::reexports::wayland_server::backend::ClientId;

/// How many live `wl_buffer`s one Wayland client may hold at once,
/// whatever created them.
///
/// 512: ~8x the heaviest reasoned legitimate use (~60 buffers for a
/// 20-window browser at triple buffering) and 256x the measured
/// single-window floor (2 live buffers for one `foot`) -- see the module
/// doc. A client past it is refused with a protocol error on the offending
/// creation request, never silently: a silent ignore would leave an
/// uninitialized object that panics the compositor the moment the client
/// touches it (the argument `dispatch.rs` already makes for the pool caps).
pub(super) const MAX_BUFFERS_PER_CLIENT: u32 = 512;

/// The per-client live-buffer count. See the module doc for the policy; the
/// only claim site is `dispatch.rs`'s buffer-creation guard, the only
/// releaser its `wl_buffer` destruction hook.
#[derive(Debug, Default)]
pub struct WlBuffers {
    /// Live buffers per client. An entry exists only while the client holds
    /// at least one -- the empty count is removed on release -- so this is
    /// bounded by live buffer objects, except for the at-most-one phantom
    /// unit per killed connection the module doc states (a failed creation
    /// claims without ever initialising; its client is already dead, so the
    /// entry never affects a live budget).
    live_per_client: HashMap<ClientId, u32>,
}

impl WlBuffers {
    /// Counts one more live buffer for `client`, refusing past
    /// [`MAX_BUFFERS_PER_CLIENT`].
    ///
    /// Returns whether the caller must refuse the creation -- and the
    /// refusal itself (the protocol error on the creating object) stays
    /// with the caller in `dispatch.rs`, which holds the object this module
    /// never sees.
    ///
    /// Called *before* delegation, so a refused buffer is never created.
    /// Called unconditionally for every creation request, including ones
    /// Smithay is about to refuse: a failed creation kills the client
    /// synchronously, so the phantom unit lands on a dead entry only (see
    /// the module doc) -- replicating upstream's parameter validation here
    /// to avoid it would couple this guard to Smithay's handler logic and
    /// drift fail-open on a rev bump.
    pub(super) fn claim_buffer_creation(&mut self, client: &Client) -> bool {
        let live = self.live_per_client.entry(client.id()).or_insert(0);
        if *live >= MAX_BUFFERS_PER_CLIENT {
            tracing::debug!(
                live = *live,
                max = MAX_BUFFERS_PER_CLIENT,
                "refusing a wl_buffer past the per-client live count (protocol error)"
            );
            return true;
        }
        *live = live.saturating_add(1);
        false
    }

    /// Forgets one live buffer for the client a dead buffer object belonged
    /// to.
    ///
    /// Called from `dispatch.rs`'s destruction hook for every destroyed
    /// `wl_buffer` of every kind, which is also what bounds the map: an
    /// entry leaves it exactly when its last buffer's protocol object does,
    /// including on client disconnect (whose cleanup destroys every object)
    /// and on a protocol-error kill. `saturating_sub`, because a destroy
    /// the count never saw would be a bookkeeping bug, not a client one.
    pub(super) fn forget_buffer(&mut self, client: &ClientId) {
        if let Some(live) = self.live_per_client.get_mut(client) {
            *live = live.saturating_sub(1);
            if *live == 0 {
                self.live_per_client.remove(client);
            }
        }
    }

    /// How many buffers all clients hold between them. Test-only: the flood
    /// and drain tests assert the bookkeeping empties, which a
    /// compositor-side count states directly -- while Smithay's own buffer
    /// objects stay private, so no test could read them.
    #[cfg(test)]
    pub(super) fn buffers_in_flight(&self) -> usize {
        self.live_per_client.values().sum::<u32>() as usize
    }
}
