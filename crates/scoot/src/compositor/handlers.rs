//! The Wayland protocol handlers Smithay dispatches into.

use std::any::Any;

use smithay::backend::input::TabletToolDescriptor;
use smithay::backend::renderer::utils::on_commit_buffer_handler;
use smithay::input::dnd::{DnDGrab, DndGrabHandler, GrabType, Source};
use smithay::input::pointer::{CursorImageStatus, Focus};
use smithay::input::tablet::TabletSeatHandler;
use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::output::Output;
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::protocol::wl_data_source::WlDataSource;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::protocol::wl_seat;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, Resource, protocol::wl_buffer};
use smithay::utils::{IsAlive, Serial};
#[cfg(feature = "xwayland")]
use smithay::utils::{Logical, Rectangle};
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::compositor::{
    CompositorClientState, CompositorHandler, CompositorState, add_pre_commit_hook, get_parent,
    get_role, is_sync_subsurface, with_states,
};
use smithay::wayland::output::OutputHandler;
use smithay::wayland::seat::WaylandFocus;
use smithay::wayland::selection::SelectionHandler;
use smithay::wayland::selection::data_device::{
    DataDeviceHandler, DataDeviceState, WaylandDndGrabHandler, set_data_device_focus,
};
use smithay::wayland::selection::ext_data_control::{
    DataControlHandler as ExtDataControlHandler, DataControlState as ExtDataControlState,
};
use smithay::wayland::selection::primary_selection::{
    PrimarySelectionHandler, PrimarySelectionState, set_primary_focus,
};
use smithay::wayland::selection::wlr_data_control::{
    DataControlHandler as WlrDataControlHandler, DataControlState as WlrDataControlState,
};
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::wayland::shell::xdg::decoration::XdgDecorationHandler;
use smithay::wayland::shell::xdg::{
    PopupSurface, PositionerState, ToplevelSurface, XDG_POPUP_ROLE, XdgShellHandler, XdgShellState,
    XdgToplevelSurfaceData,
};
use smithay::wayland::shm::{ShmHandler, ShmState};
#[cfg(feature = "xwayland")]
use smithay::wayland::xwayland_keyboard_grab::XWaylandKeyboardGrabHandler;
#[cfg(feature = "xwayland")]
use smithay::wayland::xwayland_shell::{XWaylandShellHandler, XWaylandShellState};
#[cfg(feature = "xwayland")]
use smithay::xwayland::{X11Surface, X11Wm, XWaylandClientData, XwmHandler};
#[cfg(feature = "xwayland")]
use smithay::xwayland::xwm::{Reorder, ResizeEdge, X11Window, XwmId};

use super::State;
use super::output_scale::send_preferred_buffer_scale;
use super::popup_parent::Admission;
use super::render::Backend;
use super::state::ClientState;

