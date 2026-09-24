//! Nesting scoot inside a host Wayland compositor.
//!
//! `--nested` presents the exact same pixman-rendered framebuffer
//! `headless.rs` draws into as one ordinary window in a host session (sway in
//! the dev VM; in principle any wlroots/wayland compositor), and forwards
//! that window's real input back into this compositor's own seat. Nothing
//! about rendering changes -- scoot is still a Wayland *server* to its own
//! clients exactly as in `--headless`; this module only adds scoot as a
//! Wayland *client* of a second, outer compositor.
//!
//! The window follows the host's size for as long as it lives: the first
//! configure builds the render target and the host buffers, and every later
//! one that proposes a different size rebuilds both together -- but at most
//! one rebuild per frame tick, not one per configure. A host resize is not
//! one event (dragging a window sends a configure per pixel step), so a
//! later configure only *queues* its size and the next render drains the
//! queue (see [`Host::drain_pending_resize`]). The two entry points for
//! acting on a size ([`Host::apply_first_configure`] and
//! [`Host::apply_resize`]) differ only in what a failure means, and that is
//! the whole reason they are two functions rather than one with a flag.
//!
//! Host-side protocol object handling (the `wayland_client::Dispatch` impls)
//! lives in `nested_dispatch.rs`; this file is the data (`Host`), setup
//! (`init`), and the one thing `render::draw_frame_with` calls (`Host::present`).

mod buffers;

use std::error::Error;

use calloop_wayland_source::WaylandSource;
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::wl_compositor::WlCompositor as HostCompositor;
use wayland_client::protocol::wl_keyboard::WlKeyboard as HostKeyboard;
use wayland_client::protocol::wl_pointer::WlPointer as HostPointer;
use wayland_client::protocol::wl_seat::WlSeat as HostSeat;
use wayland_client::protocol::wl_shm::WlShm as HostShm;
use wayland_client::protocol::wl_surface::WlSurface as HostSurface;
use wayland_client::{Connection, QueueHandle};
use wayland_protocols::xdg::shell::client::xdg_surface::XdgSurface as HostXdgSurface;
use wayland_protocols::xdg::shell::client::xdg_toplevel::XdgToplevel as HostToplevel;
use wayland_protocols::xdg::shell::client::xdg_wm_base::XdgWmBase as HostWmBase;

use self::buffers::BufferPool;
use super::State;
use crate::cli::MAX_OUTPUT_DIMENSION;

/// The connection presenting scoot's own framebuffer as a window in a host
/// compositor. Everything host-Wayland-specific lives here and in
/// `nested_dispatch.rs`; nothing outside this module needs to know any of
/// these types exist (see `Host::present`'s signature -- raw bytes in,
/// nothing pixman- or wayland-client-specific out).
pub struct Host {
    conn: Connection,
    qh: QueueHandle<State>,
    surface: HostSurface,
    /// Held only to keep the xdg_surface/toplevel/seat objects alive for the
    /// window's lifetime -- their events reach `nested_dispatch.rs` through
    /// the connection regardless of whether we hold these, but destroying
    /// the objects (`v1` never does) or reusing them later (`v2`, e.g. a
    /// title update) needs them kept around, same rationale as
    /// `state.rs`'s `output_manager_state`.
    #[allow(dead_code)]
    xdg_surface: HostXdgSurface,
    #[allow(dead_code)]
    toplevel: HostToplevel,
    shm: HostShm,
    #[allow(dead_code)]
    seat: HostSeat,
    keyboard: Option<HostKeyboard>,
    pointer: Option<HostPointer>,
    buffers: BufferPool,
    /// The size scoot is currently rendering at, i.e. what `buffers` is
    /// sized for. Distinct from the size on the wire in an in-flight
    /// configure that hasn't been acted on yet; once `configured`, a
    /// configure is compared against this and only a *different* size
    /// rebuilds anything. The *first* configure rebuilds either way, there
    /// being nothing built yet to keep (see [`configure_action`]).
    size: (i32, i32),
    /// xdg-shell forbids attaching a buffer before the first configure.
    ///
    /// Also which of the two entry points a configure goes to
    /// ([`Host::apply_first_configure`] while `false`,
    /// [`Host::apply_resize`] after), which is the one place the difference
    /// between "scoot is not up yet" and "scoot is up and the host moved"
    /// is decided.
    configured: bool,
    /// The size from the most recent `xdg_toplevel::Configure`, applied when
    /// its paired `xdg_surface::Configure` (the one carrying the serial to
    /// ack) arrives -- xdg-shell delivers the two separately by design.
    /// `None` means "the host proposed nothing usable (0x0, its way of saying
    /// 'you choose', or a size out of range) or hasn't sent one yet"; the
    /// size scoot is already at is kept in that case.
    pending_size: Option<(i32, i32)>,
    /// A later configure's size that has been queued but not yet acted on.
    /// Written by `nested_dispatch.rs` once per `Resize` classification and
    /// drained once per frame tick by [`Host::drain_pending_resize`] -- which
    /// is what keeps a drag's configure-per-pixel-step from rebuilding the
    /// pool and the render target per step. Overwritten, never extended
    /// (only the latest size matters), so a configure costs two `i32` stores
    /// and no allocation, whatever rate the host sends them at.
    pending_resize: PendingResize,
    /// Set when `present()` had a frame ready but no host buffer was free to
    /// write it into (both still held by the host). Checked when the host
    /// releases a buffer (`nested_dispatch::Dispatch<HostBuffer>`) so a
    /// skipped frame doesn't leave the host window stale until some
    /// unrelated redraw happens to trigger another render -- freeing a
    /// buffer while this is set re-arms rendering itself.
    present_skipped: bool,
}

