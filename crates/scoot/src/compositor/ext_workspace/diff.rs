//! The pure half of `ext-workspace-v1`: what a client has to be told to get
//! from the workspace list it was last shown to the one the core has now.
//!
//! No Wayland types here on purpose. Getting this wrong is invisible to a
//! type checker and awkward to observe over a socket -- a doubled `state`
//! event, a `removed` sent to a handle that was also just told it is active,
//! a `done` with nothing before it -- so it is a plain function over two
//! [`Workspaces`] snapshots, exercised directly by the tests below.
//! `ext_workspace.rs` only executes the list it returns.

use flexwm_core::Workspaces;

#[cfg(test)]
mod tests;

/// One step of bringing a client up to date. Indices are positions in the
/// output's workspace list, which is also the position of the handle
/// representing it (see `ext_workspace.rs`'s module doc on positional
/// identity).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Change {
    /// A workspace the client has no handle for yet. The handle is created
    /// carrying `active` in its first `state` event, rather than being
    /// created and then immediately restated.
    Added { index: usize, active: bool },
    /// An existing handle whose `state` changed.
    Restated { index: usize, active: bool },
    /// A workspace that no longer exists. Always a suffix of the list: a
    /// workspace disappears by the list getting shorter, never by a hole
    /// appearing in the middle of it.
    Removed { index: usize },
}

/// Appends what changed between `published` (what clients were last told) and
/// `current` (what the core says now) to `out`.
///
/// `out` is a caller-owned buffer rather than a returned `Vec` so the one
/// caller can keep reusing it; this runs on every `State::apply`, which is
/// every window open, close, title change and layout action.
///
/// Empty exactly when the two snapshots are equal, which is what lets the
/// caller use "nothing to do" and "send no `done`" as the same condition.
pub(super) fn changes(published: Workspaces, current: Workspaces, out: &mut Vec<Change>) {
    // Handles that exist on both sides of the change. Anything at or past
    // this is either being created or being removed, and must not also be
    // restated: a `state` event on a handle that is about to be `removed` is
    // noise at best, and one on a handle that does not exist yet is a bug.
    let surviving = published.count.min(current.count);

    // The workspace that was active and no longer is. Skipped when it is
    // being removed outright, and when it is still the active one.
    if published.active < surviving && published.active != current.active {
        out.push(Change::Restated {
            index: published.active,
            active: false,
        });
    }
    // ...and the one that now is. Only when the client already has a handle
    // for it that is also still there afterwards: a newly created handle
    // carries `active` from the start, and a list with no workspaces at all
    // has nothing to restate (`Workspaces::default()`, the snapshot before an
    // output exists, is that case -- `active` is 0 there because the type has
    // to say something, not because workspace 0 is active).
    if current.active < surviving && current.active != published.active {
        out.push(Change::Restated {
            index: current.active,
            active: true,
        });
    }
    // Last one first, so a consumer replaying these onto a positional list
    // only ever drops its own last element -- an ascending order would have
    // it removing from the middle of a list whose tail it has not been told
    // about yet.
    for index in (current.count..published.count).rev() {
        out.push(Change::Removed { index });
    }
    for index in published.count..current.count {
        out.push(Change::Added {
            index,
            active: index == current.active,
        });
    }
}
