//! `ext-foreign-toplevel-list-v1`: the window list an external tool reads.
//!
//! `ext-workspace-v1` (see `ext_workspace.rs`) tells a bar which *workspaces*
//! exist; this one tells it which *windows* do. It is the protocol-side twin
//! of what `flexwm msg windows` already answers over flexwm's own IPC, for the
//! clients that have no reason to speak a compositor-specific protocol: a
//! taskbar's window list, an alt-tab switcher, a dock. Both Quickshell shells
//! probed against flexwm (see `docs/backlog/protocols/`) show an
//! "Applications" section and no "Windows" one for exactly this reason --
//! their generic toplevel widget had no global to bind.
//!
//! ## Why this protocol, and not `wlr-foreign-toplevel-management-unstable-v1`
//!
//! The same reason `ext-workspace-v1` is here instead of the wlr workspace
//! protocols: it is the compositor-agnostic successor, and `CLAUDE.md`'s
//! standing preference is for the `ext-` protocol where one exists. The pinned
//! Smithay rev agrees -- it carries `wayland::foreign_toplevel_list` and
//! nothing at all for the wlr protocol -- so this is also the one that comes
//! with a maintained implementation rather than a hand-rolled one.
//!
//! ## Enumeration only, and that is the whole protocol
//!
//! `ext-foreign-toplevel-list-v1` is "intentionally minimalistic" in its own
//! words: a handle per toplevel carrying `identifier`, `title` and `app_id`,
//! and nothing else. There is no `activate`, no `close`, no `minimize`, no
//! geometry and no per-output state -- those live in separate extension
//! protocols that do not exist yet. So there is nothing here to decide about
//! control: a client that wants to *act* on a window uses flexwm's IPC
//! (`flexwm msg action focus-window-id N`), which the identifier below is the
//! bridge to.
//!
//! ## The identifier, and why it is shaped like that
//!
//! `<generation>-<window id>`, e.g. `3f9c1e07-12`: eight hex digits of
//! per-session randomness, a dash, and the window's flexwm id in decimal.
//!
//! - The **suffix is the id `flexwm msg windows` reports**, so a tool that
//!   enumerates windows through this protocol can then act on one through
//!   IPC. Since the protocol itself has no control requests, that bridge is
//!   what makes enumeration actionable at all.
//! - The **prefix is the "opaque generation value" the protocol recommends**.
//!   flexwm's window ids restart from 1 in every process, so without it a tool
//!   that remembered an identifier across a compositor restart would silently
//!   match a different window; with it, identifiers from two sessions never
//!   collide.
//!
//! Within one session the ids are never reused ([`State::next_id`] only ever
//! increments), which is what the protocol requires of an identifier.
//!
//! ## What "a toplevel exists" means here
//!
//! The protocol describes handles as standing for *mapped* toplevels. flexwm
//! has no map/unmap boundary to hang that on: a window enters the layout, the
//! IPC window list and the focus order when its `xdg_toplevel` is created (see
//! `shell.rs`'s [`State::add_window`]), and a client committing a null buffer
//! afterwards does not take it back out. So a handle here covers exactly the
//! lifetime flexwm itself treats as a window's -- creation to destruction --
//! and the list is always identical to `flexwm msg windows`. The visible
//! consequence is a window appearing in a taskbar a few milliseconds before it
//! has drawn anything, with the empty title and app id it was created with;
//! the alternative (inventing a map/unmap notion for this protocol alone)
//! would mean a compositor whose own two window lists disagree.
//!
//! ## While the session is locked
//!
//! Nothing changes: handles stay, titles keep updating, new windows are still
//! announced. That matches the IPC side (`flexwm msg windows` "still lists
//! your windows while locked, titles included" -- see `README.md`) and the
//! trust model the whole compositor already states: a process that can reach
//! this wayland socket runs as the same user and is inside the boundary
//! already. Sending `closed` for every window on lock would also be a lie --
//! the windows did not close -- and would burn their identifiers, since the
//! protocol forbids reusing one when the window came back.
//!
//! ## Upstream notes
//!
//! Two things about the pinned rev's implementation that this module depends
//! on, recorded because neither is visible from the call sites here:
//!
//! - Its request handlers end in `_ => unreachable!()`, which
//!   `ext_workspace.rs` deliberately avoids for a future protocol version's
//!   sake. It is unreachable at version 1 -- wayland-backend rejects an opcode
//!   the interface does not define before dispatch ever reaches a handler --
//!   and it is upstream code, not this module's to change. A version 2 of
//!   this protocol would need the Smithay bump that defines it anyway.
//! - `ForeignToplevelListState::remove_toplevel` computes a position over the
//!   *upgradable* handles and then removes that index from the full `Vec`, so
//!   a dead entry earlier in the list would make it remove the wrong one.
//!   [`State::close_foreign_toplevel`] is the only place a handle is dropped,
//!   and it removes before it drops, so no dead entry is ever in that list
//!   when this is called.

