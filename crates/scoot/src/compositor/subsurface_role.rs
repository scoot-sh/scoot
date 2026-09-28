//! Which surfaces currently hold a live `wl_subsurface` role object.
//!
//! # The rule
//!
//! `wl_subcompositor.get_subsurface` says the to-be subsurface "must not
//! have an existing `wl_subsurface` object. Otherwise the `bad_surface`
//! protocol error is raised." The pinned Smithay checks for a *parent*
//! instead (`PrivateSurfaceData::set_parent` in
//! `src/wayland/compositor/tree.rs` refuses only when the child already has
//! a parent): when a subsurface's parent `wl_surface` is destroyed,
//! `cleanup` in the same file clears the child's parent while its
//! `wl_subsurface` lives on, so a second `get_subsurface` for it is
//! accepted. The surface then has two role objects, and destroying the older
//! one runs `unset_parent`, detaching it from the parent the newer one gave
//! it. Not a crash and not a depth path (`subsurface_depth.rs` still covers
//! every link) -- one unenforced protocol rule plus a tree position that can
//! change under a live role object.
//!
//! So scoot tracks it: every surface with a live `wl_subsurface` is filed
//! here, and a `get_subsurface` for one is refused with `bad_surface` in
//! `dispatch.rs` before Smithay links anything.
//!
//! # Why the bookkeeping is exact
//!
//! Filed in `CompositorHandler::new_subsurface`, which Smithay calls only
//! after `set_parent` succeeded and the new role object was created -- never
//! for a refused request, so a refusal cannot file an entry for an object
//! that does not exist. Forgotten on both destruction paths, which is what
//! keeps a leaked entry from wrongly refusing a legitimate later subsurface
//! (and a dropped one from reopening the hole):
//!
//! - the role object dies (`wl_subsurface.destroy`, disconnect cleanup, or a
//!   protocol-error kill -- all of which run the blanket `destroyed`, which
//!   downcasts the user data back to `SubsurfaceUserData` for its surface);
//! - the surface itself dies while its role object lives on (a hostile
//!   create-subsurface-destroy-surface loop would otherwise grow this set
//!   without bound; a dead surface can never be re-subsurfaced, so dropping
//!   its entry reopens nothing).
//!
//! Keyed by [`ObjectId`](smithay::reexports::wayland_server::backend::ObjectId),
//! which carries the client id and generation serial as well as the bare id,
//! so a stale entry could never equal a later surface even without the
//! surface-death forget.
//!
//! Not per frame and not per event: entries are filed and forgotten once per
//! subsurface lifetime, and the guard is one hash lookup. `get_subsurface`
//! is rare -- no toolkit re-parents subsurfaces in a hot loop -- so a
//! `HashSet` insert per link is nothing next to the link itself.

use std::collections::HashSet;

use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::backend::ObjectId;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;

/// Every surface currently holding a live `wl_subsurface` role object, by
/// surface id. Filed in `CompositorHandler::new_subsurface`, forgotten in
/// `dispatch.rs`'s destruction hooks -- see the module doc. Never lowered
/// except by those two forgets, and neither forget can drop an entry whose
/// surface could still be re-subsurfaced: the role forget names the surface
/// the dead role belonged to, and the surface forget names a surface that no
/// longer exists.
#[derive(Default)]
pub(super) struct LiveSubsurfaces(HashSet<ObjectId>);

impl LiveSubsurfaces {
    /// `surface` has just been linked under a parent with a fresh role
    /// object: file it. Called only from `new_subsurface`, so only for
    /// links Smithay actually made.
    pub(super) fn note_linked(&mut self, surface: &WlSurface) {
        self.0.insert(surface.id());
    }

    /// Forget whatever `id` names -- a dead role object's surface, or a dead
    /// surface. Removing an absent id is a no-op: most surfaces were never
    /// subsurfaces, and a disconnect destroys each object exactly once.
    pub(super) fn forget(&mut self, id: ObjectId) {
        self.0.remove(&id);
    }

    /// Whether `surface` already holds a live `wl_subsurface`: the
    /// `get_subsurface` the guard is looking at would be its second.
    pub(super) fn has_live_role(&self, surface: &WlSurface) -> bool {
        self.0.contains(&surface.id())
    }
}
