//! How deep `wl_subsurface`s may nest: at most [`MAX_SUBSURFACE_DEPTH`]
//! levels below a surface tree's root, whatever order a client builds the
//! tree in.
//!
//! # Why the depth has to be bounded
//!
//! A `wl_subsurface` can be the parent of another, so a client can nest
//! them as deep as it likes, and every walk Smithay makes over a surface
//! tree recurses once per level at the pinned rev
//! (`src/wayland/compositor/tree.rs`): `PrivateSurfaceData::map`, behind
//! `with_surface_tree_downward`/`_upward`, which the render path runs over
//! every window's tree each frame; `commit_sync_surface_tree`, run for a
//! synchronized child on its parent's commit; `is_effectively_sync`, run
//! up a subsurface's chain on each of its commits; and `is_ancestor`, run
//! up the new parent's chain by `set_parent` on every
//! `wl_subcompositor.get_subsurface`. scoot's own `resend_scale_tree`
//! (`output_scale.rs`) recurses the same way. Measured before this bound
//! (the `popup_parent/tests` harness, one batch of nested subsurfaces
//! under a window, then a frame): a debug build overflowed the 2 MB test
//! stack at 3000 levels, a release build at 10000, and a release build on
//! an 8 MB stack -- a real session's main thread -- at 30000. A stack
//! overflow aborts the compositor, and every client goes down with it.
//!
//! # The rule
//!
//! Only one thing links two surfaces: `set_parent`, and its one caller,
//! Smithay's `wl_subcompositor.get_subsurface` handler. Everything else that
//! touches the tree removes a link -- `wl_subsurface`'s destructor
//! (`unset_parent`) and a `wl_surface`'s (`cleanup`, which orphans its
//! children) -- and a removal can only shorten a chain. So a bound checked
//! at every link holds for good. Unlike the popup tree (see
//! `popup_parent.rs`), there is no second structure that can disagree with
//! the one the walks follow: a surface's `parent` and its parent's
//! `children` are written together at all three sites.
//!
//! What the check cannot do is look only at the new link, because a link
//! does not always add a leaf. A surface's subsurface role outlives its
//! `wl_subsurface`, so after `wl_subsurface.destroy` -- or after its parent
//! `wl_surface` is destroyed -- the protocol lets it be made a subsurface
//! again, under a different parent, with all of its own subsurfaces still
//! attached. And a surface with no role at all can be given subsurfaces
//! before it is made one itself. A cap on the new parent's depth alone is
//! bypassed by building a tree bottom-up, a capped piece at a time, each
//! piece attached under the tip of the next: every link is shallow when it
//! is made, and the tree ends up as deep as the client likes. So
//! [`reject_too_deep`] admits a link only if the deepest surface it would
//! put anywhere is still within the cap: the new parent's depth, plus one,
//! plus the height of the subtree being attached.
//!
//! That height is not walked for. Each surface carries an upper bound on
//! its subtree's height ([`SubtreeHeight`]), raised along the new parent's
//! chain whenever a link is made ([`record_link`]) and never lowered, so
//! the check costs one walk up a chain of at most [`MAX_SUBSURFACE_DEPTH`]
//! steps and a read, however wide the client's trees are. Never lowering it
//! is what keeps it a bound -- scoot is not told when a link goes away --
//! and it errs in one direction only: a surface that once had a subtree
//! nearly [`MAX_SUBSURFACE_DEPTH`] deep, and was later re-attached, can be
//! refused as if it still had it. No real client comes anywhere near that
//! (see [`MAX_SUBSURFACE_DEPTH`]).
//!
//! The check runs from `dispatch.rs`'s blanket `request`, before Smithay
//! sees the request, because Smithay links the surfaces -- and runs
//! `is_ancestor` up the new parent's chain -- before it calls
//! `CompositorHandler::new_subsurface`, and does not pass that the
//! `wl_subcompositor` the error belongs on. Refused there, the link is
//! never made. That also bounds `is_ancestor` itself: the chain it walks
//! is one this rule already admitted.

use std::sync::atomic::{AtomicUsize, Ordering};

use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_subcompositor;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::compositor::{get_parent, with_states};

