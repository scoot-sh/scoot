//! The X11 focus gate: when an X window may take focus by itself.
//!
//! X11 has no activation serials. An `xdg_activation_v1` token names the
//! input event that earned it and is checked against what the user really
//! did (`activation.rs`); an X client asking for focus -- by mapping a
//! window, or by sending `_NET_ACTIVE_WINDOW` -- names nothing anyone can
//! check. Honouring either unconditionally would let any X client take the
//! keyboard from whatever the user is typing into, including a Wayland
//! window: the hole `activation.rs` closed, in X form. So both go through
//! one gate, and a window that does not pass it is announced (it is in the
//! layout, every window list and `scoot msg windows`) but not focused; a
//! click, a keybinding, a taskbar's `activate` or an IPC focus action moves
//! focus to it like to any other window.
//!
//! # The gate
//!
//! An X window may take focus by itself when:
//!
//! 1. **nothing is focused** -- no window holds focus to steal; or
//! 2. **scoot spawned it, and the spawn's token is still unspent** -- the
//!    chain from a user's own keybinding (or an agent's IPC `spawn`) to this
//!    window. Checked two ways, both against the activation token table, so
//!    every token rule there still binds (the 30-second lifetime, the shared
//!    cap, single use):
//!    - the window's `_NET_STARTUP_ID` names a live token. `State::spawn`
//!      hands a child its token as `DESKTOP_STARTUP_ID` too while XWayland
//!      is live -- the variable X toolkits (GTK, Qt) read for exactly this
//!      -- so a toolkit client redeems the same token a Wayland child would.
//!      A token a Wayland launcher minted from a real click works the same
//!      way: it passed `activation.rs`'s serial gate when it was created.
//!    - the X client's process -- read through the X-Resource extension
//!      (`XResQueryClientIds`), which the X server answers from the socket's
//!      credentials, never from the forgeable `_NET_WM_PID` -- is a child
//!      scoot spawned and has not reaped (so the pid cannot have been
//!      reused), and a token minted for that spawn is live. This is what
//!      covers the clients that do no startup notification at all (`xterm`).
//!
//! 3. **the focused window is an X window of the same client process** --
//!    again by X-Resource pid -- which is an application opening its own
//!    dialog, or moving focus between its own windows. Not a steal: that
//!    process already holds the keyboard, and inside the X server one X
//!    client can move another's focus anyway. Without this an X app's file
//!    chooser opened unfocused under its own window (measured: GTK 3's
//!    `mousepad` Ctrl+O, token already spent -- GTK sends no
//!    `_NET_ACTIVE_WINDOW` for a new dialog). Only the pid counts, never
//!    `WM_TRANSIENT_FOR`: that is client-set, and any background X client
//!    could name the focused window as its parent.
//!
//! A redeemed token is removed, so one spawn focuses one window: a second
//! window the same process maps later takes focus only through rule 3 (its
//! own window is focused) or rule 1.
//!
//! `_NET_ACTIVE_WINDOW` passes the same gate. The message's own "currently
//! active window" field is ignored: it is part of the request, so it proves
//! nothing about who sent it. And while the session is locked the request is
//! refused outright, like every activation.
//!
//! # What the gate cannot do
//!
//! It guards the *Wayland* keyboard focus -- which window scoot hands the
//! keys to. Inside the X server, X11 has no such boundary: any X client can
//! `XSetInputFocus` another X client's window, read its keystrokes while one
//! of them is focused, and synthesize input into it. That is X11 by design,
//! and why running XWayland extends full trust to every X client (see
//! `docs/protocols.md`). What the gate stops is an X client taking focus
//! from a Wayland window, or from a different X application, by asking --
//! with one known window, below.
//!
//! # The startup-id race (known, filed)
//!
//! A startup id is a property on the launched app's window, so any X client
//! can read it: one watching the root for new windows can copy a freshly
//! launched app's `_NET_STARTUP_ID` onto a window of its own and map first,
//! redeeming the token and taking focus once, for as long as that token is
//! live (up to 30 seconds after the launch, until the app's own window
//! spends it). The redemption does not check that the redeeming window's
//! process is the one the token was minted for. The tightening -- when the
//! token carries a [`SpawnedPid`], accept a startup-id redemption only from
//! that process or a descendant (a bounded parent walk, so wrapper scripts
//! still work) -- is `docs/backlog/protocols/xwayland-startup-id-race.md`.
//! (Any same-uid process can also read a child's token out of
//! `/proc/<pid>/environ`; that is the project's same-uid trust boundary, and
//! applies to every activation token, X or not.)

use smithay::wayland::xdg_activation::XdgActivationToken;
use smithay::xwayland::X11Surface;

use super::super::State;
use super::super::activation::TOKEN_LIFETIME;
use scoot_core::Action;

