//! `wlr-foreign-toplevel-management-unstable-v1`: the window list a shell's
//! taskbar reads, and clicks on.
//!
//! scoot publishes its windows through two protocols at once, from the same
//! three window-lifecycle events. `foreign_toplevel.rs` speaks
//! `ext-foreign-toplevel-list-v1`, the compositor-agnostic successor
//! `CLAUDE.md` prefers and the pinned Smithay rev implements; this module
//! speaks the older wlr protocol, which is what the clients that exist today
//! actually bind.
//!
//! ## Why both, when the project's rule says prefer the `ext-` one
//!
//! Because measurement beat the rule, and only for this one protocol. Stock
//! `quickshell` 0.3.1 -- the build both DMS and Noctalia run on -- is offered
//! `ext_foreign_toplevel_list_v1` by scoot and **never binds it**: its
//! `ToplevelManager` is a `zwlr_foreign_toplevel_manager_v1` client, and with
//! a real window open it reports `count = 0`. The full measurement, and the
//! decision to implement the wlr protocol alongside rather than instead of the
//! `ext-` one, is in
//! `docs/backlog/resolved/wlr-foreign-toplevel-management-done.md`.
//!
//! So the `ext-` list stays, unchanged, and is still what a
//! standards-following client gets. This is the compatibility half, and the
//! two are driven from the same [`State::add_window`](super::State),
//! [`State::remove_window`](super::State) and
//! [`State::refresh_window`](super::State) choke points -- a client bound to
//! both sees one window list described twice, never two lists that can drift.
//!
//! Smithay has nothing for this protocol at the pinned rev (no
//! `foreign_toplevel_management` anywhere), so the global, the object
//! lifecycle and the event batching are here directly, hanging off
//! [`Dispatch2`]/[`GlobalDispatch2`] because `dispatch.rs` owns a blanket
//! `Dispatch` impl for [`State`]. That is not hand-rolled wire format: the
//! generated server bindings ship with `wayland-protocols-wlr` and are
//! re-exported through Smithay, exactly as `output_management.rs` and
//! `gamma_control.rs` already use them.
//!
//! ## Enumeration and three requests, and nothing invented
//!
//! This is a *control* protocol as much as an enumeration one, and scoot can
//! honestly answer only part of it. What it does:
//!
//! - **Enumerates** every window with its `title`, `app_id` and the output it
//!   is on -- the same windows, with the same lifetime, that `scoot msg
//!   windows` and the `ext-` list report.
//! - **Answers `activate`** by focusing that window, through the same
//!   `Action::FocusWindowId` an IPC `focus-window-id` and a click already use.
//! - **Answers `close`** by asking that window's `xdg_toplevel` to close --
//!   the same thing the `CloseFocused` action's `Effect::Close` does.
//! - **Answers `set_fullscreen`/`unset_fullscreen`** through
//!   `Action::SetFullscreen`, the same core rules the client's own request,
//!   `toggle-fullscreen` and `Super+f` follow (see `fullscreen.rs`), with the
//!   same output-hint policy as the client's own request.
//! - **Reports two state bits, `activated` and `fullscreen`.** `activated`
//!   comes from scoot's real window focus, reconciled in
//!   [`State::refresh_wlr_activation`] from the same `self.focus` that drives
//!   `xdg_toplevel`'s own `activated` and the focus ring -- one source of
//!   truth for "which window is focused", and all three read it.
//!   `fullscreen` comes from the arrangement's `Placement::fullscreen`,
//!   reconciled in [`State::refresh_wlr_fullscreen`] on every `apply`, the
//!   same flag that puts the bit on the window's own configure. It only
//!   exists from version 2, so a version 1 handle is never sent it.
//!
//! What it deliberately does not do: `set_maximized`, `set_minimized` (and
//! their `unset_` halves) and `set_rectangle` are accepted and ignored.
//! scoot's core has no concept of maximized or minimized at all -- deciding
//! what they would *mean* in a scrolling-column layout is layout design, not
//! wire format, and inventing a meaning here to fill in a protocol enum is
//! exactly the kind of speculative semantics that would then have to be
//! unpicked. (Fullscreen was in this list until the core grew a real
//! fullscreen state, for its own reasons, first.) So the corresponding state
//! bits are never sent either: a taskbar is told the truth (no window is ever
//! maximized or minimized) rather than a plausible-looking fiction.
//!
//! `set_rectangle` is a minimize-animation hint. wlroots validates it and
//! posts `invalid_rectangle` for a negative size because it *uses* the
//! rectangle; scoot reads nothing from it, so killing a taskbar's connection
//! over a number nothing will ever look at would be a worse answer than
//! ignoring it.
//!
//! ## `parent` is never sent, and version 3 is still advertised
//!
//! Version 3's one addition is the `parent` event. scoot's layout has no
//! parent/child relation -- every `xdg_toplevel` is an independent entry in a
//! column, dialogs included -- so there is no parent change to report and the
//! event never fires. That is the same picture a version 1 or 2 client sees,
//! which is why the version is not capped at 2: capping would take away a
//! client's ability to *ask* without adding a single fact to what it is told.
//!
//! ## What "a toplevel exists" means here
//!
//! Exactly what it means for the `ext-` list, and for the same reason
//! (`foreign_toplevel.rs`'s module doc has the long version): scoot has no
//! map/unmap boundary, so a handle covers a window's whole life, from
//! `xdg_toplevel` creation to destruction. The visible consequence is a window
//! reaching a taskbar a few milliseconds before it has drawn anything, with
//! the empty title and app id it was created with -- which is what `done`
//! batching is for.
//!
//! ## Handles, and why an id is never mistaken for another window's
//!
//! Every handle object carries the [`WindowId`] it stands for in its own user
//! data ([`HandleData`]), which is what an incoming `activate` or `close`
//! resolves through. [`State::next_id`](super::State) only ever increments, so
//! a window id is never reused inside a session -- a handle left alive by a
//! client after its window closed can therefore never resolve onto a
//! *different* window later. That is the same property the `ext-` list's
//! identifier depends on.
//!
//! ## While the session is locked
//!
//! The list stays live -- handles stay, titles keep updating, new windows are
//! still announced -- for the same reason the `ext-` list does (see
//! `docs/protocols.md`'s lock section: a process that can reach this socket is
//! a same-uid process inside the trust boundary already, and `closed` for a
//! window that did not close is a lie a taskbar cannot recover from).
//!
//! The *requests* are refused while locked, which is the difference this
//! protocol introduces. Each checks `SessionLock::is_locked` itself rather
//! than leaning on [`State::act`](super::State)'s gate -- `close` has no core
//! action to route through at all, `activate` has a fast path that
//! deliberately does not go through `act` either, and `set_fullscreen` may
//! move the window to another output before its `act` -- for exactly the
//! reason that gate exists: a window the user cannot see must not be focused,
//! closed or rearranged from behind the lock screen. `act` keeps its own check
//! as the backstop `shell.rs` describes it as.