pub fn init(
    loop_handle: smithay::reexports::calloop::LoopHandle<'static, State>,
    state: &mut State,
    width: i32,
    height: i32,
) -> Result<(), Box<dyn Error>> {
    let conn = Connection::connect_to_env()?;
    let (globals, event_queue) = registry_queue_init::<State>(&conn)?;
    let qh = event_queue.handle();

    let compositor: HostCompositor = globals.bind(&qh, 4..=6, ())?;
    let shm: HostShm = globals.bind(&qh, 1..=1, ())?;
    let wm_base: HostWmBase = globals.bind(&qh, 1..=6, ())?;
    let seat: HostSeat = globals.bind(&qh, 1..=9, ())?;

    let surface = compositor.create_surface(&qh, ());
    let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg_surface.get_toplevel(&qh, ());
    toplevel.set_title("scoot".to_string());
    toplevel.set_app_id("scoot".to_string());
    // Nothing attached yet -- xdg-shell requires waiting for the first
    // configure before the first buffer, which is why `commit()` here has no
    // preceding `attach()`. This commit is what makes the host actually send
    // that configure.
    surface.commit();

    let buffers = BufferPool::new(&shm, &qh, width, height)?;

    state.host = Some(Host {
        conn: conn.clone(),
        qh,
        surface,
        xdg_surface,
        toplevel,
        shm,
        seat,
        keyboard: None,
        pointer: None,
        buffers,
        size: (width, height),
        configured: false,
        pending_size: None,
        pending_resize: PendingResize::default(),
        present_skipped: false,
    });

    WaylandSource::new(conn, event_queue)
        .insert(loop_handle)
        .map_err(|error| format!("could not register the host connection: {error}"))?;

    Ok(())
}

