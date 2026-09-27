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
//! Nor did scoot's per-X-client caps help: a window scoot refuses at the
//! window manager (`toplevel_cap.rs`'s managed and unmanaged caps) is still
//! a window the server allocates and commits buffers for -- measured, one X
//! client mapping 240 menus had 128 drawn and 480 buffers held. Only the
//! window manager may withhold `_XWAYLAND_ALLOW_COMMITS`, and that write
//! lives inside Smithay's `X11Wm`
//! (`docs/backlog/protocols/xwayland-refused-windows-still-commit.md`).
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
//! ## Identity
//!
//! The server's connection is the one carrying Smithay's
//! `XWaylandClientData`, which only `XWayland::spawn` inserts (every other
//! connection scoot accepts carries `ClientState`). A downcast of the
//! client's data, no lookup and no allocation: this runs on every pool,
//! plane, timeline and buffer creation.

use smithay::reexports::wayland_server::Client;

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
