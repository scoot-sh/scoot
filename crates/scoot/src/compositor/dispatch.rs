//! scoot's own Wayland request dispatch for [`State`].
//!
//! This is a hand-written copy of what `smithay::delegate_dispatch2!(State)`
//! expands to -- the blanket `Dispatch`/`GlobalDispatch` impls that forward
//! every request to whichever `Dispatch2` impl the object's user data
//! carries -- plus eight guards on what a client may ask for
//! ([`reject_invalid_shm_pool_resize`], [`reject_oversized_shm_pool_creation`],
//! [`reject_excess_shm_pool`], [`reject_excess_buffer`],
//! [`reject_unrepresentable_layer_size`],
//! [`reject_frozen_toplevel_icon_request`],
//! [`reject_excess_capture_frame`] and [`reject_too_deep_subsurface`]), one
//! pre-delegation
//! interception ([`prepare_post_destroy_lock_commit`]) and six
//! post-destruction hooks ([`redraw_after_lock_surface_destroyed`],
//! [`neutralize_destroyed_layer_surface`],
//! [`forget_destroyed_toplevel_icon`],
//! [`forget_destroyed_buffer`],
//! [`forget_destroyed_capture_frame`] and
//! [`forget_destroyed_shm_pool`]).
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
//! a byte total would need each pool's size at destroy time, which is
//! unknowable at the pinned rev (the wall is stated with sources in
//! `shm_pools.rs` rather than re-derived here). What *is* bounded is the
//! concurrency: [`reject_excess_shm_pool`] refuses a `create_pool` past
//! [`MAX_POOLS_PER_CLIENT`](super::shm_pools::MAX_POOLS_PER_CLIENT) live
//! pool objects for the requesting client -- which bounds live pool objects
//! and the address-space envelope (count x 512 MiB sparse), not the fds or
//! mappings a buffer surviving its pool retains: destroying a pool object
//! frees neither while its buffers live, so that quantity is bounded
//! separately by the per-client live-`wl_buffer` count (see `shm_pools.rs`
//! and `wl_buffers.rs`). See
//! `docs/backlog/resolved/shm-pool-count-cap-done.md` for the byte
//! half rather than treating either fix as closing that case too.
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
//! internal state mutex. `ClientState::disconnected` (`state.rs`) is
//! logging-only, so this is safe -- but a future version of it that
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
//! ## Why the fourth guard exists
//!
//! The same missing `return` as the first guard, in a different protocol, and
//! it only became reachable when this compositor started advertising
//! `xdg_toplevel_icon_manager_v1`. This rev's
//! `src/wayland/xdg_toplevel_icon.rs:355-372` handles both requests that
//! mutate an icon:
//!
//! ```ignore
//! Request::SetName { icon_name } => {
//!     if self.is_immutable() {
//!         icon.post_error(
//!             xdg_toplevel_icon_v1::Error::Immutable,
//!             "Request made after the icon has been assigned to a toplevel via 'set_icon'"
//!         );
//!     }
//!     // <-- no `return`
//!     self.set_icon_name(icon_name);
//! ```
//!
//! and `set_icon_name` opens with `debug_assert!(!self.is_immutable())`
//! (line 149). So `create_icon` -> `set_name` -> `set_icon` -> `set_name`
//! **panics a debug build of the compositor**, from any client, with no
//! privilege and no special buffer -- exactly the shape of the `resize(0)`
//! crash above. `AddBuffer` (line 362) falls through into `add_buffer`, whose
//! own `debug_assert!` is at line 192, so it is a second trigger for the same
//! bug. Reproduced live against this compositor before the guard existed;
//! `toplevel_icon/tests.rs` keeps both arms covered.
//!
//! Release builds compile the assertion out, so this is a debug-build crash
//! -- but every build this project develops, tests, dev-VM-runs and
//! smoke-tests with is a debug build (`scripts/smoke-test.sh` defaults to
//! one), and a buggy toolkit reaches it as easily as a malicious client.
//!
//! Unlike the three guards above, this one needs *state*: whether an icon has
//! been assigned lives in `XdgToplevelIconUserData::constructed`, which is
//! private with no public accessor, so scoot cannot ask. It tracks the
//! assignment itself instead -- `set_icon` is the request that freezes an
//! icon, and it passes through this very function -- in
//! [`State::frozen_icons`](super::State), owned by `toplevel_icon.rs`. Delete
//! this guard, and that set, once the pinned rev's two error arms `return`.
//!
//! ## Why the fifth guard exists
//!
//! The pinned Smithay rev (`0ff00983`)
//! `src/wayland/image_copy_capture/mod.rs` handles
//! `ext_image_copy_capture_session_v1.create_frame` by initialising the frame
//! object and pushing it onto the session's `active_frames` with no cap, no
//! pruning, and no `duplicate_frame` -- even though the protocol allows at
//! most one frame object per session at a time ("If a client sends a
//! create_frame request before a previous frame object has been destroyed,
//! the duplicate_frame protocol error is raised"). And scoot's own
//! `Capture::pending` throttle never sees this shape at all: `frame()`, where
//! that throttle lives, is only reached from the client's `capture` request,
//! never from `create_frame` itself. So one client, one session, and a
//! `create_frame` loop with no `capture` ever sent is unbounded protocol
//! objects -- plus a `Vec` Smithay's `capture` dispatch then scans linearly
//! on every capture, and walks again on every frame teardown.
//!
//! The guard refuses a `create_frame` past
//! [`MAX_FRAMES_PER_CLIENT`](super::screencopy::MAX_FRAMES_PER_CLIENT) live
//! frames for the requesting client, before Smithay's handler ever sees it.
//! Two deliberate choices, both worth recording rather than re-deriving:
//!
//! - **Per client, not per session.** The protocol's rule is per session, but
//!   the pinned rev exposes no way to learn *which* session a `create_frame`
//!   names before delegating it: `Session`/`SessionRef` carry no protocol id
//!   or client accessor, `SessionData`'s fields are private with no accessor,
//!   and `ImageCaptureSource` (the one thing `capture_constraints` sees)
//!   names the source, not the session or its client. What this seam *does*
//!   see is the [`Client`](smithay::reexports::wayland_server::Client), so
//!   the bound is keyed by that -- and it still bounds the ticket's exact
//!   attack, which is single-session.
//! - **A protocol error, not a silent ignore.** The spec says the error "is
//!   raised", and a silent ignore would be worse than the leak: returning
//!   without initialising the request's `New` leaves wayland-backend's
//!   `UninitObjectData` in place, whose `request` is a `panic!` -- and the
//!   dispatch loop's own `(Some(child_id), None)` arm panics too unless the
//!   client is already `killed` (wayland-backend 0.3.17
//!   `rs/server_impl/{mod.rs:126,common_poll.rs:288-296}`). Either one takes
//!   the whole compositor down the moment the client touches the frame it was
//!   never given. `post_error` kills synchronously, so both are covered, by
//!   the same argument the `wl_shm` guards above already make.
//!
//! A well-behaved client cannot trip this by racing its own frame lifecycle:
//! requests on one connection are dispatched in order, so a `destroy` the
//! client sent always runs before a later `create_frame`, and a session's
//! objects belong to exactly one client. The only clients that see
//! `duplicate_frame` are ones holding more live frames than the cap --
//! `grim`, which holds one frame per run, and the persistent-preview shape,
//! which holds one per session, are nowhere near it. And the kill is
//! per-client: one client's greed can only ever disconnect that client,
//! never deny a well-behaved one (the global-cap shape the IPC connection-cap
//! entry already filed against).
//!
//! The count itself lives in `screencopy.rs` (`frames_per_client`), counted
//! up here and back down in the destruction hook below; see
//! [`State::refuse_excess_capture_frame`](super::State) for why delegation
//! after a count cannot leak it. Delete neither half without the other.
//!
//! ## Why the sixth guard exists
//!
//! The live-pool count ([`reject_excess_shm_pool`]) does not bound fds or
//! mappings, whatever its first docs said: destroying a pool object frees
//! neither while a buffer created from it survives, so `create_pool` /
//! `create_buffer` / `destroy_pool` in a loop retains one fd and mapping
//! per iteration with the pool count back at zero (see `wl_buffers.rs` and
//! `docs/backlog/resolved/shm-pool-cap-misses-retained-fds-done.md`). The guard
//! refuses a buffer creation past
//! [`MAX_BUFFERS_PER_CLIENT`](super::wl_buffers::MAX_BUFFERS_PER_CLIENT)
//! live buffers for the requesting client, before Smithay's handler ever
//! sees it -- every bypass iteration must keep a buffer alive, so the cap
//! catches exactly the bypass shape, whatever it does with the pool object.
//!
//! Three deliberate choices, all worth recording rather than re-deriving:
//!
//! - **Every `wl_buffer` factory, not just pools.** The release hook below
//!   sees only that *a* buffer died -- never which kind -- so a selective
//!   count would drift fail-open (destroys of uncounted cheap buffers
//!   draining units claimed by retaining ones). Single-pixel buffers are
//!   counted too, even though they hold nothing: uniformity is what keeps
//!   the scalar pairing exact. The dmabuf async `create` claims like the
//!   rest since scoot started importing dmabufs for real: a successful
//!   import mints a `wl_buffer` on that path too (see `dmabuf.rs`).
//! - **Claimed unconditionally, exact for live clients by mechanism.**
//!   Smithay initialises the buffer or kills the client, never neither, so
//!   a failed creation's phantom unit lands on an already-dead entry only
//!   (at most one per killing connection; see `wl_buffers.rs`). The one
//!   creation that can be refused with the client left alive -- an async
//!   dmabuf `create` the renderer will not import -- hands its unit back in
//!   `dmabuf.rs`'s `refuse_import`, which is where that refusal is decided.
//!   No
//!   upstream parameter validation is replicated here -- that would couple
//!   this guard to Smithay's handler logic and drift fail-open on a rev
//!   bump, while over-counting a dead client is the safe direction.
//! - **A protocol error on the creating object, per interface.** `shm` gets
//!   `InvalidStride` on the pool (Smithay's own code for bad buffer
//!   parameters there), dmabuf `InvalidWlBuffer` on the params (Smithay's
//!   own immed-import failure code), single-pixel a bare 0 on the manager
//!   (the interface defines no errors at all). Same kill-one-client shape
//!   as every guard above: a silent ignore would leave the uninitialized
//!   object that panics the compositor.
//!
//! A well-behaved client cannot trip this by racing its own buffer
//! lifecycle: requests on one connection are dispatched in order, so a
//! `destroy` the client sent always runs before a later `create_buffer`.
//! `foot` holds 2 live buffers steady (measured); the cap sits at 512.
//! The count itself lives in `wl_buffers.rs`, counted up here and back down
//! in the destruction hook below. Delete neither half without the other.
//!
//! ## Why the subsurface-depth guard exists
//!
//! [`reject_too_deep_subsurface`] bounds how deep `wl_subsurface`s nest,
//! because every Smithay walk over a surface tree recurses once per level
//! and a deep enough tree overflows the compositor's stack. The rule, and
//! why it has to count the height of the subtree being attached and not
//! just the new parent's depth, is in `subsurface_depth.rs`. It is a guard
//! here, not a check in `CompositorHandler::new_subsurface`, because Smithay
//! links the two surfaces -- after running its own recursive `is_ancestor`
//! up the new parent's chain -- before it calls that, and does not pass it
//! the `wl_subcompositor` the protocol's `bad_parent` belongs on. Refused
//! here, the link is never made, and `bad_parent` goes on the object the
//! request was sent to.
//!
//! ## Why the hook exists
//!
//! Not a guard at all, and not a workaround for a Smithay bug: a callback
//! this compositor needs and the protocol implementation does not offer.
//! `SessionLockHandler` has `lock`, `unlock` and `new_surface`, but nothing
//! for a lock surface *going away* -- so when a client destroys only its
//! `ext_session_lock_surface_v1`, keeping its `wl_surface`, its lock and its
//! connection (legal, and what a well-behaved locker does when an output is
//! removed), Smithay quietly unmaps the surface and no scoot code runs at
//! all. The surface stops producing render elements, but nothing asks for the
//! frame that would show it gone, so its last pixels stay on the display
//! until something unrelated marks the screen dirty -- against the protocol's
//! own "the compositor must fall back to rendering a solid color". The
//! `destroyed` arm of the blanket impl is the only place that destruction is
//! visible, for the same reason the guards live here: it is the one seam
//! scoot owns (see below). `session_lock.rs` holds what to do about it.
//!
//! ## Why it's shaped this way
//!
//! - **Not a `Dispatch<WlShmPool, ShmPoolUserData> for State` override.**
//!   This rev has no `delegate_shm!`; `delegate_dispatch2!` generates one
//!   *blanket* impl covering every interface at once, and Rust has no
//!   specialization, so any per-interface impl overlaps it (E0119). The
//!   blanket impl is therefore the only seam scoot owns.
//! - **Not a reimplementation of the valid-size path.** `ShmPoolUserData`'s
//!   only field is private and `shm::pool::Pool` isn't exported, so there is
//!   no public way to perform the resize; every in-range request still goes to
//!   Smithay untouched.
//!
//! Delete the *first* guard (and the `size <= 0` half of this file's reason to
//! exist) once the pinned rev carries the missing `return`, and the *third*
//! once its `set_size` handler range-checks its own `uint`s. The pool size
//! cap is scoot's own policy, not a workaround, so it stays -- and so does
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

