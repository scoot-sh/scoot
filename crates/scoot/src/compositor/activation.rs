//! `xdg-activation-v1`: one client asking that another be focused.
//!
//! The protocol is two halves. A client that is about to cause something to
//! happen elsewhere -- a launcher starting an app, a notification daemon
//! whose popup was clicked -- asks for a *token* and hands it to whoever it
//! started (conventionally in `$XDG_ACTIVATION_TOKEN`). That party later
//! sends the token back with `activate(token, surface)`, and the compositor
//! decides whether to move focus there. Without it a client has no standard
//! way to ask at all: focus can only move through scoot's own keybindings
//! and IPC.
//!
//! # The policy, and why it is a policy
//!
//! The protocol explicitly leaves the decision to the compositor ("the
//! compositor may ignore the request"), because honoring every request
//! unconditionally is a focus-stealing primitive: any client, at any time,
//! could take the keyboard from whatever the user is typing into. scoot
//! applies three rules, all to the token rather than to the surface. Two of
//! them bound resources and are **not** a defense against focus-stealing
//! itself (the third, below, is the one that answers that):
//!
//! - **Freshness** ([`TOKEN_LIFETIME`]). A token is a receipt for something
//!   that just happened, not a standing permit. One older than this is
//!   refused, so a client cannot hoard tokens and spend one minutes later
//!   while the user is in the middle of something else.
//! - **A bound on how many exist at once** ([`MAX_TOKENS`]). `xdg_activation_v1.get_activation_token`
//!   is unauthenticated and unlimited, and each token upstream hands out is
//!   kept in a `HashMap` that nothing prunes -- so a client looping on it is
//!   unbounded compositor memory, the same resource-exhaustion family as the
//!   `wl_shm` pool cap in `dispatch.rs` and the IPC line-length cap. Expired
//!   tokens are swept first and only a genuinely full table refuses.
//!
//! Neither bound stops the actual focus-steal case -- a client with no
//! keyboard or pointer focus, and no user interaction at all, minting a token
//! and immediately redeeming it against its own surface, which is fresh and
//! leaves the table nowhere near full. The third rule is the one that does,
//! and it is the gate the protocol itself offers:
//!
//! - **The token must name a real, recent interaction *with the client
//!   asking*.** `set_serial(serial, seat)` is how a client says "this is the
//!   click/keypress that caused me to ask". It is optional in the protocol --
//!   a request, not a constructor argument -- so a token arrives here with no
//!   serial at all, with one naming a seat this compositor does not own, or
//!   with a stale or fabricated number, and all of those are refused at
//!   creation. What is accepted is a serial this compositor issued for a key
//!   or button event within the last few input events *and* the last few
//!   seconds, **and delivered to this same client** (see
//!   `input/interaction.rs`), which is what a launcher minting a token from
//!   inside its own input handler necessarily has. The age bound is not
//!   redundant with the count: an idle session -- including an
//!   agent-driven one, where every action arrives over IPC rather than as
//!   input -- never rotates the history at all.
//!
//!   The client half is not a formality. `SERIAL_COUNTER` is process-global
//!   and shared with events that are not input at all -- an
//!   `xdg_surface.configure` serial comes from the same counter -- so any
//!   client can read its live value for free (commit a role-less surface,
//!   read the configure) and guess nearby numbers at no cost, since a refused
//!   token posts no error. Matching the serial *and* the recipient is what
//!   makes this a check on interaction rather than on arithmetic: a client
//!   that was never focused, and never had the pointer over it, has nothing
//!   to guess with.
//!
//! Checked when the token is *created*, never when it is redeemed. A
//! launcher hands its token to a process that may take seconds to cold-start
//! before it maps a window and calls `activate`, by which point dozens of
//! newer events have happened; re-checking then would break the one case
//! this protocol exists for. The serial only has to have been valid when the
//! token was minted, and [`TOKEN_LIFETIME`] is what bounds the gap after
//! that.
//!
//! Only key and button events count, which is stricter than the loosest
//! reading of the protocol -- upstream documents the serial as one that "can
//! come from an input or focus event". A *focus* event is not consent:
//! scoot focuses every newly mapped window itself (`shell.rs`'s
//! `add_window` passes `focus: true`), so any client can collect a
//! `wl_keyboard.enter` serial just by mapping, and spending it later is the
//! steal this gate exists to refuse.
//!
//! That has a real cost, and it is narrower than "harmless" but wider than
//! nothing: a launcher that mints from its last *focus* serial, because
//! nothing was typed or clicked in its own surface, is refused. `fuzzel`
//! does exactly that when an entry is chosen with the mouse -- it only ever
//! sends its keyboard serial, which is then the one from `wl_keyboard.enter`.
//! Two different outcomes follow, and only the first is covered:
//!
//! - **A fresh spawn is unaffected.** The app the launcher started maps a
//!   window, and mapping focuses it, which is where a launched window's focus
//!   came from before this protocol existed at all.
//! - **Re-activating something already running is not.** A single-instance
//!   app (Firefox, Chromium, anything on `GApplication`) hands the token to
//!   its existing process, which calls `activate` on a window that already
//!   exists -- nothing maps, so nothing focuses it, and a refused token here
//!   means nothing visible happens. The same is true of the notification
//!   daemon case: focusing the app a clicked popup came from is exactly an
//!   activation of an existing window.
//!
//! So the rule is: a token minted from a real key or button press works; one
//! minted from a focus serial alone is refused, and for an already-running
//! target that refusal is the whole outcome.
//!
//! What this deliberately does *not* stop: a client the user really did
//! interact with can activate itself off that interaction -- including more
//! than once, since one serial may be named by several tokens, bounded only
//! by [`MAX_TOKENS`] and [`TOKEN_LIFETIME`]. That is the protocol working as
//! intended: the user just clicked it. A token the compositor mints itself
//! ([`State::mint_spawn_token`], handed to a process it spawned in
//! `$XDG_ACTIVATION_TOKEN`) also bypasses this gate -- upstream never routes
//! `create_external_token` through [`XdgActivationHandler::token_created`].
//! That is safe for the same reason the gate binds the asker rather than the
//! redeemer: the serial rule answers "did this client earn a token", and the
//! compositor is not a client that can be tricked into asking -- minting one
//! is its own focus decision, delegated to the child. What still binds a
//! minted token is everything else: the cap, the lifetime, the single-use
//! removal and the lock gate all run at redemption (see that method).
//!
//! What the compositor does *not* do with a refused request is also
//! deliberate: nothing. There is no urgency hint to raise instead -- scoot
//! has no per-window urgency state and no decoration to show it on (see
//! `decorations.rs`) -- so a refusal is logged at debug and dropped rather
//! than half-honored.

