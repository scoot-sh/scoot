//! Opt-in XWayland: an X11 server inside the session, and the window
//! management that puts its windows in the layout.
//!
//! When the session asks for it (`--xwayland` or `[xwayland] enabled`, either
//! one -- see [`resolve`]), this starts Smithay's `XWayland` around the
//! `Xwayland` binary, sequences its `READY` into `X11Wm::start_wm`, and
//! exports the resulting display number as `DISPLAY` to everything the
//! session spawns. When the session does not ask, or the binary is absent,
//! nothing here runs and the session is Wayland-only.
//!
//! ## What is here, by phase
//!
//! - **Phase 1, the server** -- this file: start, `READY`, `DISPLAY`, and
//!   the loud Wayland-only fallback.
//! - **Phase 2, mapping** -- `manage.rs`: a normal X window that asks to be
//!   mapped enters the core as a column (or floats, by the same signals and
//!   `[[window_rule]]`s an xdg window floats by), appears in `scoot msg
//!   windows` and both foreign-toplevel lists, is drawn with scoot's ring
//!   and rounded clip, and goes fullscreen through
//!   `_NET_WM_STATE_FULLSCREEN`. `unmanaged.rs`: override-redirect windows
//!   (menus, tooltips) are drawn where they put themselves, above every
//!   window, and never enter the core.
//! - **Phase 3, the focus gate** -- `focus.rs`: an X window takes focus by
//!   itself only when nothing is focused, the focused window is the same X
//!   process, or scoot spawned it (the spawn's token, redeemed once);
//!   `_NET_ACTIVE_WINDOW` is a request through the
//!   same gate. The keyboard reaches an X window through its `X11Surface`,
//!   which moves the X server's own input focus (see `keyboard_focus.rs`
//!   for why the `wl_surface` alone does not).
//! - **Phase 4 onward** (clipboard, drag-and-drop, XIM, packaging) is
//!   not here yet: see `docs/backlog/protocols/xwayland-support.md`.
//!
//! `wm.rs` holds the Smithay handler impls, each a dispatch into the
//! module that owns its policy.
//!
//! ## Why this shape
//!
//! - **Opt-in, default off.** The server is a whole extra process (~55 MB
//!   RSS measured on the dev VM -- re-measured per change, see the ticket),
//!   a hard `PATH` dependency on the `Xwayland` binary, and a trust-model
//!   change: any X client can keylog and snoop by design, so running one
//!   extends full trust to it (see `docs/protocols.md`'s trust note).
//!   Precedent is the `gpu-scanout` Cargo feature: own feature
//!   (`xwayland = ["smithay/xwayland"]`), off by default, `ldd`-gated both
//!   flavours (re-proven per change; the feature pulls pure-Rust `x11rb` --
//!   the X protocol over a socket -- so the pair is expected to show *no*
//!   new link-time library on either side).
//! - **The knob parses in every build.** `--xwayland` and `[xwayland]
//!   enabled` exist with and without the Cargo feature, the way `--renderer
//!   gles` parses without `gpu-scanout`: a build without the feature warns
//!   once and runs Wayland-only rather than refusing a flag its `--help`
//!   advertises. Only the Smithay types (`XWayland`, `X11Wm`, the shell and
//!   grab states, the handler impls) sit behind the feature gate.
//! - **`DISPLAY` is set only while our server is believed live.**
//!   `State::xdisplay` is `Some` from the synchronous display-number lock at
//!   spawn until a pre-`READY` death or a failed window-manager attach
//!   clears it -- and `State::spawn` plus `compositor::run`'s process export
//!   both read exactly that. When it is `None` (never enabled, spawn failed,
//!   server died, or no WM to manage it) both leave `DISPLAY`
//!   untouched, so a host-provided `DISPLAY` under `--nested` survives: no
//!   clobber. The number itself is never hard-coded -- the lock scan picks a
//!   free one, and `READY` carries the same number back -- so a rapid
//!   restart (or a stale `/tmp/.X11-unix` lock from a killed server) just
//!   lands on a fresh number.
//! - **Abstract socket, like anvil.** `open_abstract_socket = true`: the X
//!   socket lives in the kernel, bound by the Xwayland child, so session
//!   end (or a server kill) reclaims it with the process -- no stale-socket
//!   wedge on restart beyond the lock-file scan above, which resolves
//!   itself.
//! - **The spike's two integration hazards are structural here, not
//!   call-site discipline.** (1) The probe hung because its loop dispatched
//!   without flushing; in-tree `compositor::run`'s `post_dispatch` flushes
//!   every client after every dispatch cycle, which covers the XWayland
//!   event source with no per-site code. (2) X windows commit `wl_surface`s
//!   through XWayland's own Wayland client, which carries Smithay's
//!   `XWaylandClientData`, not scoot's `ClientState` --
//!   `CompositorHandler::client_compositor_state` (see `handlers.rs`) serves
//!   it first, the anvil pattern, or the first X commit panics the
//!   compositor.
//! - **A dead server is loud, never fatal.** Absent binary (or an
//!   unstartable one) is a synchronous `Err` from `XWayland::spawn`, which
//!   `start` returns as [`StartError::Spawn`] and `run` answers with a loud
//!   log plus a Wayland-only session -- never a crash, never a hang. A
//!   mid-session death arrives as `XWaylandEvent::Error` (pre-`READY`) or
//!   the XWM `disconnected` callback (post-`READY`, see `handlers.rs`);
//!   a `READY`-then-WM-attach-failure clears `xdisplay` the same way;
//!   all three log loudly and the session survives. What none of the three
//!   clears is the
//!   already-exported `DISPLAY`: children spawned while the server lived
//!   keep pointing at a dead `:N`, and so do later children through process
//!   inheritance. That staleness fails loudly at the X client (connection
//!   refused), never silently wrong -- and retracting it would mean
//!   `env_remove`ing a host `DISPLAY` the session does not own -- but it is
//!   a known Phase-1 edge: restart the session. A `--tty` login without the
//!   binary on `PATH` is the same shape (loud fallback), which is why the
//!   packaging phase must put the binary on the session `PATH`.
//! - **The session lock blanks X windows like every other window, and
//!   refuses them input.** The X server keeps running under lock (it must
//!   -- killing it would take every X client with it, and lock is not
//!   logout). Its windows are windows: a locked frame gathers the lock
//!   screen and nothing else, the pointer finds only lock surfaces, the
//!   keyboard goes only to the lock surface (and the X focus is released
//!   when it leaves an X window, which is what closes a toolkit's open
//!   menu), and override-redirect windows are skipped by the same branches.
//! - **Spawned children get `DISPLAY`, and their activation token doubles
//!   as `DESKTOP_STARTUP_ID`.** `State::spawn` exports `DISPLAY` to every
//!   child while the server is live, and -- only then -- hands the child's
//!   activation token over in `DESKTOP_STARTUP_ID` as well, the variable X
//!   toolkits turn into `_NET_STARTUP_ID`, and records the child's pid on
//!   the token. That is the chain `focus.rs` redeems. (Phase 1 said the
//!   opposite -- "must not mint" -- when no X launch path existed to redeem a
//!   token through; the startup id and the X-Resource pid are that path.)
//!
//! [`resolve`]: resolve()
//! [`StartError::Spawn`]: StartError::Spawn