//! ## Why the interception exists
//!
//! Not a guard either: unlike the three above it never refuses a request.
//! Destroying only an `ext_session_lock_surface_v1` role object (legal, and
//! what a locker does when an output is removed under it -- or, as a real
//! Quickshell client proved, on every unlock) makes Smithay reset that
//! surface's role state, and the client's next commit on the surviving
//! `wl_surface` then trips the role's commit-time validation on the reset:
//! `pre_commit_hook` demands the ack first, so any post-destroy commit --
//! bare or null -- posts `CommitBeforeFirstAck`. Restoring the ack alone
//! would only promote a null commit to the next check, `NullBuffer`, which
//! is why the interception also clears the pending `Removed`, turning the
//! null commit into the bare commit the now-unmapped surface means.
//! Either kills the client -- on unlock, its entire shell. See
//! `docs/backlog/resolved/session-lock-post-destroy-commit-resolved.md`, and
//! [`State::prepare_post_destroy_lock_commit`](super::session_lock) for what
//! this prepares and why the carve-out reaches only destroyed-role surfaces.
//!
//! It has to run here rather than in `CompositorHandler::commit` because the
//! kill happens in Smithay's pre-commit hooks, which run before that: this
//! blanket `request` is the only seam scoot owns ahead of them. It delegates
//! afterwards unconditionally -- the commit still applies -- so unlike a guard
//! it has no refusal path and posts no protocol error, which is also why the
//! module doc's `Client::kill` mutex precondition is unaffected by it:
//! nothing here can reach `ClientState::disconnected` holding the
//! backend mutex any differently than the delegated request already could.