use std::collections::BTreeMap;

use scoot_core::{Action, Arrangement, OutputId, WindowId, WindowInfo};
use smithay::desktop::Window;
use smithay::output::Output;
use smithay::reexports::wayland_protocols_wlr::foreign_toplevel::v1::server::zwlr_foreign_toplevel_handle_v1::{
    self, State as ToplevelState, ZwlrForeignToplevelHandleV1,
};
use smithay::reexports::wayland_protocols_wlr::foreign_toplevel::v1::server::zwlr_foreign_toplevel_manager_v1::{
    self, ZwlrForeignToplevelManagerV1,
};
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::{Client, DataInit, DisplayHandle, New, Resource};
use smithay::wayland::{Dispatch2, GlobalDispatch2};

use super::State;

#[cfg(test)]
mod tests;

/// The version advertised, which is the highest the protocol defines.
///
/// Version 2 adds `set_fullscreen`/`unset_fullscreen` (both answered, see the
/// module doc) and the `fullscreen` state bit; version 3 adds the `parent`
/// event (never sent). The `fullscreen` bit is the one version-gated thing
/// here: every *event* this module sends exists at version 1, but a value
/// inside the `state` array that a version 1 client's enum does not have is
/// not something to hand it -- see [`state_array`].
const VERSION: u32 = 3;

/// The first version whose `state` enum has `fullscreen`.
const FULLSCREEN_SINCE: u32 = 2;

/// Everything this compositor keeps for
/// `wlr-foreign-toplevel-management-unstable-v1`.
///
/// Split the way wlroots' own implementation is, and for the same reason: a
/// *manager* is one client's subscription to new windows, while a *handle* is
/// one client's view of one window, and the two have different lifetimes.
/// `stop` ends a subscription and leaves every handle it created working,
/// which is what the protocol's own teardown sequence (stop, wait for
/// `finished`, then destroy the handles) requires -- so handles cannot be
/// owned by the manager that made them.
#[derive(Debug, Default)]
pub struct ForeignToplevelManagement {
    /// Every `zwlr_foreign_toplevel_manager_v1` still subscribed to new
    /// windows.
    ///
    /// Entries leave on `stop` (answered with `finished`) and on the object's
    /// destruction, which are the only two ways a subscription ends.
    managers: Vec<ZwlrForeignToplevelManagerV1>,
    /// One entry per window scoot currently has, keyed the same way
    /// [`State::windows`](super::State) is, and ordered by window id -- which
    /// is creation order, since ids only increment. That ordering is what
    /// makes a fresh bind announce a session's windows oldest-first rather
    /// than in a hash order that would differ run to run.
    ///
    /// Written in exactly two places, both in `shell.rs`'s window lifecycle:
    /// [`State::open_wlr_toplevel`] inserts and [`State::close_wlr_toplevel`]
    /// removes. That pairing is what keeps this map in step with
    /// `State::windows`, and it is also the authority an incoming `activate`
    /// or `close` is checked against: a handle whose entry is gone is inert,
    /// exactly as the protocol says.
    toplevels: BTreeMap<WindowId, Toplevel>,
}

impl ForeignToplevelManagement {
    /// Creates the `zwlr_foreign_toplevel_manager_v1` global.
    ///
    /// No client filter, for the same reason the session-lock, gamma-control
    /// and data-control globals have none: scoot has no security-context
    /// support, so an allow-list would be theatre (see `docs/protocols.md`'s
    /// trust note). Worth saying plainly that this one has a write half,
    /// unlike `output_management.rs`: any client that can reach this socket
    /// can focus and close windows through it -- which is the same boundary
    /// `scoot msg action` already sits on.
    ///
    /// The `GlobalId` is dropped: nothing removes this global for the life of
    /// the process, and dropping the id does not remove it either.
    pub fn new(dh: &DisplayHandle) -> Self {
        let _ =
            dh.create_global::<State, ZwlrForeignToplevelManagerV1, _>(VERSION, ManagerGlobalData);
        Self::default()
    }
}