impl CompositorHandler for State {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        // XWayland's own Wayland client carries Smithay's
        // `XWaylandClientData`, not scoot's `ClientState` -- and X windows
        // commit `wl_surface`s through it (the association path), so without
        // this branch the first X commit panics the compositor on the
        // `expect` below (the spike's hazard 2, fixed the anvil way: check
        // theirs first, then ours). Only compiled with the feature, since
        // the type only exists there.
        #[cfg(feature = "xwayland")]
        if let Some(state) = client.get_data::<XWaylandClientData>() {
            return &state.compositor_state;
        }
        &client
            .get_data::<ClientState>()
            .expect("client state")
            .compositor_state
    }

    /// A new `wl_surface` exists: tell it the integer `preferred_buffer_scale`.
    ///
    /// This is the bind-time call site. Smithay runs `new_surface` for every
    /// surface `wl_compositor.create_surface` makes (subsurfaces included), and
    /// this always fires before any commit -- a per-commit call would be a
    /// no-op cache hit on the hot path, not defence in depth. A config reload
    /// re-sends the same value to every live surface at once (see
    /// `output_scale.rs`'s `resend_output_scale`), so the two can never
    /// disagree. See `send_preferred_buffer_scale`'s doc.
    ///
    /// Also where the explicit-sync acquire hook is installed, only while
    /// that global exists (see `drm_syncobj/acquire.rs`): every other session
    /// adds nothing to the commit path. The global is decided before the
    /// event loop starts, so no surface predates it.
    fn new_surface(&mut self, surface: &WlSurface) {
        send_preferred_buffer_scale(surface, self.integer_scale);
        if self.drm_syncobj.active() {
            add_pre_commit_hook::<Self, _>(surface, super::drm_syncobj::acquire::pre_commit);
        }
    }

    /// `surface` has just been made a subsurface of `parent`: records the
    /// subtree now hanging below each of `parent`'s ancestors, which is what
    /// the next `get_subsurface`'s depth check reads (see
    /// `subsurface_depth.rs`). The check itself runs before Smithay links the
    /// two, in `dispatch.rs`.
    fn new_subsurface(&mut self, surface: &WlSurface, parent: &WlSurface) {
        super::subsurface_depth::record_link(surface, parent);
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        // Immediately after the buffer handler, which is what makes the newly
        // attached buffer the surface's current one: this re-synchronises that
        // buffer with the client's GPU before anything renders from the
        // mapping the renderer caches. Nothing upstream does it on a
        // re-commit -- see `dmabuf::sync_committed_dmabufs` for what the
        // pinned rev does and does not guarantee. The gate is a plain bool in
        // an shm-only session (see `State::imports_dmabufs`), so the walk is
        // paid for only where dmabufs actually exist.
        //
        // Two conditions, not one, and they ask different questions: the flag
        // is "do dmabufs happen in this session at all", the backend's is "is
        // an imported dmabuf a CPU mapping only this process synchronises".
        // Only pixman answers yes to the second -- a GLES tier never maps the
        // buffer here, so its implicit fences are the driver's to honour, and
        // the ioctl pair would be per-commit cost (including a blocking wait
        // on the client's GPU job) for a mapping that does not exist. The flag
        // comes first because it is the cheaper test and the one that is false
        // in almost every session. Any backend answers for all of them: every
        // output's target is built with the session's one renderer kind.
        if self.imports_dmabufs
            && self
                .backends
                .values()
                .next()
                .is_some_and(Backend::maps_dmabufs_on_the_cpu)
        {
            super::dmabuf::sync_committed_dmabufs(surface);
        }
        self.last_commit = std::time::Instant::now();
        // A commit to the cursor image (an animated cursor's next frame, a
        // new hotspot, a subsurface of it) changes the cursor and nothing
        // else: the cursor's own path, so a capture that asked for the
        // pointer sees it and one that did not is not re-served. Every other
        // commit may change the scene.
        if self.cursor.owns_surface(surface) {
            self.cursor_changed();
        } else {
            self.request_render();
        }

        if !is_sync_subsurface(surface) {
            let mut root = surface.clone();
            while let Some(parent) = get_parent(&root) {
                root = parent;
            }
            if let Some(id) = self.id_of(&root) {
                if let Some(window) = self.window(id) {
                    window.on_commit();
                }
                // Before the initial configure below: an unmap discarded the
                // window's toplevel state, and the configure that answers the
                // re-map must not carry a fullscreen it no longer has.
                self.discard_fullscreen_if_unmapped(id);
                // The window's first commit: whether it floats (see
                // `floating.rs`). An empty-`Vec` test on every other commit.
                if !self.awaiting_map.is_empty() {
                    self.decide_floating_at_map(id);
                }
                send_initial_configure(self, &root);
                self.observe_frame(id);
            } else if !self.commit_layer_surface(&root) {
                // Neither a window nor a layer surface. The layer map was
                // still checked first, so an ordinary window's commit never
                // pays for its lookup and a layer surface's never reaches
                // the lock probe below. What remains may be a lock surface
                // commit, which re-derives pointer focus (see
                // `refresh_lock_pointer_focus`); nothing else needs to know
                // yet.
                self.refresh_lock_pointer_focus(&root);
            }
        }

        self.popups.commit(surface);
        send_popup_initial_configure(self, surface);
        // `wl_surface.offset` on the active cursor surface decrements its
        // hotspot -- see `Cursor::note_surface_commit`. Unconditional, like
        // the popup lines above: the method itself returns fast unless the
        // committed surface is the active cursor image.
        self.cursor.note_surface_commit(surface);
    }

    /// Smithay calls this for every `wl_surface` that goes away, whether the
    /// client destroyed it explicitly or simply quit.
    ///
    /// What this compositor keeps a `WlSurface` in outside
    /// `self.space`/`self.windows` (both already driven by their own
    /// xdg-shell destruction paths) need clearing here, plus the grab
    /// session, which needs filing rather than clearing:
    ///
    /// - the cursor's image status, which nothing upstream clears when the
    ///   surface behind it dies -- see `Cursor::forget_surface`;
    /// - a session-lock surface, so a lock client tearing one down (or
    ///   disconnecting) stops it being drawn and stops it holding the
    ///   keyboard on the very next frame rather than at the render loop's
    ///   own cleanup pass -- see `session_lock.rs`.
    /// - an idle inhibitor, so a client that disconnects (or destroys the
    ///   surface) without destroying its inhibitor stops holding the
    ///   session awake -- see `idle.rs`. A dead client can never destroy
    ///   anything explicitly, so without this the recompute would keep
    ///   seeing its surface forever.
    /// - the popup-grab session (`last_popup_grab`), filed rather than
    ///   cleared: a replacement grab is dispatched adjacently to the
    ///   destroy, so waiting for the reap would file it too late -- see
    ///   `popup.rs`.
    /// - an explicit-sync wait (`drm_syncobj/acquire.rs`): a commit of this
    ///   surface still waiting on its acquire point holds an eventfd source,
    ///   which would otherwise stay registered until -- or if -- the point
    ///   signals.
    /// - the dead entries of `mapped_layers`: a surface whose `wl_surface`
    ///   dies before its layer role object (the implicit-disconnect order)
    ///   never sees `layer_destroyed`, so nothing else removes it -- see
    ///   `layer_shell.rs`. Memory-only either way: a server-side `ObjectId`
    ///   compares its client id and generation serial as well as the bare
    ///   id (wayland-backend 0.3.17 `rs/server_impl/mod.rs`), so a stale
    ///   entry can never equal a live surface.
    fn destroyed(&mut self, surface: &WlSurface) {
        super::drm_syncobj::acquire::forget_surface(self, surface);
        if self.cursor.forget_surface(surface) {
            // The cursor's shape just changed to the fallback: a redraw
            // where frames draw it, a capture tick where only captures do --
            // the same path as `cursor_image` below.
            self.cursor_changed();
        }
        if self.forget_lock_surface(surface) {
            // Unconditional, unlike the cursor above: what the lock screen
            // shows just changed on every backend, and either focus -- or a
            // grab -- may have been on this surface. All three are handled
            // for the same reason every other lock transition does it -- see
            // `session_lock.rs`: `wl_pointer.button` follows the last
            // `enter`, not the hit test, and a grab follows neither.
            self.lock_transition();
        }
        self.forget_idle_inhibitor(surface);
        // Alongside `forget_dead_clicked_layer` (which runs on every focus
        // refresh): without this, a layer surface whose `wl_surface` died
        // first keeps its dead entry here until its role object goes too --
        // and if the client keeps the role and the connection alive, that
        // is indefinitely. Per surface death, not per frame or per event:
        // one walk over the handful of mapped layer surfaces plus an
        // aliveness probe each, so no hot-path concern.
        self.mapped_layers.retain(|mapped| mapped.alive());
        // A surface going away while a popup grab is live tears the chain
        // down or replaces it: toolkits destroy the old popup before
        // grabbing the new one in the same flush, so the replacement grab is
        // dispatched adjacently -- file the session *now*, synchronously,
        // rather than waiting for a reap, which runs on a later dispatch and
        // would file it a dispatch too late for the grace half of
        // `grab_session_continues` to see.
        //
        // Gated on the dying surface belonging to the grabbing client, not
        // merely on a grab being held: another client's churn (closing a
        // window, swapping a cursor) must not refresh someone else's
        // timestamp. Same-client over-filing (a cursor surface, a toplevel
        // going away with the menu still up) only stamps a moment the
        // session was in fact live -- and the grace then permits a re-grab
        // at most two seconds past the last such moment, which is the grace
        // semantic itself, not an extension of it.
        if self.popup_grab.is_some()
            && let Some(holder) = self
                .popup_grab
                .as_ref()
                .and_then(|grab| self.grab_holder_client(grab))
            && self.client_of(surface).as_ref() == Some(&holder)
        {
            self.last_popup_grab = Some((holder, std::time::Instant::now()));
        }
    }
}

