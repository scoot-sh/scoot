//! What the compositor owns: Smithay's protocol state, the Wayland windows, and
//! the [`World`] that decides where they go.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Instant;

use flexwm_core::{Config, Size, WindowId, World};
use smithay::desktop::{LayerSurface, PopupManager, Space, Window, WindowSurfaceType};
use smithay::input::keyboard::Keycode;
use smithay::input::{Seat, SeatState};
use smithay::output::Output;
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{EventLoop, Interest, LoopHandle, LoopSignal, Mode, PostAction};
use smithay::reexports::wayland_server::backend::{ClientData, ClientId, DisconnectReason};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{BindError, Display, DisplayHandle};
use smithay::utils::{Logical, Point};
use smithay::wayland::compositor::{CompositorClientState, CompositorState};
use smithay::wayland::fractional_scale::FractionalScaleManagerState;
use smithay::wayland::output::OutputManagerState;
use smithay::wayland::selection::data_device::DataDeviceState;
use smithay::wayland::selection::ext_data_control::DataControlState as ExtDataControlState;
use smithay::wayland::selection::primary_selection::PrimarySelectionState;
use smithay::wayland::selection::wlr_data_control::DataControlState as WlrDataControlState;
use smithay::wayland::shell::wlr_layer::WlrLayerShellState;
use smithay::wayland::shell::xdg::XdgShellState;
use smithay::wayland::shell::xdg::decoration::XdgDecorationState;
use smithay::wayland::shm::ShmState;
use smithay::wayland::socket::ListeningSocketSource;
use smithay::wayland::viewporter::ViewporterState;

use super::cursor::Cursor;
use super::decorations::{Appearance, Decorations};
use super::ext_workspace::ExtWorkspaceState;
use super::gamma_control::GammaControlState;
use super::headless::Backend;
use super::ipc::PendingIdle;
use super::keybindings::Keybindings;
use super::layer_shell;
use super::nested::Host;
use super::session_lock::SessionLock;
use super::tty::Tty;

#[cfg(test)]
mod tests;

pub struct State {
    pub start_time: Instant,
    pub display_handle: DisplayHandle,
    pub loop_handle: LoopHandle<'static, State>,
    pub loop_signal: LoopSignal,
    pub socket_name: OsString,
    pub ipc_path: Option<PathBuf>,

    /// The layout. Everything else here exists to serve it.
    pub world: World,
    pub windows: HashMap<WindowId, Window>,
    /// The size each window was last asked for, to pair with what it becomes.
    pub requested: HashMap<WindowId, Size>,
    pub next_id: u64,
    /// The focused *window*, so activation and the focus ring are only moved
    /// when they change. Not necessarily what holds the keyboard: a layer
    /// surface can (see `clicked_layer` and `layer_shell.rs`), and this stays
    /// pointing at the window focus will come back to when it doesn't.
    pub focus: Option<WindowId>,
    /// The layer surface a click gave keyboard focus to, if any -- the one
    /// piece of the layer-shell focus policy that cannot be re-derived from
    /// the layer map, because nothing else records that a click happened.
    ///
    /// Holding a `LayerSurface` here holds an `Arc` to a client's surface, so
    /// it is cleared as soon as it stops meaning anything: on that surface's
    /// destruction (`layer_destroyed`), on the next click elsewhere, on the
    /// commit where that surface stops wanting the keyboard at all
    /// (`commit_layer_surface` -- a click is spent once the thing it focused
    /// asks for `none` or unmaps itself), and defensively on every focus
    /// refresh if the client vanished without any of those
    /// (`forget_dead_clicked_layer`).
    pub clicked_layer: Option<LayerSurface>,
    /// Whether the keyboard focus `refresh_keyboard_focus` last handed out
    /// went to a layer surface rather than a window's toplevel.
    ///
    /// Written *only* there, read *only* by `commit_layer_surface`'s gate,
    /// and it means exactly that -- not "a layer surface wants the keyboard"
    /// and not "`clicked_layer` is set". It exists so a bar with
    /// `keyboard_interactivity: none` (i.e. nearly every layer surface that
    /// will ever run) skips focus resolution entirely on each of its
    /// redraws, while a surface that *stops* wanting the keyboard still gets
    /// it taken away.
    pub keyboard_on_layer: bool,