/// A token minted for a spawned child records the child's pid here (see
/// `State::spawn`), so the X-Resource half of the gate can find the token a
/// given X client's process was started with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::compositor) struct SpawnedPid(pub u32);

/// An X window's client pid, read once per window through X-Resource and
/// kept on the surface: the query is a synchronous round trip, and a
/// client's process never changes.
#[derive(Clone, Copy, Debug)]
struct ClientPid(Option<u32>);

impl State {
    /// Whether a newly mapped X window takes focus (the module doc's gate).
    /// Redeems the token that lets it, when one does.
    pub(super) fn x11_focus_on_map(&mut self, window: &X11Surface) -> bool {
        self.focus.is_none()
            || self.same_client_as_focused(window)
            || self.redeem_x11_spawn_token(window)
    }

    /// `_NET_ACTIVE_WINDOW` for `window`: honoured only through the gate.
    /// The cheap refusals run first -- the lock, an unmanaged or already
    /// focused window -- so a client spamming the request costs no X round
    /// trip until one of the pid checks is actually needed, and at most one
    /// per window ever after (the pid is cached).
    pub(super) fn x11_activation_request(&mut self, window: &X11Surface) {
        let xid = window.window_id();
        if self.session_lock.is_locked() {
            tracing::debug!(xid, "refusing _NET_ACTIVE_WINDOW: the session is locked");
            return;
        }
        let Some(id) = self.id_of_x11(window) else {
            tracing::debug!(
                xid,
                "refusing _NET_ACTIVE_WINDOW for a window scoot does not manage"
            );
            return;
        };
        if self.focus == Some(id) {
            return;
        }
        let allowed = self.focus.is_none()
            || self.same_client_as_focused(window)
            || self.redeem_x11_spawn_token(window);
        if !allowed {
            tracing::debug!(
                ?id,
                xid,
                "refusing _NET_ACTIVE_WINDOW: another application holds focus and this window has no spawn token"
            );
            return;
        }
        tracing::debug!(?id, xid, "honouring _NET_ACTIVE_WINDOW");
        // As `activation.rs` does before its own `act`: a click that
        // focused an `on_demand` layer surface is spent once focus moves
        // on, or the refresh would hand the keyboard straight back to it.
        self.clicked_layer = None;
        self.act(Action::FocusWindowId(id));
    }

    /// Whether the focused window is an X window of the same client process
    /// as `window`.
    fn same_client_as_focused(&self, window: &X11Surface) -> bool {
        let Some(focused) = self
            .focus
            .and_then(|id| self.windows.get(&id))
            .and_then(|focused| focused.x11_surface())
        else {
            return false;
        };
        match (client_pid(focused), client_pid(window)) {
            (Some(focused), Some(asking)) => focused == asking,
            _ => false,
        }
    }

    /// Finds and spends the spawn token that chains `window` to a spawn of
    /// scoot's own (the module doc's rule 2). `false` when none does.
    fn redeem_x11_spawn_token(&mut self, window: &X11Surface) -> bool {
        let fresh = |data: &smithay::wayland::xdg_activation::XdgActivationTokenData| {
            data.timestamp.elapsed() < TOKEN_LIFETIME
        };
        if let Some(startup) = window.startup_id() {
            let token = XdgActivationToken::from(startup);
            if self
                .xdg_activation
                .data_for_token(&token)
                .is_some_and(fresh)
            {
                self.xdg_activation.remove_token(&token);
                tracing::debug!(
                    xid = window.window_id(),
                    "X11 window redeemed its startup id"
                );
                return true;
            }
        }
        let Some(pid) = client_pid(window) else {
            return false;
        };
        if !self.spawned_children.contains(&pid) {
            return false;
        }
        let token = self
            .xdg_activation
            .tokens()
            .find(|(_, data)| {
                fresh(data) && data.user_data.get::<SpawnedPid>() == Some(&SpawnedPid(pid))
            })
            .map(|(token, _)| token.clone());
        let Some(token) = token else {
            return false;
        };
        self.xdg_activation.remove_token(&token);
        tracing::debug!(
            xid = window.window_id(),
            pid,
            "X11 window redeemed its spawn's token by process"
        );
        true
    }
}

/// `window`'s X client pid through X-Resource, cached on the surface.
/// `None` when the server cannot say: Smithay reports a failed query as pid
/// `0`, which is no process, so it counts as unknown -- and unknown never
/// matches anything.
fn client_pid(window: &X11Surface) -> Option<u32> {
    let data = window.user_data();
    if let Some(cached) = data.get::<ClientPid>() {
        return cached.0;
    }
    let pid = window.get_client_pid().ok().filter(|&pid| pid != 0);
    data.insert_if_missing_threadsafe(|| ClientPid(pid));
    pid
}