/// An `xdg_popup` needs one configure before it may attach a buffer, the
/// same shape as [`send_initial_configure`] for toplevels: nothing ever
/// sends it (the pinned rev's `PopupManager::commit` only moves the popup
/// from unmapped to mapped), so the popup surface's first commit earns it
/// here. Once mapped, the popup needs nothing else from this file to
/// appear: `Space`'s `Window` element draws each window's popups itself
/// (`popups_for_surface`), and `Window::send_frame` completes their frame
/// callbacks the same way.
///
/// The configure carries the popup's geometry constrained against its
/// target (see `popup_constraint.rs`), written into the pending state just
/// before it goes out. Here rather than in `new_popup` because this is the
/// first moment every popup is guaranteed a parent -- a layer surface's
/// dropdown is created parentless and adopted afterwards, and committing a
/// parentless popup is a protocol error Smithay posts before this runs --
/// and the constraint is then measured against where the parent is when the
/// client is told, not where it was at creation.
///
/// Guarded twice: the role check keeps ordinary commits from paying for
/// the popup-tree lookup, and `is_initial_configure_sent` keeps later
/// commits quiet -- a second configure would be a protocol error for a
/// non-reactive positioner (`AlreadyConfigured`/`NotReactive`), not a
/// harmless repeat. An initial configure cannot fail either way (both
/// errors require a first configure already sent), so an `Err` here is
/// logged, not retried specially: the flag stays unset and the next
/// commit tries again.
fn send_popup_initial_configure(state: &State, surface: &WlSurface) {
    if get_role(surface) != Some(XDG_POPUP_ROLE) {
        return;
    }
    let Some(smithay::desktop::PopupKind::Xdg(popup)) = state.popups.find_popup(surface) else {
        // Not tracked: gone between the role check and the lookup. The role
        // check above is also what keeps input-method popups out of here --
        // they are tracked (see `input_method.rs`) but carry
        // `zwp_input_popup_surface_v2`, not `xdg_popup`, and have no
        // configure to send at all.
        return;
    };
    if popup.is_initial_configure_sent() {
        return;
    }
    state.constrain_popup_before_initial_configure(&popup);
    if let Err(error) = popup.send_configure() {
        tracing::warn!(?error, "xdg_popup initial configure failed");
    }
}