    pub space: Space<Window>,
    pub popups: PopupManager,
    pub output: Option<Output>,
    /// The output scale resolved from `[output] scale` (see
    /// `output_scale.rs`), fixed for the process's lifetime. Read by
    /// `headless`'s `set_mode` (which applies it to the `Output`), by the
    /// `wp_fractional_scale_v1` handler (which advertises it per surface), and
    /// by `ipc.rs` (which reports it to an agent that must convert between
    /// logical rects and physical screenshot pixels). It is *not* the source
    /// the input clamp reads: that goes through `output_scale::logical_size`,
    /// which derives the logical extent from the `Output` itself, so a site
    /// handling input can never disagree with what the `Space` laid out.
    /// `compositor::run` forces this to 1.0 under `--nested`, where the host
    /// compositor owns the scale.
    pub output_scale: f64,
    /// The integer form of [`Self::output_scale`] -- `ceil(output_scale)`, the
    /// value sent on `wl_surface.preferred_buffer_scale` and the same one
    /// Smithay advertises on `wl_output.scale` (see `output_scale.rs`'s
    /// `integer_scale`). Precomputed once at construction because
    /// `CompositorHandler::commit` reads it on every surface commit, and it can
    /// never disagree with `output_scale`: that field is fixed for the
    /// process's life and nothing writes either one after `new`.
    pub integer_scale: i32,
    pub backend: Option<Backend>,
    /// Set only under `--nested`: the connection presenting `backend`'s
    /// framebuffer as a window in a host compositor, and forwarding that
    /// window's input back into this seat. `None` under `--headless`.
    pub host: Option<Host>,
    /// Set only under `--tty`: the session, DRM device/surface and dumb
    /// buffers presenting `backend`'s framebuffer on a real display, and
    /// the libinput context feeding this seat from real input devices.
    /// `None` under `--headless`/`--nested`.
    pub tty: Option<Tty>,

