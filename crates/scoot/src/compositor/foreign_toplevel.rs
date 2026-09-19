//! `ext-foreign-toplevel-list-v1`: the window list an external tool reads.
//!
//! `ext-workspace-v1` (see `ext_workspace.rs`) tells a bar which *workspaces*
//! exist; this one tells it which *windows* do. It is the protocol-side twin
//! of what `scoot msg windows` already answers over scoot's own IPC, for the
//! clients that have no reason to speak a compositor-specific protocol: a
//! taskbar's window list, an alt-tab switcher, a dock.
//!
//! ## Why this protocol, and why it is not the only one
//!
//! It is here for the same reason `ext-workspace-v1` is here instead of the
//! wlr workspace protocols: it is the compositor-agnostic successor, and
//! `CLAUDE.md`'s standing preference is for the `ext-` protocol where one
//! exists.
//!
//! It is not the only one because measurement said so. Stock `quickshell`
//! 0.3.1 -- the build both Quickshell shells probed against scoot (DMS,
//! Noctalia) run on -- is offered this global and **never binds it**: its
//! `ToplevelManager` is a `zwlr_foreign_toplevel_manager_v1` client. So
//! `foreign_toplevel_management.rs` implements the older wlr protocol
//! alongside this one, from the same three window-lifecycle events, and the
//! two lists are one window list described twice. This module is what a
//! standards-following client gets; see
//! `docs/backlog/resolved/wlr-foreign-toplevel-management-done.md` for the
//! measurement and the decision.
//!
//! ## Enumeration only, and that is the whole protocol
//!
//! `ext-foreign-toplevel-list-v1` is "intentionally minimalistic" in its own
//! words: a handle per toplevel carrying `identifier`, `title` and `app_id`,
//! and nothing else. There is no `activate`, no `close`, no `minimize`, no
//! geometry and no per-output state -- those live in separate extension
//! protocols that do not exist yet. So there is nothing here to decide about
//! control: a client that wants to *act* on a window uses scoot's IPC
//! (`scoot msg action focus-window-id N`), which the identifier below is the
//! bridge to.
//!
//! ## The identifier, and why it is shaped like that
//!
//! `<generation>-<window id>`, e.g. `3f9c1e07-12`: eight hex digits of
//! per-session randomness, a dash, and the window's scoot id in decimal.
//!
//! - The **suffix is the id `scoot msg windows` reports**, so a tool that
//!   enumerates windows through this protocol can then act on one through
//!   IPC. Since the protocol itself has no control requests, that bridge is
//!   what makes enumeration actionable at all.
//! - The **prefix is the "opaque generation value" the protocol recommends**.
//!   scoot's window ids restart from 1 in every process, so without it a tool
//!   that remembered an identifier across a compositor restart would silently
//!   match a different window; with it, identifiers from two sessions never
//!   collide.
//!
//! Within one session the ids are never reused ([`State::next_id`] only ever
//! increments), which is what the protocol requires of an identifier.
//!
//! ## What "a toplevel exists" means here
//!
//! The protocol describes handles as standing for *mapped* toplevels. scoot
//! has no map/unmap boundary to hang that on: a window enters the layout, the
//! IPC window list and the focus order when its `xdg_toplevel` is created (see
//! `shell.rs`'s [`State::add_window`]), and a client committing a null buffer
//! afterwards does not take it back out. So a handle here covers exactly the
//! lifetime scoot itself treats as a window's -- creation to destruction --
//! and the list is always identical to `scoot msg windows`. The visible
//! consequence is a window appearing in a taskbar a few milliseconds before it
//! has drawn anything, with the empty title and app id it was created with;
//! the alternative (inventing a map/unmap notion for this protocol alone)
//! would mean a compositor whose own two window lists disagree.
//!
//! ## While the session is locked
//!
//! Nothing changes: handles stay, titles keep updating, new windows are still
//! announced. That matches the IPC side (`scoot msg windows` "still lists
//! your windows while locked, titles included" -- see `docs/protocols.md`) and
//! the trust model the whole compositor already states: a process that can
//! reach this wayland socket runs as the same user and is inside the boundary
//! already. Sending `closed` for every window on lock would also be a lie --
//! the windows did not close -- and would burn their identifiers, since the
//! protocol forbids reusing one when the window came back.
//!
//! ## Why hand-rolled instead of Smithay's `ForeignToplevelListState`
//!
//! The pinned Smithay rev *has* an implementation of this protocol, and this
//! module used it until per-client bind accounting landed. It had to go, for
//! one reason: nothing in its API intercepts a bind. `ForeignToplevelListState`
//! keeps its bound lists and its toplevels in private fields with no accessor,
//! its global data is unconstructible, and its bind announces every existing
//! window to the new list unconditionally -- so there is no place to count the
//! bind against [`crate::compositor::bind_budget`]'s shared budget, and no
//! place to refuse it either. The other three globals of that shape are all
//! scoot-owned already, so this one is too now: the same global, lifecycle
//! and event batching, mirroring `foreign_toplevel_management.rs` (which was
//! hand-rolled from the start for the same reason -- no Smithay support at
//! all), minus the control half this protocol does not have.
//!
//! What that means concretely: `stop` is answered with `finished` and the list
//! is forgotten; a bind past the shared budget is answered with `finished`
//! immediately and never registered, so no later window walks it. `finished`
//! here is *not* a destructor event (unlike the other three globals): the
//! protocol says "the client should destroy the object", so the object stays
//! valid until the client destroys it, and the `destroyed` hook repeats the
//! same idempotent release. Only `stop` and `destroy` exist as requests, and
//! the handlers ignore anything else rather than `unreachable!()` on it --
//! the request enums are `#[non_exhaustive]`, and an opcode a future version
//! defines never reaches here anyway.
//!
//! ## The per-bind cost, and who pays it
//!
//! One list object plus one handle object per window scoot currently has,
//! each stated and closed by its own `done` -- so a bind is O(windows), and a
//! window opening is O(bound lists). The budget above is what bounds the
//! product; see `bind_budget.rs` for the number and the refusal form.

