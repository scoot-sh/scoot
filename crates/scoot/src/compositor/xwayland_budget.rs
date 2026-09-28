//! The session's own XWayland server gets its own per-client bounds.
//!
//! Every per-Wayland-client resource bound scoot carries -- the fd ledger
//! (`client_fds.rs`, 512 fds), the live-buffer count (`wl_buffers.rs`, 512)
//! and the timeline count inside the ledger (`drm_syncobj.rs`, 128) -- is
//! sized for *one app*, and tripping any of them disconnects the client.
//! XWayland is one Wayland client carrying *every* X client's windows: each
//! window's pixmaps are its own `wl_shm` pools (or dma-bufs, each with its
//! own timeline where explicit sync is offered) on that single connection.
//! So one app's bound applied to the server was a bound on the sum of every
//! X app, and a disconnect there takes every X window in the session with it.
//!
//! ## What was measured
//!
//! Headless harness, XWayland 24.1.13, real `x11rb` clients mapping 120x90
//! override-redirect windows (`docs/backlog/resolved/xwayland-server-death-many-unmanaged-done.md`):
//! every window cost the server exactly **2 pools (2 ledger fds) and 2
//! buffers** -- XWayland double-buffers a window and closes each pool object
//! right after making its buffer, so the pool fd lives on in the buffer. The
//! 257th window's pool took the server past 512 and scoot killed it with
//! `wl_shm` error 1 ("this compositor still holds 512 file descriptors for
//! this client"); XWayland exited on the protocol error, every X client's
//! connection broke, and the whole unmanaged draw list drained. The same
//! point whether one X client or several mapped the windows, and for managed
//! windows too (three X clients' managed windows died at 251, their resizes
//! costing a few pools more). With every per-client bound lifted, the server
//! itself carried 3050 windows from 24 X clients without complaint (287 MB
//! RSS, 35 fds of its own, unchanged): there was no XWayland limit to
//! protect, only scoot's.
//!
//! Nor did scoot's per-X-client caps help with menus: an override-redirect
//! window scoot refuses at the window manager (`toplevel_cap.rs`) is still
//! one the server has allocated and committed buffers for before scoot
//! hears of the map -- measured, one X client mapping 240 menus had 128
//! drawn and 480 buffers held. Withholding `_XWAYLAND_ALLOW_COMMITS` from
//! it was measured too and changes nothing (a refused *managed* window
//! costs nothing to begin with: its frame is never mapped). See
//! `xwayland/tests/refused_cost.rs` and
//! `docs/backlog/protocols/xwayland-refused-windows-still-commit.md`.
//!
//! ## The number
//!
//! [`bound`] is one sixteenth of the fd table scoot runs with, clamped to
//! [`MIN`]..=[`MAX`] (512..=4096), and it replaces all three bounds for the
//! server's connection (the timeline count is folded into the total rather
//! than kept at 128: XWayland imports one timeline per window pixmap where
//! explicit sync is offered -- `xwl_glamor_dri3_syncobj_create` per
//! `xwl_pixmap` in 24.1.13's `xwayland-glamor-gbm.c`, read from the source,
//! not measured: that tier needs a GPU -- so its timelines grow with its
//! windows exactly as its pools do).
//!
//! - **4096 on the table scoot raises to** (65536, `nofile.rs`): 2048 static
//!   X windows at the measured 2 each -- eight X clients each drawing all
//!   the 128 managed and 128 override-redirect windows scoot's X-side caps
//!   allow -- and still one sixteenth of the table, so the server alone
//!   cannot bring on the fd pressure that sheds newcomers (`fd_pressure.rs`,
//!   65408 on that table).
//! - **512 on a 1024-fd table** (a hard limit scoot cannot raise): exactly
//!   the ordinary bound, so every margin `fd_pressure.rs` and `client_fds.rs`
//!   derive for a 1024-fd table holds for the server too. More would let one
//!   connection reach that table's pressure line on its own.
//! - Between, it scales: 1250 on a 20000-fd table.
//!
//! What the bigger bound costs is one bigger sweep: the ledger checks a
//! client's records only when an arrival would take it past its bound, at
//! most once per `SWEEP_MARGIN` (16) arrivals. `client_fds`' `sweep_cost`
//! measured 441 ns a pool record (debug), so ~1.8 ms for a server holding
//! 4096, and only while it sits at its budget.
//!
//! Past it the server is still refused and disconnected, as any client past
//! its bound is: the refusal is the only answer the protocol leaves
//! (`dispatch.rs`), and an unbounded server would let one runaway X client
//! grow scoot's fd table until fd pressure refused everyone.
//!
//! ## Acquire waits
//!
//! The one per-client bound the fd budget does not replace is the count of
//! commits waiting on unsignalled acquire points (`drm_syncobj/acquire.rs`,
//! 64 per app, a disconnect past it). It exists only where explicit sync is
//! offered (the `--tty` GPU tier), and there the server holds one wait per
//! X window whose latest commit's acquire point has not signalled: XWayland
//! posts a window's next frame only once the last one's frame callback has
//! fired (`xwl_screen_post_damage`, "If we're waiting on a frame callback
//! from the server, don't attach a new buffer", 24.1.13's
//! `xwayland-screen.c`), and a blocked commit's callback waits with it. A
//! window flipped by Present without vsync is paced by a `wl_display.sync`
//! instead, so it can have a few (up to the images the X client's GPU
//! driver swaps between). All of that is read from the XWayland source,
//! not measured: this machine class has no GPU. So 64 was a bound on every
//! X app's GPU windows together, and ~64 X windows committing GPU frames in
//! the same instant would have disconnected every X app in the session.
//!
//! [`acquire_waits_for`] scales it with the fd budget, in an app's own
//! ratio (64 waits to 512 fds): **512 waits on the raised table**, exactly
//! 64 on a 1024-fd table (so, again, `fd_pressure.rs`'s arithmetic for that
//! table holds unchanged), 156 on a 20000-fd one. Why not one per window
//! the fd budget admits (2048): every wait is also a blocked transaction
//! in the server's queue, which Smithay scans linearly on each of the
//! server's commits (`TransactionQueue::take_ready`), so the bound is a
//! bound on that scan too; 512 is a quarter of the windows the budget
//! holds, all blocked at once. Each wait is an eventfd scoot opens: the
//! server at every bound is 4096 + 512 fds on a 65536-fd table, still
//! small next to the 65408 pressure line. The pressure grace (16) is
//! unchanged, as the fd grace is: under fd pressure the server is a
//! contributor like any client past it.
//!
//! ## Identity
//!
//! The server's connection is the one carrying Smithay's
//! `XWaylandClientData`, which only `XWayland::spawn` inserts (every other
//! connection scoot accepts carries `ClientState`). A downcast of the
//! client's data, no lookup and no allocation: this runs on every pool,
//! plane, timeline and buffer creation.