use std::any::{Any, TypeId};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd};

use smithay::reexports::wayland_protocols::ext::image_copy_capture::v1::server::{
    ext_image_copy_capture_frame_v1, ext_image_copy_capture_session_v1,
};
use smithay::reexports::wayland_protocols::ext::session_lock::v1::server::ext_session_lock_surface_v1;
use smithay::reexports::wayland_protocols::wp::linux_dmabuf::zv1::server::zwp_linux_buffer_params_v1;
use smithay::reexports::wayland_protocols::wp::single_pixel_buffer::v1::server::wp_single_pixel_buffer_manager_v1;
use smithay::reexports::wayland_protocols::xdg::toplevel_icon::v1::server::{
    xdg_toplevel_icon_manager_v1, xdg_toplevel_icon_v1,
};
use smithay::reexports::wayland_protocols_wlr::layer_shell::v1::server::zwlr_layer_surface_v1;
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::protocol::{
    wl_buffer, wl_shm, wl_shm_pool, wl_subcompositor, wl_surface,
};
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use smithay::wayland::{Dispatch2, GlobalDispatch2};

use super::State;

/// Test-only, and `pub(super)` for one reason: the fd-flood serialisation
/// lock in `tests` is shared with the icon-buffer flood in
/// `toplevel_icon/tests.rs`, which holds the same ~512 server fds and
/// would exhaust the test process's shared fd table running beside one
/// of these floods.
#[cfg(test)]
pub(super) mod tests;

