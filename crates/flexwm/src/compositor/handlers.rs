//! The Wayland protocol handlers Smithay dispatches into.

use std::any::Any;

use smithay::backend::renderer::utils::on_commit_buffer_handler;
use smithay::input::dnd::{DnDGrab, DndGrabHandler, GrabType, Source};
use smithay::input::pointer::Focus;
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
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::compositor::{
    CompositorClientState, CompositorHandler, CompositorState, get_parent, get_role,
    is_sync_subsurface, with_states,
};
use smithay::wayland::output::OutputHandler;
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
use smithay::wayland::shell::xdg::decoration::XdgDecorationHandler;
use smithay::wayland::shell::xdg::{
    PopupSurface, PositionerState, ToplevelSurface, XDG_POPUP_ROLE, XdgShellHandler, XdgShellState,
    XdgToplevelSurfaceData,
};
use smithay::wayland::shm::{ShmHandler, ShmState};

use super::State;
use super::output_scale::send_preferred_buffer_scale;
use super::state::ClientState;

impl CompositorHandler for State {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client
            .get_data::<ClientState>()
            .expect("client state")
            .compositor_state
    }

    /// A new `wl_surface` exists: tell it the integer `preferred_buffer_scale`.
    ///
    /// This is the only call site. Smithay runs `new_surface` for every
    /// surface `wl_compositor.create_surface` makes (subsurfaces included), and
    /// the scale is fixed for the process's life, so this always fires before
    /// any commit -- a per-commit call would be a no-op cache hit on the hot
    /// path, not defence in depth. See `send_preferred_buffer_scale`'s doc.
    fn new_surface(&mut self, surface: &WlSurface) {
        send_preferred_buffer_scale(surface, self.integer_scale);
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        self.last_commit = std::time::Instant::now();
        self.request_render();

        if !is_sync_subsurface(surface) {
            let mut root = surface.clone();
            while let Some(parent) = get_parent(&root) {
                root = parent;
            }
            if let Some(id) = self.id_of(&root) {
                if let Some(window) = self.window(id) {
                    window.on_commit();
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
    }

    /// Smithay calls this for every `wl_surface` that goes away, whether the
    /// client destroyed it explicitly or simply quit.
    ///
    /// Three things this compositor keeps a `WlSurface` in outside
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
    /// - the dead entries of `mapped_layers`: a surface whose `wl_surface`
    ///   dies before its layer role object (the implicit-disconnect order)
    ///   never sees `layer_destroyed`, so nothing else removes it -- see
    ///   `layer_shell.rs`. Memory-only either way: a server-side `ObjectId`
    ///   compares its client id and generation serial as well as the bare
    ///   id (wayland-backend 0.3.17 `rs/server_impl/mod.rs`), so a stale
    ///   entry can never equal a live surface.
    fn destroyed(&mut self, surface: &WlSurface) {
        if self.cursor.forget_surface(surface) && self.tty.is_some() {
            // The cursor's shape just changed to the fallback; only `--tty`
            // draws one at all, same gate as `cursor_image` below.
            self.request_render();
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
    if !popup.is_initial_configure_sent()
        && let Err(error) = popup.send_configure()
    {
        tracing::warn!(?error, "xdg_popup initial configure failed");
    }
}

/// A toplevel needs one configure before it may attach a buffer.
fn send_initial_configure(state: &State, surface: &WlSurface) {
    let sent = with_states(surface, |states| {
        states.data_map.get::<XdgToplevelSurfaceData>().map(|data| {
            data.lock()
                .expect("toplevel attributes")
                .initial_configure_sent
        })
    });
    if sent == Some(false)
        && let Some(toplevel) = state
            .id_of(surface)
            .and_then(|id| state.window(id))
            .and_then(|w| w.toplevel())
    {
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

    /// Every `xdg_popup` is tracked here, whatever it will end up parented
    /// to -- including one created with no xdg parent at all, which is how
    /// a layer surface's own dropdown is built
    /// (`xdg_surface.get_popup(None, ..)` then
    /// `zwlr_layer_surface_v1.get_popup`). Such a popup lands in
    /// `PopupManager`'s unmapped list until its first commit, by which time
    /// the layer-shell request has set its parent, and `PopupManager::commit`
    /// moves it into the parent's tree from there.
    ///
    /// That is why `WlrLayerShellHandler::new_popup` stays unimplemented:
    /// tracking a popup a second time on that path puts a *second* node for
    /// the same surface in the layer surface's `PopupTree` -- measured, see
    /// `layer_shell/tests/popup.rs` -- which every tree walk then sees twice
    /// and which a dismissal only half removes.
    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        let _ = self
            .popups
            .track_popup(smithay::desktop::PopupKind::Xdg(surface));
    }

    fn grab(&mut self, surface: PopupSurface, seat: wl_seat::WlSeat, serial: Serial) {
        self.grab_popup(surface, seat, serial);
    }

    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        surface.with_pending_state(|state| {
            state.geometry = positioner.get_geometry();
            state.positioner = positioner;
        });
        surface.send_repositioned(token);
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
    type KeyboardFocus = WlSurface;
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
        // Only `--tty` ever draws a cursor element (see `cursor.rs`'s module
        // doc), so only it needs a redraw when the request changes -- a
        // gratuitous render on every such event under headless/nested would
        // do real work for a status this project never looks at there.
        if self.tty.is_some() {
            self.request_render();
        }
    }

    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&WlSurface>) {
        let handle = &self.display_handle;
        let client = focused.and_then(|surface| handle.get_client(surface.id()).ok());
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
/// dispatch is bounded on this trait whether or not a compositor offers
/// `zwp_tablet_manager_v2` -- which this one does not. Implementing it
/// advertises no global on its own (only `TabletManagerState::new` does
/// that), so no client can ever construct the `zwp_tablet_tool_v2` the
/// tablet half of `get_tablet_tool_v2` needs; the type exists here purely to
/// satisfy the bound so the *pointer* half compiles.
///
/// `ToolFocus` is `WlSurface` to match the three focus types in
/// [`SeatHandler`] above: the trait requires the same `WaylandFocus` bound
/// they satisfy, and picking a different type here would only invent a second
/// notion of focus for a device this compositor has no input path for.
impl TabletSeatHandler for State {
    type ToolFocus = WlSurface;
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
                // a grab's seat. In practice flexwm owns exactly one seat, so
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
/// Anything else -- a compositor-internal source type flexwm does not have
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