use std::time::Duration;

use scoot_core::Action;
use smithay::input::Seat;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::xdg_activation::{
    XdgActivationHandler, XdgActivationState, XdgActivationToken, XdgActivationTokenData,
};

use super::State;

#[cfg(test)]
mod tests;

/// How long a token may be redeemed for after it was created.
///
/// Long enough for the case the protocol exists for -- a launcher starts an
/// app, the app's own startup (a cold Electron or JVM launch is seconds, not
/// milliseconds) finishes, and only then does it map a window and redeem its
/// token -- and short enough that a token is still a receipt for something
/// the user just did rather than a permit a client can sit on. Ten seconds,
/// the figure Smithay's own documentation uses, was measurably too short for
/// exactly that cold-start case; a minute would make "the user just asked for
/// this" untrue.
const TOKEN_LIFETIME: Duration = Duration::from_secs(30);

/// How many unredeemed tokens may exist at once, across every client.
///
/// A token is ~100 bytes of `HashMap` entry, so this is not a memory bound
/// worth tuning -- it is a bound at all, which is the point (see the module
/// doc). It is far above any real workload: a token is created per user
/// action, redeemed within seconds, and swept when it expires, so a session
/// that ever holds 64 live ones has a client in a loop, not a busy desktop.
const MAX_TOKENS: usize = 64;

impl XdgActivationHandler for State {
    fn activation_state(&mut self) -> &mut XdgActivationState {
        &mut self.xdg_activation
    }