impl Host {
    /// Copies an already-rendered frame into a free host buffer and presents
    /// it. Takes raw pixels plus dimensions rather than any renderer-specific
    /// type -- pixman is the only renderer behind `render.rs`'s seam today,
    /// but this module has no reason to know that; a future GPU renderer
    /// reading its own output back into a byte slice presents through this
    /// exact same call, unchanged (see `render.rs`'s `read_back`, which is
    /// what hands the slice over).
    ///
    /// Returns whether the frame reached the host. Anything else -- the
    /// surface not configured yet, a frame whose size is not the one the
    /// buffers are for, no free host buffer, or a flush the dead host refused
    /// -- silently drops the frame (does not block) and returns `false`, so
    /// the caller knows this frame presented nothing: `render()` leaves
    /// pending presentation feedback queued on `false` rather than stamping
    /// it with a time nothing was shown at (see `presentation_time.rs`).
    ///
    /// Silently does nothing (does not block) if the surface hasn't been
    /// configured yet, or if the frame's dimensions don't match what
    /// `buffers` is currently sized for, or if neither host buffer is free
    /// (the host hasn't released one back in time -- `present_skipped` is set
    /// so a release re-triggers a render instead of leaving the host window
    /// stale, see `nested_dispatch`'s `Dispatch<HostBuffer>`).
    ///
    /// The size check is a guard on an invariant, not a routine path: the
    /// render target and `buffers` are only ever changed together, by
    /// `replace_render_target`, which is also why a *failed* resize does not
    /// trip it. Dropping a frame is the safe answer if it ever did -- writing
    /// a frame of one size into a buffer described to the host as another is
    /// how a compositor hands out garbage or gets killed for it.
    pub fn present(&mut self, pixels: &[u8], width: i32, height: i32) -> bool {
        if !self.configured || (width, height) != self.size {
            return false;
        }
        let Some(buffer) = self.buffers.write_free(pixels) else {
            self.present_skipped = true;
            return false;
        };
        self.present_skipped = false;
        self.surface.attach(Some(buffer), 0, 0);
        self.surface.damage_buffer(0, 0, i32::MAX, i32::MAX);
        self.surface.commit();
        // Third instance of the same class of bug as state.rs's client
        // dispatch flush and ipc.rs's post-request flush: wayland-client
        // queues outgoing requests (attach/damage/commit above) until
        // conn.flush() actually writes them to the socket. Nothing else on
        // this connection flushes on its own -- skip this and the window
        // just never updates, with no error anywhere.
        //
        // The result is the return, not ignored: a dead host refuses the
        // flush, which means nothing left this process -- stamping that
        // frame `presented` would date pixels nobody will ever scan out.
        self.conn.flush().is_ok()
    }

    /// Comes up at the size the host's **first** configure asked for.
    ///
    /// A failure here is fatal: it stops the event loop, having logged what
    /// failed. That is not a judgement `replace_render_target` makes -- it is
    /// this entry point's whole reason for existing separately from
    /// [`Host::apply_resize`], which takes the opposite decision for the same
    /// error. Before the first configure nothing is on screen and no client
    /// has mapped anything, so there is no session to lose; and a nested
    /// window whose render target never got built shows the host a blank
    /// surface forever, with nothing but a log line to say why. Failing
    /// loudly beats running wrong.
    ///
    /// Marks the host surface configured on success and only on success:
    /// until that happens xdg-shell forbids attaching a buffer, and
    /// [`Host::present`] honours it.
    pub(super) fn apply_first_configure(state: &mut State, width: i32, height: i32) {
        match Self::replace_render_target(state, width, height) {
            Ok(()) => {
                if let Some(host) = &mut state.host {
                    host.mark_configured();
                }
            }
            Err(error) => {
                tracing::error!(
                    %error,
                    "could not set up the nested backend's render target at the host's requested size; stopping"
                );
                state.loop_signal.stop();
            }
        }
    }

    /// Follows a **later** configure: the host resized scoot's window (a
    /// browser window resized under webtop, a tiling host relaying out, an
    /// interactive drag), so the desktop resizes with it.
    ///
    /// Reached once per frame tick from [`Host::drain_pending_resize`], never
    /// directly from dispatch -- see that function's doc for why the two are
    /// split that way.
    ///
    /// A failure here is *not* fatal, deliberately and asymmetrically with
    /// [`Host::apply_first_configure`] -- the one decision this split exists
    /// to make explicit rather than leave `replace_render_target` to infer
    /// from `is_configured()`. Ending a live session with windows open in it,
    /// because a *bigger* buffer pool could not be allocated, is the
    /// data-loss case `CLAUDE.md` weighs a crash against, for a failure the
    /// user neither caused nor can avoid. It is safe to differ because
    /// `replace_render_target` leaves the losing case *consistent* rather
    /// than half-applied -- see its doc for the ordering that buys that: the
    /// render target and the host buffers are both still at the old size,
    /// which is a working session. The host letterboxes the difference, which
    /// is an annoyance; a dead compositor is lost work.
    ///
    /// Takes no `Result` for that reason: there is nothing a caller could
    /// usefully do with one, and a signature that cannot be handled as
    /// "fatal" is what keeps the two paths from being conflated later.
    pub(super) fn apply_resize(state: &mut State, width: i32, height: i32) {
        if let Err(error) = Self::replace_render_target(state, width, height) {
            tracing::warn!(
                %error,
                width,
                height,
                "could not follow the host's resize; staying at the previous size"
            );
        }
    }

