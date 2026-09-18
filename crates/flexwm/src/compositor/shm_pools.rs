//! How many live `wl_shm_pool` protocol objects one Wayland client may hold
//! at once.
//!
//! `dispatch.rs`'s per-pool cap (512 MiB) bounds one pool; nothing there
//! bounds how many pools one client holds open concurrently, so this caps
//! the concurrency, not the bytes: at most 128 live pool *objects* per
//! connection, plus the address-space envelope that implies (128 x 512 MiB
//! sparse, via the per-pool cap).
//!
//! What this does *not* bound is the compositor's fds or mappings. A
//! destroyed pool object frees neither while a `wl_buffer` created from it
//! is still alive -- the protocol mandates the retention, and at the pinned
//! rev a buffer's user data holds an `Arc<Pool>` owning both the mapping
//! and its `OwnedFd` -- so `create_pool` / `create_buffer` / `destroy_pool`
//! in a loop keeps one fd and mapping per iteration with the live-pool
//! count back at zero. That quantity is bounded separately, by the
//! per-client live-`wl_buffer` count (`wl_buffers.rs`), which catches
//! exactly the bypass shape: every iteration must keep a buffer alive.
//! (Earlier revisions of this doc said 128 live pools meant 128 fds; see
//! `docs/backlog/resolved/shm-pool-cap-misses-retained-fds-done.md`.)
//!
//! ## The number
//!
//! [`MAX_POOLS_PER_CLIENT`] is 128. The floor is measured, not guessed: one
//! `foot` window holds 2 pools of 512 MiB each (a double-buffered arena --
//! real framebuffers are ~2 MiB placed at offsets inside it), one KeePassXC
//! (Qt) window holds 2 pools totalling ~8 MiB, and an idle quickshell holds
//! none (`WAYLAND_DEBUG=1` wire logs, dev VM, 2026-09-17). The shapes that
//! multiply it are multi-window apps -- a browser at ~2 pools per surface
//! over 20 windows lands near 40 -- so 128 is ~3x the heaviest reasoned
//! legitimate use and 64x the measured single-window floor. The margin errs
//! generous on purpose: tripping this disconnects the client, so a miscount
//! must not kill a heavy-but-legitimate session.
//!
//! What 128 bounds per connection: 128 live pool objects, and -- only in
//! the address-space sense -- 128 x 512 MiB sparse. It does *not* bound fds,
//! mappings (a surviving buffer retains both -- see above), or bytes
//! the way the ticket that filed this
//! (`docs/backlog/resolved/shm-pool-count-cap-done.md`) asked: a
//! byte total needs each pool's size at destroy time, and at the pinned
//! Smithay rev that is unknowable -- the new pool's id is sealed inside
//! `New<WlShmPool>` (no accessor), `ShmPoolUserData`'s only field is
//! private with no size method, `Pool` is unexported, `ShmState` keeps no
//! pool registry, and no server API enumerates a client's objects (all
//! re-verified against `0ff0098` and wayland-server 0.31.14, not assumed).
//! Until an upstream size accessor lands, per-pool sizes cannot be tracked
//! and a byte total cannot be released exactly; this count is the sound
//! bound available, and the byte total stays open behind that accessor.
//!
//! ## How it is counted
//!
//! Keyed by [`ClientId`](smithay::reexports::wayland_server::backend::ClientId),
//! in `State` beside `BindBudget` -- not in `ClientState`: `Client::get_data`
//! hands out only `&Data`, so per-request mutation there would need a lock,
//! while a `State`-side map keyed the same way needs none. Own counter, not
//! a second column in `BindBudget`: pools are counted objects with a
//! kill-the-offender refusal, binds are teardown-with-`finished` ones, and
//! sharing one table across those two refusal forms would be the awkward
//! shoehorn `bind_budget.rs`'s seam warned against.
//!
//! Claimed in `dispatch.rs` before delegation, released in its destruction
//! hook -- which is also what drains a disconnect (whose cleanup destroys
//! every object) and a protocol-error kill. A refused creation is never
//! counted: sizes the per-pool cap or upstream already refuse (`<= 0`,
//! `> 512 MiB`) return before the claim, and fds Smithay cannot map are
//! probed first (an unmappable fd is refused with Smithay's own `InvalidFd`,
//! uncounted -- without that probe the never-initialized pool's no-op
//! `destroyed` would leak one unit per connection, attacker-paced). So
//! every counted pool is a pool whose mapping succeeded at claim time, and
//! every destroyed one releases exactly once; the one exception is a
//! mapping that stops succeeding between the probe and Smithay's own call
//! (a cross-thread TOCTOU -- see `dispatch.rs`), which leaks a single unit
//! for an already-dead client.
//!
//! Release is `saturating_sub`, because a destroy the count never saw would
//! be a bookkeeping bug, not a client one -- and a compositor must not panic
//! on its own accounting. The counter itself cannot overflow: every live
//! pool holds at least one compositor fd, so live pools can never approach
//! `u32::MAX` (fd exhaustion at ~2^10 on this machine's limits, hard caps at
//! ~2^20 anywhere); saturation is belt-and-braces, not the bound.
//!
//! ## Per connection, plus a compositor-wide ceiling
//!
//! Wayland connections are unbounded, so N connections hold up to 128N
//! pools -- which the per-connection cap alone cannot stop. The
//! compositor-wide ceiling (`fd_pressure`, enforced in `dispatch.rs`)
//! closes it the same way as for buffers: while the process table is
//! pressured, a client already holding past the 64-pool grace is refused
//! its next `create_pool` with the same protocol error, and a client under
//! grace is never refused for another's greed.