use std::collections::HashMap;

use flexwm_core::{WindowId, WindowInfo};
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::wayland::foreign_toplevel_list::{
    ForeignToplevelHandle, ForeignToplevelListHandler, ForeignToplevelListState,
};

use super::State;

#[cfg(test)]
mod tests;

/// How many hex digits of per-session randomness prefix every identifier.
///
/// Four bytes' worth. The prefix only has to make identifiers from two
/// sessions unlikely to collide -- nothing depends on it being unguessable --
/// and it is part of a string the protocol caps at 32 bytes, which
/// [`identifier`] spends the rest of on the window id.
const GENERATION_HEX: usize = 8;

/// Everything this compositor keeps for `ext-foreign-toplevel-list-v1`.
#[derive(Debug)]
pub struct ForeignToplevels {
    /// The global, the bound list objects, and the per-toplevel protocol
    /// objects -- all owned by Smithay.
    list: ForeignToplevelListState,
    /// One entry per window flexwm currently has, keyed the same way
    /// [`State::windows`] is.
    ///
    /// Written in exactly two places, both in `shell.rs`'s window lifecycle:
    /// [`State::open_foreign_toplevel`] inserts, and
    /// [`State::close_foreign_toplevel`] removes. That pairing is what keeps
    /// this map in step with `State::windows` -- and what keeps a handle from
    /// ever being dropped without being removed from Smithay's own list
    /// first (see this module's doc for why that ordering matters).
    handles: HashMap<WindowId, ForeignToplevelHandle>,
    /// The generation prefix every identifier from this process carries, as
    /// [`GENERATION_HEX`] hex digits. Drawn once at startup; see
    /// [`identifier`].
    generation: String,
}

impl ForeignToplevels {
    /// Creates the `ext_foreign_toplevel_list_v1` global.
    ///
    /// No client filter, for the same reason the session-lock, data-control
    /// and input-method globals have none: flexwm has no security-context
    /// support, so an allow-list would be theatre (see `README.md`'s trust
    /// note). The protocol explicitly leaves this to compositor policy.
    pub fn new(dh: &DisplayHandle) -> Self {
        Self {
            list: ForeignToplevelListState::new::<State>(dh),
            handles: HashMap::new(),
            generation: generation(),
        }
    }
}

impl ForeignToplevelListHandler for State {
    fn foreign_toplevel_list_state(&mut self) -> &mut ForeignToplevelListState {
        &mut self.foreign_toplevels.list
    }
}

impl State {
    /// Announces a new window to every client watching the list.
    ///
    /// Called from [`State::add_window`](super::State) with the info that
    /// window was created with, which for nearly every client is empty at
    /// this point: a toolkit sends `set_app_id`/`set_title` just after
    /// `get_toplevel`, and each of those arrives here as its own
    /// [`State::publish_foreign_toplevel`] batch. That is what `done` is for,
    /// and why this sends one of its own (Smithay's `init_new_instance`) --
    /// a client draws on `done`, not on each event.
    pub(super) fn open_foreign_toplevel(&mut self, id: WindowId, info: &WindowInfo) {
        let foreign = &mut self.foreign_toplevels;
        let identifier = identifier(&foreign.generation, id);
        let handle = foreign.list.new_toplevel_with_identifier::<State>(
            info.title.as_str(),
            info.app_id.as_str(),
            identifier,
        );
        // `add_window` allocates a fresh id for every window, so this never
        // displaces a live handle -- which would drop it here without taking
        // it out of Smithay's own list first, leaving a dead entry there (see
        // this module's doc on `remove_toplevel`).
        let displaced = foreign.handles.insert(id, handle);
        debug_assert!(displaced.is_none(), "a window id was reused: {id:?}");
    }