use std::collections::BTreeMap;

use scoot_core::{WindowId, WindowInfo};
use smithay::reexports::wayland_protocols::ext::foreign_toplevel_list::v1::server::{
    ext_foreign_toplevel_handle_v1::{self, ExtForeignToplevelHandleV1},
    ext_foreign_toplevel_list_v1::{self, ExtForeignToplevelListV1},
};
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::{Client, DataInit, DisplayHandle, New, Resource};
use smithay::wayland::{Dispatch2, GlobalDispatch2};

use super::State;

#[cfg(test)]
mod tests;

/// The only version of this protocol there is.
const VERSION: u32 = 1;

/// How many hex digits of per-session randomness prefix every identifier.
///
/// Four bytes' worth. The prefix only has to make identifiers from two
/// sessions unlikely to collide -- nothing depends on it being unguessable --
/// and it is part of a string the protocol caps at 32 bytes, which
/// [`identifier`] spends the rest of on the window id.
const GENERATION_HEX: usize = 8;

/// Everything this compositor keeps for `ext-foreign-toplevel-list-v1`.
///
/// Split the way the wlr twin's state is, and for the same reason: a *list*
/// is one client's subscription to new windows, while a *handle* is one
/// client's view of one window, and the two have different lifetimes. `stop`
/// ends a subscription and leaves every handle it created working, which is
/// what the protocol's own teardown sequence (stop, wait for `finished`, then
/// destroy the handles) requires -- so handles cannot be owned by the list
/// that made them.
#[derive(Debug)]
pub struct ForeignToplevels {
    /// Every `ext_foreign_toplevel_list_v1` still subscribed to new windows.
    ///
    /// Entries leave on `stop` (answered with `finished`) and on the object's
    /// destruction, which are the only two ways a subscription ends.
    lists: Vec<ExtForeignToplevelListV1>,
    /// One entry per window scoot currently has, keyed the same way
    /// [`State::windows`] is, and ordered by window id -- which is creation
    /// order, since ids only increment. That ordering is what makes a fresh
    /// bind announce a session's windows oldest-first rather than in a hash
    /// order that would differ run to run.
    ///
    /// Written in exactly two places, both in `shell.rs`'s window lifecycle:
    /// [`State::open_foreign_toplevel`] inserts and
    /// [`State::close_foreign_toplevel`] removes. That pairing is what keeps
    /// this map in step with `State::windows`, and it is also the authority
    /// an incoming request would be checked against -- though this protocol
    /// has no request that names a window, so unlike the wlr twin nothing
    /// here resolves through it.
    toplevels: BTreeMap<WindowId, ExtToplevel>,
    /// The generation prefix every identifier from this process carries, as
    /// [`GENERATION_HEX`] hex digits. Drawn once at startup; see
    /// [`identifier`].
    generation: String,
}