    /// A client asked for a token. Refuses one that does not name a real,
    /// recent interaction on this compositor's own seat; otherwise sweeps
    /// expired tokens and accepts unless the table is genuinely full.
    ///
    /// Returning `false` makes Smithay drop the token immediately rather than
    /// track it; the client still gets its `done` event with a token string,
    /// which simply will not be honored later. That is the protocol's own
    /// shape for a refusal -- there is no error to post for "I would rather
    /// not" -- and it is why each refusal is logged here, with the reason:
    /// from the client's side all of them are indistinguishable from an
    /// activation the compositor chose to ignore.
    ///
    /// The serial is checked before the sweep, so a client looping on
    /// unqualified tokens is rejected without touching the table at all.
    /// The sweep is here rather than on a timer because this is the only
    /// place the table can grow: an expired token costs nothing until
    /// something tries to add another one.
    fn token_created(&mut self, token: XdgActivationToken, data: XdgActivationTokenData) -> bool {
        let Some((serial, seat)) = &data.serial else {
            tracing::debug!(
                app_id = ?data.app_id,
                "refusing an xdg-activation token: it names no input event (set_serial was never called)"
            );
            return false;
        };
        // Resolved rather than assumed: scoot has exactly one seat, so in
        // practice any `wl_seat` a client can name is this one -- but a
        // resource that resolves to some other seat, or to no live seat at
        // all, has no bearing on what this compositor's keyboard and pointer
        // did, and a serial is only meaningful against the seat that issued
        // it.
        match Seat::<State>::from_resource(seat) {
            Some(named) if named == self.seat => {}
            _ => {
                tracing::debug!(
                    app_id = ?data.app_id,
                    "refusing an xdg-activation token: its serial names a seat this compositor does not own"
                );
                return false;
            }
        }
        // Whose event it was, not just which number: `SERIAL_COUNTER` is
        // process-global and a client can read its live value for free (an
        // `xdg_surface.configure` serial comes out of the same counter), so a
        // value-only check is guessable by a client that received no input at
        // all -- see `input/interaction.rs`. `client_id` is filled in by
        // Smithay from the client that sent the request, so it cannot be
        // forged; it is `None` only for a token the compositor minted itself
        // (`create_external_token`), which never reaches this handler.
        let Some(client) = &data.client_id else {
            tracing::debug!(
                app_id = ?data.app_id,
                "refusing an xdg-activation token: it has no requesting client"
            );
            return false;
        };
        if !self.interaction_serials.contains(*serial, client) {
            tracing::debug!(
                app_id = ?data.app_id,
                "refusing an xdg-activation token: its serial is not a recent key or button event this client received"
            );
            return false;
        }
        self.xdg_activation
            .retain_tokens(|_, data| data.timestamp.elapsed() < TOKEN_LIFETIME);
        if self.xdg_activation.tokens().count() >= MAX_TOKENS {
            tracing::debug!(
                app_id = ?data.app_id,
                max = MAX_TOKENS,
                "refusing an xdg-activation token: too many are already outstanding"
            );
            return false;
        }
        tracing::debug!(app_id = ?data.app_id, token = token.as_str(), "xdg-activation token created");
        true
    }

    /// A client redeemed a token against a surface.
    ///
    /// The token is removed either way, redeemed or refused: it is a receipt
    /// for one event, and leaving a spent one in the table would let the same
    /// client re-activate itself from the same user action indefinitely.
    ///
    /// Refused before anything is touched: an expired token, a surface that
    /// is not a window, and -- like `wlr_toplevel_activate`, and for the same
    /// reason -- a locked session. The lock is checked here rather than left
    /// to [`State::act`]'s own gate because what follows spends the
    /// launcher's click (see below), and a refused request must not disturb
    /// anything: `clicked_layer` has to survive the lock so the session comes
    /// back as the user left it. `act`'s gate remains as the backstop
    /// `shell.rs` describes it as.
    ///
    /// Focus moves through [`State::act`], not by writing `self.focus`: that
    /// is the one path every requested action goes through, and the core
    /// re-derives the whole arrangement -- scrolling the activated column
    /// into view, which is what actually makes an off-screen window visible
    /// in a scrolling layout.
    fn request_activation(
        &mut self,
        token: XdgActivationToken,
        token_data: XdgActivationTokenData,
        surface: WlSurface,
    ) {
        self.xdg_activation.remove_token(&token);

        let age = token_data.timestamp.elapsed();
        if age >= TOKEN_LIFETIME {
            tracing::debug!(
                ?age,
                app_id = ?token_data.app_id,
                "ignoring an xdg-activation request: its token has expired"
            );
            return;
        }
        let Some(id) = self.id_of(&surface) else {
            // A surface this compositor does not lay out: a layer surface, a
            // popup, a cursor surface, or a toplevel that has already been
            // destroyed between the request being sent and dispatched. None
            // of those is a window focus can move to.
            tracing::debug!(
                app_id = ?token_data.app_id,
                "ignoring an xdg-activation request for a surface that is not a window"
            );
            return;
        };
        if self.session_lock.is_locked() {
            tracing::debug!(
                ?id,
                app_id = ?token_data.app_id,
                "ignoring an xdg-activation request: the session is locked"
            );
            return;
        }
        tracing::debug!(?id, app_id = ?token_data.app_id, "activating a window");
        // Mirrors `input.rs`'s `focus_under_pointer`, which clears this on the
        // line before its own `act(FocusWindowId)` -- and what
        // `wlr_toplevel_activate` does for its own `activate`: without it
        // `layer_shell.rs`'s `layer_keyboard_focus` hands the keyboard straight
        // back to a still-mapped `on_demand` surface named here, so the refresh
        // `act` ends in would re-derive the launcher -- which is exactly what
        // sent this request, and may well still be mapped -- instead of the
        // window. The click is spent only once the request is known to be
        // honored, which is why the refusals above (and the lock gate ahead of
        // them) come first.
        self.clicked_layer = None;
        self.act(Action::FocusWindowId(id));
    }
}