    /// Tells every client watching that a window is gone.
    ///
    /// Called from [`State::remove_window`](super::State). `remove_toplevel`
    /// sends `closed` and takes the handle out of Smithay's own list before
    /// the strong handle is dropped here, which is required -- see this
    /// module's doc.
    pub(super) fn close_foreign_toplevel(&mut self, id: WindowId) {
        let foreign = &mut self.foreign_toplevels;
        // `None` for a window that never had a handle, which nothing can
        // produce today: the two writers are paired with `State::windows`'s
        // own insert and remove.
        let Some(handle) = foreign.handles.remove(&id) else {
            return;
        };
        foreign.list.remove_toplevel(&handle);
    }

    /// Publishes a window's current title and app id.
    ///
    /// Called from [`State::refresh_window`](super::State), whose only
    /// callers are Smithay's `title_changed` and `app_id_changed` -- and the
    /// pinned rev raises those only when the value really changed (it
    /// compares against the role's stored one first). So this sends the one
    /// event that changed, and exactly one `done` to close it: the other
    /// `send_*` is dropped by Smithay's own equality check, which is why
    /// nothing here has to compare anything itself.
    pub(super) fn publish_foreign_toplevel(&mut self, id: WindowId, info: &WindowInfo) {
        let Some(handle) = self.foreign_toplevels.handles.get(&id) else {
            return;
        };
        handle.send_title(&info.title);
        handle.send_app_id(&info.app_id);
        handle.send_done();
    }
}

/// The `identifier` for `id`: the session's generation prefix, a dash, and the
/// window id in decimal.
///
/// The length bound is not decoration. Smithay's
/// `new_toplevel_with_identifier` *asserts* the identifier is non-empty,
/// ASCII and at most 32 bytes -- a panic in a compositor is every client's
/// session -- so this is written to be provably inside it:
/// [`GENERATION_HEX`] (8) + `-` (1) + at most 20 digits for a `u64` = 29
/// bytes, all ASCII. `generation` is produced only by [`generation`] below,
/// which always yields exactly [`GENERATION_HEX`] hex digits.
fn identifier(generation: &str, id: WindowId) -> String {
    debug_assert_eq!(generation.len(), GENERATION_HEX, "a generation prefix");
    // `u64`'s decimal form is at most 20 ASCII digits.
    format!("{generation}-{}", id.0)
}

/// Eight hex digits of randomness for this process's identifiers.
///
/// Falls back to the pid when the kernel's pool cannot be read -- which
/// `getrandom(2)` only fails at for a bad buffer or an interrupted *blocking*
/// first read, neither reachable here. A pid is a worse generation value (two
/// sessions can share one) but it is a fixed-width, non-panicking answer, and
/// the alternative in a compositor's startup path is not a better one.
fn generation() -> String {
    let mut bytes = [0u8; 4];
    let mut filled = 0;
    while filled < bytes.len() {
        match rustix::rand::getrandom(&mut bytes[filled..], rustix::rand::GetRandomFlags::empty()) {
            // Short only if a signal interrupted it, which is not an error.
            Ok(read) if read > 0 => filled += read,
            _ => {
                tracing::debug!("no randomness for the toplevel identifier prefix; using the pid");
                return format!("{:0width$x}", std::process::id(), width = GENERATION_HEX);
            }
        }
    }
    // Exactly `GENERATION_HEX` digits: `u32` is four bytes, zero-padded.
    format!(
        "{:0width$x}",
        u32::from_ne_bytes(bytes),
        width = GENERATION_HEX
    )
}