/// One window, as every client watching it has been told about it.
///
/// The three described fields are a *published snapshot*, not a second copy of
/// the window's state: they mean "what every handle below has already been
/// sent". They are compared against the live [`WindowInfo`] and `State::focus`
/// on each refresh so a change sends only the event that actually changed --
/// which is what keeps this protocol's wire traffic identical to the `ext-`
/// list's, where Smithay does the same comparison inside its own `send_title`.
#[derive(Debug)]
struct Toplevel {
    title: String,
    app_id: String,
    /// Whether this window has been reported as `activated`, i.e. whether the
    /// last `state` event sent for it carried the bit.
    activated: bool,
    /// Whether this window has been reported as `fullscreen` -- to handles
    /// of version 2 and up, the only ones that can be (see [`state_array`]).
    /// The same published-snapshot meaning as `activated`: compared against
    /// `Placement::fullscreen` on every `apply` by
    /// [`State::refresh_wlr_fullscreen`].
    fullscreen: bool,
    /// The output this window's handles were last told it is on -- the core
    /// id behind the `wl_output` objects the last `output_enter` named.
    ///
    /// Compared against the arrangement on every `apply`, so a window a
    /// cross-output move carried is told `output_leave` for the old screen
    /// and `output_enter` for the new one (milestone 19, phase F). `None`
    /// only when the window was announced with no output to name (no outputs
    /// at all, or a client holding no `wl_output`): then the next `apply`
    /// records without sending, because the protocol guarantees a `leave`
    /// only ever follows an `enter` for the same output.
    output: Option<OutputId>,
    /// One handle per client that has been told about this window -- several
    /// if a client bound the manager more than once, which is legal.
    ///
    /// Entries leave when the client destroys a handle (see
    /// [`HandleData::destroyed`]); the whole entry goes when the window does.
    handles: Vec<ZwlrForeignToplevelHandleV1>,
}

impl Toplevel {
    /// States this window in full on a handle that has just been created, and
    /// closes the batch with `done`.
    ///
    /// The order wlroots uses, and the one the protocol describes ("all
    /// initial details ... will be sent immediately after this event"): the two
    /// strings, the outputs, the state array -- empty or not, because a client
    /// learns "not activated" from an empty array and from nothing else -- then
    /// `done`.
    fn describe(
        &self,
        handle: &ZwlrForeignToplevelHandleV1,
        output: Option<&Output>,
        client: &Client,
    ) {
        handle.title(self.title.clone());
        handle.app_id(self.app_id.clone());
        if let Some(output) = output {
            enter_output(handle, output, client);
        }
        handle.state(state_array(
            self.activated,
            self.fullscreen,
            handle.version(),
        ));
        handle.done();
    }

    /// Sends the full `state` array, closed by `done`, to every handle --
    /// what either bit changing needs, since the array is always sent whole.
    ///
    /// `fullscreen_only` skips the handles that cannot see the change: a
    /// version 1 handle's array never carries the `fullscreen` bit, so a
    /// change to that bit alone would be an empty-looking `state` + `done`
    /// for nothing.
    fn send_state(&self, fullscreen_only: bool) {
        for handle in &self.handles {
            let version = handle.version();
            if fullscreen_only && version < FULLSCREEN_SINCE {
                continue;
            }
            handle.state(state_array(self.activated, self.fullscreen, version));
            handle.done();
        }
    }
}

/// Sends `output_enter` for every `wl_output` `client` holds for `output`.
///
/// Per *resource*, not per output: a client that bound `wl_output` twice has
/// two objects for the one screen and the protocol's argument is an object, so
/// each is told. This is what wlroots does too.
///
/// A client that has not bound `wl_output` at all is sent nothing here and
/// hears about the output when it does bind one --
/// [`State::wlr_toplevel_output_bound`] is that path, and it exists because
/// registry order is the server's choice: a taskbar may well bind this manager
/// before it binds the screen.
///
/// Holding the [`Output`]'s internal lock for the length of the loop is safe
/// and deliberate: `client_outputs` returns an iterator that keeps that guard,
/// and nothing inside the loop reads the `Output` again (see `handlers.rs`'s
/// `output_bound` note for the same hazard on the other side).
fn enter_output(handle: &ZwlrForeignToplevelHandleV1, output: &Output, client: &Client) {
    for wl_output in output.client_outputs(client) {
        handle.output_enter(&wl_output);
    }
}

/// The `state` event's array for one window, as a handle of `version` may be
/// told it: the `activated` entry, the `fullscreen` entry (version 2 and up
/// only), both, or nothing.
///
/// The protocol's array is a list of `zwlr_foreign_toplevel_handle_v1.state`
/// values, each a `uint` in the host's own byte order -- a `wl_array` is opaque
/// bytes on the wire and both ends of it run in this machine. `maximized` and
/// `minimized` are never in it; see the module doc for why.
///
/// The empty case allocates nothing (`Vec::new` has no backing buffer), and
/// an occupied one is a single allocation of at most eight bytes that the
/// generated `state(Vec<u8>)` signature makes unavoidable. One of those per
/// handle whose bits actually changed -- at most two windows per focus
/// change, one per fullscreen change.
fn state_array(activated: bool, fullscreen: bool, version: u32) -> Vec<u8> {
    let fullscreen = fullscreen && version >= FULLSCREEN_SINCE;
    let mut array = Vec::new();
    if activated || fullscreen {
        array.reserve_exact(4 * (usize::from(activated) + usize::from(fullscreen)));
    }
    if activated {
        array.extend_from_slice(&u32::from(ToplevelState::Activated).to_ne_bytes());
    }
    if fullscreen {
        array.extend_from_slice(&u32::from(ToplevelState::Fullscreen).to_ne_bytes());
    }
    array
}