    /// The ring/background palette, the fallback cursor's size/color and the
    /// `prefer_no_csd` policy, resolved from `[appearance]` (or its defaults)
    /// once at startup -- see `config.rs` and `decorations.rs`'s module docs.
    ///
    /// Read live wherever it is used -- the render path for the ring and
    /// background, `handlers.rs`'s `XdgDecorationHandler` for
    /// `prefer_no_csd` -- with one exception: the two cursor fields, which
    /// `Cursor::new` consumes once below to build a bitmap, so a later write
    /// to those two here would change nothing. Nothing writes to this field
    /// at all today.
    pub appearance: Appearance,
    /// Per-window persistent ring buffers -- see `decorations.rs`'s module
    /// doc for why these live here rather than being rebuilt every frame.
    pub decorations: Decorations,
    /// The pointer's last-requested image -- a client-supplied cursor
    /// surface, a named shape, or hidden -- plus the render buffer behind
    /// the fallback shape. Drawn only under `--tty`; see `cursor.rs`'s
    /// module doc.
    pub cursor: Cursor,

    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    /// `zwlr_layer_shell_v1`: bars, docks, wallpapers and notification
    /// daemons. Unlike the two `#[allow(dead_code)]` states below this one is
    /// read again -- `WlrLayerShellHandler::shell_state` (see
    /// `layer_shell.rs`) routes every layer-shell request through it. The
    /// surfaces themselves live in Smithay's per-output `LayerMap`, not here.
    pub layer_shell_state: WlrLayerShellState,
    /// `ext_workspace_manager_v1`: what a bar reads workspaces from and
    /// switches them through. Unlike the two `#[allow(dead_code)]` states
    /// below, this is read on every `apply` -- see `ext_workspace.rs`, which
    /// owns both the protocol objects and the "what have clients been told"
    /// snapshot behind them.
    pub ext_workspace: ExtWorkspaceState,
    /// `ext_session_lock_manager_v1`: the compositor-enforced screen lock.
    /// Unlike the two `#[allow(dead_code)]` states below, this is read on
    /// every render, every focus refresh and every pointer hit test -- see
    /// `session_lock.rs`, which owns both the protocol objects and the one
    /// field that says whether the session is locked at all.
    pub session_lock: SessionLock,
    /// `wp_fractional_scale_v1`: the fractional value `wl_output.scale` can
    /// only round. Held only to keep the global alive; the handler itself
    /// (`FractionalScaleHandler`) lives in `output_scale.rs`.
    #[allow(dead_code)]
    pub fractional_scale_manager_state: FractionalScaleManagerState,
    /// `wp_viewporter`: the global a client needs to render a fractionally
    /// scaled buffer (it sets a logical destination size and lets the
    /// compositor scale the buffer into it). Held only to keep the global
    /// alive -- the render path reads each surface's `ViewportCachedState`
    /// through `on_commit_buffer_handler` and needs nothing from this field.
    #[allow(dead_code)]
    pub viewporter_state: ViewporterState,
    /// Held only to keep the `zxdg_decoration_manager_v1` global alive --
    /// like `output_manager_state`, `XdgDecorationHandler` (see
    /// `handlers.rs`) has no `&mut XdgDecorationState` accessor to route
    /// dispatch through, so nothing reads this field again after `new`.
    #[allow(dead_code)]
    pub xdg_decoration_state: XdgDecorationState,
    pub shm_state: ShmState,
    /// Held only to keep the xdg-output global alive.
    #[allow(dead_code)]
    pub output_manager_state: OutputManagerState,
    pub seat_state: SeatState<State>,
    pub data_device_state: DataDeviceState,
    /// `zwlr_data_control_manager_v1` (version 2): clipboard managers
    /// (`cliphist`, `clipman`). Held only to keep the global alive -- the
    /// handler (`DataControlHandler`, see `handlers.rs`) routes through it.
    /// No client filter: like the session-lock global, an allow-list would be
    /// theatre without security-context support (see `README.md`'s trust
    /// note). Constructed after `primary_selection_state` because it borrows
    /// it, so data-control clients can also touch the primary selection.
    #[allow(dead_code)]
    pub wlr_data_control_state: WlrDataControlState,
    /// `ext_data_control_manager_v1` (version 1): the successor to the wlr
    /// clipboard protocol, exposed alongside it the way current compositors
    /// do. Same allow-list rationale and same borrow of
    /// `primary_selection_state` as above.
    #[allow(dead_code)]
    pub ext_data_control_state: ExtDataControlState,
    /// `zwp_primary_selection_device_manager_v1` (version 1): middle-click
    /// paste. Held only to keep the global alive, same rationale as above.
    #[allow(dead_code)]
    pub primary_selection_state: PrimarySelectionState,
    /// `zwlr_gamma_control_manager_v1`: night-light tools. Unlike the three
    /// above this is read again -- every `get_gamma_control`/`set_gamma`
    /// goes through it (see `gamma_control.rs`).
    pub gamma_control: GammaControlState,
    pub seat: Seat<State>,

    pub keybindings: Keybindings,
    /// Keycodes currently held that a keybinding intercepted on press, so
    /// their matching release is intercepted too instead of forwarded to
    /// whatever the focused client becomes in between. See `input::key`.
    ///
    /// Invariant: only `key()` inserts or removes entries here. If any
    /// future code path ever forwards or intercepts a key release *without*
    /// going through `key()` -- e.g. Smithay's `KeyboardHandle::release_source`,
    /// used to tear down a virtual keyboard or a departing libinput device
    /// by forwarding its releases directly -- it must also remove that
    /// keycode from here, or the next real press+release of the same
    /// keycode will have its legitimate release wrongly intercepted as a
    /// stale stuck entry.
    pub suppressed_keys: HashSet<Keycode>,

    /// Something changed that the framebuffer doesn't show yet.
    pub needs_render: bool,
    /// Whether the frame timer (see `headless::ensure_ticking`) is currently
    /// running. It drops itself when there's nothing to do rather than
    /// polling forever, so this is how callers know whether to re-arm it.
    pub timer_armed: bool,
    /// When a client last committed, which is what `wait-idle` waits on.
    pub last_commit: Instant,
    pub pending_idle: Vec<PendingIdle>,
}

