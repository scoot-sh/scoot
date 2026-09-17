//! How many live `wl_shm` pools one Wayland client may hold at once.
//!
//! `dispatch.rs`'s per-pool cap (512 MiB) bounds one pool; nothing there
//! bounds how many pools one client holds open concurrently, and each live
//! pool costs the compositor a mapping, a `Pool` object dropped on a worker
//! thread, and -- the sharp edge -- one fd of its own (`InnerPool` owns an
//! `OwnedFd`). With the dev VM's soft `RLIMIT_NOFILE` at 1024, on the order
//! of a thousand pools from a single connection exhausts the compositor's
//! fds for every client, not just the hoarder. This caps the concurrency,
//! not the bytes.
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
//! What 128 bounds per connection: 128 fds (an eighth of a 1024-fd
//! `RLIMIT_NOFILE`), 128 mappings/objects, and -- only in the
//! address-space sense -- 128 x 512 MiB sparse. It does *not* bound bytes
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
//! counted, and sizes the per-pool cap or upstream already refuse (`<= 0`,
//! `> 512 MiB`) never reach the claim, so every counted pool is a pool
//! Smithay actually created and every destroyed one releases exactly once:
//! the count cannot leak on a path that counts but never creates.
//!
//! Release is `saturating_sub`, because a destroy the count never saw would
//! be a bookkeeping bug, not a client one -- and a compositor must not panic
//! on its own accounting. The counter itself cannot overflow: every live
//! pool holds at least one compositor fd, so live pools can never approach
//! `u32::MAX` (fd exhaustion at ~2^10 on this machine's limits, hard caps at
//! ~2^20 anywhere); saturation is belt-and-braces, not the bound.
//!
//! ## Per connection, not per machine
//!
//! Wayland connections are unbounded, so N connections hold up to 128N
//! pools. Stated rather than solved -- still strictly better than unbounded
//! per connection, the same per-connection shape as the capture-frame cap
//! and the bind budget, and cross-connection abuse is connection-count
//! territory, not a bigger pool count.

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
    /// bounded by live protocol objects the same way wayland-backend already
    /// bounds them, and a counter that only grows (the same leak in a new
    /// place) is impossible by construction.
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
    /// Called *before* delegation, so a refused pool is never created and a
    /// counted one always is: every size this sees (1 through the per-pool
    /// cap -- anything else returns before the claim) makes Smithay's
    /// `create_pool` initialise the object unconditionally, with one
    /// exception past that. If the `mmap` itself fails, Smithay posts its
    /// own error and the client is killed -- and the claim for that
    /// never-created pool leaks one unit for a dead client, which no
    /// destruction will release. That path needs real memory pressure to
    /// reach (a valid fd whose mapping fails), leaks ~one map entry per
    /// such event rather than per client or per pool, and is invisible to
    /// every live client -- stated here rather than claimed exact.
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

    /// How many pools all clients hold between them. Test-only: the flood
    /// and drain tests assert the bookkeeping empties, which a
    /// compositor-side count states directly -- while Smithay's own pool
    /// objects stay private, so no test could read them.
    #[cfg(test)]
    pub(super) fn pools_in_flight(&self) -> usize {
        self.live_per_client.values().sum::<u32>() as usize
    }
}
