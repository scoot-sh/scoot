//! flexwm's own Wayland request dispatch for [`State`].
//!
//! This is a hand-written copy of what `smithay::delegate_dispatch2!(State)`
//! expands to -- the blanket `Dispatch`/`GlobalDispatch` impls that forward
//! every request to whichever `Dispatch2` impl the object's user data
//! carries -- plus three guards on sizes a client chooses
//! ([`reject_invalid_shm_pool_resize`], [`reject_oversized_shm_pool_creation`]
//! and [`reject_unrepresentable_layer_size`]) and one post-destruction hook
//! ([`redraw_after_lock_surface_destroyed`]).
//!
//! ## Why the first guard exists
//!
//! At the Smithay rev this project pins (`0ff00983`, see `Cargo.toml`),
//! `src/wayland/shm/handlers.rs:185-190` handles `wl_shm_pool.resize`:
//!
//! ```ignore
//! Request::Resize { size } => {
//!     if size <= 0 {
//!         pool.post_error(wl_shm::Error::InvalidFd, "invalid wl_shm_pool size");
//!     }
//!     // <-- no `return`
//!     if let Err(err) = arc_pool.resize(NonZeroUsize::try_from(size as usize).unwrap()) {
//! ```
//!
//! The error is posted but execution falls straight through, so `size == 0`
//! reaches `NonZeroUsize::try_from(0).unwrap()` and panics. Any connected
//! client can reach that with `wl_shm.create_pool(fd, 1)` followed by
//! `wl_shm_pool.resize(0)` -- no special fd contents, no privilege. With
//! `panic = "abort"` (this workspace's release profile) that is not one
//! client's protocol error, which wayland-backend would contain: it takes
//! the whole compositor down, and every other client's session with it.
//! The same missing `return` is on Smithay's `master` as of 2026-09-12, so
//! there is no version to bump to.
//!
//! Negative sizes never reached the panic (`-1i32 as usize` sign-extends to
//! a huge non-zero value, which fails later in `Pool::resize` instead), so
//! the guard rejecting the whole `size <= 0` range -- which is what
//! upstream's own `if` intended -- changes nothing a client can see for
//! them: upstream already posted this exact error before falling through,
//! and only the *first* protocol error a client provokes is ever delivered,
//! so both before and after they get `wl_shm::Error::InvalidFd` /
//! "invalid wl_shm_pool size" and are disconnected (verified on a release
//! build both ways). What the guard does drop for them is the pointless
//! trip through `Pool::resize`, whose `MemMap::remap` unmaps the existing
//! mapping *before* discovering the new one can't be made.
//!
//! ## Why the second guard exists
//!
//! Nothing upstream bounds a pool's size from above either. A pool's size is
//! a client's own number, taken verbatim into `mmap` (`Pool::new`) or
//! `mremap` (`Pool::resize`), so `wl_shm.create_pool(fd, i32::MAX)` -- or the
//! same size through `resize`, which reaches the same mapping -- reserves
//! ~2 GiB of this process's address space per pool, for as long as the pool
//! lives, repeatable per pool and per connection. Unlike the `resize(0)`
//! crash above this is not a memory-safety problem (Smithay installs its own
//! SIGBUS handler for reads and writes past the backing fd's real size, so a
//! sparse pool is merely sparse), just unbounded reservation: the same
//! resource-exhaustion family as the IPC line-length cap and the screenshot
//! throttle. [`MAX_SHM_POOL_BYTES`] bounds this **per pool, not in total** --
//! nothing here stops one client from opening many pools each just under the
//! cap (measured: 40 pools at exactly the cap reserve ~20 GiB from a single
//! connection). Bounding the sum needs per-client accounting this module
//! doesn't have yet; see
//! `docs/backlog/security/shm-total-per-client-unbounded.md` for it rather than
//! treating this fix as closing that case too.
//!
//! [`MAX_SHM_POOL_BYTES`] is the bound, and a request past it is **rejected,
//! not clamped**: a clamp would leave the client and the compositor
//! disagreeing about how big the pool is, so the client would go on placing
//! buffers at offsets it believes are inside its own mapping and collect
//! confusing `invalid offset` errors from `wl_shm_pool.create_buffer` later,
//! somewhere other than the request that was actually wrong. A protocol error
//! on the oversized request says what is wrong where it is wrong, which is
//! also what upstream itself does for every other bad size.
//!
//! Rejecting `wl_shm.create_pool` means returning without initializing the
//! `New<WlShmPool>` it carries, which is safe for reasons worth recording
//! rather than re-deriving. This isn't a novel pattern -- Smithay's own
//! `create_pool` handler already does exactly this for `size <= 0`, posting
//! `InvalidStride` and returning without ever initializing its own `New`
//! (`0ff0098/src/wayland/shm/handlers.rs:68-70`) -- but two things can go
//! wrong with it, and both are covered:
//!
//! - An uninitialized object keeps wayland-backend's `UninitObjectData`,
//!   whose `request` is a `panic!` ("Received a message on an uninitialized
//!   object") -- but `post_error` calls `kill` synchronously, and
//!   `Client::next_request` returns `EPIPE` as soon as `killed` is set, so no
//!   further request from that client is ever dispatched, including one
//!   already buffered in the same `write()`.
//! - Less obviously: `common_poll.rs`'s dispatch loop has its *own* panic for
//!   exactly this shape, in the arm that runs right after a request handler
//!   returns without providing object data for a `New` it was given --
//!   `dispatch_events_for`'s `(Some(child_id), None)` arm panics unless the
//!   client is already `killed`. Same synchronous-`kill` fact covers this
//!   one too, but it's a second, independent panic site with the same
//!   precondition, not a detail of the first.
//!
//! `UninitObjectData::destroyed` is an empty no-op, so the later
//! `cleanup`/`queue_all_destructors` pass over that never-initialized object
//! does nothing either, and `wayland-server`'s `New` has no `Drop` impl to
//! assert on. (All five checked in wayland-backend 0.3.17
//! `rs/server_impl/{client,mod,common_poll}.rs` and wayland-server 0.31.14
//! `dispatch.rs`, plus Smithay's own pinned-rev source for the precedent.)
//!
//! One more precondition this whole argument leans on, generic to *every*
//! `post_error` call in this codebase, not just this one: `Client::kill`
//! runs `ClientData::disconnected` while still holding wayland-backend's
//! internal state mutex. `ClientState::disconnected` (`state.rs`) is an
//! empty no-op today, so this is safe -- but a future version of it that
//! touches the `DisplayHandle` (directly or through `State`) would deadlock
//! the compositor from inside every `post_error` call, not just this one.
//!
//! ## Why the third guard exists
//!
//! Same family, different protocol. `zwlr_layer_surface_v1.set_size` takes
//! two **`uint`**s, and this rev's handler
//! (`src/wayland/shell/wlr_layer/handlers.rs:189-193`) converts them with no
//! range check at all:
//!
//! ```ignore
//! Request::SetSize { width, height } => {
//!     let _ = with_surface_pending_state(layer_surface, |data| {
//!         data.size = (width as i32, height as i32).into();
//!     });
//! }
//! ```
//!
//! Every value above `i32::MAX` becomes negative, and
//! `Size::new` (`src/utils/geometry.rs:777`) holds a
//! `debug_assert!(w.non_negative() && h.non_negative())`. So one request --
//! `set_size(u32::MAX, u32::MAX)` from any client, no privilege, no buffer
//! -- **panics a debug build of the compositor**, taking every other
//! client's session with it, exactly like item 7's `wl_shm_pool.resize(0)`.
//! Found by this project's own layer-shell tests, which are debug builds; a
//! release build instead compiles the assertion out and carries the negative
//! size into `LayerMap::arrange`, which saturates and clamps its way to a
//! nonsense (but non-crashing) geometry for that surface.
//!
//! Refused rather than clamped, for the same reason the pool-size cap is:
//! a clamp would leave the client and the compositor disagreeing about a
//! size the client is about to draw at. `invalid_size` is the protocol's own
//! error for a size a compositor will not accept, and the rest of the
//! interface's numbers need no guard -- margins and the exclusive zone are
//! already `int`, and anchor/layer/keyboard-interactivity/exclusive-edge are
//! all validated by Smithay before use.
//!
//! ## Why the hook exists
//!
//! Not a guard at all, and not a workaround for a Smithay bug: a callback
//! this compositor needs and the protocol implementation does not offer.
//! `SessionLockHandler` has `lock`, `unlock` and `new_surface`, but nothing
//! for a lock surface *going away* -- so when a client destroys only its
//! `ext_session_lock_surface_v1`, keeping its `wl_surface`, its lock and its
//! connection (legal, and what a well-behaved locker does when an output is
//! removed), Smithay quietly unmaps the surface and no flexwm code runs at
//! all. The surface stops producing render elements, but nothing asks for the
//! frame that would show it gone, so its last pixels stay on the display
//! until something unrelated marks the screen dirty -- against the protocol's
//! own "the compositor must fall back to rendering a solid color". The
//! `destroyed` arm of the blanket impl is the only place that destruction is
//! visible, for the same reason the guards live here: it is the one seam
//! flexwm owns (see below). `session_lock.rs` holds what to do about it.
//!
//! ## Why it's shaped this way
//!
//! - **Not a `Dispatch<WlShmPool, ShmPoolUserData> for State` override.**
//!   This rev has no `delegate_shm!`; `delegate_dispatch2!` generates one
//!   *blanket* impl covering every interface at once, and Rust has no
//!   specialization, so any per-interface impl overlaps it (E0119). The
//!   blanket impl is therefore the only seam flexwm owns.
//! - **Not a reimplementation of the valid-size path.** `ShmPoolUserData`'s
//!   only field is private and `shm::pool::Pool` isn't exported, so there is
//!   no public way to perform the resize; every in-range request still goes to
//!   Smithay untouched.
//!
//! Delete the *first* guard (and the `size <= 0` half of this file's reason to
//! exist) once the pinned rev carries the missing `return`, and the *third*
//! once its `set_size` handler range-checks its own `uint`s. The pool size
//! cap is flexwm's own policy, not a workaround, so it stays -- and so does
//! the lock-surface hook, until `SessionLockHandler` grows a callback of its
//! own for it. Either one keeps this file alive on its own, whatever happens
//! to the guards, unless Smithay also grows a `delegate_shm!` to override
//! instead.
//!
//! ## Maintenance hazard this creates
//!
//! This is now a hand-maintained copy of a macro expansion, not the macro
//! itself. If a future Smithay (or wayland-server) revision adds a method
//! with a default body to `Dispatch2`/`GlobalDispatch2` (or `Dispatch`/
//! `GlobalDispatch`), this file keeps compiling -- there's no trait-method
//! count to mismatch -- and silently never calls it: no compile error, no
//! test failure, just a protocol callback quietly dropped for every
//! interface. Re-diff this file against `delegate_dispatch2!`'s expansion
//! (`cargo expand`, or the macro's own source) on every Smithay version
//! bump, not just when this bug is eventually fixed upstream.