use std::collections::HashMap;

use smithay::reexports::wayland_server::Client;
use smithay::reexports::wayland_server::backend::ClientId;

/// How many live `wl_shm` pools one Wayland client may hold at once.
///
/// 128: ~3x the heaviest reasoned legitimate use (~40 pools for a 20-window
/// browser at ~2 pools per surface) and 64x the measured single-window
/// floor (2 pools) -- see the module doc. A client past it is refused with
/// a protocol error on the offending `create_pool`, never silently: a
/// silent ignore would leave an uninitialized object that panics the
/// compositor the moment the client touches it (the argument `dispatch.rs`
/// already makes for the per-pool cap).
pub(super) const MAX_POOLS_PER_CLIENT: u32 = 128;

/// The per-client live-pool count. See the module doc for the policy; the
/// only writer is `dispatch.rs`'s `create_pool` claim, the only releaser its
/// pool destruction hook.
#[derive(Debug, Default)]
pub struct ShmPools {
    /// Live pools per client. An entry exists only while the client holds at
    /// least one -- the empty count is removed on release -- so this is
    /// bounded by live pool objects, each counted creation pairing with
    /// exactly one destruction the same way wayland-backend already bounds
    /// them. The single exception is the probe/Smithay TOCTOU (see the
    /// module doc): at most one phantom unit per such event, for a client
    /// that is already dead -- never attacker-paced growth of live entries.
    live_per_client: HashMap<ClientId, u32>,
}

impl ShmPools {
    /// Counts one more live pool for `client`, refusing past
    /// [`MAX_POOLS_PER_CLIENT`].
    ///
    /// Returns whether the caller must refuse the `create_pool` -- and the
    /// refusal itself (the protocol error on `wl_shm`) stays with the caller
    /// in `dispatch.rs`, which holds the object this module never sees.
    ///
    /// Called *before* delegation, so a refused pool is never created. The
    /// caller probes fd mappability first (see `dispatch.rs`): every size
    /// this sees (1 through the per-pool cap on a mappable fd) makes
    /// Smithay's `create_pool` initialise the object unconditionally, with
    /// one nanosecond-wide exception past that -- a mapping that stops
    /// succeeding between the probe and Smithay's own call, which leaks a
    /// single unit for an already-dead client. The deterministic leak this
    /// probe closes (any unmappable fd, e.g. `/dev/null`, attacker-paced,
    /// no memory pressure) is pinned by
    /// `an_unmappable_fd_create_consumes_no_count_budget`.
    pub(super) fn refuse_pool_creation(&mut self, client: &Client) -> bool {
        let live = self.live_per_client.entry(client.id()).or_insert(0);
        if *live >= MAX_POOLS_PER_CLIENT {
            tracing::debug!(
                live = *live,
                max = MAX_POOLS_PER_CLIENT,
                "refusing a wl_shm pool past the per-client live count (protocol error)"
            );
            return true;
        }
        *live = live.saturating_add(1);
        false
    }

    /// Forgets one live pool for the client a dead pool object belonged to.
    ///
    /// Called from `dispatch.rs`'s destruction hook for every destroyed
    /// `wl_shm_pool`, which is also what bounds the map: an entry leaves it
    /// exactly when its last pool's protocol object does, including on
    /// client disconnect (whose cleanup destroys every object) and on a
    /// protocol-error kill. `saturating_sub`, because a destroy the count
    /// never saw would be a bookkeeping bug, not a client one.
    pub(super) fn forget_pool(&mut self, client: &ClientId) {
        if let Some(live) = self.live_per_client.get_mut(client) {
            *live = live.saturating_sub(1);
            if *live == 0 {
                self.live_per_client.remove(client);
            }
        }
    }

    /// How many live pools `client` holds right now. Zero for a client
    /// with no entry, for the same pressure-guard comparison
    /// (`fd_pressure`) the buffer count's twin serves.
    pub(super) fn live_for(&self, client: &Client) -> u32 {
        self.live_per_client.get(&client.id()).copied().unwrap_or(0)
    }

    /// How many pools all clients hold between them. Test-only: the flood
    /// and drain tests assert the bookkeeping empties, which a
    /// compositor-side count states directly -- while Smithay's own pool
    /// objects stay private, so no test could read them.
    #[cfg(test)]
    pub(super) fn pools_in_flight(&self) -> usize {
        self.live_per_client.values().sum::<u32>() as usize
    }
}
