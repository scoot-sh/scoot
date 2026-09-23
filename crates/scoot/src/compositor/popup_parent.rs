//! An `xdg_popup`'s parent chain: the one step up it that every walk here
//! takes, and the rules that keep every chain finite, loop-free and at most
//! [`MAX_POPUP_DEPTH`] popups long.
//!
//! # Why a chain has to be bounded, not just loop-free
//!
//! Smithay's popup tree recurses once per level: `PopupNode::try_insert`
//! when a popup is tracked, `iter_popups_relative_to` whenever a window's
//! popups are drawn, `cleanup_and_get_alive` after every dispatch. None of
//! that is bounded at the pinned rev, and neither is the walk up a chain
//! (`find_popup_root_surface`, and `popup_constraint.rs`'s own). So a client
//! that nests popups deeply enough overflows the compositor's stack -- a
//! chain about 2000 deep did, drawing a frame, on a 2 MB stack in a debug
//! build -- or freezes it for seconds per frame, and every other client goes
//! down with it. A loop is the extreme case: every walk up it runs forever.
//!
//! # The rules
//!
//! A popup's parent is written at these sites, and nowhere else:
//!
//! 1. `xdg_surface.get_popup` -- Smithay writes the role data, then calls
//!    `new_popup`, which runs [`admit`] before anything walks the chain;
//! 2. `zwlr_layer_surface_v1.get_popup` -- always a layer surface, which is
//!    not a popup, so it ends a chain rather than extending one: it can
//!    shorten a popup's chain and never lengthen it;
//! 3. `xdg_popup`'s destructor -- Smithay resets the role data, parent
//!    included, to `None`, which also only shortens chains;
//! 4. [`admit`]'s own refusal, which clears the refused popup's parent to
//!    `None` (see [`refuse`]).
//!
//! Only (1) can lengthen a chain, and it is checked. What it has to rule out
//! is not just a deep *new* popup but an old popup's chain growing under it
//! after it was admitted -- which is how a check that only counts a new
//! popup's ancestors is bypassed, by re-parenting something that already has
//! children. That takes one of three things, and each is refused:
//!
//! - **A second `get_popup` on a surface whose popup is still alive**
//!   (`xdg_surface.already_constructed`). Smithay gives the same role twice
//!   without complaint and overwrites the live popup's parent. Checked per
//!   `wl_surface` rather than per `xdg_surface`, because nothing at the
//!   pinned rev stops a client making a second `xdg_surface` for the same
//!   `wl_surface` either.
//! - **Destroying a popup that still has child popups**
//!   (`xdg_wm_base.not_the_topmost_popup`), after which its surface could be
//!   made a popup again, somewhere deeper, children and all. Checked when the
//!   popup is destroyed ([`popup_destroyed`]).
//! - **A parent with no live role object.** A child made of a bare, role-less
//!   `xdg_surface` -- or of a popup surface whose `xdg_popup` is gone, since a
//!   surface's role outlives its role object -- deepens the moment that parent
//!   is made a popup. The protocol already requires a popup's parent to be
//!   mapped; this refuses only what it has to, the parent that could later
//!   become a popup. A toplevel or layer surface parent is a chain's root for
//!   good (a surface's role is permanent), so it is never refused.
//!
//! With those three refused, the tree only grows at its leaves: a popup's
//! chain is exactly as long as it was when it was admitted, and [`admit`]
//! bounds that at [`MAX_POPUP_DEPTH`]. That also makes the chain loop-free --
//! the only loop left for a client to try is a popup that names its own
//! `xdg_surface` as its parent, and the walk refuses that -- so every other
//! walk up a chain, Smithay's and `popup_constraint.rs`'s, terminates in at
//! most that many steps.
//!
//! The tree Smithay draws from is kept to the same depth by one more step,
//! in `new_popup`: see [`Admission::Reused`].
//!
//! Children are counted only for `xdg_popup`s. An input-method popup is
//! placed against whichever surface has the text field -- which can be
//! another client's popup -- but it cannot be anyone's parent, so it cannot
//! deepen anything, and an application closing its menu must not be told
//! off for an input method's candidate window.

use std::sync::{Mutex, PoisonError};

use smithay::reexports::wayland_protocols::xdg::shell::server::{
    xdg_popup::XdgPopup, xdg_surface, xdg_wm_base,
};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Resource, Weak};
use smithay::utils::{Logical, Point};
use smithay::wayland::compositor::{get_role, with_states};
use smithay::wayland::shell::xdg::{
    PopupCachedState, PopupSurface, XDG_POPUP_ROLE, XdgPopupSurfaceData, XdgShellSurfaceUserData,
};