impl ForeignToplevels {
    /// Creates the `ext_foreign_toplevel_list_v1` global.
    ///
    /// No client filter, for the same reason the session-lock, data-control
    /// and input-method globals have none: scoot has no security-context
    /// support, so an allow-list would be theatre (see `docs/protocols.md`'s
    /// trust note). The protocol explicitly leaves this to compositor policy
    /// -- and per-client *accounting* (see `bind_budget.rs`) is not a filter.
    /// Enumeration-only, so there is no write half to gate either.
    ///
    /// The `GlobalId` is dropped: nothing removes this global for the life of
    /// the process, and dropping the id does not remove it either.
    pub fn new(dh: &DisplayHandle) -> Self {
        let _ = dh.create_global::<State, ExtForeignToplevelListV1, _>(VERSION, ListGlobalData);
        Self {
            lists: Vec::new(),
            toplevels: BTreeMap::new(),
            generation: generation(),
        }
    }
}

/// One window, as every client watching it has been told about it.
///
/// The three described fields are a *published snapshot*, not a second copy
/// of the window's state: they mean "what every handle below has already been
/// sent". They are compared against the live [`WindowInfo`] on each refresh
/// so a change sends only the event that actually changed.
#[derive(Debug)]
struct ExtToplevel {
    title: String,
    app_id: String,
    identifier: String,
    /// One handle per client that has been told about this window -- several
    /// if a client bound the list more than once, which is legal.
    ///
    /// Entries leave when the client destroys a handle (see
    /// [`HandleData::destroyed`]); the whole entry goes when the window does.
    handles: Vec<ExtForeignToplevelHandleV1>,
}