#[cfg(feature = "xwayland")]
use std::fmt;

#[cfg(feature = "xwayland")]
use smithay::reexports::calloop::LoopHandle;
#[cfg(feature = "xwayland")]
use smithay::reexports::wayland_server::DisplayHandle;
#[cfg(feature = "xwayland")]
use smithay::xwayland::{XWayland, XWaylandEvent};

#[cfg(feature = "xwayland")]
use super::State;

#[cfg(feature = "xwayland")]
mod focus;
#[cfg(feature = "xwayland")]
pub(in crate::compositor) mod manage;
#[cfg(test)]
mod tests;
#[cfg(feature = "xwayland")]
pub(in crate::compositor) mod unmanaged;
#[cfg(feature = "xwayland")]
mod wm;

#[cfg(feature = "xwayland")]
pub(in crate::compositor) use focus::SpawnedPid;

/// The Smithay XWayland types the rest of the compositor names, re-exported
/// so `state.rs` and `handlers.rs` have one seam to read them through rather
/// than three separate `smithay::` paths to keep in step.
#[cfg(feature = "xwayland")]
pub use smithay::wayland::xwayland_keyboard_grab::XWaylandKeyboardGrabState;
#[cfg(feature = "xwayland")]
pub use smithay::wayland::xwayland_shell::XWaylandShellState;
#[cfg(feature = "xwayland")]
pub use smithay::xwayland::X11Wm;