use std::any::{Any, TypeId};

use smithay::reexports::wayland_protocols::ext::session_lock::v1::server::ext_session_lock_surface_v1;
use smithay::reexports::wayland_protocols_wlr::layer_shell::v1::server::zwlr_layer_surface_v1;
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::protocol::{wl_shm, wl_shm_pool};
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use smithay::wayland::{Dispatch2, GlobalDispatch2};

use super::State;

#[cfg(test)]
mod tests;

/// The largest `wl_shm` pool flexwm will map, in bytes (512 MiB).
///
/// Why this number, not a round one for its own sake:
///
/// - **It is four times the largest buffer any real display needs.** A
///   full-screen 8K (7680x4320) frame in a 4-byte format is 126.6 MiB, and a
///   4K one 31.6 MiB -- so 512 MiB still holds four of the former, or sixteen
///   of the latter, in a *single* pool. A client double- or triple-buffering
///   at a resolution nobody ships yet is nowhere near it; toolkits size a pool
///   to the buffers they actually attach.
/// - **It is far below where the size itself is the problem.** `i32::MAX` is
///   ~2 GiB of address space reserved per pool; this is a quarter of that, and
///   leaves the `size as usize` conversion Smithay does trivially in range.
///
/// What it deliberately does *not* claim: a total. A client may still hold
/// several pools, and bounding the sum would need per-client accounting, which
/// is a larger change than this bound and a separate concern from "one request
/// must not reserve 2 GiB". See this module's doc for why an oversized request
/// is refused rather than clamped.
const MAX_SHM_POOL_BYTES: i32 = 512 * 1024 * 1024;