/// The most levels of `wl_subsurface` a surface tree may hold below its
/// root: a toplevel's own subsurface is level 1, the 64th nested one is
/// admitted and a 65th refused.
///
/// Real clients nest subsurfaces a level or two deep -- a video under its
/// player's window, GTK 4's offloaded textures, a browser's compositor
/// layers, now and then a subsurface of one of those -- so this is an order
/// of magnitude above anything a toolkit does, and it is far below where
/// Smithay's recursion becomes a problem: the test suite draws a tree this
/// deep, synchronized and not, in a debug build on a 2 MB test-thread
/// stack, where 3000 levels overflowed it. It is also the popup cap
/// (`popup_parent::MAX_POPUP_DEPTH`), and the two do not multiply on the
/// stack: a popup's surface tree is walked on its own, never from inside
/// the walk of its parent's popups (`PopupManager::popups_for_surface`
/// collects the popups first). `tests/bench.rs` measures a frame with both
/// at their caps.
pub(super) const MAX_SUBSURFACE_DEPTH: usize = 64;

/// An upper bound on how many subsurface levels hang below a surface, in
/// its data map. Absent means none has ever been attached: zero.
///
/// An atomic rather than a `Mutex`, because nothing else is read or written
/// with it: every read and every raise is a single operation.
#[derive(Default)]
struct SubtreeHeight(AtomicUsize);

/// The bound on `surface`'s subtree height; 0 if nothing was ever attached
/// under it.
fn subtree_height(surface: &WlSurface) -> usize {
    with_states(surface, |states| {
        states
            .data_map
            .get::<SubtreeHeight>()
            .map_or(0, |height| height.0.load(Ordering::Relaxed))
    })
}

/// Refuses a `wl_subcompositor.get_subsurface` that would nest a surface
/// deeper than [`MAX_SUBSURFACE_DEPTH`] -- posting `bad_parent` on
/// `subcompositor`, which disconnects the client -- and returns whether it
/// did. The caller must not pass a refused request on: returning without
/// initializing its `wl_subsurface` is safe for the reasons `dispatch.rs`'s
/// module doc records.
///
/// A request Smithay refuses anyway, because `parent` is `surface` or one of
/// its descendants, is left to Smithay, so that client gets the error it
/// always did. Every other request is checked, including ones Smithay goes
/// on to refuse for another reason (a surface with another role, or already
/// a subsurface): the client is disconnected either way.
///
/// Bounded whatever the tree looks like: the walk up from `parent` stops
/// past [`MAX_SUBSURFACE_DEPTH`] links, which it can only reach if the rule
/// had somehow been broken already, and refuses.
pub(super) fn reject_too_deep(
    subcompositor: &impl Resource,
    surface: &WlSurface,
    parent: &WlSurface,
) -> bool {
    let mut depth = 0usize;
    let mut cursor = parent.clone();
    loop {
        if cursor == *surface {
            return false;
        }
        match get_parent(&cursor) {
            None => break,
            Some(next) => {
                depth += 1;
                if depth > MAX_SUBSURFACE_DEPTH {
                    break;
                }
                cursor = next;
            }
        }
    }
    let deepest = depth
        .saturating_add(1)
        .saturating_add(subtree_height(surface));
    if deepest <= MAX_SUBSURFACE_DEPTH {
        return false;
    }
    tracing::warn!(
        client = ?surface.client().map(|client| client.id()),
        deepest,
        "refusing a wl_subsurface nested deeper than {MAX_SUBSURFACE_DEPTH}; \
         disconnecting the client"
    );
    subcompositor.post_error(
        wl_subcompositor::Error::BadParent,
        format!(
            "bad_parent: subsurfaces nest at most {MAX_SUBSURFACE_DEPTH} deep, and this one \
             would put a surface {deepest} deep"
        ),
    );
    true
}

/// `CompositorHandler::new_subsurface`: `surface` has just been linked under
/// `parent`, so raises every ancestor's [`SubtreeHeight`] to cover the
/// subtree that now hangs below it.
///
/// Walks at most [`MAX_SUBSURFACE_DEPTH`] ancestors: [`reject_too_deep`]
/// admitted this link only because the chain up from `parent` is shorter
/// than that.
pub(super) fn record_link(surface: &WlSurface, parent: &WlSurface) {
    let mut height = subtree_height(surface).saturating_add(1);
    let mut cursor = Some(parent.clone());
    for _ in 0..MAX_SUBSURFACE_DEPTH {
        let Some(ancestor) = cursor else {
            return;
        };
        with_states(&ancestor, |states| {
            states
                .data_map
                .get_or_insert_threadsafe(SubtreeHeight::default)
                .0
                .fetch_max(height, Ordering::Relaxed);
        });
        height = height.saturating_add(1);
        cursor = get_parent(&ancestor);
    }
}

#[cfg(test)]
mod tests;
