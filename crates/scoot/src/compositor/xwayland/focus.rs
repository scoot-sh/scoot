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
//! 2. **it redeems its launch's activation token** -- the chain from a
//!    user's own keybinding (or an agent's IPC `spawn`) to this window.
//!    Checked two ways, both against the activation token table, so every
//!    token rule there still binds (the 30-second lifetime, the shared cap,
//!    single use):
//!    - the window's `_NET_STARTUP_ID` -- or, when the window has none, its
//!      client leader's (the `WM_HINTS` window group, when that leader
//!      belongs to the same X client: GTK sets the startup id on that
//!      unmapped leader window, never on the toplevel it maps, so reading
//!      the toplevel alone redeemed nothing, measured by review; Qt is
//!      believed to do the same, unverified) -- names a live token *bound
//!      to this window's process* (below). `State::spawn` hands a child its
//!      token as `DESKTOP_STARTUP_ID` too while XWayland is live, the
//!      variable X toolkits read for exactly this.
//!
//!      **The binding.** A startup id is a property every X client can
//!      read, and a toolkit sets it on its client leader at startup, well
//!      before its first window maps -- so the id alone would let a
//!      background X client copy it onto a window of its own, map first,
//!      and take focus with the launch's token. What the id may redeem
//!      therefore depends on who the token was minted for:
//!      - a **spawn token** (it carries the [`SpawnedPid`] `State::spawn`
//!        records while XWayland is live), **while the spawn runs** (tracked
//!        and unreaped, so its pid cannot have been reused), redeems only
//!        for a window whose X client process -- by X-Resource pid, below
//!        -- is that spawn or descends from it within [`MAX_ANCESTRY_DEPTH`](super::ancestry::MAX_ANCESTRY_DEPTH) parent links of
//!        `/proc/<pid>/stat` (see `ancestry.rs`). So an app behind a
//!        wrapper (`sh -c`, measured) or a launcher shim still redeems it;
//!        `flatpak run` should too, if its `bwrap` chain fits the bound
//!        (unverified). An unknown pid -- a failed or refused X-Resource
//!        query -- is refused: the binding fails closed. A refused window
//!        leaves the token live for the process it belongs to.
//!      - a spawn token **whose spawn has exited** without redeeming it
//!        keeps the unbound rule: any window that asks (maps, or sends
//!        `_NET_ACTIVE_WINDOW`) naming it redeems it, once, within its
//!        lifetime. Two ordinary launch shapes end there: a single-instance
//!        app whose second launch forwards to the running instance and
//!        exits (GApplication, `KDBusService`), and an app that forks into
//!        the background and lets the spawn exit (`gvim` without `-f`).
//!        Either way the window comes from a process the spawn is not an
//!        ancestor of, and binding it would open that window unfocused
//!        behind whatever the user was typing into.
//!
//!        A forwarder was measured to exit a few milliseconds *after* the
//!        running instance's window maps, so the spawn's reap re-asks
//!        ([`State::x11_focus_for_exited_spawns`]) -- narrowly: only for a
//!        window that asked naming the token *while that spawn ran* and was
//!        refused for it ([`RefusedSpawn`]), within [`REFUSAL_GRACE`] before
//!        the reap, with focus unmoved since; of several, the first to ask.
//!        A window that merely carries the id and never asked, or asked for
//!        another spawn, gets nothing from an exit, and no other child's
//!        exit re-asks for anything.
//!
//!        What stays open for these launches: an X client that copies the
//!        id and *asks* with it before the app's window does -- while the
//!        spawn runs (both refused; the first asker wins at the exit) or
//!        after it has exited -- takes that launch's focus, once, within
//!        the token's 30 s, as it could before the binding. Closed: a
//!        client that only sets the property, and every launch whose
//!        process is still running when its window maps. And one launch
//!        shape is now refused where the unbound rule would have granted
//!        it: a window whose spawn exits more than [`REFUSAL_GRACE`] after
//!        the window asked, or after focus moved, stays unfocused, and the
//!        token is spent at that exit so no later asker can take it.
//!      - a **Wayland client's token** (a launcher's, minted from a real
//!        click: it passed `activation.rs`'s serial gate) keeps the unbound
//!        rule -- any window naming it redeems it. scoot never learns which
//!        process the launcher started (a launcher typically exits right
//!        after, reparenting the app away from it, so no process tree ties
//!        them), and GLib hands its launch token over as
//!        `DESKTOP_STARTUP_ID` too, so refusing these would open every X
//!        app a GTK launcher or file manager starts behind the window the
//!        user launched it from. The race stays open for these tokens:
//!        a watching X client can win one launch's focus, once, within the
//!        token's 30 s.
//!      - a token with neither -- one scoot minted for a spawn while
//!        XWayland was not live, which was never handed to any X toolkit
//!        as a startup id -- redeems nothing for an X window: an X window
//!        naming it copied it from somewhere.
//!    - the X client's process -- read through the X-Resource extension
//!      (`XResQueryClientIds`), which the X server answers from the socket's
//!      credentials, never from the forgeable `_NET_WM_PID` -- is a child
//!      scoot spawned and has not reaped (so the pid cannot have been
//!      reused), and a token minted for that spawn is live. This covers
//!      clients that do no startup notification at all (`xterm`).
//! 3. **the focused window is an X window of the same client process** --
//!    again by X-Resource pid -- which is an application opening its own
//!    window or dialog, or moving focus between its own windows. Not a
//!    steal: that process already holds the keyboard, and inside the X
//!    server one X client can move another's focus anyway. Without this an
//!    X app's file chooser opened unfocused under its own window (measured:
//!    GTK 3's `mousepad` Ctrl+O -- GTK sends no `_NET_ACTIVE_WINDOW` for a
//!    new dialog). Only the pid counts, never `WM_TRANSIENT_FOR`: that is
//!    client-set, and any background X client could name the focused
//!    window as its parent.
//!
//! **A live token a mapping window may redeem is spent whichever rule
//! grants it focus**, rule 1 or 3 included. (One it may not -- a spawn
//! token its process is not bound to -- is left alone: spending it would
//! hand a racer the power to take the launched app's focus away without
//! taking it for itself.) Rule 1 used to short-circuit past it
//! and leave the token live for 30 s -- and a startup id is a readable
//! property, so any X client could copy it off the launched app's window
//! and redeem it later with `_NET_ACTIVE_WINDOW` to take focus from a
//! Wayland window (found live by review: zenity focused by rule 1, then
//! `xeyes` copied the token and took focus from `foot`). One spawn focuses
//! at most one window through its token; later windows of that process
//! take focus through rule 3 while it is focused.
//!
//! `_NET_ACTIVE_WINDOW` passes the same gate, and spends a token the same
//! way. The message's own "currently active window" field is ignored: it
//! is part of the request, so it proves nothing about who sent it. And
//! while the session is locked the request is refused outright, like every
//! activation.
//!
//! # What the gate cannot do
//!
//! It guards the *Wayland* keyboard focus -- which window scoot hands the
//! keys to. Inside the X server, X11 has no such boundary: any X client can
//! `XSetInputFocus` another X client's window, read its keystrokes while one
//! of them is focused, and synthesize input into it. That is X11 by design,
//! and why running XWayland extends full trust to every X client (see
//! `site/src/content/docs/scoot/protocols.md`). What the gate stops is an X client taking focus
//! from a Wayland window, or from a different X application, by asking --
//! except with a Wayland launcher's token, or the token of a spawn that
//! exited before its app redeemed it, which it can race the launched app to
//! by asking first (rule 2's binding says why those stay unbound).
//!
//! Nor does it defend against the same user's own processes: any same-uid
//! process can read a spawned child's token out of `/proc/<pid>/environ`,
//! or run an X client under the spawn's own process tree. That is the
//! project's same-uid trust boundary, and applies to every activation
//! token, X or not.