/// The most popups a parent chain may hold, the new one included: the 64th
/// nested popup is admitted, a 65th is refused.
///
/// Real menus nest a handful deep -- a menubar menu, a submenu or two, now
/// and then a third -- so this is an order of magnitude above anything a
/// person could navigate, and no legitimate client comes near it. It is
/// also far below where Smithay's recursion becomes a problem: every
/// recursive walk of the tree is at most this deep (one level more for an
/// input-method popup, which is a leaf), which costs a few tens of
/// kilobytes of stack in a debug build against the ~2000 levels that
/// overflowed 2 MB, and a max-depth chain draws and is created in
/// microseconds (measured: `tests/bench.rs`).
pub(super) const MAX_POPUP_DEPTH: usize = 64;

/// One step up the chain from a popup-role `surface`: its parent (`None`
/// when it has none yet -- created parentless for a layer surface that has
/// not adopted it) and its committed position (as last acked and committed,
/// the position the render tree places it by; the origin before its first
/// configured commit).
///
/// `None` for a surface that is not a popup -- the root of a chain -- or
/// whose popup role data is gone.
pub(super) fn popup_link(surface: &WlSurface) -> Option<(Option<WlSurface>, Point<i32, Logical>)> {
    if get_role(surface) != Some(XDG_POPUP_ROLE) {
        return None;
    }
    with_states(surface, |states| {
        let parent = states
            .data_map
            .get::<XdgPopupSurfaceData>()?
            .lock()
            .ok()?
            .parent
            .clone();
        let loc = states
            .cached_state
            .get::<PopupCachedState>()
            .current()
            .last_acked
            .as_ref()
            .map(|configure| configure.state.geometry.loc)
            .unwrap_or_default();
        Some((parent, loc))
    })
}

/// scoot's own record of a surface's current `xdg_popup`, in the surface's
/// data map: Smithay keeps nothing that says which popup object currently
/// owns a surface's role, nor how many popups hang off it.
///
/// Weak handles only: the `xdg_popup`'s own user data holds its
/// `wl_surface`, so a strong handle back to it from that surface's data
/// would be a reference cycle, and neither would ever be freed.
#[derive(Default)]
struct PopupRecord {
    /// The surface's current `xdg_popup`, or the last one it had. The
    /// popup is alive exactly while this upgrades.
    owner: Option<Weak<XdgPopup>>,
    /// The popup-role parent it was admitted under and counted against, if
    /// any -- recorded, not read back from Smithay's role data at destroy,
    /// because a layer surface's `get_popup` overwrites that parent.
    counted_parent: Option<Weak<WlSurface>>,
    /// How many live `xdg_popup`s were admitted with this surface as their
    /// parent.
    live_children: u32,
}

impl PopupRecord {
    /// The live popup that owns this surface's role, if there is one.
    fn live_owner(&self) -> Option<XdgPopup> {
        self.owner.as_ref()?.upgrade().ok()
    }
}

/// Runs `f` on `surface`'s [`PopupRecord`], creating an empty one first if
/// it has none.
fn with_record<T>(surface: &WlSurface, f: impl FnOnce(&mut PopupRecord) -> T) -> T {
    with_states(surface, |states| {
        let cell = states
            .data_map
            .get_or_insert_threadsafe(|| Mutex::new(PopupRecord::default()));
        f(&mut cell.lock().unwrap_or_else(PoisonError::into_inner))
    })
}

/// What [`admit`] decided about a new popup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Admission {
    /// Refused, and its client disconnected: `new_popup` must not track it.
    Refused,
    /// Admitted, on a surface that has never been a popup before.
    Fresh,
    /// Admitted, on a surface that was a popup before, whose earlier
    /// `xdg_popup` is dead (GTK re-shows a menu this way when it keeps the
    /// `wl_surface`).
    ///
    /// `new_popup` reaps dead popups from the tree (`PopupManager::cleanup`)
    /// before tracking this one. The earlier popup's node stays in its
    /// parent's `PopupTree` until then -- reaping otherwise runs once per
    /// dispatch cycle -- and `PopupNode::try_insert` matches a parent node by
    /// `wl_surface` alone, dead or alive. A child of this popup made in the
    /// same flush would be inserted under the *dead* node instead of the new
    /// one: invisible (the draw walk skips dead nodes), killed by Smithay's
    /// own `not_the_topmost_popup` check at the next reaping, and -- repeated
    /// within one flush -- as deep in the tree as a client cares to make it,
    /// whatever its chain says. With the dead node gone, every live surface
    /// has exactly one node, at its chain's depth.
    Reused,
}

