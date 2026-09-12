//! flexwm's own Wayland request dispatch for [`State`].
//!
//! This is a hand-written copy of what `smithay::delegate_dispatch2!(State)`
//! expands to -- the blanket `Dispatch`/`GlobalDispatch` impls that forward
//! every request to whichever `Dispatch2` impl the object's user data
//! carries -- plus one guard, [`reject_invalid_shm_pool_resize`].
//!
//! ## Why the guard exists
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
//! ## Why it's shaped this way
//!
//! - **Not a `Dispatch<WlShmPool, ShmPoolUserData> for State` override.**
//!   This rev has no `delegate_shm!`; `delegate_dispatch2!` generates one
//!   *blanket* impl covering every interface at once, and Rust has no
//!   specialization, so any per-interface impl overlaps it (E0119). The
//!   blanket impl is therefore the only seam flexwm owns.
//! - **Not a reimplementation of the valid-size path.** `ShmPoolUserData`'s
//!   only field is private and `shm::pool::Pool` isn't exported, so there is
//!   no public way to perform the resize; every `size > 0` request still
//!   goes to Smithay untouched.
//!
//! Delete the guard (and go back to `smithay::delegate_dispatch2!(State)`)
//! once the pinned rev carries the missing `return`.
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

use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::protocol::{wl_shm, wl_shm_pool};
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use smithay::wayland::{Dispatch2, GlobalDispatch2};

use super::State;

#[cfg(test)]
mod tests;

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
        if reject_invalid_shm_pool_resize(resource, &request) {
            return;
        }
        data.request(state, client, resource, request, dhandle, data_init);
    }

    fn destroyed(state: &mut Self, client: ClientId, resource: &I, data: &UserData) {
        data.destroyed(state, client, resource);
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
/// `wl_shm_pool.resize` that must not reach Smithay -- see the module doc.
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
    if *size > 0 {
        return false;
    }
    // Same error code upstream's own (unreachable) check posts, so a future
    // Smithay bump that adds the missing `return` changes nothing a client
    // can observe.
    resource.post_error(wl_shm::Error::InvalidFd, "invalid wl_shm_pool size");
    true
}