impl<I, UserData> Dispatch<I, UserData> for State
where
    I: Resource,
    // Needed by the guard's `TypeId` check below. Every wayland-scanner
    // request enum is a plain owned type, so this holds for every interface
    // in the tree; an interface that ever broke it would fail to compile
    // here rather than silently losing its dispatch.
    I::Request: 'static,
    UserData: Dispatch2<I, State>,
{
    fn request(
        state: &mut Self,
        client: &Client,
        resource: &I,
        request: I::Request,
        data: &UserData,
        dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if reject_invalid_shm_pool_resize(resource, &request)
            || reject_oversized_shm_pool_creation(resource, &request)
            || reject_unrepresentable_layer_size(resource, &request)
        {
            return;
        }
        data.request(state, client, resource, request, dhandle, data_init);
    }

    fn destroyed(state: &mut Self, client: ClientId, resource: &I, data: &UserData) {
        data.destroyed(state, client, resource);
        // *After* the delegate, not before: Smithay's own
        // `ExtLockSurfaceUserData::destroyed` is what unmaps the surface, and
        // this is the redraw that shows the result.
        redraw_after_lock_surface_destroyed::<I>(state);
    }
}

impl<I, UserData> GlobalDispatch<I, UserData> for State
where
    I: Resource,
    UserData: GlobalDispatch2<I, State>,
{
    fn bind(
        state: &mut Self,
        dhandle: &DisplayHandle,
        client: &Client,
        resource: New<I>,
        data: &UserData,
        data_init: &mut DataInit<'_, Self>,
    ) {
        data.bind(state, dhandle, client, resource, data_init);
    }

    fn can_view(client: Client, data: &UserData) -> bool {
        data.can_view(&client)
    }
}