impl State {
    /// Builds the compositor's state and opens its wayland socket.
    ///
    /// Fallible because that socket is: `$XDG_RUNTIME_DIR` may be unset (the
    /// common case -- a bare `ssh` or `su` shell has none), unwritable, or
    /// already full of other compositors' sockets. None of those is a bug to
    /// panic on, they are an operator's environment to report back to them
    /// (see [`Self::listen`]).
    pub fn new(
        event_loop: &mut EventLoop<'static, State>,
        display: Display<State>,
        config: Config,
        keybindings: Keybindings,
        appearance: Appearance,
        scale: f64,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let dh = display.handle();
        let compositor_state = CompositorState::new_v6::<Self>(&dh);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        let xdg_decoration_state = XdgDecorationState::new::<Self>(&dh);
        let layer_shell_state = WlrLayerShellState::new::<Self>(&dh);
        let ext_workspace = ExtWorkspaceState::new(&dh);
        let session_lock = SessionLock::new(&dh);
        let fractional_scale_manager_state = FractionalScaleManagerState::new::<Self>(&dh);
        let viewporter_state = super::output_scale::viewporter(&dh);
        let shm_state = ShmState::new::<Self>(&dh, vec![]);
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);
        let data_device_state = DataDeviceState::new::<Self>(&dh);
        // Construction order is load-bearing: both data-control states borrow
        // the primary-selection state, so it must exist first.
        let primary_selection_state = PrimarySelectionState::new::<Self>(&dh);
        let wlr_data_control_state =
            WlrDataControlState::new::<Self, _>(&dh, Some(&primary_selection_state), |_| true);
        let ext_data_control_state =
            ExtDataControlState::new::<Self, _>(&dh, Some(&primary_selection_state), |_| true);
        let gamma_control = GammaControlState::new(&dh);

        let mut seat_state = SeatState::new();
        let mut seat = seat_state.new_wl_seat(&dh, "flexwm");
        seat.add_keyboard(Default::default(), 200, 25)
            .expect("a keymap for the default layout");
        seat.add_pointer();

        let socket_name = Self::listen(display, event_loop)?;

        // Built here rather than in the struct literal below, which moves
        // `appearance` before `cursor`'s own field initializer could read it.
        // The fallback bitmap is built exactly once, from the config this
        // process started with -- see `Cursor::new`.
        let cursor = Cursor::new(appearance.cursor_size, appearance.cursor_color);