impl State {
    /// The output a window is on, for the `output_enter` its handles are told.
    ///
    /// Read off where the window is actually drawn -- the first output whose
    /// geometry overlaps the window's bounding box in the `Space` -- rather
    /// than filed at creation, because "which output" is a fact about the
    /// layout, and the layout is what `apply()` just pushed onto the space.
    /// A window with no bounding box yet (announced from `add_window` before
    /// the core has placed it, or never mapped) reads as the pointer's
    /// output, which is where the window is about to open (`shell.rs` files
    /// `WindowOpened` there too), falling back to the primary when the
    /// pointer is over no output. `None` only when there is no output at all.
    ///
    /// Costs one bounding-box read and one geometry overlap per output until
    /// it hits. Announcement and binds are cold paths (per window, per
    /// `wl_output` bind), not per-event ones.
    fn output_of_window(&self, id: WindowId) -> Option<Output> {
        let window = self.windows.get(&id)?;
        let bbox = self.space.element_bbox(window);
        let placed = bbox.and_then(|bbox| {
            self.outputs
                .iter()
                .find(|output| {
                    self.space
                        .output_geometry(output)
                        .is_some_and(|region| region.overlaps(bbox))
                })
                .cloned()
        });
        placed
            .or_else(|| self.pointer_output())
            .or_else(|| self.outputs.primary().cloned())
    }

    /// Announces a new window to every client subscribed to the list.
    ///
    /// Called from [`State::add_window`](super::State) with the info that
    /// window was created with -- for nearly every client two empty strings,
    /// since a toolkit sends `set_app_id`/`set_title` just after
    /// `get_toplevel`, and each of those arrives later as its own
    /// [`State::publish_wlr_toplevel`] batch.
    ///
    /// The window is not focused *yet* at this point either: `add_window`
    /// announces before it tells the core about the window, so the initial
    /// state array is empty and the `activated` bit arrives in the batch
    /// [`State::refresh_wlr_activation`] sends a moment later, from the same
    /// `apply` that lays the window out. That is the announce order the `ext-`
    /// list already uses, kept identical on purpose: the two protocols publish
    /// one window list, so they publish it at the same instant.
    pub(super) fn open_wlr_toplevel(&mut self, id: WindowId, info: &WindowInfo) {
        let dh = self.display_handle.clone();
        // Cheap: `Output` is a handle around an `Arc`. Cloned so the
        // per-manager loop below can borrow `self.foreign_toplevel_management`
        // mutably at the same time.
        //
        // The output the window is actually on -- which at this point, before
        // the core has placed it, is the pointer's output by
        // `output_of_window`'s fallback (the primary when the pointer names
        // no output), and stays that output until a cross-output move carries
        // it elsewhere (which `refresh_wlr_output_membership` then tells
        // these same handles about). Announcing a window on an output it is
        // not on would be worse than announcing it on one.
        let output = self.output_of_window(id);
        let output_id = output.as_ref().and_then(|o| self.outputs.id_of(o));
        let management = &mut self.foreign_toplevel_management;
        // Not fullscreen yet either, for the same reason: the core has not
        // heard of the window, and nothing can have asked for it.
        let mut toplevel = Toplevel {
            title: info.title.clone(),
            app_id: info.app_id.clone(),
            activated: false,
            fullscreen: false,
            output: output_id,
            handles: Vec::new(),
        };
        management.managers.retain(|manager| {
            let Some(client) = manager.client() else {
                // The object outlived its client. `destroyed` removes it too;
                // doing it here as well keeps a burst of window openings from
                // walking a dead entry once per window.
                return false;
            };
            let created = client.create_resource::<ZwlrForeignToplevelHandleV1, _, State>(
                &dh,
                manager.version(),
                HandleData { window: id },
            );
            let Ok(handle) = created else {
                // See `announce_wlr_toplevels` for the only way this fails.
                return false;
            };
            manager.toplevel(&handle);
            toplevel.describe(&handle, output.as_ref(), &client);
            toplevel.handles.push(handle);
            true
        });
        // `add_window` allocates a fresh id for every window, so this never
        // displaces a live entry -- which would strand its handles, leaving a
        // taskbar showing a window nothing will ever send `closed` for.
        let displaced = management.toplevels.insert(id, toplevel);
        debug_assert!(displaced.is_none(), "a window id was reused: {id:?}");
    }

    /// Tells every client watching that a window is gone.
    ///
    /// Called from [`State::remove_window`](super::State). The entry leaves
    /// this map first and the handles are left behind on the client side,
    /// inert: the protocol says a handle stays valid after `closed` until the
    /// client destroys it, and every request this module answers checks the
    /// map, so an inert handle resolves to nothing.
    pub(super) fn close_wlr_toplevel(&mut self, id: WindowId) {
        // `None` for a window that never had an entry, which nothing can
        // produce today: the two writers are paired with `State::windows`'s
        // own insert and remove.
        let Some(toplevel) = self.foreign_toplevel_management.toplevels.remove(&id) else {
            return;
        };
        for handle in &toplevel.handles {
            handle.closed();
        }
    }

    /// Publishes a window's title and app id, sending only what changed.
    ///
    /// Called from [`State::refresh_window`](super::State), whose only callers
    /// are Smithay's `title_changed` and `app_id_changed` -- and the pinned rev
    /// raises those only when the value really changed (it compares against the
    /// role's stored one first, `handlers/surface/toplevel.rs`). So this is
    /// reached once per real change, and the comparison below is what narrows
    /// it further to the one field that moved, rather than re-sending the other
    /// one alongside it.
    pub(super) fn publish_wlr_toplevel(&mut self, id: WindowId, info: &WindowInfo) {
        let Some(toplevel) = self.foreign_toplevel_management.toplevels.get_mut(&id) else {
            return;
        };
        let title_changed = toplevel.title != info.title;
        let app_id_changed = toplevel.app_id != info.app_id;
        if !title_changed && !app_id_changed {
            return;
        }
        // Rewritten in place rather than reassigned, so a terminal retitling on
        // every prompt reuses one buffer instead of allocating a string per
        // change (the per-handle `clone` below is the generated signature's,
        // and there is no way around that one).
        if title_changed {
            toplevel.title.clear();
            toplevel.title.push_str(&info.title);
        }
        if app_id_changed {
            toplevel.app_id.clear();
            toplevel.app_id.push_str(&info.app_id);
        }
        for handle in &toplevel.handles {
            if title_changed {
                handle.title(toplevel.title.clone());
            }
            if app_id_changed {
                handle.app_id(toplevel.app_id.clone());
            }
            handle.done();
        }
    }