    /// Records a later configure's size for the next render tick, overwriting
    /// whatever an earlier configure in the same frame queued -- only the
    /// latest size is ever acted on. Two `i32` stores and no allocation, so
    /// this is safe to call at whatever rate the host sends configures (a
    /// drag is one per pixel step). The caller must [`State::request_render`]
    /// afterwards, or nothing will drain the queue.
    pub(super) fn queue_resize(&mut self, width: i32, height: i32) {
        self.pending_resize.queue((width, height));
    }

    /// Applies the size configures queued since the last frame, if any --
    /// at most one rebuild per call, however many configures arrived.
    ///
    /// Called at the top of [`State::render`](super::State::render), before
    /// the clean-screen early return, so a queued resize is acted on even
    /// when nothing else dirtied the screen (queueing always marks it dirty
    /// anyway; this ordering is belt and braces for a flag cleared in
    /// between). Draining here rather than in dispatch is the whole
    /// coalescing: a drag's hundred configures become one rebuild per frame
    /// tick, each at the latest size, instead of a pool rebuild, a render
    /// target rebuild, a mode mint and a linear mode-list extension per
    /// pixel step. A configure that only undoes a still-queued one (the drag
    /// came back to the size scoot is already at within one frame) drains to
    /// nothing at all.
    ///
    /// A resize applied here still draws in the *same* frame: draining runs
    /// before the frame is composited, and `apply_resize` ends in
    /// `request_render`, so the tick that picks the size up also shows it.
    /// The cost is a delay of at most one frame between the host's configure
    /// and the pixels, against unbounded rebuilds without it.
    pub(super) fn drain_pending_resize(state: &mut State) {
        let pending = match &mut state.host {
            Some(host) => {
                let current = host.size();
                host.pending_resize.take_if_changed(current)
            }
            None => None,
        };
        if let Some((width, height)) = pending {
            Self::apply_resize(state, width, height);
        }
    }

    /// Recreates the render target and the host-side buffers together, at a
    /// size the host proposed, so the two can't end up mismatched.
    ///
    /// Private, with two public entry points above, because what an `Err`
    /// *means* differs by when it happens and that belongs at the call site
    /// rather than here: see [`Host::apply_first_configure`] (fatal) and
    /// [`Host::apply_resize`] (logged, session kept). What this function
    /// guarantees to both is the same either way, and `apply_resize` rests
    /// its whole case on it: **on `Err` nothing has moved.** `state.host` is
    /// still `Some`, still at its old `size`, still holding the buffers it
    /// had, and the render target is still at that same old size -- so the
    /// session keeps working, just at the size it was already at.
    ///
    /// The order is what buys that, and it is the reverse of the obvious one:
    /// allocate the new pool *first*, while nothing is committed, and only
    /// then touch the render target. Resizing first and allocating second
    /// would leave the one failure the resize path exists for -- a bigger
    /// pool that could not be allocated -- with the render target already at
    /// the new size and the host buffers at the old one. `present()`'s size
    /// guard drops every frame in that state, silently and permanently: a
    /// live session whose window never updates again, which is worse than
    /// either failure policy above was meant to allow.
    fn replace_render_target(
        state: &mut State,
        width: i32,
        height: i32,
    ) -> Result<(), Box<dyn Error>> {
        let Some(host) = &state.host else {
            return Ok(());
        };
        // Fallible, and deliberately first: this borrow of `state.host` ends
        // with the call (a `BufferPool` owns its host objects outright and
        // borrows nothing), which is what lets `state.resize_output` -- which
        // needs `&mut State` and knows nothing about `Host` -- run below
        // without a double borrow or a `take()`/put-back dance.
        let buffers = BufferPool::new(&host.shm, &host.qh, width, height)?;
        // Every `BufferPool` that does not end up installed is `destroy`ed
        // rather than dropped: it owns host-side `wl_buffer`/`wl_shm_pool`
        // objects that only `destroy` releases, so dropping one leaks it on
        // the host connection. That is no longer only tidiness -- a session
        // survives a failed resize now, so a leak here would accumulate one
        // pool per failed resize for the rest of it.
        if !state.resize_output(width, height) {
            buffers.destroy();
            // `resize_output` has already logged what actually failed.
            return Err("could not resize the render target".into());
        }
        let Some(host) = &mut state.host else {
            // Unreachable: nothing between the borrow above and here can
            // clear `state.host`. Written out rather than `unwrap`ed because
            // a panic on a host event would take every client's unsaved state
            // with it, and a leaked pool is the cheaper wrong answer.
            buffers.destroy();
            return Ok(());
        };
        let old = std::mem::replace(&mut host.buffers, buffers);
        old.destroy();
        host.size = (width, height);
        Ok(())
    }