/// A toplevel needs one configure before it may attach a buffer. On a
/// re-map Smithay has discarded everything pending, so it is rebuilt from
/// the layout first (`State::restore_layout_state`).
fn send_initial_configure(state: &State, surface: &WlSurface) {
    let sent = with_states(surface, |states| {
        states.data_map.get::<XdgToplevelSurfaceData>().map(|data| {
            data.lock()
                .expect("toplevel attributes")
                .initial_configure_sent
        })
    });
    if sent == Some(false)
        && let Some(id) = state.id_of(surface)
        && let Some(toplevel) = state.window(id).and_then(|w| w.toplevel())
    {
        state.restore_layout_state(id);
        toplevel.send_configure();
    }
}

impl BufferHandler for State {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
}

impl ShmHandler for State {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl XdgShellHandler for State {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        self.add_window(surface);
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        if let Some(id) = self.id_of(surface.wl_surface()) {
            self.remove_window(id);
        }
    }

    fn app_id_changed(&mut self, surface: ToplevelSurface) {
        if let Some(id) = self.id_of(surface.wl_surface()) {
            self.refresh_window(id);
        }
    }

    fn title_changed(&mut self, surface: ToplevelSurface) {
        if let Some(id) = self.id_of(surface.wl_surface()) {
            self.refresh_window(id);
        }
    }

    /// The core keeps each window's parent (`WindowInfo::parent`): a window
    /// that floats is centred on it, and a focused floating window that
    /// closes hands focus back to it (see `floating.rs`).
    fn parent_changed(&mut self, surface: ToplevelSurface) {
        if let Some(id) = self.id_of(surface.wl_surface()) {
            self.refresh_window(id);
        }
    }

    /// The window's own fullscreen request -- see `fullscreen.rs` for what
    /// it does, the output hint included, and why it is honoured while
    /// locked.
    fn fullscreen_request(&mut self, surface: ToplevelSurface, output: Option<WlOutput>) {
        self.client_fullscreen_request(&surface, true, output);
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        self.client_fullscreen_request(&surface, false, None);
    }

    /// A CSD titlebar drag: honoured for a floating window while the press
    /// it rides on is held, ignored for a tiled one -- see
    /// `floating/grab.rs`.
    fn move_request(&mut self, surface: ToplevelSurface, seat: wl_seat::WlSeat, serial: Serial) {
        self.client_floating_drag(&surface, &seat, serial, None);
    }

    /// A CSD border drag: the same rules as `move_request`. `none` resizes
    /// nothing.
    fn resize_request(
        &mut self,
        surface: ToplevelSurface,
        seat: wl_seat::WlSeat,
        serial: Serial,
        edges: xdg_toplevel::ResizeEdge,
    ) {
        if let Some(edges) = super::floating::grab::requested_edges(edges) {
            self.client_floating_drag(&surface, &seat, serial, Some(edges));
        }
    }

    /// Every `xdg_popup` is tracked here, whatever it will end up parented
    /// to -- including one created with no xdg parent at all, which is how
    /// a layer surface's own dropdown is built
    /// (`xdg_surface.get_popup(None, ..)` then
    /// `zwlr_layer_surface_v1.get_popup`). Such a popup lands in
    /// `PopupManager`'s unmapped list until its first commit, by which time
    /// the layer-shell request has set its parent, and `PopupManager::commit`
    /// moves it into the parent's tree from there.
    ///
    /// That is why `WlrLayerShellHandler::new_popup` never tracks: tracking
    /// a popup a second time on that path puts a *second* node for the same
    /// surface in the layer surface's `PopupTree` -- measured, see
    /// `layer_shell/tests/popup.rs` -- which every tree walk then sees twice
    /// and which a dismissal only half removes. It only checks that the
    /// popup may be adopted at all (`popup_parent::check_adoption`).
    ///
    /// The positioner is not read here: the popup's constraint adjustment is
    /// applied at its initial configure instead (see
    /// `send_popup_initial_configure`), which is the first point a layer
    /// surface's popup has a parent to be constrained against.
    ///
    /// A popup that would loop, nest more than `MAX_POPUP_DEPTH` deep, or
    /// let an existing chain grow later is refused first, and its client
    /// disconnected: tracking it would run Smithay's unbounded walk up that
    /// chain and its unbounded recursion down the tree -- see
    /// `popup_parent.rs`. A popup on a surface that was a popup before has
    /// the dead one reaped from the tree first, so its own children cannot
    /// be inserted under the dead node (see `Admission::Reused`).
    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        match super::popup_parent::admit(&surface) {
            Admission::Refused => return,
            Admission::Reused => self.popups.cleanup(),
            Admission::Fresh => {}
        }
        let _ = self
            .popups
            .track_popup(smithay::desktop::PopupKind::Xdg(surface));
    }

    /// Closes the popup's record and refuses a destroy that leaves child
    /// popups behind -- see `popup_parent.rs`.
    fn popup_destroyed(&mut self, surface: PopupSurface) {
        super::popup_parent::popup_destroyed(&surface);
    }

    fn grab(&mut self, surface: PopupSurface, seat: wl_seat::WlSeat, serial: Serial) {
        self.grab_popup(surface, seat, serial);
    }

    /// `xdg_popup.reposition`: the new positioner's geometry, constrained the
    /// same way the initial configure's is (see `popup_constraint.rs`), or
    /// the positioner's own where no target is known. `send_repositioned`
    /// sends the `repositioned` + `configure` pair the protocol asks for.
    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        let geometry = self
            .constrained_popup_geometry(&surface, positioner)
            .unwrap_or_else(|| positioner.get_geometry());
        surface.with_pending_state(|state| {
            state.geometry = geometry;
            state.positioner = positioner;
        });
        surface.send_repositioned(token);
    }
}