        Ok(Self {
            start_time: Instant::now(),
            display_handle: dh,
            loop_handle: event_loop.handle(),
            loop_signal: event_loop.get_signal(),
            socket_name,
            ipc_path: None,
            world: World::new(config),
            windows: HashMap::new(),
            requested: HashMap::new(),
            next_id: 0,
            focus: None,
            clicked_layer: None,
            keyboard_on_layer: false,
            space: Space::default(),
            popups: PopupManager::default(),
            output: None,
            output_scale: scale,
            integer_scale: super::output_scale::integer_scale(scale),
            backend: None,
            host: None,
            tty: None,
            appearance,
            decorations: Decorations::default(),
            cursor,
            compositor_state,
            xdg_shell_state,
            layer_shell_state,
            ext_workspace,
            session_lock,
            fractional_scale_manager_state,
            viewporter_state,
            xdg_decoration_state,
            shm_state,
            output_manager_state,
            seat_state,
            data_device_state,
            wlr_data_control_state,
            ext_data_control_state,
            primary_selection_state,
            gamma_control,
            seat,
            keybindings,
            suppressed_keys: HashSet::new(),
            // true without going through request_render(), so nothing has
            // armed the frame timer yet. That's only safe because
            // headless::init() unconditionally and synchronously calls
            // state.apply() -- which does call request_render() -- before
            // the event loop ever runs. If init ever stopped doing that
            // unconditionally, this would silently reintroduce "first frame
            // never draws, timer never arms" with no test failure pointing
            // here.
            needs_render: true,
            timer_armed: false,
            last_commit: Instant::now(),
            pending_idle: Vec::new(),
        })
    }

    /// Opens the Wayland socket and wires it into the event loop.
    ///
    /// Every failure here is a startup error the caller reports and exits on
    /// (`main` prints it and returns `FAILURE`), never a panic: this used to
    /// be `.expect("a free wayland socket")`, which turned the most ordinary
    /// misconfiguration there is -- no `$XDG_RUNTIME_DIR`, i.e.
    /// [`BindError::RuntimeDirNotSet`] -- into a panic and, under this
    /// workspace's `panic = "abort"` release profile, a core dump, with
    /// nothing in it to tell an operator what to fix.
    fn listen(
        display: Display<State>,
        event_loop: &mut EventLoop<'static, State>,
    ) -> Result<OsString, Box<dyn std::error::Error>> {
        // First, before `display` moves into the `Generic` below: this is the
        // failure that actually happens, and nothing else should have been
        // set up by the time it is reported.
        let socket = ListeningSocketSource::new_auto().map_err(socket_error)?;
        let name = socket.socket_name().to_os_string();
        let handle = event_loop.handle();
        handle
            .insert_source(socket, |stream, _, state: &mut State| {
                // A new connection can fail under fd/id exhaustion; that's the
                // misbehaving-client's problem; a single bad file descriptor
                // shouldn't take down every other client's session.
                if let Err(error) = state
                    .display_handle
                    .insert_client(stream, Arc::new(ClientState::default()))
                {
                    tracing::warn!(%error, "could not accept a new wayland client");
                }
            })
            // `InsertError`'s own payload is the source that could not be
            // inserted, which is of no use to an operator and would force
            // this function's error type to carry a `ListeningSocketSource`;
            // only the reason is kept.
            .map_err(|error| error.error)?;
        handle
            .insert_source(
                Generic::new(display, Interest::READ, Mode::Level),
                |_, display, state: &mut State| {
                    // Safety: the display outlives the event loop.
                    let dispatched = unsafe { display.get_mut().dispatch_clients(state) };
                    if let Err(error) = dispatched {
                        // wayland-backend catches a single client's protocol
                        // errors internally and drops just that client;
                        // this only surfaces when the underlying epoll/kevent
                        // wait itself fails, which is process-fatal (there is
                        // nothing left to dispatch to), so stop cleanly
                        // rather than limp on or panic.
                        tracing::error!(%error, "wayland client dispatch failed; stopping");
                        state.loop_signal.stop();
                        return Ok(PostAction::Continue);
                    }
                    // A disconnecting client -- or one destroying its lock
                    // object without disconnecting -- is seen *here* and
                    // nowhere else, which matters for exactly one thing: it
                    // may have been holding the session lock, and nothing
                    // about a destroyed protocol object marks the screen
                    // dirty, drops the surfaces it left behind or moves the
                    // focus off them. See `session_lock.rs`.
                    state.refresh_lock_state();
                    // Replies (e.g. the initial registry globals) should reach
                    // the socket now rather than wait for `mod.rs`'s
                    // `post_dispatch` (which also flushes every client, once
                    // per dispatch cycle, strictly after this source runs) --
                    // a client with no surface yet, like `wayland-info` or
                    // `foot` before its first frame, has nothing else that
                    // would prompt a flush before its own next read, and this
                    // one costs nothing extra when there's nothing queued (see
                    // `post_dispatch`'s doc).
                    let _ = state.display_handle.flush_clients();
                    Ok(PostAction::Continue)
                },
            )
            .map_err(|error| error.error)?;
        Ok(name)
    }

    /// Milliseconds since start, which is what Wayland input events carry.
    pub fn millis(&self) -> u32 {
        self.start_time.elapsed().as_millis() as u32
    }

    pub fn window(&self, id: WindowId) -> Option<&Window> {
        self.windows.get(&id)
    }

    pub fn id_of(&self, surface: &WlSurface) -> Option<WindowId> {
        self.windows
            .iter()
            .find(|(_, window)| window.toplevel().is_some_and(|t| t.wl_surface() == surface))
            .map(|(&id, _)| id)
    }

    /// What the pointer is over: the top-most surface at `pos` and where that
    /// surface sits, in the same global coordinates the pointer moves in.
    ///
    /// The search order is the render order read from the front: a bar or an
    /// overlay wins over any window, and a wallpaper loses to every one of
    /// them. A layer surface that declines `pos` -- an unmapped one, or one
    /// whose client set an input region that excludes the point -- falls
    /// through to whatever is behind it rather than swallowing the pointer,
    /// because `layer_surface_under` asks the surface tree (which honours
    /// `wl_surface.set_input_region`) rather than the layer's bounding box.
    /// Note this is the *input* region, not opacity: a bar drawn fully
    /// transparent but leaving its input region at the default takes the
    /// pointer, which is the protocol's answer and what a click-through bar
    /// has to opt out of explicitly.
    ///
    /// One caveat on "falls through": it falls through to the next *layer*
    /// (overlay, then top), not to another surface on the same layer --
    /// `LayerMap::layer_under` hands back a single layer surface rather than
    /// an iterator, so if the front-most one on a layer declines the point,
    /// a second surface overlapping it on that same layer is not asked.
    /// Overlapping surfaces on one layer are already drawn on top of each
    /// other, so this costs nothing in practice; anvil has the same
    /// limitation.
    pub fn surface_under(
        &self,
        pos: Point<f64, Logical>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        // Before every other candidate, and returning whatever it finds --
        // including `None`. While the session is locked nothing but a lock
        // surface may be pointed at, so this is a replacement for the search
        // below, never a first entry in it (see `session_lock.rs`).
        if self.session_lock.is_locked() {
            return self.lock_surface_under(pos);
        }
        self.layer_surface_under(&layer_shell::ABOVE_WINDOWS, pos)
            .or_else(|| self.window_under(pos))
            .or_else(|| self.layer_surface_under(&layer_shell::BELOW_WINDOWS, pos))
    }

    /// The window surface at `pos`, ignoring layer surfaces entirely.
    pub(super) fn window_under(
        &self,
        pos: Point<f64, Logical>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        self.space
            .element_under(pos)
            .and_then(|(window, location)| {
                window
                    .surface_under(pos - location.to_f64(), WindowSurfaceType::ALL)
                    .map(|(surface, point)| (surface, (point + location).to_f64()))
            })
    }

    /// Runs a command inside this session.
    pub fn spawn(&self, command: &[String]) {
        let Some((program, args)) = command.split_first() else {
            return;
        };
        let mut child = Command::new(program);
        child.args(args).env("WAYLAND_DISPLAY", &self.socket_name);
        if let Some(path) = &self.ipc_path {
            child.env(flexwm_ipc::SOCKET_ENV, path);
        }
        match child.spawn() {
            Ok(_) => tracing::info!(?command, "spawned"),
            Err(error) => tracing::warn!(?command, %error, "could not spawn"),
        }
    }
}