    pub(super) fn buffers_mut(&mut self) -> &mut BufferPool {
        &mut self.buffers
    }

    pub(super) fn is_configured(&self) -> bool {
        self.configured
    }

    fn mark_configured(&mut self) {
        self.configured = true;
    }

    /// The size scoot is rendering at right now, which is also what the host
    /// buffers are sized for -- the two are only ever changed together, by
    /// `replace_render_target`.
    pub(super) fn size(&self) -> (i32, i32) {
        self.size
    }

    /// The host's proposed size, or the size scoot is already at if the
    /// host never sent a usable one -- always returns *some* size, so callers
    /// don't need their own fallback. Consumed on every configure, not only
    /// the first: a proposal that has been compared against `size` has been
    /// acted on, whether or not it turned out to differ.
    pub(super) fn take_pending_size(&mut self) -> (i32, i32) {
        self.pending_size.take().unwrap_or(self.size)
    }

    /// Records a size the host proposed, if it is one this compositor can
    /// act on -- see [`usable_size`]. An unusable proposal leaves whatever
    /// was already pending alone rather than overwriting it with nothing.
    pub(super) fn set_pending_size(&mut self, width: i32, height: i32) {
        match usable_size(width, height) {
            Some(size) => self.pending_size = Some(size),
            // A zero (or, from a broken host, negative) axis is xdg-shell
            // saying "you choose" and is entirely ordinary, so it stays
            // silent. A host that named *both* axes and still got refused
            // named something out of range, which is worth a trace --
            // `debug!` rather than `warn!` because a host that proposes it
            // once proposes it on every configure, and a line per configure
            // during a drag is the flood
            // `docs/backlog/resolved/clean-disconnect-log-flood-done.md`
            // exists about.
            None if width > 0 && height > 0 => tracing::debug!(
                width,
                height,
                max = MAX_OUTPUT_DIMENSION,
                "ignoring a host configure: size out of range"
            ),
            None => {}
        }
    }

    pub(super) fn keyboard(&self) -> Option<&HostKeyboard> {
        self.keyboard.as_ref()
    }

    pub(super) fn set_keyboard(&mut self, keyboard: HostKeyboard) {
        self.keyboard = Some(keyboard);
    }

    pub(super) fn pointer(&self) -> Option<&HostPointer> {
        self.pointer.as_ref()
    }

    pub(super) fn set_pointer(&mut self, pointer: HostPointer) {
        self.pointer = Some(pointer);
    }

    /// Clears and returns whether a presentation was skipped for lack of a
    /// free host buffer -- see `present_skipped`'s field doc.
    pub(super) fn take_present_skipped(&mut self) -> bool {
        std::mem::take(&mut self.present_skipped)
    }
}