    /// Brings every window's `activated` bit back in step with `State::focus`.
    ///
    /// Called from `shell.rs`'s `set_focus`, inside the branch that runs only
    /// when the focused window actually changed -- and deliberately written as
    /// a *reconcile* over every window rather than a diff of the two ids that
    /// moved. `remove_window` writes `State::focus` directly without going
    /// through `set_focus`, so "the window that just lost focus" is not always
    /// knowable at this call site, while "which window does `State::focus`
    /// name right now" always is. Reading the one source of truth is also what
    /// keeps this bit meaning the same thing as `xdg_toplevel`'s own
    /// `activated` and the focus ring, which the same `set_focus` sets from the
    /// same field.
    ///
    /// Costs a walk of the window map and a `bool` compare per window, and
    /// sends nothing at all for the windows whose bit did not move -- which is
    /// every window but at most two.
    pub(super) fn refresh_wlr_activation(&mut self) {
        let focus = self.focus;
        for (id, toplevel) in &mut self.foreign_toplevel_management.toplevels {
            let activated = focus == Some(*id);
            if toplevel.activated == activated {
                continue;
            }
            toplevel.activated = activated;
            // The whole array, `fullscreen` included: a `state` event
            // replaces the previous one, so leaving the other bit out would
            // tell a taskbar a fullscreen window just stopped being one.
            toplevel.send_state(false);
        }
    }

    /// Brings every window's `fullscreen` bit back in step with the
    /// arrangement the core just published.
    ///
    /// Called from `shell.rs`'s `apply`, beside the output-membership
    /// refresh and for the same reason: every way a window's fullscreen can
    /// change ends in an `apply`. Costs one map lookup and one `bool` compare
    /// per window, and sends only for the window whose bit moved -- to its
    /// version 2+ handles (see [`Toplevel::send_state`]).
    pub(super) fn refresh_wlr_fullscreen(&mut self, arrangement: &Arrangement) {
        for placement in &arrangement.placements {
            let Some(toplevel) = self
                .foreign_toplevel_management
                .toplevels
                .get_mut(&placement.id)
            else {
                continue;
            };
            if toplevel.fullscreen == placement.fullscreen {
                continue;
            }
            toplevel.fullscreen = placement.fullscreen;
            toplevel.send_state(true);
        }
    }

    /// Brings every window's `output_enter` membership back in step with the
    /// arrangement the core just published.
    ///
    /// Called from `shell.rs`'s `apply`, which every event and action that
    /// can move a window ends in -- so a cross-output move is told as one
    /// `output_leave` for the old screen plus one `output_enter` for the new
    /// one, closed by `done`, on every handle of exactly the window that
    /// moved. Windows that stayed put cost one map lookup and one `Option`
    /// compare each, and send nothing.
    ///
    /// What this deliberately does *not* send: a `leave` when a window
    /// closes (its `closed` covers the handle's whole death -- nothing may
    /// be sent after it), when a window moves between workspaces of one
    /// output (membership is per output, and no code ever sent leave there),
    /// or when the stored output is `None` (no `enter` was ever sent for it,
    /// and the protocol guarantees a `leave` only ever follows an `enter`
    /// for the same output).
    pub(super) fn refresh_wlr_output_membership(&mut self, arrangement: &Arrangement) {
        for placement in &arrangement.placements {
            let Some(toplevel) = self
                .foreign_toplevel_management
                .toplevels
                .get_mut(&placement.id)
            else {
                continue;
            };
            if toplevel.output == Some(placement.output) {
                continue;
            }
            let old = toplevel.output.and_then(|id| self.outputs.get(id));
            let new = self.outputs.get(placement.output);
            for handle in &toplevel.handles {
                // The handle's own client, so the `wl_output` objects below
                // belong to the client they are sent to -- the same
                // wrong-client panic `wlr_toplevel_output_bound` guards
                // against. A handle whose client is gone sends nothing; its
                // `destroyed` reaps it.
                let Some(client) = handle.client() else {
                    continue;
                };
                let mut said = false;
                if let Some(old) = old {
                    for wl_output in old.client_outputs(&client) {
                        handle.output_leave(&wl_output);
                        said = true;
                    }
                }
                if let Some(new) = new {
                    for wl_output in new.client_outputs(&client) {
                        handle.output_enter(&wl_output);
                        said = true;
                    }
                }
                // Only with something to close: a bare `done` would read as
                // a change to a client that draws on it, and a client
                // holding no `wl_output` for either screen has nothing to
                // redraw from this move.
                if said {
                    handle.done();
                }
            }
            toplevel.output = Some(placement.output);
        }
    }