/// The environment variable X11 clients read for their display.
pub const DISPLAY_ENV: &str = "DISPLAY";

/// Whether this session runs an XWayland server: the `--xwayland` flag
/// OR-ed with `[xwayland] enabled`. A flag can only say yes, so OR is the
/// only resolution that lets either one turn it on; there is no way to say
/// no from either side, and none is needed -- off is the default both
/// sides agree on.
///
/// Pure, so the opt-in default is unit-testable without starting a server.
pub fn resolve(flag: bool, configured: bool) -> bool {
    flag || configured
}

/// The `DISPLAY` value for `display`: `:N`, the local form X clients
/// expect (the abstract socket `open_abstract_socket` opens makes it
/// reachable without a filesystem path).
pub fn display_value(display: u32) -> String {
    format!(":{display}")
}

/// What `start` can report. Only [`StartError::Spawn`] is a designed
/// fallback (loud log, Wayland-only session); an event-loop registration
/// failure means the loop itself is broken, so `run` treats it as a hard
/// startup error.
#[cfg(feature = "xwayland")]
#[derive(Debug)]
pub enum StartError {
    /// The `Xwayland` binary would not start -- absent from `PATH`, not
    /// executable, or failing its own exec. The session continues
    /// Wayland-only.
    Spawn(std::io::Error),
    /// The server started but watching it for readiness failed. Near
    /// unreachable (registration fails only when the loop is already
    /// broken); kept distinct so `run` does not mistake it for the
    /// fallback above.
    Insert(String),
}

#[cfg(feature = "xwayland")]
impl fmt::Display for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(error) => write!(
                f,
                "could not start the Xwayland server (`Xwayland` must be on PATH and executable): {error}"
            ),
            Self::Insert(error) => {
                write!(
                    f,
                    "could not watch the Xwayland server for readiness: {error}"
                )
            }
        }
    }
}

#[cfg(feature = "xwayland")]
impl std::error::Error for StartError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Spawn(error) => Some(error),
            Self::Insert(_) => None,
        }
    }
}