/// Posts a protocol error and returns `true` when `request` is a
/// `wl_shm_pool.resize` that must not reach Smithay -- either the `size <= 0`
/// that panics upstream, or one past [`MAX_SHM_POOL_BYTES`]. See the module
/// doc for both.
///
/// This sits on the per-request path, so it is written to cost nothing for
/// the interfaces it doesn't care about: `TypeId::of` is a `const fn`, so
/// once the caller is monomorphized both sides of the first comparison are
/// compile-time constants and the entire body folds away for every
/// interface other than `wl_shm_pool`.
fn reject_invalid_shm_pool_resize<I>(resource: &I, request: &I::Request) -> bool
where
    I: Resource,
    I::Request: 'static,
{
    if TypeId::of::<I::Request>() != TypeId::of::<wl_shm_pool::Request>() {
        return false;
    }
    let Some(wl_shm_pool::Request::Resize { size }) =
        (request as &dyn Any).downcast_ref::<wl_shm_pool::Request>()
    else {
        return false;
    };
    if *size <= 0 {
        // Same error code *and message* upstream's own (unreachable) check
        // posts, so a future Smithay bump that adds the missing `return`
        // changes nothing a client can observe.
        resource.post_error(wl_shm::Error::InvalidFd, "invalid wl_shm_pool size");
        return true;
    }
    if *size > MAX_SHM_POOL_BYTES {
        // `InvalidFd` again: it is the code upstream uses for every bad size
        // on *this* request (see the module doc's quoted handler), so a client
        // sees one consistent code for "that size is not acceptable",
        // whichever side decided it.
        resource.post_error(wl_shm::Error::InvalidFd, too_large(*size));
        return true;
    }
    false
}