impl ExtToplevel {
    /// States this window in full on a handle that has just been created, and
    /// closes the batch with `done`.
    ///
    /// The order Smithay's implementation used, kept so clients see the same
    /// bytes from the hand-rolled one: identifier, title, app id, then `done`.
    fn describe(&self, handle: &ExtForeignToplevelHandleV1) {
        handle.identifier(self.identifier.clone());
        handle.title(self.title.clone());
        handle.app_id(self.app_id.clone());
        handle.done();
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
    /// and why this sends one of its own -- a client draws on `done`, not on
    /// each event.
    pub(super) fn open_foreign_toplevel(&mut self, id: WindowId, info: &WindowInfo) {
        let dh = self.display_handle.clone();
        let foreign = &mut self.foreign_toplevels;
        let mut toplevel = ExtToplevel {
            title: info.title.clone(),
            app_id: info.app_id.clone(),
            identifier: identifier(&foreign.generation, id),
            handles: Vec::new(),
        };
        foreign.lists.retain(|list| {
            let Some(client) = list.client() else {
                // The object outlived its client, or died without `stop`
                // while its `destroyed` is still queued for cleanup (see
                // `bind_budget.rs`'s dead-but-unpruned transient).
                // `destroyed` removes it too; doing it here as well keeps a
                // burst of window openings from walking a dead entry once per
                // window. Skipped rather than sent to: an event on the dead
                // list would be swallowed, but the handle events below go on
                // a *live* object the client never heard of, which the client
                // would read as events for an unknown object.
                return false;
            };
            let created = client.create_resource::<ExtForeignToplevelHandleV1, _, State>(
                &dh,
                list.version(),
                HandleData { window: id },
            );
            let Ok(handle) = created else {
                // The one and only way this fails: the client is already gone
                // by the time it runs (`Handle::create_object` errors solely
                // from `get_client_mut` missing the client; the server's own
                // id allocation for a new object never fails). The list stays
                // subscribed -- it misses this window the way Smithay's
                // implementation skipped it, and its `destroyed` prunes it
                // when cleanup runs.
                return true;
            };
            list.toplevel(&handle);
            toplevel.describe(&handle);
            toplevel.handles.push(handle);
            true
        });
        // `add_window` allocates a fresh id for every window, so this never
        // displaces a live entry -- which would strand its handles, leaving a
        // taskbar showing a window nothing will ever send `closed` for.
        let displaced = foreign.toplevels.insert(id, toplevel);
        debug_assert!(displaced.is_none(), "a window id was reused: {id:?}");
    }

    /// Tells every client watching that a window is gone.
    ///
    /// Called from [`State::remove_window`](super::State). The entry leaves
    /// this map first and the handles are left behind on the client side,
    /// inert: the protocol says the server emits no further events on a
    /// `closed` handle, and this protocol has no request that could resolve
    /// through a stale one.
    pub(super) fn close_foreign_toplevel(&mut self, id: WindowId) {
        // `None` for a window that never had an entry, which nothing can
        // produce today: the two writers are paired with `State::windows`'s
        // own insert and remove.
        let Some(toplevel) = self.foreign_toplevels.toplevels.remove(&id) else {
            return;
        };
        for handle in &toplevel.handles {
            handle.closed();
        }
    }

    /// Publishes a window's current title and app id.
    ///
    /// Called from [`State::refresh_window`](super::State), whose only
    /// callers are Smithay's `title_changed` and `app_id_changed` -- and the
    /// pinned rev raises those only when the value really changed (it
    /// compares against the role's stored one first). So this is reached once
    /// per real change, and the comparison below narrows it further to the
    /// one field that moved, rather than re-sending the other one alongside
    /// it -- the same dedupe Smithay's own `send_title`/`send_app_id` did
    /// when this module was delegated to it.
    pub(super) fn publish_foreign_toplevel(&mut self, id: WindowId, info: &WindowInfo) {
        let Some(toplevel) = self.foreign_toplevels.toplevels.get_mut(&id) else {
            return;
        };
        let title_changed = toplevel.title != info.title;
        let app_id_changed = toplevel.app_id != info.app_id;
        if !title_changed && !app_id_changed {
            return;
        }
        // Rewritten in place rather than reassigned, so a terminal retitling
        // on every prompt reuses one buffer instead of allocating a string
        // per change (the per-handle `clone` below is the generated
        // signature's, and there is no way around that one). Same shape as
        // the wlr twin's publish, on purpose: the two lists are one window
        // list described twice.
        if title_changed {
            toplevel.title.clear();
            toplevel.title.push_str(&info.title);
        }
        if app_id_changed {
            toplevel.app_id.clear();
            toplevel.app_id.push_str(&info.app_id);
        }
        // One field across every handle before the next, then a single `done`
        // round -- the order the delegated implementation sent, kept so
        // clients see the same bytes from the hand-rolled one: a client
        // drawing a handle on its `done` still closes the right batch either
        // way, but the suites pin this order and nothing is gained by moving
        // it.
        for handle in &toplevel.handles {
            if title_changed {
                handle.title(toplevel.title.clone());
            }
            if app_id_changed {
                handle.app_id(toplevel.app_id.clone());
            }
        }
        for handle in &toplevel.handles {
            handle.done();
        }
    }

    /// Builds a freshly bound list's whole world: a handle per window that
    /// already exists, each described and closed by its own `done`.
    ///
    /// Announces oldest window first, which is what the [`BTreeMap`] keyed by
    /// window id gives for free.
    fn announce_foreign_toplevels(
        &mut self,
        dh: &DisplayHandle,
        client: &Client,
        list: ExtForeignToplevelListV1,
    ) {
        let version = list.version();
        let foreign = &mut self.foreign_toplevels;
        for (id, toplevel) in &mut foreign.toplevels {
            let created = client.create_resource::<ExtForeignToplevelHandleV1, _, State>(
                dh,
                version,
                HandleData { window: *id },
            );
            let Ok(handle) = created else {
                // The one and only way this fails: the client is already gone
                // by the time it runs (see `open_foreign_toplevel` for why
                // nothing else can fail). The list stays subscribed and misses
                // this window -- the delegated implementation's shape -- and
                // its `destroyed` prunes it when cleanup runs. Either way the
                // bind-time budget claim below is released with it, so the
                // count never leaks a bind nobody holds.
                continue;
            };
            list.toplevel(&handle);
            toplevel.describe(&handle);
            toplevel.handles.push(handle);
        }
        foreign.lists.push(list);
    }
}

/// The `identifier` for `id`: the session's generation prefix, a dash, and the
/// window id in decimal.
///
/// The length bound is not decoration. The protocol caps the identifier at 32
/// bytes -- a panic in a compositor is every client's session -- so this is
/// written to be provably inside it: [`GENERATION_HEX`] (8) + `-` (1) + at
/// most 20 digits for a `u64` = 29 bytes, all ASCII. `generation` is produced
/// only by [`generation`] below, which always yields exactly
/// [`GENERATION_HEX`] hex digits.
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

/// User data on the `ext_foreign_toplevel_list_v1` global itself.
struct ListGlobalData;

/// ...and on a list a client has bound.
struct ListData;

/// User data on an `ext_foreign_toplevel_handle_v1`: the window it stands for.
///
/// Storing the id is what makes a destroyed handle removable from its
/// window's entry without a map lookup -- and it is safe to keep past the
/// window's death because ids are never reused (see the module doc).
struct HandleData {
    window: WindowId,
}

impl GlobalDispatch2<ExtForeignToplevelListV1, State> for ListGlobalData {
    fn bind(
        &self,
        state: &mut State,
        dh: &DisplayHandle,
        client: &Client,
        resource: New<ExtForeignToplevelListV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        let list = data_init.init(resource, ListData);
        if state.bind_budget.refuse_bind(client, &list.id()) {
            // Over the shared per-client budget (see `bind_budget.rs`):
            // deferred like the other three rather than sent here, so every
            // refusal shares one timing story. (`finished` is *not* a
            // destructor event on this interface -- "the client should
            // destroy the object" -- so an inline send would survive the
            // bind epilogue; uniformity wins over the special case.) Not
            // counted, not registered, so no later window walks it.
            state.defer_bind_refusal(super::bind_budget::RefusedBind::ToplevelList(list));
            return;
        }
        state.announce_foreign_toplevels(dh, client, list);
    }
}

impl Dispatch2<ExtForeignToplevelListV1, State> for ListData {
    fn request(
        &self,
        state: &mut State,
        client: &Client,
        list: &ExtForeignToplevelListV1,
        request: ext_foreign_toplevel_list_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            ext_foreign_toplevel_list_v1::Request::Stop => {
                // Retracted before anything else: if this list was refused
                // for being over budget, its `finished` is still queued for
                // loop idle (see `bind_budget.rs`) -- and `finished` here is
                // a plain event, not a destructor, so without the retraction
                // the idle send would be a live duplicate. (The other three
                // capped globals need no equivalent: their `finished` events
                // destroy the object first, so a second send dies swallowed.)
                // Idempotent with the `destroyed` release -- removing an
                // absent id is a no-op.
                state.bind_budget.undefer_bind_refusal(&list.id());
                // Released synchronously rather than left to `destroyed`: a
                // stop-and-rebind in one batch must see the freed slot without
                // waiting for post-batch cleanup. Idempotent with the
                // `destroyed` release -- removing an absent id is a no-op.
                state.bind_budget.release_bind(&client.id(), &list.id());
                // Unregistered first: nothing more may be sent on this list
                // after `finished` ("the compositor must not send any more
                // toplevel events"), while the handles it created keep
                // reporting until the client destroys them -- which is what
                // the protocol's own teardown sequence (stop, wait for
                // `finished`, then destroy the handles) requires.
                // `destroyed` runs this same `retain` later, which is
                // idempotent.
                state.foreign_toplevels.lists.retain(|entry| entry != list);
                list.finished();
            }
            // `Destroy` among them: wayland-backend destroys the object
            // itself and `destroyed` below does the bookkeeping.
            ext_foreign_toplevel_list_v1::Request::Destroy => {}
            // The request enums are `#[non_exhaustive]`; an opcode this
            // version does not define never reaches here (wayland-backend
            // rejects it first), so there is nothing to do but ignore it --
            // and certainly not panic, which is what `unreachable!()` would
            // make of a future protocol version.
            _ => {}
        }
    }