/// Why a popup was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Refusal {
    /// Its surface already has a live `xdg_popup`.
    AlreadyConstructed,
    /// Its parent chain comes back to itself.
    Loop,
    /// Its parent chain would hold more than [`MAX_POPUP_DEPTH`] popups.
    TooDeep,
    /// Its parent is a bare `xdg_surface`, or a popup surface whose
    /// `xdg_popup` is gone.
    NoLiveParent,
}

/// Decides whether a popup Smithay has just created may stay, refusing it
/// (and disconnecting its client) if it breaks one of the module doc's
/// rules, and records it if not. `new_popup` calls this before tracking the
/// popup, because tracking is itself the first walk up its chain.
///
/// The order matters in one place: the walk runs before the parent-liveness
/// check, so a popup that names its own `xdg_surface` as its parent is
/// reported as the loop it is, not as a parent with no live popup (which,
/// mid-creation, it also is).
pub(super) fn admit(popup: &PopupSurface) -> Admission {
    let surface = popup.wl_surface();
    let (previous_owner, live_owner) = with_record(surface, |record| {
        (record.owner.is_some(), record.live_owner())
    });
    if live_owner.is_some_and(|owner| owner != *popup.xdg_popup()) {
        refuse(popup, Refusal::AlreadyConstructed);
        return Admission::Refused;
    }
    if let Some(refusal) = walk_up(surface) {
        refuse(popup, refusal);
        return Admission::Refused;
    }
    let parent = popup_link(surface).and_then(|(parent, _)| parent);
    let counted_parent = match parent.as_ref().map(|parent| (parent, get_role(parent))) {
        // Parentless (a layer surface's popup, adopted later), or a toplevel
        // or layer surface: a root for good.
        None => None,
        Some((_, Some(role))) if role != XDG_POPUP_ROLE => None,
        Some((parent, Some(_))) => {
            let alive = with_record(parent, |record| {
                let alive = record.live_owner().is_some();
                if alive {
                    record.live_children = record.live_children.saturating_add(1);
                }
                alive
            });
            if !alive {
                refuse(popup, Refusal::NoLiveParent);
                return Admission::Refused;
            }
            Some(parent.downgrade())
        }
        Some((_, None)) => {
            refuse(popup, Refusal::NoLiveParent);
            return Admission::Refused;
        }
    };
    with_record(surface, |record| {
        *record = PopupRecord {
            owner: Some(popup.xdg_popup().downgrade()),
            counted_parent,
            live_children: 0,
        };
    });
    if previous_owner {
        Admission::Reused
    } else {
        Admission::Fresh
    }
}

/// Walks up from `popup`'s parent, at most [`MAX_POPUP_DEPTH`] steps: a
/// refusal if the chain comes back to `popup` or would be longer than that,
/// `None` once it reaches its root (a surface that is not a popup, or a
/// popup with no parent).
///
/// Bounded whatever the chain looks like -- it never needs the module doc's
/// rules to have held so far in order to stop.
fn walk_up(popup: &WlSurface) -> Option<Refusal> {
    let mut depth = 1;
    let mut next = popup_link(popup).and_then(|(parent, _)| parent);
    while let Some(surface) = next {
        if surface == *popup {
            return Some(Refusal::Loop);
        }
        let (parent, _) = popup_link(&surface)?;
        depth += 1;
        if depth > MAX_POPUP_DEPTH {
            return Some(Refusal::TooDeep);
        }
        next = parent;
    }
    None
}