/// The opt-in XWayland skeleton's protocol halves (see `xwayland.rs`).
///
/// `XWaylandShellHandler` is the association protocol (`xwayland_shell_v1`,
/// which pairs each X window with a `wl_surface`); the default
/// `surface_associated` no-op stands, because Phase 1 files nothing about
/// the pair anywhere -- mapping is Phase 2.
///
/// `XwmHandler` is the window-manager side. Every required method lands
/// here, and every one refuses-by-default in the Phase-1 sense: `new_*`
/// log, `map_window_request` deliberately never calls
/// `X11Surface::set_mapped` (no X window enters the core -- the boundary
/// `xwayland.rs` states), `configure_request` never calls `configure`, and
/// `resize_request`/`move_request` start no grab. `active_window_request`
/// is *not* implemented on purpose: the trait's default is a no-op (refuse),
/// and that is the spike-verified answer -- X11 has no activation serials,
/// so honouring `NET_ACTIVE_WINDOW` unconditionally would be the exact
/// focus-stealing hole `activation-serial-validation` closed for xdg, now
/// in X form. Phase 3 owns the gate (focus-on-map only if
/// spawned-here-with-chain or nothing focused; `active_window_request`
/// through the same gate, never a command).
#[cfg(feature = "xwayland")]
impl XWaylandShellHandler for State {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        &mut self.xwayland_shell_state
    }
}

#[cfg(feature = "xwayland")]
impl XwmHandler for State {
    fn xwm_state(&mut self, _xwm: XwmId) -> &mut X11Wm {
        // Callbacks originate from the stored manager's own event handling,
        // so it is always there when one fires -- the way anvil reads its
        // `Option` too. An `expect`, not an `Option` return, because the
        // trait gives no other shape and inventing a dummy would be worse.
        self.xwm
            .as_mut()
            .expect("an XWM callback fired without a running X server")
    }

    fn new_window(&mut self, _xwm: XwmId, window: X11Surface) {
        tracing::debug!(
            id = window.window_id(),
            "X11 window created (Phase-1 skeleton: not mapped)"
        );
    }

    fn new_override_redirect_window(&mut self, _xwm: XwmId, window: X11Surface) {
        tracing::debug!(
            id = window.window_id(),
            "X11 override-redirect window created (Phase-1 skeleton: unmanaged, as ever)"
        );
    }

    fn map_window_request(&mut self, _xwm: XwmId, window: X11Surface) {
        // Refuse-by-default: `set_mapped(true)` is never called, so the
        // window never becomes visible and never enters the core. `debug!`,
        // not `warn!`: a 30-window storm is 30 lines, and refusal is the
        // designed Phase-1 answer, not an anomaly.
        tracing::debug!(
            id = window.window_id(),
            "X11 map request refused (Phase-1 skeleton: X windows do not enter the layout)"
        );
    }

    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, window: X11Surface) {
        // A notification, not a request: override-redirect windows map
        // themselves and the XWM cannot prohibit it. Nothing to manage in
        // this phase (and nothing managed in any phase -- they stay
        // unmanaged by policy), so this only logs.
        tracing::debug!(
            id = window.window_id(),
            "X11 override-redirect window mapped itself (unmanaged)"
        );
    }

    fn unmapped_window(&mut self, _xwm: XwmId, window: X11Surface) {
        tracing::debug!(id = window.window_id(), "X11 window unmapped");
    }

    fn destroyed_window(&mut self, _xwm: XwmId, window: X11Surface) {
        tracing::debug!(id = window.window_id(), "X11 window destroyed");
    }

    fn configure_request(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        x: Option<i32>,
        y: Option<i32>,
        w: Option<u32>,
        h: Option<u32>,
        reorder: Option<Reorder>,
    ) {
        // Refuse-by-default, like the map request above: `configure` is
        // never called, so the window keeps whatever geometry it has (and,
        // unmapped, is invisible anyway). The ask is logged whole -- Phase
        // 2's mapping will need to expect this chatter per window, not one
        // shot each (measured: ~2 configures plus ~13 property notifies per
        // xterm).
        tracing::debug!(
            id = window.window_id(),
            ?x,
            ?y,
            ?w,
            ?h,
            ?reorder,
            "X11 configure request refused (Phase-1 skeleton)"
        );
    }

    fn configure_notify(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        geometry: Rectangle<i32, Logical>,
        above: Option<X11Window>,
    ) {
        tracing::debug!(
            id = window.window_id(),
            ?geometry,
            ?above,
            "X11 window reconfigured"
        );
    }

    fn resize_request(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        button: u32,
        resize_edge: ResizeEdge,
    ) {
        // No pointer grabs in this phase: refuse by doing nothing.
        tracing::debug!(
            id = window.window_id(),
            button,
            ?resize_edge,
            "X11 resize request refused (Phase-1 skeleton)"
        );
    }

    fn move_request(&mut self, _xwm: XwmId, window: X11Surface, button: u32) {
        // Same as above: no grabs, no moves.
        tracing::debug!(
            id = window.window_id(),
            button,
            "X11 move request refused (Phase-1 skeleton)"
        );
    }

    fn disconnected(&mut self, _xwm: XwmId) {
        // The post-`READY` death signal (the pre-`READY` one is
        // `XWaylandEvent::Error` -- see `xwayland.rs`): the server is gone,
        // the session is not. `warn!`, not `debug!`: an operator whose X
        // apps just died needs this line, and it fires once per session
        // death, not per event. `xdisplay` is deliberately *not* cleared
        // (see `xwayland.rs`'s staleness note).
        tracing::warn!(
            display = ?self.xdisplay,
            "XWayland connection lost; the session continues Wayland-only (restart for X11)"
        );
    }
}