    /// A client bound a `wl_output`. Every handle it already holds has to be
    /// told the window is on that screen.
    ///
    /// The protocol's `output_enter` is described as firing when a toplevel
    /// "becomes visible on the given output", and a client that had no
    /// `wl_output` object when its handle was created never got one -- registry
    /// order is the server's choice, so a taskbar binding this manager before
    /// the screen is ordinary, not a misbehaving client. Without this it would
    /// show windows belonging to no output forever. `ext_workspace.rs`'s
    /// `workspace_group_output_bound` is the same hook for the same reason.
    pub(super) fn wlr_toplevel_output_bound(&mut self, output: &Output, wl_output: &WlOutput) {
        // Only the windows actually on the bound output are told they
        // entered it: a bind of any other output must not produce an
        // `output_enter` that was never announced. (Every window `open`
        // announced is on the output `output_of_window` resolved for it, so
        // the two agree.)
        let bound = wl_output.id();
        // Resolved up front, one immutable pass: the loop below only reads
        // the toplevel map, so both borrows can coexist -- but resolving
        // inside it would re-walk the space per handle instead of per window.
        // In key order (`toplevels` is a `BTreeMap`), so the membership test
        // below is a binary search rather than a scan.
        let on_this_output: Vec<WindowId> = self
            .foreign_toplevel_management
            .toplevels
            .keys()
            .filter(|id| self.output_of_window(**id).as_ref() == Some(output))
            .copied()
            .collect();
        for (id, toplevel) in self.foreign_toplevel_management.toplevels.iter() {
            if on_this_output.binary_search(id).is_err() {
                continue;
            }
            for handle in &toplevel.handles {
                // Load-bearing, not tidiness: wayland-backend *panics* when an
                // event carries an object belonging to a different client than
                // the one it is sent to ("Attempting to send an event with
                // objects from wrong client", `rs/server_impl/client.rs`), and
                // a panic here takes every client's session down with it.
                //
                // Compared as ids rather than through `Resource::client`,
                // because this runs once per handle per `wl_output` bind and
                // any client may provoke it: `same_client_as` is a comparison
                // of the two `ObjectId`s' stored client ids, while `client()`
                // takes the backend's state mutex twice and clones an
                // `Arc<dyn ClientData>` to answer the same question. It is
                // also the *exact* question -- the panic above is literally
                // `o.id.client_id != self.id` on the object argument. A handle
                // that has since died is skipped under the system backend and
                // harmlessly kept under the Rust one, where the event is
                // swallowed as `InvalidId` rather than sent (same file's
                // `get_object`, whose `?` the generated `let _ =` eats).
                if !handle.id().same_client_as(&bound) {
                    continue;
                }
                handle.output_enter(wl_output);
                handle.done();
            }
        }
    }

    /// Builds a freshly bound manager's whole world: a handle per window that
    /// already exists, each described and closed by its own `done`.
    ///
    /// Announces oldest window first, which is what the [`BTreeMap`] keyed by
    /// window id gives for free.
    fn announce_wlr_toplevels(
        &mut self,
        dh: &DisplayHandle,
        client: &Client,
        manager: ZwlrForeignToplevelManagerV1,
    ) {
        let version = manager.version();
        // One immutable pass first, in key order: the walk below holds
        // `management` mutably (each new handle is pushed into its entry),
        // so per-window resolution cannot borrow `self` from inside it --
        // and zipping keeps the two in step without a lookup per window.
        // Resolved to core ids up front for the same reason: the membership
        // refresh below diffs those, and the announce above already sent
        // `output_enter` for exactly these outputs.
        let outputs: Vec<Option<OutputId>> = self
            .foreign_toplevel_management
            .toplevels
            .keys()
            .map(|id| {
                self.output_of_window(*id)
                    .as_ref()
                    .and_then(|output| self.outputs.id_of(output))
            })
            .collect();
        let management = &mut self.foreign_toplevel_management;
        for ((id, toplevel), output) in management.toplevels.iter_mut().zip(outputs) {
            let created = client.create_resource::<ZwlrForeignToplevelHandleV1, _, State>(
                dh,
                version,
                HandleData { window: *id },
            );
            let Ok(handle) = created else {
                // The one and only way this fails: the client is already gone
                // by the time it runs. `Client::create_resource` forwards to
                // `Handle::create_object`, whose sole error is
                // `ClientStore::get_client_mut` no longer finding the client
                // (wayland-backend 0.3.17, `rs/server_impl/handle.rs`) -- the
                // server's own id allocation for a new object cannot fail, so
                // object-id exhaustion is not a failure mode here.
                //
                // Counted at bind but never registered, so the claim is given
                // back: a leak here would be a counter that only grows.
                //
                // Nothing is sent in answer: there is nobody left to hear a
                // `finished`. The manager is simply not registered, so no later
                // window walks it, and whatever handles were created before
                // this point are reaped by their own `destroyed` when the
                // client is cleaned up.
                tracing::debug!(
                    "could not create a zwlr_foreign_toplevel_handle_v1; \
                     the client is gone, dropping its manager"
                );
                self.bind_budget.release_bind(&client.id(), &manager.id());
                return;
            };
            manager.toplevel(&handle);
            // The membership the refresh below diffs against: this announce
            // just sent `output_enter` for exactly this output (or none, when
            // the client holds no `wl_output` for it -- then the bind hook
            // sends the enter later, and the stored id still tells it which
            // output that bind has to be for).
            toplevel.output = output;
            let smithay_output = output.and_then(|id| self.outputs.get(id));
            toplevel.describe(&handle, smithay_output, client);
            toplevel.handles.push(handle);
        }
        management.managers.push(manager);
    }