impl State {
    /// The environment variable a launcher sets for the process it starts, so
    /// the child can activate its own window when it maps one.
    /// [`State::spawn`] sets it for every child it mints a token for; a
    /// toolkit reads the literal name, so this is the one spelling.
    pub(super) const ACTIVATION_TOKEN_ENV: &str = "XDG_ACTIVATION_TOKEN";

    /// Mints the activation token a process scoot spawned itself carries in
    /// [`Self::ACTIVATION_TOKEN_ENV`].
    ///
    /// Upstream never routes this through [`XdgActivationHandler::token_created`]
    /// -- no serial gate, no sweep, no cap -- so this method applies the two
    /// resource bounds itself, uniformly with client-minted tokens:
    ///
    /// - **Freshness** ([`TOKEN_LIFETIME`]). The token is stamped with the
    ///   spawn time as its event time (the `Default` data's `Instant::now()`),
    ///   and `request_activation` refuses it past 30s exactly like a
    ///   launcher's. No longer lifetime: the motivating slow cold start is
    ///   seconds, which is the case the 30s figure was sized for, and a
    ///   second per-token lifetime would need a table this method refuses to
    ///   build.
    /// - **The shared cap** ([`MAX_TOKENS`]). Expired tokens are swept first,
    ///   the way `token_created` does -- including expired spawn tokens, so a
    ///   burst of dead spawns cannot wedge what follows -- and a genuinely
    ///   full table mints nothing: the spawn proceeds without a token, which
    ///   is today's behavior (mapping focuses the new window itself), never
    ///   an eviction. The eviction direction is the whole point: rapid spawns
    ///   (an agent loop starting apps) must not break interactive tokens, so
    ///   a spawn never removes anyone else's live entry. The reverse -- 64
    ///   live spawn tokens refusing one interactive mint -- is bounded and
    ///   self-healing (every spawn token is redeemed once or expires within
    ///   30s), where a separate uncapped table would be an unbounded leak.
    ///
    /// `app_id` names the spawned program, for the debug lines at both ends.
    /// `None` is "no token for this child", never an error: the caller still
    /// spawns, and the child still gets focused on map.
    pub(super) fn mint_spawn_token(&mut self, program: &str) -> Option<XdgActivationToken> {
        self.xdg_activation
            .retain_tokens(|_, data| data.timestamp.elapsed() < TOKEN_LIFETIME);
        if self.xdg_activation.tokens().count() >= MAX_TOKENS {
            tracing::debug!(
                max = MAX_TOKENS,
                program,
                "not minting an xdg-activation token for a spawned child: too many are already outstanding"
            );
            return None;
        }
        let data = XdgActivationTokenData {
            app_id: Some(program.to_owned()),
            ..XdgActivationTokenData::default()
        };
        let (token, _) = self.xdg_activation.create_external_token(data);
        let token = token.clone();
        tracing::debug!(
            token = token.as_str(),
            program,
            "xdg-activation token minted for a spawned child"
        );
        Some(token)
    }
}