use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use scoot_core::{Action, WindowId};
use smithay::wayland::xdg_activation::{XdgActivationToken, XdgActivationTokenData};
use smithay::xwayland::X11Surface;

use super::super::State;
use super::super::activation::TOKEN_LIFETIME;
use super::ancestry::descends_from;

/// A token minted for a spawned child records the child's pid here (see
/// `State::spawn`), so the X-Resource half of the gate can find the token a
/// given X client's process was started with, and a startup id naming the
/// token redeems only for that process or its descendants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::compositor) struct SpawnedPid(pub u32);

/// An X window's client pid, read once per window through X-Resource and
/// kept on the surface: the query is a synchronous round trip, and a
/// client's process never changes.
#[derive(Clone, Copy, Debug)]
struct ClientPid(Option<u32>);

/// The spawn an X window's process was first found not to descend from, when
/// the window asked for focus with its token (on map, or by
/// `_NET_ACTIVE_WINDOW`), kept on the surface. Two readers:
///
/// - the binding's cache: a client spamming `_NET_ACTIVE_WINDOW` with a
///   copied startup id costs one `/proc` walk, not one per request. Only a
///   refusal is kept: a process never gains an ancestor (an orphan is
///   reparented to init or a subreaper already above it), and a spawn that
///   is reaped fails the tracking check first -- so a cached "no" stays true
///   for as long as it is consulted. (A subreaper spawn can shorten an
///   orphaned descendant's chain back under the depth bound; the cache then
///   keeps refusing, which is the closed side to fail on.)
/// - the re-ask on the spawn's exit ([`State::x11_focus_for_exited_spawns`]):
///   only a window that asked while the spawn ran is owed anything when it
///   exits, and only if it asked moments before ([`REFUSAL_GRACE`]) and
///   focus has not moved since (`State::focus_generation`).
///
/// A later refusal against the same spawn leaves the record alone (the
/// cache answers first), so `at` is when the window first asked.
#[derive(Debug)]
struct RefusedSpawn(Mutex<Refusal>);