/// Refuses `popup`: clears its parent, posts the protocol error, which
/// disconnects its client, and logs at `warn` like the popup-grab refusals,
/// so the disconnect is diagnosable.
///
/// **Where the error goes.** `already_constructed` is posted on the popup's
/// own `xdg_surface`, the object the protocol names. Every other refusal is
/// an `xdg_wm_base.invalid_popup_parent` -- the protocol has no "too deep",
/// and that is the nearest error it does have -- but the pinned rev keeps
/// the `xdg_wm_base` resource behind a `pub(crate)` field, so that one is
/// posted on the popup itself, with that error's code and a message naming
/// it. Either way the client is disconnected, which is what matters.
///
/// **Why the parent is cleared.** Smithay wrote the popup's role data,
/// parent included, before `new_popup` ran, so without this a refused
/// popup's loop, or its too-deep chain, would outlive the refusal until the
/// client's teardown resets it. Nothing can walk it through a request in
/// that window -- once killed, a client's requests are no longer read
/// (wayland-backend 0.3.17, `rs/server_impl/client.rs`, `next_request`
/// answers `EPIPE` for a killed client), which the same-flush `reposition`
/// in this module's tests confirms -- but the teardown itself runs
/// destructors in no stated order, and the draw walk runs in between.
/// Clearing it makes the refused surface a chain's root.
///
/// For `already_constructed` the role data is the *live* popup's: the
/// second `get_popup` overwrote it. Clearing it makes that popup a root
/// too, which can only shorten its chain and its children's.
fn refuse(popup: &PopupSurface, refusal: Refusal) {
    with_states(popup.wl_surface(), |states| {
        if let Some(data) = states.data_map.get::<XdgPopupSurfaceData>() {
            data.lock().unwrap_or_else(PoisonError::into_inner).parent = None;
        }
    });
    let reason = match refusal {
        Refusal::AlreadyConstructed => "this surface already has a live xdg_popup".to_owned(),
        Refusal::Loop => "this popup's parent chain loops back to the popup itself".to_owned(),
        Refusal::TooDeep => {
            format!("popups nest at most {MAX_POPUP_DEPTH} deep, and this one would be deeper")
        }
        Refusal::NoLiveParent => {
            "this popup's parent xdg_surface has no live xdg_toplevel or xdg_popup".to_owned()
        }
    };
    tracing::warn!(
        client = ?popup.wl_surface().client().map(|client| client.id()),
        %reason,
        "refusing an xdg_popup; disconnecting the client"
    );
    if refusal == Refusal::AlreadyConstructed
        && let Some(data) = popup.xdg_popup().data::<XdgShellSurfaceUserData>()
    {
        data.xdg_surface().post_error(
            xdg_surface::Error::AlreadyConstructed,
            format!("already_constructed: {reason}"),
        );
        return;
    }
    popup.xdg_popup().post_error(
        xdg_wm_base::Error::InvalidPopupParent as u32,
        format!("invalid_popup_parent: {reason}"),
    );
}

/// `XdgShellHandler::popup_destroyed`: closes `popup`'s [`PopupRecord`] and
/// refuses the destroy if the popup still had live child popups
/// (`xdg_wm_base.not_the_topmost_popup`).
///
/// Smithay calls this for every `xdg_popup` that goes away, refused ones
/// included, so the record is only touched if this popup is the one that
/// owns it -- a refused `already_constructed` popup shares its surface, and
/// record, with the live popup it tried to replace.
///
/// **Teardown is not a violation.** A disconnecting client's objects are
/// destroyed in no stated order, parents before children as often as not.
/// wayland-backend takes a dying client out of its client store before it
/// runs any of its destructors (`ClientStore::cleanup`), so `client()` is
/// `None` for every one of its objects then, and never during a request.
///
/// **Where the error goes.** The `xdg_popup` is already dead here -- Smithay
/// only calls this from the popup's destructor -- so an error on it would
/// name an object the client has already forgotten. It goes on the popup's
/// `xdg_surface` instead, which is still alive (destroying it before its
/// role object is already an error Smithay posts), with `xdg_wm_base`'s
/// code for the error -- which on `xdg_surface` happens to be the number of
/// its own `already_constructed`, so the message names the real error.
pub(super) fn popup_destroyed(popup: &PopupSurface) {
    let record = with_record(popup.wl_surface(), |record| {
        let owns = record
            .owner
            .as_ref()
            .is_some_and(|owner| owner.id() == popup.xdg_popup().id());
        owns.then(|| (record.counted_parent.take(), record.live_children))
    });
    let Some((counted_parent, live_children)) = record else {
        return;
    };
    if let Some(parent) = counted_parent.and_then(|parent| parent.upgrade().ok()) {
        with_record(&parent, |record| {
            record.live_children = record.live_children.saturating_sub(1);
        });
    }
    if live_children == 0 || popup.xdg_popup().client().is_none() {
        return;
    }
    tracing::warn!(
        client = ?popup.wl_surface().client().map(|client| client.id()),
        live_children,
        "xdg_popup destroyed while it still has child popups; disconnecting the client"
    );
    if let Some(data) = popup.xdg_popup().data::<XdgShellSurfaceUserData>() {
        data.xdg_surface().post_error(
            xdg_wm_base::Error::NotTheTopmostPopup as u32,
            "not_the_topmost_popup: an xdg_popup was destroyed while it still had child popups",
        );
    }
}

#[cfg(test)]
mod tests;