/// `zwp_xwayland_keyboard_grab_manager_v1`: present once the server started
/// (see `xwayland::start`), but Phase 1 answers no grab for any surface --
/// `None` means Smithay creates nothing, so a grab request is a silent
/// no-op rather than a keyboard handoff. The focus half (`keyboard_focus`
/// for a real X surface, the serial-less trust question) is Phase 3's gate
/// to design, not a default to fall into here.
#[cfg(feature = "xwayland")]
impl XWaylandKeyboardGrabHandler for State {
    fn keyboard_focus_for_xsurface(
        &self,
        _surface: &WlSurface,
    ) -> Option<super::keyboard_focus::KeyboardFocus> {
        None
    }
}

/// `zxdg_decoration_manager_v1`: lets a client ask the compositor whether it
/// or the client itself should draw window decorations. This project draws
/// none (see `decorations.rs`'s module doc) but always answers `ServerSide`
/// when `appearance.prefer_no_csd` is set (niri's own default, and this
/// project's), which is enough to stop a well-behaved client (e.g. foot)
/// from drawing its own titlebar -- the actual protocol-correctness fix
/// behind this feature, independent of the ring/background pixels.
///
/// The three methods share one policy: if `prefer_no_csd`, force
/// `ServerSide` regardless of what the client asked for or asks again
/// later; if not, do exactly what the client asked -- `request_mode` sets
/// that exact mode, and `new_decoration`/`unset_mode` leave `decoration_mode`
/// unset, which Smithay's own toplevel configure logic already treats as
/// `ClientSide` (see `wayland::shell::xdg::mod.rs`'s
/// `decoration_mode.unwrap_or(Mode::ClientSide)`) -- so "leave it unset" and
/// "explicitly request ClientSide" are the same outcome here, and there's no
/// need to set it explicitly to get that behavior.
impl XdgDecorationHandler for State {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        tracing::debug!(
            prefer_no_csd = self.appearance.prefer_no_csd,
            "zxdg_toplevel_decoration_v1 created"
        );
        if self.appearance.prefer_no_csd {
            toplevel.with_pending_state(|state| state.decoration_mode = Some(Mode::ServerSide));
        }
        toplevel.send_configure();
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, requested: Mode) {
        let chosen = if self.appearance.prefer_no_csd {
            Mode::ServerSide
        } else {
            requested
        };
        // Logged as two separate fields, not one: when prefer_no_csd forces
        // an override, `requested` and `chosen` differ, and collapsing them
        // into a single "mode" field (as an earlier draft of this did) reads
        // as "the client asked for ServerSide" even when it asked for the
        // opposite and got overridden.
        tracing::debug!(?requested, ?chosen, "client requested a decoration mode");
        toplevel.with_pending_state(|state| state.decoration_mode = Some(chosen));
        toplevel.send_configure();
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        tracing::debug!("client unset its decoration mode preference");
        toplevel.with_pending_state(|state| {
            state.decoration_mode = self.appearance.prefer_no_csd.then_some(Mode::ServerSide);
        });
        toplevel.send_configure();
    }
}

impl SeatHandler for State {
    /// A Wayland surface or an X11 window -- see `keyboard_focus.rs` for why
    /// the X half cannot be its `wl_surface`.
    type KeyboardFocus = super::keyboard_focus::KeyboardFocus;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<State> {
        &mut self.seat_state
    }

