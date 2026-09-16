//! Nesting flexwm inside a host Wayland compositor.
//!
//! `--nested` presents the exact same pixman-rendered framebuffer
//! `headless.rs` draws into as one ordinary window in a host session (sway in
//! the dev VM; in principle any wlroots/wayland compositor), and forwards
//! that window's real input back into this compositor's own seat. Nothing
//! about rendering changes -- flexwm is still a Wayland *server* to its own
//! clients exactly as in `--headless`; this module only adds flexwm as a
//! Wayland *client* of a second, outer compositor.
//!
//! Host-side protocol object handling (the `wayland_client::Dispatch` impls)
//! lives in `nested_dispatch.rs`; this file is the data (`Host`), setup
//! (`init`), and the one thing `headless::render` calls (`Host::present`).

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

/// The connection presenting flexwm's own framebuffer as a window in a host
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
    /// The size flexwm is currently rendering at, i.e. what `buffers` is
    /// sized for. Distinct from the size on the wire in an in-flight
    /// configure that hasn't been acted on yet (v1 only ever acts on the
    /// first one -- see `nested_dispatch`).
    size: (i32, i32),
    /// xdg-shell forbids attaching a buffer before the first configure.
    configured: bool,
    /// The size from the most recent `xdg_toplevel::Configure`, applied when
    /// its paired `xdg_surface::Configure` (the one carrying the serial to
    /// ack) arrives -- xdg-shell delivers the two separately by design.
    /// `None` means "the host proposed 0x0 (its way of saying 'you choose')
    /// or hasn't sent one yet"; the size flexwm started with is kept in that
    /// case.
    pending_size: Option<(i32, i32)>,
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
    toplevel.set_title("flexwm".to_string());
    toplevel.set_app_id("flexwm".to_string());
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
    /// type -- headless.rs's CPU/pixman path is the only renderer that exists
    /// today, but this module has no reason to know that; a future GPU
    /// renderer reading its own output back into a byte slice could present
    /// through this exact same call.
    ///
    /// Silently does nothing (does not block) if the surface hasn't been
    /// configured yet, or if the frame's dimensions don't match what
    /// `buffers` is currently sized for (a resize was requested but hasn't
    /// been acted on yet -- the next frame after that catches up), or if
    /// neither host buffer is free (the host hasn't released one back in
    /// time -- `present_skipped` is set so a release re-triggers a render
    /// instead of leaving the host window stale, see `nested_dispatch`'s
    /// `Dispatch<HostBuffer>`).
    pub fn present(&mut self, pixels: &[u8], width: i32, height: i32) {
        if !self.configured || (width, height) != self.size {
            return;
        }
        let Some(buffer) = self.buffers.write_free(pixels) else {
            self.present_skipped = true;
            return;
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
        let _ = self.conn.flush();
    }

    /// Applies a size the host proposed (its first configure, per v1 scope --
    /// see `nested_dispatch`), recreating the render target and the host-side
    /// buffers together so they can't end up mismatched. `state.host` is
    /// briefly taken out of `state` for the duration so `state.resize_output`
    /// (which needs `&mut State` and knows nothing about `Host`) can be
    /// called without a double-borrow.
    ///
    /// Returns `Err` if the render target or the host-side buffers couldn't
    /// be (re)created. The caller (`nested_dispatch`) only calls
    /// `mark_configured` on `Ok` -- v1 only ever calls this once, gated by
    /// `is_configured`, so a failure here is equivalent to a fatal startup
    /// condition, not a transient one to limp on from: `present()`'s size
    /// guard would otherwise silently and permanently mismatch (one of the
    /// two having resized successfully while the other stayed at the old
    /// size), dropping every future frame with nothing but a warning to show
    /// it -- the same silent-failure shape this project has hit before
    /// elsewhere.
    pub(super) fn apply_size(
        state: &mut State,
        width: i32,
        height: i32,
    ) -> Result<(), Box<dyn Error>> {
        let Some(host) = state.host.take() else {
            return Ok(());
        };
        let resized = state.resize_output(width, height);
        let result = BufferPool::new(&host.shm, &host.qh, width, height);
        // Put `host` back before propagating any error, so `state.host` is
        // never left `None` -- the caller stops the event loop on `Err`
        // regardless, but leaving this `Some` keeps every other path (e.g.
        // `render()`'s `if let Some(host) = &mut self.host`) simple.
        state.host = Some(host);
        let buffers = result?;
        // Checked after `state.host` is whole again, and before the buffers
        // are swapped in: a render target still at the old size with
        // host-side buffers at the new one is the exact mismatch this
        // function's doc says must not be limped on from. `resize_output`
        // has already logged what failed.
        if !resized {
            return Err("could not resize the render target".into());
        }
        if let Some(host) = &mut state.host {
            let old = std::mem::replace(&mut host.buffers, buffers);
            old.destroy();
            host.size = (width, height);
        }
        Ok(())
    }

    pub(super) fn buffers_mut(&mut self) -> &mut BufferPool {
        &mut self.buffers
    }

    pub(super) fn is_configured(&self) -> bool {
        self.configured
    }

    pub(super) fn mark_configured(&mut self) {
        self.configured = true;
    }

    /// The host's proposed size, or the size flexwm is already at if the
    /// host never sent one (0x0, "you choose") -- always returns *some*
    /// size, so callers don't need their own fallback.
    pub(super) fn take_pending_size(&mut self) -> (i32, i32) {
        self.pending_size.take().unwrap_or(self.size)
    }

    pub(super) fn set_pending_size(&mut self, width: i32, height: i32) {
        self.pending_size = Some((width, height));
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

#[cfg(test)]
mod tests {
    // Host itself needs a live host compositor to construct, so its
    // meaningful logic (buffer sizing/free-tracking) is tested directly in
    // buffers.rs instead, where it doesn't need one.
}