/// One queued host resize: the latest size a `Resize` configure proposed
/// since the last frame tick, or nothing queued.
///
/// A struct rather than a bare `Option` so the two halves of the coalescing
/// contract -- "queueing overwrites" and "draining drops a size scoot is
/// already at" -- are methods with unit tests, not conventions callers have
/// to remember. Neither allocates: this is two `i32`s on the per-configure
/// path, which runs at pixel-step rate during a drag.
#[derive(Debug, Default)]
pub(super) struct PendingResize(Option<(i32, i32)>);

impl PendingResize {
    /// Queues `size`, replacing whatever was queued before. Only the latest
    /// proposal matters: every intermediate size would have been superseded
    /// by the next frame anyway.
    fn queue(&mut self, size: (i32, i32)) {
        self.0 = Some(size);
    }

    /// Takes the queued size if it differs from what is already showing.
    /// `None` covers both "nothing queued" and "the drag came back to the
    /// size scoot is already at within one frame" -- either way there is
    /// nothing to rebuild. Always consumes: a proposal that has been
    /// compared against the current size has been acted on, whether or not
    /// it turned out to differ.
    fn take_if_changed(&mut self, current: (i32, i32)) -> Option<(i32, i32)> {
        match self.0.take() {
            Some(pending) if pending != current => Some(pending),
            _ => None,
        }
    }
}

/// What an `xdg_surface::Configure` means for this window.
///
/// A free function over three plain values rather than a method on [`Host`]
/// so the decision is unit-testable: `Host` needs a live host compositor to
/// construct, so a method could only ever be pinned by running nested inside
/// one. Same rationale as `buffers.rs`'s `first_free` and `headless.rs`'s
/// `tty_blocks_render`. `nested_dispatch.rs` is the only caller.
pub(super) fn configure_action(
    configured: bool,
    proposed: (i32, i32),
    current: (i32, i32),
) -> ConfigureAction {
    if !configured {
        // Even when it matches the size scoot started at: nothing has been
        // built yet, and this is what builds it.
        return ConfigureAction::FirstConfigure;
    }
    if proposed == current {
        ConfigureAction::Nothing
    } else {
        ConfigureAction::Resize
    }
}

/// The outcome of [`configure_action`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ConfigureAction {
    /// Nothing is on screen yet: build the render target and the host buffers
    /// at this size, and a failure is fatal ([`Host::apply_first_configure`]).
    FirstConfigure,
    /// The host moved scoot's window to a different size: rebuild both at it,
    /// and a failure keeps the session at the old size
    /// ([`Host::apply_resize`]).
    Resize,
    /// A configure proposing the size scoot is already at, which hosts send
    /// on every state change that is not a resize -- activation, maximize,
    /// a tiling-edge update. Load-bearing rather than an optimisation:
    /// without it every focus change in the host would throw away a working
    /// render target and buffer pool to build an identical pair, plus the
    /// full redraw and host commit a resized target needs.
    Nothing,
}

/// The size a host configure proposes, or `None` for one that cannot be acted
/// on.
///
/// Two refusals, neither of them speculative:
///
/// - **A zero axis** is xdg-shell's "you choose that dimension". scoot keeps
///   the size it is already at rather than resizing to nothing. A *mixed*
///   proposal (one axis named, the other zero) is treated the same way --
///   the whole proposal is dropped rather than half-applied, which is what
///   this has always done; hosts that resize scoot's window name both axes.
/// - **An axis past [`MAX_OUTPUT_DIMENSION`]**, the same `1..=65535` window
///   `--width`/`--height` are parsed into, for the same reason: DRM stores a
///   mode axis in a `u16`, so nothing real is bigger, and a mode scoot
///   advertises on `wl_output` should be one a client can believe. It is
///   *not* what keeps the host buffer pool's byte count inside the `i32`
///   `wl_shm.create_pool` takes -- 65535x8192 is inside this bound and past
///   that one -- so `BufferPool::new` checks its own arithmetic rather than
///   trusting a caller's range.
pub(super) fn usable_size(width: i32, height: i32) -> Option<(i32, i32)> {
    let ok = |axis: i32| (1..=MAX_OUTPUT_DIMENSION).contains(&axis);
    (ok(width) && ok(height)).then_some((width, height))
}

#[cfg(test)]
mod tests;