#[derive(Clone, Copy, Debug)]
struct Refusal {
    /// The spawn the window's process does not descend from.
    pid: u32,
    /// When the window was first refused against it.
    at: Instant,
    /// `State::focus_generation` at that moment.
    focus_generation: u64,
}

impl RefusedSpawn {
    /// The record, if any. The lock is never held across anything else, so
    /// it cannot deadlock; a poisoned one still holds a whole `Copy` value.
    fn get(window: &X11Surface) -> Option<Refusal> {
        let refused = window.user_data().get::<RefusedSpawn>()?;
        Some(*refused.0.lock().unwrap_or_else(PoisonError::into_inner))
    }

    fn set(window: &X11Surface, refusal: Refusal) {
        let refused = window
            .user_data()
            .get_or_insert_threadsafe(|| RefusedSpawn(Mutex::new(refusal)));
        *refused.0.lock().unwrap_or_else(PoisonError::into_inner) = refusal;
    }
}

/// How recently a window must have been refused for a spawn's exit to grant
/// it focus. The forwarder this exists for (GTK `mousepad`'s second launch)
/// was measured reaped 8 ms after the running instance's window mapped; a
/// second leaves two orders of magnitude for a loaded machine's scheduling,
/// and is short enough that an ask cannot be cashed in long after the user
/// has moved on -- which `State::focus_generation` covers within it.
pub(in crate::compositor) const REFUSAL_GRACE: Duration = Duration::from_secs(1);