/// Starts the XWayland server for an opted-in session and watches it for
/// readiness. On success returns the display number -- already stored in
/// `state.xdisplay`, so `State::spawn` and `run`'s process export gate on
/// it from the first child -- and `READY` later attaches the window
/// manager (see the callback below) and logs loudly.
///
/// `loop_handle` is taken by value (it is a cheap clone; `State` already
/// holds one) because the `READY` callback needs its own copy for
/// `X11Wm::start_wm`, which outlives this call.
#[cfg(feature = "xwayland")]
pub fn start(
    loop_handle: LoopHandle<'static, State>,
    state: &mut State,
) -> Result<u32, StartError> {
    use std::process::Stdio;

    let display_handle: DisplayHandle = state.display_handle.clone();
    let (xwayland, client) = XWayland::spawn(
        &display_handle,
        None::<u32>,
        std::iter::empty::<(String, String)>(),
        std::iter::empty::<String>(),
        true,
        Stdio::null(),
        Stdio::null(),
        |_| (),
    )
    .map_err(StartError::Spawn)?;
    // Synchronous, from the just-acquired display lock -- the same number
    // `READY` will carry back. Read before the move into the event source
    // below; stored so the very first spawned child already gates on it.
    let display = xwayland.display_number();
    state.xdisplay = Some(display);
    // Alongside the spawn, not at `READY`: with no XWM there are no X
    // surfaces to grab for yet, but a 70ms registry gap would move the
    // global's appearance past a client that lists globals once at
    // connect. Its `can_view` gate (XWayland clients only -- see the
    // dispatch in Smithay's `xwayland_keyboard_grab.rs`) keeps it out of
    // every regular client's registry either way, so a failed or
    // never-asked session stays byte-identical by never reaching this
    // line at all.
    state.xwayland_grab = Some(
        smithay::wayland::xwayland_keyboard_grab::XWaylandKeyboardGrabState::new::<State>(
            &state.display_handle,
        ),
    );

    let wm_handle = loop_handle.clone();
    let wm_display = display_handle.clone();
    loop_handle
        .insert_source(xwayland, move |event, _, state: &mut State| match event {
            XWaylandEvent::Ready {
                x11_socket,
                display_number,
            } => attach_window_manager(
                state,
                wm_handle.clone(),
                &wm_display,
                x11_socket,
                client.clone(),
                display_number,
            ),
            XWaylandEvent::Error => {
                // The server died before `READY`: retract the number so
                // later spawns inherit (a host `DISPLAY`, if any) rather
                // than point at a dead `:N`. Already-spawned children
                // and the process environment keep the stale value --
                // the documented edge in this module's doc.
                state.xdisplay = None;
                tracing::warn!("XWayland exited before it was ready; continuing Wayland-only");
            }
        })
        .map_err(|error| StartError::Insert(format!("{error:?}")))?;
    Ok(display)
}

/// `READY`'s half of [`start`]: attaches the window manager to the server
/// that just became ready, or -- when it cannot attach -- withdraws the
/// display number and leaves the session Wayland-only.
///
/// A named function rather than the callback's body so the failure arm can
/// be driven deterministically: a test holds `READY` back, lets a rival
/// window manager claim the display, then calls this (see the
/// `a_rival_window_manager_withdraws_the_display` test). Driving it through
/// `start` cannot be made deterministic: XWayland only finishes starting --
/// and only then answers any X client, rival included -- once the
/// compositor has dispatched its Wayland setup, and the dispatch that
/// completes it can deliver `READY` and attach before the rival claims
/// anything.
#[cfg(feature = "xwayland")]
fn attach_window_manager(
    state: &mut State,
    loop_handle: LoopHandle<'static, State>,
    display_handle: &DisplayHandle,
    x11_socket: std::os::unix::net::UnixStream,
    client: smithay::reexports::wayland_server::Client,
    display_number: u32,
) {
    match X11Wm::start_wm(loop_handle, display_handle, x11_socket, client) {
        Ok(wm) => {
            state.xwm = Some(wm);
            tracing::info!(
                display = display_number,
                "XWayland is ready; X11 clients can connect"
            );
        }
        Err(error) => {
            // No window manager means no reparenting, no surface
            // association, no map requests -- X clients could still
            // connect, but to a bare server this session cannot manage.
            // Withdraw the number (the `xdisplay` invariant is "set only
            // while our server is believed live", and a WM-less server is
            // not live for our purposes -- same as the pre-READY death in
            // `start`), so no explicit `DISPLAY` is handed out anymore. The
            // process-wide staleness is unchanged (see the module doc):
            // later spawns still inherit the exported `:N`. The grab
            // manager stays (created at spawn, `can_view`-gated, harmless
            // without X surfaces), and the server process itself is
            // untouched (reaped with the session through
            // `XWaylandClientData`).
            state.xdisplay = None;
            tracing::error!(
                %error,
                "XWayland is up but its window manager could not attach; withdrawing DISPLAY and continuing Wayland-only"
            );
        }
    }
}