/// Posts a protocol error and returns `true` when `request` is a
/// `wl_shm.create_pool` asking for more than [`MAX_SHM_POOL_BYTES`].
///
/// Separate from [`reject_invalid_shm_pool_resize`] because it guards a
/// different interface, carries a different error code, and -- the reason it
/// cannot be folded into one match -- the cap has to be applied at *both*
/// requests: `create_pool`'s size and `resize`'s size reach the same `mmap`,
/// so a cap on only one of them is no cap at all.
///
/// `size <= 0` is deliberately left alone here: upstream's `create_pool`
/// handles that correctly (it posts `InvalidStride` and, unlike `resize`,
/// does `return`), so intercepting it would only risk changing an error a
/// client already gets right.
///
/// Folds away for every interface other than `wl_shm`, for the same
/// monomorphization reason as the guard above.
fn reject_oversized_shm_pool_creation<I>(resource: &I, request: &I::Request) -> bool
where
    I: Resource,
    I::Request: 'static,
{
    if TypeId::of::<I::Request>() != TypeId::of::<wl_shm::Request>() {
        return false;
    }
    let Some(wl_shm::Request::CreatePool { size, .. }) =
        (request as &dyn Any).downcast_ref::<wl_shm::Request>()
    else {
        return false;
    };
    if *size <= MAX_SHM_POOL_BYTES {
        return false;
    }
    // `InvalidStride` is what upstream posts for a bad size on this request,
    // same principle as the resize guard's `InvalidFd`: match whichever code
    // the client would already have got for a size this handler refused.
    resource.post_error(wl_shm::Error::InvalidStride, too_large(*size));
    true
}

/// Posts a protocol error and returns `true` when `request` is a
/// `zwlr_layer_surface_v1.set_size` whose `uint` dimensions don't fit the
/// `i32` every surface-local coordinate in Wayland is measured in. See the
/// module doc for the compositor-wide panic that reaches.
///
/// Folds away for every interface other than `zwlr_layer_surface_v1`, for
/// the same monomorphization reason as the two guards above -- which matters
/// here in the same way: this runs on every request of every interface,
/// including a bar's own per-frame traffic.
fn reject_unrepresentable_layer_size<I>(resource: &I, request: &I::Request) -> bool
where
    I: Resource,
    I::Request: 'static,
{
    if TypeId::of::<I::Request>() != TypeId::of::<zwlr_layer_surface_v1::Request>() {
        return false;
    }
    let Some(zwlr_layer_surface_v1::Request::SetSize { width, height }) =
        (request as &dyn Any).downcast_ref::<zwlr_layer_surface_v1::Request>()
    else {
        return false;
    };
    const LIMIT: u32 = i32::MAX as u32;
    if *width <= LIMIT && *height <= LIMIT {
        return false;
    }
    resource.post_error(
        zwlr_layer_surface_v1::Error::InvalidSize,
        format!(
            "layer surface size {width}x{height} does not fit in the i32 \
             surface-local coordinates wayland uses (maximum {LIMIT})"
        ),
    );
    true
}

/// Tells the session-lock module that an `ext_session_lock_surface_v1` has
/// just been destroyed, which is the one protocol object destruction this
/// compositor would otherwise never hear about. See the module doc's "Why the
/// hook exists", and `session_lock.rs`'s [`State::lock_surface_destroyed`]
/// for what it has to catch up.
///
/// Folds away for every interface other than `ext_session_lock_surface_v1`,
/// for the same monomorphization reason as the three guards above: both sides
/// of the comparison are compile-time constants once this is monomorphized,
/// so every other interface's destruction pays nothing -- which matters,
/// because this sits on the destruction path of every object of every
/// interface, including a client's per-frame `wl_buffer`s.
fn redraw_after_lock_surface_destroyed<I>(state: &mut State)
where
    I: Resource,
    I::Request: 'static,
{
    if TypeId::of::<I::Request>() != TypeId::of::<ext_session_lock_surface_v1::Request>() {
        return;
    }
    state.lock_surface_destroyed();
}

/// The message both size-cap refusals carry. Allocating is fine here and
/// nowhere else in this file: this runs only on the path that has just
/// disconnected a client, never on a served request.
fn too_large(size: i32) -> String {
    format!("wl_shm pool size {size} exceeds flexwm's maximum of {MAX_SHM_POOL_BYTES} bytes")
}
