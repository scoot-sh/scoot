//! What the compositor owns: Smithay's protocol state, the Wayland windows, and
//! the [`World`] that decides where they go.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Instant;

use flexwm_core::{Config, Size, WindowId, World};
use smithay::desktop::{PopupManager, Space, Window, WindowSurfaceType};
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
use smithay::wayland::output::OutputManagerState;
use smithay::wayland::selection::data_device::DataDeviceState;
use smithay::wayland::shell::xdg::XdgShellState;
use smithay::wayland::shell::xdg::decoration::XdgDecorationState;
use smithay::wayland::shm::ShmState;
use smithay::wayland::socket::ListeningSocketSource;

use super::cursor::Cursor;
use super::decorations::{Appearance, Decorations};
use super::headless::Backend;
use super::ipc::PendingIdle;
use super::keybindings::Keybindings;
use super::nested::Host;
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
    /// What holds keyboard focus, so focus is only moved when it changes.
    pub focus: Option<WindowId>,

    pub space: Space<Window>,
    pub popups: PopupManager,
    pub output: Option<Output>,
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
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let dh = display.handle();
        let compositor_state = CompositorState::new::<Self>(&dh);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        let xdg_decoration_state = XdgDecorationState::new::<Self>(&dh);
        let shm_state = ShmState::new::<Self>(&dh, vec![]);
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);
        let data_device_state = DataDeviceState::new::<Self>(&dh);

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
            space: Space::default(),
            popups: PopupManager::default(),
            output: None,
            backend: None,
            host: None,
            tty: None,
            appearance,
            decorations: Decorations::default(),
            cursor,
            compositor_state,
            xdg_shell_state,
            xdg_decoration_state,
            shm_state,
            output_manager_state,
            seat_state,
            data_device_state,
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

    pub fn surface_under(
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
/// real release binary instead; see this item's entry in `ROADMAP.md`.
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
