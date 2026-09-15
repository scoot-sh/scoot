//! `xdg-activation-v1`: one client asking that another be focused.
//!
//! The protocol is two halves. A client that is about to cause something to
//! happen elsewhere -- a launcher starting an app, a notification daemon
//! whose popup was clicked -- asks for a *token* and hands it to whoever it
//! started (conventionally in `$XDG_ACTIVATION_TOKEN`). That party later
//! sends the token back with `activate(token, surface)`, and the compositor
//! decides whether to move focus there. Without it a client has no standard
//! way to ask at all: focus can only move through flexwm's own keybindings
//! and IPC.
//!
//! # The policy, and why it is a policy
//!
//! The protocol explicitly leaves the decision to the compositor ("the
//! compositor may ignore the request"), because honoring every request
//! unconditionally is a focus-stealing primitive: any client, at any time,
//! could take the keyboard from whatever the user is typing into. flexwm's
//! answer is two bounds, both on the token rather than on the surface:
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
//! Deliberately *not* part of the policy: requiring the token to carry a seat
//! and input serial. The protocol makes both optional (`set_serial` is a
//! request, not a constructor argument), and real launchers -- the exact
//! case this exists for -- routinely create a token from a keyboard-driven
//! selection and hand it to a process that takes a second to start. Refusing
//! those would make the feature not work for the one thing it is for, to stop
//! an attack that a malicious client can mount anyway by simply clicking
//! first.
//!
//! What the compositor does *not* do with a refused request is also
//! deliberate: nothing. There is no urgency hint to raise instead -- flexwm
//! has no per-window urgency state and no decoration to show it on (see
//! `decorations.rs`) -- so a refusal is logged at debug and dropped rather
//! than half-honored.

use std::time::Duration;

use flexwm_core::Action;
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

    /// A client asked for a token. Sweeps expired ones, then accepts unless
    /// the table is genuinely full.
    ///
    /// Returning `false` makes Smithay drop the token immediately rather than
    /// track it; the client still gets its `done` event with a token string,
    /// which simply will not be honored later. That is the protocol's own
    /// shape for a refusal -- there is no error to post for "I would rather
    /// not" -- and it is why the refusal is logged here: from the client's
    /// side it is indistinguishable from an activation the compositor chose
    /// to ignore.
    ///
    /// The sweep is here rather than on a timer because this is the only
    /// place the table can grow: an expired token costs nothing until
    /// something tries to add another one.
    fn token_created(&mut self, token: XdgActivationToken, data: XdgActivationTokenData) -> bool {
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
    /// Focus moves through [`State::act`], not by writing `self.focus`: that
    /// is the one path every requested action goes through, so an activation
    /// while the session is locked is refused by the same gate a keybinding
    /// is (see `shell.rs::act`), and the core re-derives the whole
    /// arrangement -- scrolling the activated column into view, which is what
    /// actually makes an off-screen window visible in a scrolling layout.
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
        tracing::debug!(?id, app_id = ?token_data.app_id, "activating a window");
        self.act(Action::FocusWindowId(id));
    }
}