    /// Answers `zwlr_foreign_toplevel_handle_v1.activate`: focus that window.
    ///
    /// The `wl_seat` argument is ignored. scoot has exactly one seat, created
    /// in `State::new` and never replaced, so "which seat" has one answer; the
    /// protocol offers the argument for compositors that have more.
    ///
    /// ## Refused before anything is touched
    ///
    /// - **An inert handle** (its window closed while the click was in flight)
    ///   resolves to no entry, and the protocol says such a handle's requests
    ///   are ignored.
    /// - **A locked session.** Checked here rather than left to
    ///   [`State::act`](super::State)'s own gate, because the fast path below
    ///   does not go through `act` at all and because nothing before the check
    ///   may be disturbed on a refusal -- `clicked_layer` in particular has to
    ///   survive the lock so the session comes back as the user left it.
    ///   `act`'s gate remains as the backstop `shell.rs` describes it as.
    ///
    /// ## Then: exactly what clicking the window itself does
    ///
    /// `input.rs`'s `focus_under_pointer` is the reference implementation of
    /// this gesture, and it is two statements: clear
    /// [`State::clicked_layer`](super::State), then run
    /// `Action::FocusWindowId`. Both are needed, and the first is the one
    /// that is easy to leave out -- `layer_shell.rs`'s `layer_keyboard_focus`
    /// consults `clicked_layer` and hands the keyboard straight back to a
    /// still-mapped `on_demand` surface, so without clearing it a
    /// `refresh_keyboard_focus` here re-derives the *taskbar* and changes
    /// nothing. And the taskbar is exactly what sent this request: it is a
    /// layer surface, the user clicked it to reach the window list, and that
    /// click may well have given it the keyboard.
    ///
    /// So the click is spent here, the same way a click on the window spends
    /// it. What follows splits only on cost:
    ///
    /// - **The window is already focused**: skip
    ///   [`State::act`](super::State), which would run a full `apply` -- an
    ///   arrange, a configure per window and a render -- for a client that can
    ///   repeat `activate` as fast as it can write to its socket (the hazard
    ///   `ext_workspace.rs` guards the same way for workspace activation), and
    ///   run only the keyboard half, which is what actually has to happen.
    /// - **Otherwise**: `act`, whose `apply` ends in the same
    ///   `refresh_keyboard_focus` -- now with `clicked_layer` already cleared,
    ///   so it reaches the window rather than stopping at the taskbar.
    ///
    /// Either way the two paths agree about the same gesture, which is the
    /// whole point.
    fn wlr_toplevel_activate(&mut self, id: WindowId) {
        if !self.foreign_toplevel_management.toplevels.contains_key(&id) {
            return;
        }
        if self.session_lock.is_locked() {
            tracing::debug!(
                ?id,
                "ignoring a foreign-toplevel activate: the session is locked"
            );
            return;
        }
        // Mirrors `input.rs`'s `focus_under_pointer`, which clears this on the
        // line before its own `act(FocusWindowId)` -- see above for why it is
        // load-bearing rather than tidiness.
        self.clicked_layer = None;
        if self.focus == Some(id) {
            self.refresh_keyboard_focus();
            return;
        }
        self.act(Action::FocusWindowId(id));
    }

    /// Answers `zwlr_foreign_toplevel_handle_v1.set_fullscreen` (`true`,
    /// with its optional output) and `unset_fullscreen` (`false`).
    ///
    /// A taskbar asking on the user's behalf, so it takes the path a user's
    /// own request takes -- `Action::SetFullscreen` through
    /// [`State::act`](super::State) -- with the same two refusals up front as
    /// `activate` and `close`: an inert handle, and a locked session. The
    /// output argument gets the policy the client's own `set_fullscreen`
    /// gets (see `fullscreen.rs`): honoured only for the focused window,
    /// otherwise ignored.
    ///
    /// `act`'s `apply` configures the window if it is visible; the answer
    /// after it tells an invisible one too, and sends nothing when nothing
    /// changed -- unlike the client's own request, nobody asked this window
    /// for a configure.
    fn wlr_toplevel_set_fullscreen(
        &mut self,
        id: WindowId,
        fullscreen: bool,
        output: Option<&WlOutput>,
    ) {
        if !self.foreign_toplevel_management.toplevels.contains_key(&id) {
            return;
        }
        if self.session_lock.is_locked() {
            tracing::debug!(
                ?id,
                fullscreen,
                "ignoring a foreign-toplevel fullscreen request: the session is locked"
            );
            return;
        }
        if fullscreen {
            self.honour_output_hint(id, output);
        }
        self.act(Action::SetFullscreen { id, fullscreen });
        self.tell_fullscreen(id);
    }

    /// Answers `zwlr_foreign_toplevel_handle_v1.close`: ask that window to go.
    ///
    /// The same `xdg_toplevel.close` that `Action::CloseFocused`'s
    /// `Effect::Close` sends, aimed at a named window instead of the focused
    /// one. It does not go through [`State::act`](super::State) because there
    /// is no core action for "close this specific window" -- adding one would
    /// mean changing `scoot-core`, which is fuzz-tested and
    /// platform-independent by design, to carry a wire protocol's convenience.
    /// So the one thing `act` would have contributed, its session-lock gate, is
    /// applied here explicitly and for exactly the reason `shell.rs` gives for
    /// it: a window the user cannot see must not be closed from behind the lock
    /// screen.
    ///
    /// A request the window ignores is the client's problem, as the protocol
    /// says: `closed` follows if and when the window really goes.
    fn wlr_toplevel_close(&mut self, id: WindowId) {
        if !self.foreign_toplevel_management.toplevels.contains_key(&id) {
            return;
        }
        if self.session_lock.is_locked() {
            tracing::debug!(
                ?id,
                "ignoring a foreign-toplevel close: the session is locked"
            );
            return;
        }
        if let Some(toplevel) = self.window(id).and_then(Window::toplevel) {
            toplevel.send_close();
        }
    }
}

/// User data on the `zwlr_foreign_toplevel_manager_v1` global itself.
struct ManagerGlobalData;

/// ...and on a manager a client has bound.
struct ManagerData;