    fn cursor_image(
        &mut self,
        _seat: &Seat<Self>,
        image: smithay::input::pointer::CursorImageStatus,
    ) {
        self.cursor.set_status(image);
        // Only `--tty` ever draws a cursor element into a frame (see
        // `cursor.rs`'s module doc), so only it needs a redraw when the
        // request changes -- a gratuitous render on every such event under
        // headless/nested would do real work for nothing on screen. A capture
        // that asked for the pointer still sees the new image: see
        // `cursor_changed`.
        self.cursor_changed();
    }

    fn focus_changed(
        &mut self,
        seat: &Seat<Self>,
        focused: Option<&super::keyboard_focus::KeyboardFocus>,
    ) {
        let handle = &self.display_handle;
        // The surface keys go to, whichever kind of window owns it: an X
        // window's is XWayland's own client, which is who the selection is
        // offered to on its behalf.
        let client = focused
            .and_then(WaylandFocus::wl_surface)
            .and_then(|surface| handle.get_client(surface.id()).ok());
        set_data_device_focus(handle, seat, client.clone());
        // Without this, no regular primary-selection device is ever offered
        // anything: Smithay only sends the primary selection to a device
        // whose client holds the primary focus, and nothing else sets it.
        // (Data-control devices bypass focus, which is why the clipboard
        // protocols worked without it.) Same pair anvil calls.
        set_primary_focus(handle, seat, client);
    }
}

/// `wp_cursor_shape_v1` reaches a tablet tool as well as a pointer, so its
/// dispatch is bounded on this trait -- and since `zwp_tablet_manager_v2`
/// is advertised, tools really exist here: a client names a shape for its
/// tool (or uploads a cursor surface for it) and
/// [`State::set_tool_cursor_image`] lands it in the same cursor status the
/// pointer half uses.
///
/// `ToolFocus` is `WlSurface` to match the three focus types in
/// [`SeatHandler`] above: the trait requires the same `WaylandFocus` bound
/// they satisfy, and the tool events in `tablet.rs` are addressed to
/// whatever `surface_under` finds -- the same surfaces pointer focus
/// names, not a second notion of focus.
impl TabletSeatHandler for State {
    type ToolFocus = WlSurface;

    fn tablet_tool_image(&mut self, _tool: &TabletToolDescriptor, image: CursorImageStatus) {
        self.set_tool_cursor_image(image);
    }
}

impl SelectionHandler for State {
    type SelectionUserData = ();
}

impl DataDeviceHandler for State {
    fn data_device_state(&mut self) -> &mut DataDeviceState {
        &mut self.data_device_state
    }
}

/// `zwlr_data_control_manager_v1` (clipboard managers) and
/// `ext_data_control_manager_v1` (its successor): both Smithay states, both
/// routed here. The two `DataControlHandler` traits share a name across
/// modules -- hence the aliases on import -- but route to different fields,
/// so there is no ambiguity about which clipboard generation a request came
/// through.
impl WlrDataControlHandler for State {
    fn data_control_state(&mut self) -> &mut WlrDataControlState {
        &mut self.wlr_data_control_state
    }
}

impl ExtDataControlHandler for State {
    fn data_control_state(&mut self) -> &mut ExtDataControlState {
        &mut self.ext_data_control_state
    }
}

/// `zwp_primary_selection_device_manager_v1` (middle-click paste).
impl PrimarySelectionHandler for State {
    fn primary_selection_state(&mut self) -> &mut PrimarySelectionState {
        &mut self.primary_selection_state
    }
}

impl DndGrabHandler for State {}