use smithay::reexports::wayland_server::Client;

use super::drm_syncobj::MAX_ACQUIRE_WAITS_PER_CLIENT;

/// The smallest the server's bound gets: the ordinary per-client bound, so
/// the server is never held tighter than an app.
pub(crate) const MIN: u32 = super::client_fds::MAX_FDS_PER_CLIENT;

/// The largest the server's bound gets, reached from a 65536-fd table up.
/// See the module doc for the number.
pub(crate) const MAX: u32 = 4096;

/// The server's bound on a table of `soft` fds: a sixteenth of it, clamped
/// to [`MIN`]..=[`MAX`]. Total on any input (`RLIM_INFINITY` included).
pub(crate) fn bound_for(soft: u64) -> u32 {
    // Clamped in u64 before narrowing, so the cast cannot truncate.
    (soft / 16).clamp(u64::from(MIN), u64::from(MAX)) as u32
}

/// The server's outstanding acquire-wait bound when its fd budget is
/// `bound`: an app's ratio of waits to fds (64 to 512), so [`MIN`] gives
/// exactly an app's 64 and [`MAX`] gives 512. See the module doc.
pub(crate) const fn acquire_waits_for(bound: u32) -> u32 {
    bound / (MIN / MAX_ACQUIRE_WAITS_PER_CLIENT)
}

/// The server's bound in this process: [`bound_for`] the soft limit
/// `nofile::raise` left, or [`MIN`] where it could not read one. One atomic
/// load (the limit is read once per process).
pub(crate) fn bound() -> u32 {
    super::nofile::soft().map_or(MIN, bound_for)
}

/// Whether `client` is the session's own XWayland server. Always `false`
/// without the `xwayland` feature, where no such connection exists.
pub(crate) fn is_server(client: &Client) -> bool {
    #[cfg(feature = "xwayland")]
    {
        client
            .get_data::<smithay::xwayland::XWaylandClientData>()
            .is_some()
    }
    #[cfg(not(feature = "xwayland"))]
    {
        let _ = client;
        false
    }
}

#[cfg(test)]
mod tests;