/// What to tell the operator when the wayland socket cannot be created.
///
/// `BindError`'s own `Display` names the cause; each message here adds what to
/// do about it, in the shape this project's other startup errors already have
/// (`ipc::init`'s "no socket path: set FLEXWM_SOCKET or XDG_RUNTIME_DIR",
/// `config.rs`'s `ConfigFileError`). Printed by `main` as
/// `flexwm: <this message>`.
///
/// Pure, and a function rather than inline `match` arms, because that is what
/// makes it testable: the variant that matters is
/// [`BindError::RuntimeDirNotSet`], and provoking it for real means unsetting
/// a process-global environment variable that every other test in this binary
/// needs (`State::new` binds a real socket) -- the same hazard item 9's umask
/// attempt hit and abandoned. The end-to-end behavior is verified against a
/// real release binary instead; see
/// `docs/roadmap/12-four-bounds.md` for that verification.
fn socket_error(error: BindError) -> String {
    match error {
        BindError::RuntimeDirNotSet => {
            "no wayland socket: $XDG_RUNTIME_DIR is not set or invalid; \
             set it to a writable directory (a login session normally provides one)"
                .to_string()
        }
        BindError::PermissionDenied => {
            "no wayland socket: $XDG_RUNTIME_DIR is not writable".to_string()
        }
        // `ListeningSocketSource::new_auto` tries `wayland-1` through
        // `wayland-32` (1..33, verified in the pinned rev's
        // `wayland/socket.rs`), so reaching this means all 32 are taken.
        BindError::AlreadyInUse => {
            "no wayland socket: wayland-1 through wayland-32 are all in use \
             in $XDG_RUNTIME_DIR; stop another compositor, or remove a stale socket and its .lock"
                .to_string()
        }
        BindError::Io(source) => format!("no wayland socket: {source}"),
    }
}

/// Per-client data Smithay hands back on every request.
#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _id: ClientId) {}
    fn disconnected(&self, _id: ClientId, _reason: DisconnectReason) {}
}