    fn destroyed(&self, state: &mut State, client: ClientId, list: &ExtForeignToplevelListV1) {
        // The path an ordinary client disconnect takes, and the only thing
        // that stops a dead client's list from being walked on every window
        // opening -- and the path its budget claim takes back, including for
        // a bare destroy with no `stop` before it.
        state.bind_budget.release_bind(&client, &list.id());
        state.foreign_toplevels.lists.retain(|entry| entry != list);
    }
}

impl Dispatch2<ExtForeignToplevelHandleV1, State> for HandleData {
    fn request(
        &self,
        _state: &mut State,
        _client: &Client,
        _handle: &ExtForeignToplevelHandleV1,
        request: ext_foreign_toplevel_handle_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        // `Destroy` is this version's only request: wayland-backend destroys
        // the object itself and `destroyed` below does the bookkeeping.
        // Anything else is ignored -- the request enum is `#[non_exhaustive]`,
        // and an opcode this version does not define never reaches here
        // (wayland-backend rejects it first).
        if let ext_foreign_toplevel_handle_v1::Request::Destroy = request {}
    }

    fn destroyed(&self, state: &mut State, _client: ClientId, handle: &ExtForeignToplevelHandleV1) {
        // Covers both ends of a handle's life: a client destroying one early
        // (legal -- it just will not be given another for that window), and a
        // client disconnecting. `None` when the window closed first, in which
        // case the whole entry is already gone.
        let Some(toplevel) = state.foreign_toplevels.toplevels.get_mut(&self.window) else {
            return;
        };
        toplevel.handles.retain(|entry| entry != handle);
    }
}