/// User data on a `zwlr_foreign_toplevel_handle_v1`: the window it stands for.
///
/// The one place a handle object is tied back to a window. Storing the id
/// rather than looking the handle up in the map is what makes `activate` and
/// `close` O(1) and, more importantly, unambiguous -- and it is safe to keep
/// past the window's death because ids are never reused (see the module doc).
struct HandleData {
    window: WindowId,
}

impl GlobalDispatch2<ZwlrForeignToplevelManagerV1, State> for ManagerGlobalData {
    fn bind(
        &self,
        state: &mut State,
        dh: &DisplayHandle,
        client: &Client,
        resource: New<ZwlrForeignToplevelManagerV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        let manager = data_init.init(resource, ManagerData);
        if state.bind_budget.refuse_bind(client, &manager.id()) {
            // Over the shared per-client budget (see `bind_budget.rs`):
            // deferred, not sent here -- `finished` is a destructor event,
            // and sending one inside `bind` panics wayland-backend's bind
            // epilogue. Not counted, not registered, so no later window
            // walks it.
            state.defer_bind_refusal(super::bind_budget::RefusedBind::WlrToplevel(manager));
            return;
        }
        state.announce_wlr_toplevels(dh, client, manager);
    }
}

impl Dispatch2<ZwlrForeignToplevelManagerV1, State> for ManagerData {
    fn request(
        &self,
        state: &mut State,
        client: &Client,
        manager: &ZwlrForeignToplevelManagerV1,
        request: zwlr_foreign_toplevel_manager_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        // `stop` is this interface's only request, and the `if let` is what
        // covers the rest: the request enums are `#[non_exhaustive]`, and an
        // opcode this version does not define never reaches here (wayland-
        // backend rejects it first), so anything else is ignored -- and
        // certainly not panicked on, which is what `unreachable!()` would make
        // of a future protocol version.
        if let zwlr_foreign_toplevel_manager_v1::Request::Stop = request {
            // Released synchronously rather than left to `destroyed`: a
            // stop-and-rebind in one batch must see the freed slot without
            // waiting for post-batch cleanup. Idempotent with the `destroyed`
            // release -- `finished` queues it, and removing an absent id is a
            // no-op.
            state.bind_budget.release_bind(&client.id(), &manager.id());
            // Unregistered first: `finished` is a destructor event, so the
            // object is gone as soon as it is sent (wayland-backend removes it
            // from the client's object map and queues its `destroyed`
            // callback), and nothing may try to send to it afterwards.
            // `destroyed` runs this same `retain` later, which is idempotent.
            //
            // The handles this manager created are deliberately left alone and
            // keep reporting: the protocol's teardown is stop, wait for
            // `finished`, then destroy them, which a client cannot do safely if
            // the compositor has already stopped saying what they are doing.
            // wlroots keeps them too.
            state
                .foreign_toplevel_management
                .managers
                .retain(|entry| entry != manager);
            manager.finished();
        }
    }

    fn destroyed(
        &self,
        state: &mut State,
        client: ClientId,
        manager: &ZwlrForeignToplevelManagerV1,
    ) {
        // The path an ordinary client disconnect takes, and the only thing that
        // stops a dead client's manager from being walked on every window
        // opening -- and the path its budget claim takes back, including for
        // a bare destroy with no `stop` before it.
        state.bind_budget.release_bind(&client, &manager.id());
        state
            .foreign_toplevel_management
            .managers
            .retain(|entry| entry != manager);
    }
}

impl Dispatch2<ZwlrForeignToplevelHandleV1, State> for HandleData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        _handle: &ZwlrForeignToplevelHandleV1,
        request: zwlr_foreign_toplevel_handle_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            zwlr_foreign_toplevel_handle_v1::Request::Activate { seat: _ } => {
                state.wlr_toplevel_activate(self.window)
            }
            zwlr_foreign_toplevel_handle_v1::Request::Close => {
                state.wlr_toplevel_close(self.window)
            }
            zwlr_foreign_toplevel_handle_v1::Request::SetFullscreen { output } => {
                state.wlr_toplevel_set_fullscreen(self.window, true, output.as_ref())
            }
            zwlr_foreign_toplevel_handle_v1::Request::UnsetFullscreen => {
                state.wlr_toplevel_set_fullscreen(self.window, false, None)
            }
            // Accepted and ignored, on purpose -- see the module doc. Listed
            // one by one rather than folded into the catch-all below so that
            // "scoot has nothing to attach this to" stays a decision written
            // down here, and a protocol version that adds a *new* request still
            // lands in the catch-all rather than silently joining this list.
            zwlr_foreign_toplevel_handle_v1::Request::SetMaximized
            | zwlr_foreign_toplevel_handle_v1::Request::UnsetMaximized
            | zwlr_foreign_toplevel_handle_v1::Request::SetMinimized
            | zwlr_foreign_toplevel_handle_v1::Request::UnsetMinimized
            | zwlr_foreign_toplevel_handle_v1::Request::SetRectangle { .. } => {}
            // `Destroy` among them: wayland-backend destroys the object itself
            // and `destroyed` below does the bookkeeping.
            _ => {}
        }
    }

    fn destroyed(
        &self,
        state: &mut State,
        _client: ClientId,
        handle: &ZwlrForeignToplevelHandleV1,
    ) {
        // Covers both ends of a handle's life: a client destroying one early
        // (legal -- it just will not be given another for that window), and a
        // client disconnecting. `None` when the window closed first, in which
        // case the whole entry is already gone.
        let Some(toplevel) = state
            .foreign_toplevel_management
            .toplevels
            .get_mut(&self.window)
        else {
            return;
        };
        toplevel.handles.retain(|entry| entry != handle);
    }
}