impl WaylandDndGrabHandler for State {
    fn dnd_requested<S: Source>(
        &mut self,
        source: S,
        _icon: Option<WlSurface>,
        seat: Seat<Self>,
        serial: Serial,
        type_: GrabType,
    ) {
        match type_ {
            GrabType::Pointer => {
                let Some(pointer) = seat.get_pointer() else {
                    return source.cancel();
                };
                let Some(start_data) = pointer.grab_start_data() else {
                    return source.cancel();
                };
                // The serial is only meaningful against the seat that issued
                // it, and a second seat one day must not silently inherit
                // this seat's history -- the same check `popup.rs` applies to
                // a grab's seat. In practice scoot owns exactly one seat, so
                // this never fires; Smithay already resolved `seat` from the
                // data device's own `wl_seat` before calling this at all.
                if seat != self.seat {
                    tracing::warn!("refusing a drag: it names a seat this compositor does not own");
                    return source.cancel();
                }
                // Whose drag this is: the data source's own client, which no
                // client can forge -- not a surface owner like the popup
                // gate's `client_of`. See `dnd_source_client`.
                let Some(client) = dnd_source_client(&source) else {
                    tracing::warn!("refusing a drag: its source names no known client");
                    return source.cancel();
                };
                // The serial has to name a real, recent key or button event
                // actually delivered to the client asking -- the strict
                // `contains` half of `input/interaction.rs`, byte-for-byte
                // the activation gate, deliberately *not* the popup gate's
                // looser `contains_seen`:
                //
                // - The protocol's serial is "the serial number of the
                //   implicit grab on the origin" (`wayland.xml`,
                //   `wl_data_device.start_drag`), i.e. the button press being
                //   converted -- and Smithay's dispatch only calls this with
                //   a serial that already equals the live grab's
                //   (`pointer.has_grab(serial)`, "in response to a pointer
                //   implicit grab"). A focus `enter` can only be a live
                //   grab's serial while someone's explicit popup grab holds
                //   the seat, and spending that here would bless converting
                //   an explicit grab into a drag. No real toolkit needs it:
                //   GTK mints the drag from the press, Qt from its last-seen
                //   serial, which a press has just overwritten.
                // - That dispatch check is also why this gate is not
                //   vacuous: `has_grab` compares the serial alone, and
                //   serials are process-global numbers any client can read
                //   off a configure and spray for free (a wrong guess is a
                //   silent dispatch deny). What it cannot do is bind the
                //   serial to who received the press -- so without the pair
                //   check below, any client naming the victim's live press
                //   serial while the user holds any button starts a drag
                //   from its own source. The press is recorded under whoever
                //   the pointer focus named (bare-desktop presses under no
                //   one), so only that client can spend it.
                //
                // One accepted limitation, stated not hidden: the ring's age
                // bound applies, so a press held past `INTERACTION_WINDOW`
                // (10s) is refused even still held -- motion deliberately
                // refreshes nothing. Real drags cross their motion threshold
                // within milliseconds of the press; a menu-style session
                // rule would only re-open the cross-client hole above, so
                // there is none.
                if !self.interaction_serials.contains(serial, &client) {
                    // `warn`, not `debug`: cancelling the source posts no
                    // protocol error, so the log is the only way to tell "no
                    // recent interaction" apart from a broken drag source --
                    // the same debuggability the popup gate logs for.
                    // Rate is one line per drag requested, not per event or
                    // frame.
                    tracing::warn!(
                        ?serial,
                        ?client,
                        "refusing a drag: its serial is not a recent key or button event this client received"
                    );
                    return source.cancel();
                }
                let grab = DnDGrab::new_pointer(&self.display_handle, start_data, source, seat);
                pointer.set_grab(self, grab, serial, Focus::Keep);
            }
            GrabType::Touch => source.cancel(),
        }
    }
}

/// Who asked for a drag: the data source's own client.
///
/// `dnd_requested` is generic over `S: Source`, which carries no client
/// accessor, so this downcasts to the two concrete sources Smithay's
/// dispatch can pass: the `WlDataSource` the client offered, or the origin
/// `WlSurface` when `source` was NULL (a client-internal drag, whose owner
/// is still the client that must have received the press). Both are
/// `Resource`s, so the id names a client no sender can forge.
///
/// Anything else -- a compositor-internal source type scoot does not have
/// today -- resolves to nothing and fails closed at the call site: the only
/// producer of these calls is Smithay's dispatch with the two types above,
/// so an unknown type is unexpected, not a third legitimate shape.
fn dnd_source_client<S: Source>(source: &S) -> Option<ClientId> {
    let any = source as &dyn Any;
    any.downcast_ref::<WlDataSource>()
        .and_then(|owned| owned.client())
        .or_else(|| {
            any.downcast_ref::<WlSurface>()
                .and_then(|owned| owned.client())
        })
        .map(|client| client.id())
}

/// A client binding a `wl_output` is the second half of
/// `ext-workspace-v1`'s `output_enter` rule: a workspace group has to name
/// the outputs it covers, including ones the client only binds *after* it
/// bound the workspace manager. See `ext_workspace.rs`.
///
/// Checked rather than assumed, because it decides what this hook may do:
/// the pinned Smithay rev calls it with the output's own lock **released**
/// (`wayland/output/handlers.rs` ends its `bind` with `drop(inner);
/// state.output_bound(..)`), so reading the output back -- `client_outputs`,
/// `current_mode`, anything on `Output::inner` -- is safe here. That is a
/// fact about this rev, not a guarantee: if a future one ever calls this
/// while holding that guard, any such call from inside this hook deadlocks
/// the compositor against itself, the same hazard `layer_shell.rs`'s
/// guard-discipline note describes for layer maps.
impl OutputHandler for State {
    fn output_bound(&mut self, output: Output, wl_output: WlOutput) {
        self.workspace_group_output_bound(&output, &wl_output);
        // The same hook for the same reason, one protocol over:
        // `wlr-foreign-toplevel-management-v1`'s `output_enter` is owed to a
        // client that bound this manager before it bound the screen. See
        // `foreign_toplevel_management.rs`.
        self.wlr_toplevel_output_bound(&output, &wl_output);
    }
}

// The `Dispatch`/`GlobalDispatch` impls these handlers are reached through
// are hand-written in `dispatch.rs` rather than generated by
// `smithay::delegate_dispatch2!(State)` -- see that module's doc for the
// client-triggerable compositor panic that forced it.