impl State {
    /// Whether a newly mapped X window takes focus (the module doc's gate).
    ///
    /// The token is redeemed first and unconditionally -- not after the
    /// other two rules, which would short-circuit past it and leave a
    /// copyable token live (see the module doc).
    pub(super) fn x11_focus_on_map(&mut self, window: &X11Surface) -> bool {
        let redeemed = self.redeem_x11_spawn_token(window);
        redeemed || self.focus.is_none() || self.same_client_as_focused(window)
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
        // Redeemed first, whichever rule grants it, as on map.
        let redeemed = self.redeem_x11_spawn_token(window);
        let allowed = redeemed || self.focus.is_none() || self.same_client_as_focused(window);
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

    /// The startup id on `window`'s client leader -- its `WM_HINTS` window
    /// group, where GTK sets it -- when that leader is a window the
    /// XWM has seen carry one. One map lookup.
    ///
    /// The leader must belong to the same X client as `window`: a window's
    /// `WM_HINTS` is client-set, so without this any X client could name
    /// another application's leader as its own group and borrow its startup
    /// id. Compared by [`same_x_client`], on ids the server allocated.
    fn leader_startup_id(&self, window: &X11Surface) -> Option<String> {
        let leader = window.hints()?.window_group?;
        if leader == window.window_id() || !same_x_client(leader, window.window_id()) {
            return None;
        }
        self.x11_startup_carriers.get(&leader)?.startup_id()
    }

    /// Files (or forgets) `window` as a carrier of a startup id, as the XWM
    /// creates it or its `_NET_STARTUP_ID` changes; see
    /// [`State::x11_startup_carriers`].
    pub(super) fn note_x11_startup_id(&mut self, window: &X11Surface) {
        if window.startup_id().is_some() {
            self.x11_startup_carriers
                .insert(window.window_id(), window.clone());
        } else {
            self.x11_startup_carriers.remove(&window.window_id());
        }
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

    /// Whether `window` may redeem a live token its startup id names (the
    /// module doc's binding): the token is a spawn's whose process has
    /// exited, or a Wayland client's -- both keep the unbound rule -- or it
    /// is a running spawn's and the X-Resource pid of `window`'s client is
    /// that spawn or descends from it. The cheap checks run first: the X
    /// round trip (once per window, cached) and the `/proc` walk only while
    /// the spawn runs, and at most once per window and spawn when the
    /// answer is no ([`RefusedSpawn`], which also records the refusal for
    /// the re-ask on the spawn's exit). Every caller is a window asking for
    /// focus -- a map or `_NET_ACTIVE_WINDOW` -- which is what makes a
    /// recorded refusal an ask.
    fn startup_id_bound_to(&self, data: &XdgActivationTokenData, window: &X11Surface) -> bool {
        let Some(&SpawnedPid(spawned)) = data.user_data.get::<SpawnedPid>() else {
            return data.client_id.is_some();
        };
        // Reaped: the spawn exited without its token being redeemed (a
        // forwarder, a fork into the background), so no process tree can
        // tie the app to it any more -- the unbound rule, as for a
        // launcher's token. Checked before the refusal cache, which a
        // refusal while the spawn ran left keyed to its pid. And only a
        // tracked spawn is walked from: its pid cannot be reused while
        // it is unreaped.
        if !self.spawned_children.contains(&spawned) {
            return true;
        }
        let Some(pid) = client_pid(window) else {
            return false;
        };
        if RefusedSpawn::get(window).is_some_and(|refused| refused.pid == spawned) {
            return false;
        }
        let bound = descends_from(pid, spawned);
        if !bound {
            RefusedSpawn::set(
                window,
                Refusal {
                    pid: spawned,
                    at: Instant::now(),
                    focus_generation: self.focus_generation,
                },
            );
        }
        bound
    }

    /// Finds and spends the spawn token that chains `window` to a spawn of
    /// scoot's own (the module doc's rule 2). `false` when none does.
    fn redeem_x11_spawn_token(&mut self, window: &X11Surface) -> bool {
        let fresh = |data: &XdgActivationTokenData| data.timestamp.elapsed() < TOKEN_LIFETIME;
        if let Some(startup) = window
            .startup_id()
            .or_else(|| self.leader_startup_id(window))
        {
            let token = XdgActivationToken::from(startup);
            if let Some(data) = self.xdg_activation.data_for_token(&token)
                && fresh(data)
            {
                if self.startup_id_bound_to(data, window) {
                    self.xdg_activation.remove_token(&token);
                    tracing::debug!(
                        xid = window.window_id(),
                        "X11 window redeemed its (or its client leader's) startup id"
                    );
                    return true;
                }
                // Left live: the process it was minted for has yet to map.
                tracing::debug!(
                    xid = window.window_id(),
                    "refusing an X11 startup id: the window's process is not the one its token was minted for"
                );
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

    /// Re-asks, for a window refused while it ran, the token of each spawn
    /// the current `SIGCHLD` sweep has just reaped (`State::reaped_spawns`)
    /// without redeeming it. A forwarder was measured to exit after the
    /// running instance's window mapped (GTK `mousepad`'s second launch:
    /// reaped 8 ms after the map), so a decision made only at map time
    /// would refuse a window naming the forwarded id for good.
    ///
    /// Narrow on purpose -- a window is granted the reaped spawn's token only
    /// if all of these hold:
    /// - it **asked** for focus (mapped, or sent `_NET_ACTIVE_WINDOW`) with
    ///   that token while the spawn ran, and was refused for it
    ///   ([`RefusedSpawn`] names this spawn's pid): a window that only
    ///   carries the startup id, or was refused for another spawn, is owed
    ///   nothing;
    /// - it did so within [`REFUSAL_GRACE`] before the reap;
    /// - focus has not moved since (`State::focus_generation`): whatever the
    ///   user did in between decided focus;
    /// - its startup id (own, or its client leader's) still names the token.
    ///
    /// Of several, the one refused first -- the first to ask -- settles the
    /// token: it is granted if its claim holds, and otherwise the token is
    /// spent with no grant, so no later asker can take it.
    /// A focused window that was refused for the spawn (rule 1 or 3 then
    /// focused it) only spends the token, which would otherwise outlive the
    /// spawn as copyable. With no refused asker at all the token is left for
    /// the unbound rule at a later map or request (the first to ask then). Nothing happens without an XWM or while
    /// the session is locked, like every activation; the token then expires
    /// unspent. A grant moves focus, so later spawns in the same sweep find
    /// the generation changed and grant nothing: one sweep focuses at most
    /// one window.
    ///
    /// Costs one pass over the token table (bounded by its cap) per reaped
    /// spawn, and a pass over the windows only when that spawn's token is
    /// still live; a window's startup id is read only once its refusal
    /// matches. Allocation-free but for that read and the token handle.
    pub(in crate::compositor) fn x11_focus_for_exited_spawns(&mut self) {
        let now = Instant::now();
        let mut next = 0;
        // Indexed, not iterated: granting focus needs `&mut self`. Nothing
        // below touches `reaped_spawns`, and `get` stays in range anyway.
        while let Some(&pid) = self.reaped_spawns.get(next) {
            next += 1;
            if self.xwm.is_none() || self.session_lock.is_locked() {
                return;
            }
            self.x11_focus_for_exited_spawn(pid, now);
        }
    }

    /// [`State::x11_focus_for_exited_spawns`] for one reaped spawn `pid`.
    fn x11_focus_for_exited_spawn(&mut self, pid: u32, now: Instant) {
        let Some(token) = self
            .xdg_activation
            .tokens()
            .find(|(_, data)| {
                data.timestamp.elapsed() < TOKEN_LIFETIME
                    && data.user_data.get::<SpawnedPid>() == Some(&SpawnedPid(pid))
            })
            .map(|(token, _)| token.clone())
        else {
            return;
        };
        let refused_for = |state: &State, x11: &X11Surface| {
            let refusal = RefusedSpawn::get(x11).filter(|refusal| refusal.pid == pid)?;
            let names = x11
                .startup_id()
                .or_else(|| state.leader_startup_id(x11))
                .is_some_and(|startup| startup == token.as_str());
            names.then_some(refusal)
        };
        if let Some(focused) = self
            .focus
            .and_then(|id| self.windows.get(&id))
            .and_then(|window| window.x11_surface())
            && refused_for(self, focused).is_some()
        {
            self.xdg_activation.remove_token(&token);
            return;
        }
        // The first window to ask with the token while the spawn ran, owed
        // or not: as under the unbound rule, the first asker settles the
        // token. If its claim has lapsed the token is spent with no grant,
        // so a later asker (one that copied the id) cannot take it either.
        let mut first: Option<(WindowId, Refusal)> = None;
        for (&id, window) in &self.windows {
            let Some(x11) = window.x11_surface() else {
                continue;
            };
            let Some(refusal) = refused_for(self, x11) else {
                continue;
            };
            if first.is_none_or(|(_, earliest)| refusal.at < earliest.at) {
                first = Some((id, refusal));
            }
        }
        let Some((id, refusal)) = first else {
            return;
        };
        self.xdg_activation.remove_token(&token);
        let owed = refusal.focus_generation == self.focus_generation
            && now.saturating_duration_since(refusal.at) <= REFUSAL_GRACE;
        if !owed {
            tracing::debug!(
                ?id,
                pid,
                "the first X11 window refused for an exited spawn's token no longer has a claim; \
                 the token is spent with no grant"
            );
            return;
        }
        tracing::debug!(
            ?id,
            pid,
            "X11 window refused while its spawn ran redeemed the spawn's token on its exit"
        );
        // As `x11_activation_request` does before its own `act`.
        self.clicked_layer = None;
        self.act(Action::FocusWindowId(id));
    }
}

/// The resource-id mask XWayland hands every client: the low bits of an X
/// id are the client's own, the bits above them name the client. An X
/// server gives each connection its own `resource-id-base` and refuses a
/// window id outside it (`BadIDChoice`), so the client bits of a window's id
/// are the server's word on which connection created it -- nothing a client
/// can forge. The value follows from the X server's default client limit
/// (256 clients, so 8 client bits over a 29-bit id space, leaving 21): Smithay
/// starts XWayland with no `-maxclients`, and the live test
/// `the_resource_id_mask_is_the_servers` checks it against what the server
/// reports, so a change surfaces as a failing test rather than a gate that
/// quietly compares the wrong bits.
pub(in crate::compositor) const X_CLIENT_RESOURCE_MASK: u32 = 0x001f_ffff;

/// Whether two X window ids were allocated to the same X client (the same
/// connection); see [`X_CLIENT_RESOURCE_MASK`].
pub(super) fn same_x_client(a: u32, b: u32) -> bool {
    x_client_key(a) == x_client_key(b)
}

/// Which X client an X window id was allocated to: the client bits above
/// [`X_CLIENT_RESOURCE_MASK`], the server's word on which connection created
/// the window (see [`X_CLIENT_RESOURCE_MASK`]) -- not a Wayland `ClientId`,
/// and never confused with one. What the per-X-client toplevel cap and the
/// per-X-client unmanaged-window cap (`toplevel_cap.rs`) each charge a
/// mapped window to.
pub(in crate::compositor) fn x_client_key(window_id: u32) -> u32 {
    window_id & !X_CLIENT_RESOURCE_MASK
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