/// The largest `wl_shm` pool scoot will map, in bytes (512 MiB).
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
/// What it deliberately does *not* claim: a byte total, or an fd/mapping
/// total. A client may hold
/// up to [`MAX_POOLS_PER_CLIENT`](super::shm_pools::MAX_POOLS_PER_CLIENT)
/// live pool objects, and bounding the byte sum would need each pool's size
/// at destroy time, which is unknowable at the pinned rev (see
/// `shm_pools.rs`) -- and a destroyed pool's fd and mapping outlive it
/// while its buffers do, so those are bounded by the live-`wl_buffer` count
/// instead (see `wl_buffers.rs`). The live-object concurrency is what
/// [`reject_excess_shm_pool`] bounds. See this module's doc
/// for why an oversized request is refused rather than clamped.
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
            || reject_excess_shm_pool(state, client, resource, &request)
            || reject_excess_buffer(state, client, resource, &request)
            || reject_unrepresentable_layer_size(resource, &request)
            || reject_frozen_toplevel_icon_request(state, resource, &request)
            || reject_excess_capture_frame(state, client, resource, &request)
            || reject_too_deep_subsurface(resource, &request)
        {
            return;
        }
        note_assigned_toplevel_icon::<I>(state, &request);
        prepare_post_destroy_lock_commit(state, resource, &request, dhandle);
        data.request(state, client, resource, request, dhandle, data_init);
    }

    fn destroyed(state: &mut Self, client: ClientId, resource: &I, data: &UserData) {
        // *Before* the delegate, unlike every hook below: this only touches
        // scoot's own per-client frame count, which Smithay's teardown
        // neither reads nor writes, and `data.destroyed` moves `client`.
        forget_destroyed_capture_frame::<I>(state, &client, resource);
        forget_destroyed_shm_pool::<I>(state, &client, resource);
        forget_destroyed_buffer::<I>(state, &client, resource);
        data.destroyed(state, client, resource);
        // *After* the delegate, not before: Smithay's own
        // `ExtLockSurfaceUserData::destroyed` is what unmaps the surface, and
        // this is the redraw that shows the result. Same ordering reason for
        // the layer neutralize below: Smithay's layer destruction handler
        // resets the surface's layer state after scoot's `layer_destroyed`
        // has run, so only something here can prepare its next commit.
        redraw_after_lock_surface_destroyed::<I>(state);
        neutralize_destroyed_layer_surface::<I>(state);
        forget_destroyed_toplevel_icon::<I>(state, resource);
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
/// `wl_shm.create_pool` that would push its client past
/// [`MAX_POOLS_PER_CLIENT`](super::shm_pools::MAX_POOLS_PER_CLIENT) live
/// pools.
///
/// Only creations are claimed: a `resize` grows the pool it names rather
/// than opening a new one, so it leaves the count alone, and a destroy
/// releases through [`forget_destroyed_shm_pool`] below. Sizes the per-pool
/// cap or upstream already refuse (`size <= 0`, past
/// [`MAX_SHM_POOL_BYTES`]) never reach the claim -- each is refused (and the
/// client killed) without ever creating a pool, so counting one would leak
/// a unit no destruction could release. Checking both here rather than
/// leaning on the chain order is what keeps that true however the guards
/// are ordered.
///
/// A third never-creates shape needs its own check: a valid size on an
/// fd Smithay cannot map (`/dev/null`, an `O_RDONLY` fd with a `WRITE`
/// mapping, ...). Smithay posts `InvalidFd` and returns without
/// initialising the pool, whose `UninitObjectData::destroyed` is a no-op --
/// so no destruction hook ever runs for it, disconnect cleanup included,
/// and a claim would leak one unit per connection, attacker-paced, with no
/// memory pressure at all. [`pool_fd_mappable`] probes the exact mapping
/// first (same call Smithay is about to make), and a probe failure is
/// refused with Smithay's own code and message, uncounted.
///
/// The refusal is `InvalidStride` on `wl_shm`, the same code (and object)
/// the per-pool cap's own creation refusal uses: one consistent answer for
/// "this pool cannot be opened", whichever bound said so. (The probe
/// failure above is the exception: `InvalidFd`, matching what Smithay
/// posts for the same fd.)
///
/// Past [`PRESSURE_GRACE_POOLS`](super::fd_pressure::PRESSURE_GRACE_POOLS)
/// live pools *and* a pressured process table, the same refusal answers
/// for the compositor-wide ceiling instead of the per-client one (see
/// `fd_pressure` for why the grace makes this a ceiling rather than a
/// lottery). Checked before the claim, so a pressure refusal never takes a
/// count unit it would then have to give back: the bookkeeping stays
/// balanced by construction, not by a compensating release.
///
/// Folds away for every interface other than `wl_shm`, for the same
/// monomorphization reason as the guards above -- which matters here too:
/// this runs on every request of every interface.
fn reject_excess_shm_pool<I>(
    state: &mut State,
    client: &Client,
    resource: &I,
    request: &I::Request,
) -> bool
where
    I: Resource,
    I::Request: 'static,
{
    if TypeId::of::<I::Request>() != TypeId::of::<wl_shm::Request>() {
        return false;
    }
    let Some(wl_shm::Request::CreatePool { size, fd, .. }) =
        (request as &dyn Any).downcast_ref::<wl_shm::Request>()
    else {
        return false;
    };
    if *size <= 0 || *size > MAX_SHM_POOL_BYTES {
        return false;
    }
    if !pool_fd_mappable(fd.as_fd(), *size as usize) {
        // Same code *and message* Smithay's own `create_pool` posts when
        // its identical mapping fails, so whichever side refused, the
        // client learns one thing: this fd cannot back a pool.
        resource.post_error(
            wl_shm::Error::InvalidFd,
            format!("Failed to mmap fd {}", fd.as_raw_fd()),
        );
        return true;
    }
    if pressure_refusal(
        state.shm_pools.live_for(client),
        super::fd_pressure::PRESSURE_GRACE_POOLS,
    ) {
        resource.post_error(
            wl_shm::Error::InvalidStride,
            too_many_pools_under_pressure(),
        );
        return true;
    }
    if !state.shm_pools.refuse_pool_creation(client) {
        return false;
    }
    resource.post_error(wl_shm::Error::InvalidStride, too_many_pools());
    true
}

/// Probes whether `fd` can be mapped exactly the way Smithay's
/// `create_pool` is about to: `mmap(NULL, size, READ|WRITE, SHARED, fd, 0)`,
/// unmapped straight away.
///
/// Parameter-for-parameter replication of `0ff0098`
/// `src/wayland/shm/pool.rs`'s `map()` -- which is what makes a probe
/// failure (or success) predictive rather than heuristic: the two syscalls
/// run back-to-back in one dispatch, same thread, same open file
/// description, same size, so they agree unless another thread sharing the
/// fd changes its mappability in between. Both directions of that race are
/// safe-shaped: a probe failure refuses *without claiming* (no leak on any
/// path), and a probe success Smithay then fails to repeat kills the
/// client with at most one phantom unit for a dead connection -- the
/// pre-probe leak, now reachable only through a nanosecond TOCTOU instead
/// of a deterministic loop. A false refusal of a legitimate client needs
/// the same race in reverse (e.g. transient `ENOMEM` healing between the
/// two calls).
///
/// Costs one `mmap`+`munmap` per `create_pool` -- lazy mappings, no page
/// tables until touched -- and runs nowhere else (the `wl_shm` `TypeId`
/// gate above already excluded every other interface, and `resize` carries
/// no fd: its pool object already exists, so a failed `remap` kills a
/// client whose pools drain through the normal hook, exactly).
fn pool_fd_mappable(fd: BorrowedFd<'_>, size: usize) -> bool {
    if size == 0 {
        return false;
    }
    // SAFETY: `addr` is null (the kernel chooses the address), `length` is
    // nonzero (checked above), and `fd` is a valid borrowed fd. The mapping
    // is never touched through the returned pointer -- no read, no write,
    // so no SIGBUS from short backing and no aliasing -- it exists only to
    // learn whether the call succeeds, and is unmapped below.
    let mapped = unsafe {
        rustix::mm::mmap(
            std::ptr::null_mut(),
            size,
            rustix::mm::ProtFlags::READ | rustix::mm::ProtFlags::WRITE,
            rustix::mm::MapFlags::SHARED,
            fd,
            0,
        )
    };
    let Ok(ptr) = mapped else {
        return false;
    };
    // SAFETY: `ptr`/`size` are exactly what the successful `mmap` above
    // returned/was given, unmapped here before anything else runs, so this
    // cannot double-unmap or unmap anything else's. A just-made mapping
    // cannot fail to unmap (Smithay's own `unmap` says the same); the
    // result is irrelevant either way.
    let _ = unsafe { rustix::mm::munmap(ptr, size) };
    true
}

/// Forgets one live `wl_shm` pool when its protocol object dies, which is
/// what keeps [`MAX_POOLS_PER_CLIENT`](super::shm_pools::MAX_POOLS_PER_CLIENT)'s
/// bookkeeping exact: every counted `create_pool` is paired with exactly one
/// destruction, including on client disconnect (whose cleanup destroys every
/// object) and on a protocol-error kill.
///
/// Folds away for every interface other than `wl_shm_pool`, which matters in
/// the same way as the hooks above: this sits on the destruction path of
/// every object of every interface.
fn forget_destroyed_shm_pool<I>(state: &mut State, client: &ClientId, _resource: &I)
where
    I: Resource,
    I::Request: 'static,
{
    if TypeId::of::<I::Request>() != TypeId::of::<wl_shm_pool::Request>() {
        return;
    }
    state.shm_pools.forget_pool(client);
}

/// Posts a protocol error and returns `true` when `request` is a
/// `wl_buffer` creation that would push its client past
/// [`MAX_BUFFERS_PER_CLIENT`](super::wl_buffers::MAX_BUFFERS_PER_CLIENT)
/// live buffers.
///
/// Four creation sites, one budget (see the module doc's "Why the sixth
/// guard exists" for why the count is uniform): `wl_shm_pool.create_buffer`,
/// *both* `zwp_linux_buffer_params_v1.create_immed` and its asynchronous
/// sibling `create`, and
/// `wp_single_pixel_buffer_manager_v1.create_u32_rgba_buffer`.
/// Every other request of every other interface -- including `wl_shm_pool`
/// `resize`/`destroy` and params `add`/`destroy` -- falls through.
///
/// `create` claims because scoot now really imports dmabufs
/// (`dmabuf.rs`): `ImportNotifier::successful` on a `Falliable` notifier
/// mints a real, fd-retaining `wl_buffer`, so leaving that path uncounted
/// would let a GL client hold unbounded buffers outside
/// `MAX_BUFFERS_PER_CLIENT` entirely. It is also the one creation whose
/// *refusal* leaves the client alive, so `dmabuf.rs`'s `refuse_import` hands
/// the unit back when the renderer says no -- see its doc for why that
/// release is written to be correct on the `create_immed` path too.
///
/// The refusal is posted on the creating object with that interface's own
/// code for a creation that cannot be honoured (`InvalidStride` on the
/// pool, `InvalidWlBuffer` on the params; the single-pixel manager defines
/// no errors, so a bare 0 with an explicit message). `create` is refused the
/// same fatal way as `create_immed` rather than with the protocol's softer
/// `failed` event, because the two are one budget with one message and only
/// a client already holding 512 live buffers ever sees either -- abuse by
/// construction, where the kill is the message. Same uninitialized-object
/// argument as the pool guards: returning without initialising the request's
/// `New` is safe only because `post_error` kills synchronously -- and
/// `create` has no `New` at all, so it is safer still.
///
/// Past [`PRESSURE_GRACE_BUFFERS`](super::fd_pressure::PRESSURE_GRACE_BUFFERS)
/// live buffers *and* a pressured process table, the same refusal answers
/// for the compositor-wide ceiling (see `fd_pressure`, and the pool guard
/// above for the check-before-claim shape that keeps the count balanced).
///
/// Folds away for every interface other than the three factories, for the
/// same monomorphization reason as the guards above -- which matters here
/// too: this runs on every request of every interface.
fn reject_excess_buffer<I>(
    state: &mut State,
    client: &Client,
    resource: &I,
    request: &I::Request,
) -> bool
where
    I: Resource,
    I::Request: 'static,
{
    if TypeId::of::<I::Request>() == TypeId::of::<wl_shm_pool::Request>() {
        let Some(wl_shm_pool::Request::CreateBuffer { .. }) =
            (request as &dyn Any).downcast_ref::<wl_shm_pool::Request>()
        else {
            return false;
        };
        if pressure_refusal(
            state.wl_buffers.live_for(client),
            super::fd_pressure::PRESSURE_GRACE_BUFFERS,
        ) {
            resource.post_error(
                wl_shm::Error::InvalidStride,
                too_many_buffers_under_pressure(),
            );
            return true;
        }
        if !state.wl_buffers.claim_buffer_creation(client) {
            return false;
        }
        resource.post_error(wl_shm::Error::InvalidStride, too_many_buffers());
        return true;
    }
    if TypeId::of::<I::Request>() == TypeId::of::<zwp_linux_buffer_params_v1::Request>() {
        // Both factories on this interface, not just the immediate one: each
        // produces exactly one `wl_buffer` that the destruction hook will
        // release (see this function's doc and `dmabuf.rs`).
        let creates_a_buffer = matches!(
            (request as &dyn Any).downcast_ref::<zwp_linux_buffer_params_v1::Request>(),
            Some(
                zwp_linux_buffer_params_v1::Request::CreateImmed { .. }
                    | zwp_linux_buffer_params_v1::Request::Create { .. }
            )
        );
        if !creates_a_buffer {
            return false;
        };
        if pressure_refusal(
            state.wl_buffers.live_for(client),
            super::fd_pressure::PRESSURE_GRACE_BUFFERS,
        ) {
            resource.post_error(
                zwp_linux_buffer_params_v1::Error::InvalidWlBuffer,
                too_many_buffers_under_pressure(),
            );
            return true;
        }
        if !state.wl_buffers.claim_buffer_creation(client) {
            return false;
        }
        resource.post_error(
            zwp_linux_buffer_params_v1::Error::InvalidWlBuffer,
            too_many_buffers(),
        );
        return true;
    }
    if TypeId::of::<I::Request>() == TypeId::of::<wp_single_pixel_buffer_manager_v1::Request>() {
        let Some(wp_single_pixel_buffer_manager_v1::Request::CreateU32RgbaBuffer { .. }) =
            (request as &dyn Any).downcast_ref::<wp_single_pixel_buffer_manager_v1::Request>()
        else {
            return false;
        };
        if pressure_refusal(
            state.wl_buffers.live_for(client),
            super::fd_pressure::PRESSURE_GRACE_BUFFERS,
        ) {
            resource.post_error(0u32, too_many_buffers_under_pressure());
            return true;
        }
        if !state.wl_buffers.claim_buffer_creation(client) {
            return false;
        }
        // No `Error` enum exists on this interface (verified against the
        // protocol XML), so there is no code to name: 0 with a message that
        // says what happened. Only a client already holding 512 live
        // buffers ever sees it.
        resource.post_error(0u32, too_many_buffers());
        return true;
    }
    false
}

/// Forgets one live `wl_buffer` when its protocol object dies, which is
/// what keeps [`MAX_BUFFERS_PER_CLIENT`](super::wl_buffers::MAX_BUFFERS_PER_CLIENT)'s
/// bookkeeping exact: every counted creation is paired with exactly one
/// destruction, including on client disconnect (whose cleanup destroys every
/// object) and on a protocol-error kill. Fires for buffers of every kind --
/// which is what keeps the uniform count uniform (see `wl_buffers.rs`).
///
/// Also queues the drain of the renderer's dmabuf mapping cache, which is the
/// *other* thing a dying `wl_buffer` may have made collectable and which
/// nothing else in the compositor would notice: a destroyed buffer causes no
/// damage, so no frame is asked for, so nothing calls the renderer's own
/// cleanup. See `dmabuf.rs`'s `schedule_cache_drain` -- it is a no-op (one
/// bool test) until this session imports its first dmabuf, and it queues at
/// most one idle per dispatch however many buffers died in it. The kind of
/// the dying buffer is deliberately not consulted, for the same reason the
/// count above is uniform: the hook cannot observe it.
///
/// Folds away for every interface other than `wl_buffer`, which matters in
/// the same way as the hooks above: this sits on the destruction path of
/// every object of every interface.
fn forget_destroyed_buffer<I>(state: &mut State, client: &ClientId, _resource: &I)
where
    I: Resource,
    I::Request: 'static,
{
    if TypeId::of::<I::Request>() != TypeId::of::<wl_buffer::Request>() {
        return;
    }
    state.wl_buffers.forget_buffer(client);
    super::dmabuf::schedule_cache_drain(state);
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

/// Prepares a `wl_surface.commit` on a lock surface whose role object has
/// already been destroyed, before Smithay's own commit handler (and its
/// pre-commit hooks) sees it. See the module doc's "Why the interception
/// exists", and `session_lock.rs`'s [`State::prepare_post_destroy_lock_commit`]
/// for what it prepares.
///
/// Unlike the guards above this never refuses the request: the commit is
/// always delegated afterwards. Folds away for every interface other than
/// `wl_surface`, and for every `wl_surface` request other than `commit`, for
/// the same monomorphization reason as the guards -- which matters in the same
/// way: this runs on every request of every interface.
fn prepare_post_destroy_lock_commit<I>(
    state: &mut State,
    resource: &I,
    request: &I::Request,
    dhandle: &DisplayHandle,
) where
    I: Resource,
    I::Request: 'static,
{
    if TypeId::of::<I::Request>() != TypeId::of::<wl_surface::Request>() {
        return;
    }
    let Some(wl_surface::Request::Commit) =
        (request as &dyn Any).downcast_ref::<wl_surface::Request>()
    else {
        return;
    };
    // The request's interface is `wl_surface`, so its resource is the
    // committed surface itself -- recovered by object id rather than by
    // downcasting `resource`, which would need an `I: 'static` bound this
    // blanket impl does not (and should not) carry.
    state.prepare_post_destroy_lock_commit(resource.id(), dhandle);
}

/// Neutralizes the pending layer state of every surface whose
/// `zwlr_layer_surface_v1` role has just been destroyed, so its next commit
/// cannot trip the role's commit-time size validation on the default state
/// Smithay's own destruction handler leaves behind. See `layer_shell.rs`'s
/// [`State::neutralize_destroyed_layers`] for why this has to run here
/// rather than in `layer_destroyed`, and what it writes.
///
/// Folds away for every interface other than `zwlr_layer_surface_v1`, for
/// the same monomorphization reason as the guards above -- which matters in
/// the same way: this sits on the destruction path of every object of every
/// interface.
fn neutralize_destroyed_layer_surface<I>(state: &mut State)
where
    I: Resource,
    I::Request: 'static,
{
    if TypeId::of::<I::Request>() != TypeId::of::<zwlr_layer_surface_v1::Request>() {
        return;
    }
    state.neutralize_destroyed_layers();
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

/// Posts a protocol error and returns `true` when `request` would mutate an
/// `xdg_toplevel_icon_v1` that has already been assigned to a toplevel. See
/// the module doc for the upstream fall-through that reaches a
/// `debug_assert!` and takes the whole compositor with it.
///
/// Both mutating requests are covered, because both fall through the same
/// way. `Destroy` is not a mutation and is left alone.
///
/// Folds away for every interface other than `xdg_toplevel_icon_v1`, for the
/// same monomorphization reason as the guards above.
fn reject_frozen_toplevel_icon_request<I>(
    state: &mut State,
    resource: &I,
    request: &I::Request,
) -> bool
where
    I: Resource,
    I::Request: 'static,
{
    if TypeId::of::<I::Request>() != TypeId::of::<xdg_toplevel_icon_v1::Request>() {
        return false;
    }
    let Some(
        xdg_toplevel_icon_v1::Request::SetName { .. }
        | xdg_toplevel_icon_v1::Request::AddBuffer { .. },
    ) = (request as &dyn Any).downcast_ref::<xdg_toplevel_icon_v1::Request>()
    else {
        return false;
    };
    // The request's interface is `xdg_toplevel_icon_v1`, so its resource is
    // the icon itself -- recovered through the client's own object rather
    // than by downcasting `resource`, which would need an `I: 'static` bound
    // this blanket impl does not (and should not) carry.
    let Some(icon) = state
        .display_handle
        .get_client(resource.id())
        .ok()
        .and_then(|client| {
            client
                .object_from_protocol_id::<xdg_toplevel_icon_v1::XdgToplevelIconV1>(
                    &state.display_handle,
                    resource.id().protocol_id(),
                )
                .ok()
        })
    else {
        return false;
    };
    state.refuse_frozen_toplevel_icon(&icon)
}

/// Records an icon as frozen when `request` is the
/// `xdg_toplevel_icon_manager_v1.set_icon` that hands it to a toplevel.
///
/// Runs *before* delegation, which is what the guard above needs: upstream
/// freezes the icon inside its own handling of this same request, so
/// recording it here means the very next request on that icon is already
/// refusable. `set_icon(toplevel, None)` carries no icon and freezes nothing.
///
/// Folds away for every interface other than `xdg_toplevel_icon_manager_v1`.
fn note_assigned_toplevel_icon<I>(state: &mut State, request: &I::Request)
where
    I: Resource,
    I::Request: 'static,
{
    if TypeId::of::<I::Request>() != TypeId::of::<xdg_toplevel_icon_manager_v1::Request>() {
        return;
    }
    let Some(xdg_toplevel_icon_manager_v1::Request::SetIcon {
        icon: Some(icon), ..
    }) = (request as &dyn Any).downcast_ref::<xdg_toplevel_icon_manager_v1::Request>()
    else {
        return;
    };
    state.note_toplevel_icon_assigned(icon.id());
}

/// Drops a destroyed `xdg_toplevel_icon_v1` from the frozen set, which is what
/// keeps that set bounded by the client's live objects. See
/// [`State::forget_toplevel_icon`](super::State).
///
/// Folds away for every interface other than `xdg_toplevel_icon_v1`, which
/// matters in the same way as the hooks above: this sits on the destruction
/// path of every object of every interface.
fn forget_destroyed_toplevel_icon<I>(state: &mut State, resource: &I)
where
    I: Resource,
    I::Request: 'static,
{
    if TypeId::of::<I::Request>() != TypeId::of::<xdg_toplevel_icon_v1::Request>() {
        return;
    }
    state.forget_toplevel_icon(&resource.id());
}

/// Posts a protocol error and returns `true` when `request` is a
/// `create_frame` that would push its client past
/// [`MAX_FRAMES_PER_CLIENT`](super::screencopy::MAX_FRAMES_PER_CLIENT) live
/// capture frames.
///
/// See the module doc's "Why the fifth guard exists" for the full argument;
/// the short form: the pinned Smithay rev pushes every `create_frame` onto an
/// unbounded per-session list and never raises `duplicate_frame`, and
/// scoot's own `Capture::pending` throttle only runs on `capture`, so a
/// `create_frame` loop with no `capture` ever sent is an unbounded-objects
/// shape nothing else bounds. The refusal is the protocol's own
/// `duplicate_frame` error on the offending session, which disconnects only
/// the client that overflowed.
///
/// Folds away for every interface other than
/// `ext_image_copy_capture_session_v1`, for the same monomorphization reason
/// as the guards above -- which matters here too: this runs on every request
/// of every interface.
fn reject_excess_capture_frame<I>(
    state: &mut State,
    client: &Client,
    resource: &I,
    request: &I::Request,
) -> bool
where
    I: Resource,
    I::Request: 'static,
{
    if TypeId::of::<I::Request>() != TypeId::of::<ext_image_copy_capture_session_v1::Request>() {
        return false;
    }
    let Some(ext_image_copy_capture_session_v1::Request::CreateFrame { .. }) =
        (request as &dyn Any).downcast_ref::<ext_image_copy_capture_session_v1::Request>()
    else {
        return false;
    };
    if !state.refuse_excess_capture_frame(client) {
        return false;
    }
    resource.post_error(
        ext_image_copy_capture_session_v1::Error::DuplicateFrame,
        format!(
            "create_frame refused: this client already holds the maximum of {} \
             live capture frames (duplicate_frame)",
            super::screencopy::MAX_FRAMES_PER_CLIENT,
        ),
    );
    true
}

/// Posts `wl_subcompositor.bad_parent` and returns `true` when `request` is
/// a `get_subsurface` that would nest a surface deeper than
/// [`MAX_SUBSURFACE_DEPTH`](super::subsurface_depth::MAX_SUBSURFACE_DEPTH).
/// See the module doc's "Why the subsurface-depth guard exists", and
/// `subsurface_depth.rs` for the rule.
///
/// Folds away for every interface other than `wl_subcompositor`, for the
/// same monomorphization reason as the guards above -- this runs on every
/// request of every interface.
fn reject_too_deep_subsurface<I>(resource: &I, request: &I::Request) -> bool
where
    I: Resource,
    I::Request: 'static,
{
    if TypeId::of::<I::Request>() != TypeId::of::<wl_subcompositor::Request>() {
        return false;
    }
    let Some(wl_subcompositor::Request::GetSubsurface {
        surface, parent, ..
    }) = (request as &dyn Any).downcast_ref::<wl_subcompositor::Request>()
    else {
        return false;
    };
    super::subsurface_depth::reject_too_deep(resource, surface, parent)
}

/// Forgets one live capture frame when its protocol object dies, which is
/// what keeps [`MAX_FRAMES_PER_CLIENT`](super::screencopy::MAX_FRAMES_PER_CLIENT)'s
/// bookkeeping exact: every counted `create_frame` is paired with exactly one
/// destruction, including on client disconnect (whose cleanup destroys every
/// object) and for frames that outlive their session.
///
/// Folds away for every interface other than
/// `ext_image_copy_capture_frame_v1`, which matters in the same way as the
/// hooks above: this sits on the destruction path of every object of every
/// interface.
fn forget_destroyed_capture_frame<I>(state: &mut State, client: &ClientId, _resource: &I)
where
    I: Resource,
    I::Request: 'static,
{
    if TypeId::of::<I::Request>() != TypeId::of::<ext_image_copy_capture_frame_v1::Request>() {
        return;
    }
    state.forget_capture_frame(client);
}

/// The message both size-cap refusals carry. Allocating is fine here and
/// nowhere else in this file: this runs only on the path that has just
/// disconnected a client, never on a served request.
fn too_large(size: i32) -> String {
    format!("wl_shm pool size {size} exceeds scoot's maximum of {MAX_SHM_POOL_BYTES} bytes")
}

/// Whether a creation holding `live` counted units against a `grace` must
/// be refused for compositor-wide fd pressure: past the grace *and* the
/// table pressured (see `fd_pressure`).
///
/// Both operators are exact. `>` (not `>=`): `live == grace` still passes,
/// so a client may hold grace+1 units (129 buffers / 65 pools) -- the
/// refused creation is the one that would take it past grace+1, and any
/// "two at grace" fd arithmetic understates the permitted maximum by 4
/// (2 x (129 + 65 + 1) + 14 = 404, not 400). `&&` (not `||`): either half
/// alone passes, so the kill always lands on a contributor, never on an
/// under-grace innocent during someone else's pressure.
///
/// The grace lookup short-circuits the table observation, so creations
/// under grace cost one `HashMap` lookup and no syscall -- which is every
/// legitimate creation, since no legitimate client holds past grace (see
/// the grace sizing in `fd_pressure`). The guard below is load-bearing for
/// that: `table()` observes the process fd table (getrlimit + readdir),
/// so it must only run once a client is already past its grace. Checked
/// before the per-client claim, so a pressure refusal never takes a count
/// unit it would then have to give back.
fn pressure_refusal(live: u32, grace: u32) -> bool {
    if live <= grace {
        return false;
    }
    pressure_refusal_for(
        live,
        grace,
        super::fd_pressure::table().is_some_and(|table| table.pressured()),
    )
}

/// The pure conjunction inside [`pressure_refusal`], split out so the
/// operator choice (`>` vs `&&`) is unit-testable without filling the test
/// process's own fd table: `pressured` is the already-observed table
/// verdict, not a fresh observation. Pinned in `tests` below.
fn pressure_refusal_for(live: u32, grace: u32, pressured: bool) -> bool {
    live > grace && pressured
}

/// The message the live-pool-count refusal carries: which bound said no and
/// what it is. Same allocation rule as [`too_large`] -- refusal path only.
fn too_many_pools() -> String {
    format!(
        "wl_shm pool refused: this client already holds the maximum of {} live pools",
        super::shm_pools::MAX_POOLS_PER_CLIENT,
    )
}

/// The message the pressure-grace pool refusal carries: the table is
/// pressured *and* this client holds past its grace, so the kill lands on
/// a contributor, never an innocent. Same allocation rule as [`too_large`]
/// -- refusal path only.
fn too_many_pools_under_pressure() -> String {
    format!(
        "wl_shm pool refused: compositor-wide file-descriptor pressure, and this client \
         holds more than the {}-pool pressure grace",
        super::fd_pressure::PRESSURE_GRACE_POOLS,
    )
}

/// The message the live-buffer-count refusal carries: which bound said no
/// and what it is. Same allocation rule as [`too_large`] -- refusal path
/// only. One message for all three factories: the count is shared, so the
/// bound that said no is the same whichever factory the client came
/// through.
fn too_many_buffers() -> String {
    format!(
        "wl_buffer refused: this client already holds the maximum of {} live buffers",
        super::wl_buffers::MAX_BUFFERS_PER_CLIENT,
    )
}

/// The message the pressure-grace buffer refusal carries, for all three
/// factories for the same shared-count reason as [`too_many_buffers`].
/// Same allocation rule as [`too_large`] -- refusal path only.
fn too_many_buffers_under_pressure() -> String {
    format!(
        "wl_buffer refused: compositor-wide file-descriptor pressure, and this client \
         holds more than the {}-buffer pressure grace",
        super::fd_pressure::PRESSURE_GRACE_BUFFERS,
    )
}
