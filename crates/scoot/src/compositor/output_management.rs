//! `wlr-output-management-unstable-v1`: the display page of a shell's settings.
//!
//! `wl_output` tells a client what the screen *is*; this protocol is what a
//! display-configuration tool (`wlr-randr`, `kanshi`) and a shell's
//! Settings -> Display page read and write. Both Quickshell shells probed
//! against scoot hit its absence (see `docs/backlog/protocols/`): DMS logs
//! `Received empty outputs list` and Noctalia's Display page renders with
//! nothing behind it, because their daemon initializes an output-management
//! client and finds no global to bind.
//!
//! ## Why the wlr protocol, against this project's `ext-` preference
//!
//! `CLAUDE.md`'s standing rule is to prefer the `ext-` successor -- which is
//! why `ext_workspace.rs` and `foreign_toplevel.rs` exist instead of their wlr
//! predecessors. There is no successor to prefer here: at the pinned Smithay
//! rev's `wayland-protocols` 0.32.13, `protocols/staging/` carries no
//! output-management protocol at all, so `wlr-output-management-unstable-v1`
//! (in `wayland-protocols-wlr` 0.3.12) is the only one that exists.
//!
//! Smithay has no helper for it either -- no `output_management` module
//! anywhere in the pinned rev -- so the global, the object lifecycle and the
//! event batching are implemented here directly, hanging off
//! [`Dispatch2`]/[`GlobalDispatch2`] because `dispatch.rs` owns a blanket
//! `Dispatch` impl for [`State`]. That is not hand-rolled wire format: the
//! generated server bindings ship with `wayland-protocols-wlr` and are
//! re-exported through Smithay, exactly as `gamma_control.rs` already uses
//! them for `wlr-gamma-control-unstable-v1`.
//!
//! ## Read-only: enumeration is implemented, reconfiguration is refused
//!
//! scoot has exactly one [`Output`], created once at startup and never moved,
//! rotated, disabled or rescaled (see `headless.rs`'s `OUTPUT_ID`). There is
//! nothing for `apply` to apply. So the halves are split at the one seam the
//! protocol gives: `zwlr_output_manager_v1.create_configuration` is the only
//! way into the write half, and every configuration it hands out answers
//! `apply` and `test` with `failed` -- see `configuration.rs`.
//!
//! That is a refusal, not a stub. A configuration that silently succeeded and
//! changed nothing would give a shell a settings page that appears to work,
//! which is worse than having no page at all; `failed` is the protocol's own
//! word for "the compositor rejects the changes", and a client acts on it.
//!
//! ## One source of truth
//!
//! Every value below is read from the [`Output`] itself -- the same object
//! `wl_output`, `xdg-output` and the render path are configured from -- and
//! nothing is cached from the config or duplicated from a backend. A head's
//! state and the `wl_output` state a client sees alongside it are therefore
//! the same numbers by construction, not by agreement. Two consequences worth
//! knowing, both inherited rather than introduced:
//!
//! - **`refresh` is always 60000 mHz**, including under `--tty` on a 144 Hz
//!   panel: `headless.rs`'s `set_mode` hard-codes it and nothing in `tty/`
//!   calls `change_current_state` with the real DRM refresh. `wl_output`
//!   already reports the same 60 Hz, so this mirrors the existing inaccuracy
//!   rather than adding a second one.
//! - **[`Output::modes`] only ever grows, never shrinks.**
//!   `change_current_state` pushes any mode it has not seen and nothing
//!   calls `delete_mode`. The only production caller that can hand it a
//!   second mode is [`State::resize_output`], which two backends reach:
//!   `--nested`'s dispatch loop (`nested_dispatch.rs`) calls it at most
//!   once per process, gated by `Host::is_configured` (only the host's
//!   *initial* configure, if it proposes a size other than
//!   `--width`/`--height`, grows the list; a host resizing scoot's window
//!   afterwards is acked and otherwise ignored), and `--tty`'s hotplug
//!   handler (`tty/hotplug.rs`) calls it once per mode change the display
//!   underneath actually makes. So under `--tty` the list can grow more
//!   than once -- a vfkit window moved between a 2x and a 1x screen a few
//!   times leaves a mode for each distinct size it settled at. It is
//!   bounded by the number of distinct sizes the connector has offered,
//!   not by uevent traffic: a re-probe that lands on a size already in the
//!   list adds nothing, and `plan` in `tty/hotplug.rs` does not even reach
//!   `set_mode` unless the size changed. `wl_output` advertises every known mode for the same
//!   reason this protocol does, and the `preferred` flag tracks it the same
//!   way on both: `headless.rs`'s `set_mode` marks the new mode preferred
//!   *before* `change_current_state` sends it to already-bound `wl_output`
//!   clients (that call snapshots the preferred mode synchronously, so the
//!   order is what the wire sees), while this protocol's snapshot is taken
//!   once `set_mode` has fully returned -- either way the newly-added mode
//!   is `preferred`. Pinned by
//!   `wl_output_tells_an_already_bound_client_the_resized_mode_is_preferred`
//!   in `output_management/tests.rs`, which binds both on one connection and
//!   asserts the resize batch agrees.
//!
//! Two properties are deliberately *not* sent, which the protocol explicitly
//! allows:
//!
//! - **`physical_size`**, because scoot's is `(0, 0)` -- no backend knows a
//!   panel's millimetres -- and the protocol sends it "only if the head has a
//!   physical size".
//! - **`serial_number`**, because scoot's is the literal `"0"` placeholder
//!   `headless.rs` fills in, not a real serial. The protocol says a compositor
//!   may never send it, and warns that a false positive is the risk; a client
//!   keying a saved per-monitor profile off `"0"` would match every scoot
//!   session on every machine.
//!
//! `make` and `model` *are* sent: they are the same strings `wl_output.geometry`
//! already carries, so omitting them would make this protocol say less than
//! `wl_output` about the same output.
//!
//! ## When the advertised state changes
//!
//! [`State::refresh_output_heads`] is the one path, and it is driven from the
//! only two places that ever change the output's state -- `headless::init_named`
//! and `State::resize_output`, the two callers of `headless.rs`'s `set_mode`,
//! which is the sole caller of `Output::change_current_state`. A bind runs it
//! too, so a manager bound later starts from the snapshot every other manager
//! has already been sent. It diffs against [`OutputManagement::published`] and
//! sends nothing when nothing moved, so it is safe to call from anywhere and
//! is not on any per-frame or per-event path.
//!
//! A `--tty` VT switch changes nothing here *by itself*, and that is the right
//! answer: the [`Output`] object is untouched across one (the session pauses
//! rendering and drops DRM master, see `tty/mod.rs`), the head never stops
//! existing, and its mode, position, scale and transform are all still what
//! the compositor will present the moment the VT comes back. wlroots keeps its
//! heads enabled across a VT switch for the same reason. So the head stays
//! `enabled(1)` and no `done` is sent.
//!
//! The one thing a switch *back* can produce is a mode change, and it is not
//! the switch that caused it: a display plugged in or resized while this
//! session was on another VT could not be acted on then (no DRM master), so
//! `tty/mod.rs`'s reactivation re-probes the device and applies whatever
//! changed, which reaches `resize_output` and therefore this module by the
//! ordinary path. A VT switch across which nothing about the display moved
//! still sends nothing.
//!
//! ## Batching
//!
//! Every change goes out as: the events, then one `done` carrying a serial.
//! The serial is the protocol's handle for `create_configuration`; scoot
//! refuses every configuration whatever serial it names, so it is purely
//! informational here -- but a client that tracks it (`wlr-randr` does) sees
//! it advance on every real change, which is what the protocol promises.

use smithay::output::{Mode, Output};
use smithay::reexports::wayland_protocols_wlr::output_management::v1::server::zwlr_output_head_v1::{
    self, AdaptiveSyncState, ZwlrOutputHeadV1,
};
use smithay::reexports::wayland_protocols_wlr::output_management::v1::server::zwlr_output_manager_v1::{
    self, ZwlrOutputManagerV1,
};
use smithay::reexports::wayland_protocols_wlr::output_management::v1::server::zwlr_output_mode_v1::{
    self, ZwlrOutputModeV1,
};
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::{
    Client, DataInit, DisplayHandle, New, Resource, Weak,
};
use smithay::utils::{Logical, Point, Transform};
use smithay::wayland::{Dispatch2, GlobalDispatch2};

use super::State;

mod configuration;

#[cfg(test)]
mod tests;

/// The version advertised, which is the highest the protocol defines.
///
/// Version 2 adds `make`/`model`/`serial_number`, version 3 the `release`
/// destructors on heads and modes, and version 4 `adaptive_sync` (plus a
/// `set_adaptive_sync` request, which lands in the refused half). All four are
/// implemented, so a client may bind at any of them and gets exactly what that
/// version defines -- every event below is gated on the object's own version,
/// because wayland-backend does *not* check one on the way out (verified in
/// 0.3.17's `rs/server_impl/client.rs`: the `since` check is on the receive
/// path only), so an ungated event would reach a v1 client as an opcode it
/// cannot decode.
const VERSION: u32 = 4;

/// Everything this compositor keeps for `wlr-output-management-unstable-v1`.
///
/// [`Self::published`] means exactly one thing: **what every registered
/// manager has already been told**. It is never read as "what the output is" --
/// that is the [`Output`], re-read on every refresh -- and the two meet in
/// exactly one place ([`State::refresh_output_heads`]), which is also the only
/// writer. Keeping every manager in lockstep is what makes one shared snapshot
/// correct rather than one per client.
#[derive(Debug, Default)]
pub struct OutputManagement {
    managers: Vec<Manager>,
    published: Option<HeadState>,
    /// The serial last sent on `done`. Advances once per change batch, shared
    /// by every manager because every manager is sent the same batch.
    serial: u32,
}

impl OutputManagement {
    /// Creates the `zwlr_output_manager_v1` global.
    ///
    /// No client filter, for the same reason the session-lock, gamma-control
    /// and data-control globals have none: scoot has no security-context
    /// support, so an allow-list would be theatre (see `docs/protocols.md`'s
    /// trust note). Nothing here is writable in any case.
    ///
    /// The `GlobalId` is dropped: nothing removes this global for the life of
    /// the process, and dropping the id does not remove it either.
    pub fn new(dh: &DisplayHandle) -> Self {
        let _ = dh.create_global::<State, ZwlrOutputManagerV1, _>(VERSION, ManagerGlobalData);
        Self::default()
    }
}

/// The head state scoot advertises, read whole from the [`Output`].
///
/// Split in use, not in shape: the first five fields are the head's
/// *identity*, which the protocol says is sent once per head object and never
/// changes (see [`HeadState::is_same_head`]); the rest is what a `done` batch
/// can carry an update for.
#[derive(Clone, Debug, PartialEq)]
struct HeadState {
    name: String,
    description: String,
    make: String,
    model: String,
    /// Millimetres, `(0, 0)` when unknown -- which it always is today. See the
    /// module doc for why that means the event is not sent.
    physical_size: (i32, i32),
    /// Every mode the output knows, in the order it learned them.
    modes: Vec<Mode>,
    current_mode: Option<Mode>,
    preferred_mode: Option<Mode>,
    /// The output's origin in the global compositor space -- the same value
    /// `xdg_output.logical_position` carries, since both read
    /// [`Output::current_location`].
    position: Point<i32, Logical>,
    transform: Transform,
    /// The *fractional* scale, which this protocol's `fixed` argument can
    /// carry in full -- unlike `wl_output.scale`, which only gets
    /// `ceil(scale)`. A client cross-checking the two sees `1.5` here and `2`
    /// there, which is the protocols' own difference, not a disagreement.
    scale: f64,
}

impl HeadState {
    /// Reads the whole advertised state off the output.
    fn of(output: &Output) -> Self {
        let physical = output.physical_properties();
        Self {
            name: output.name(),
            description: output.description(),
            make: physical.make,
            model: physical.model,
            physical_size: (physical.size.w, physical.size.h),
            modes: output.modes(),
            current_mode: output.current_mode(),
            preferred_mode: output.preferred_mode(),
            position: output.current_location(),
            transform: output.current_transform(),
            scale: output.current_scale().fractional_scale(),
        }
    }

    /// Whether `other` is the same head, i.e. whether the properties the
    /// protocol only lets a head object state *once* still hold.
    ///
    /// Always true today: scoot creates its one output in
    /// `headless::init_named` and never renames or replaces it, so only the
    /// mutable half below can ever differ. It is checked rather than assumed
    /// because the alternative failure is silent -- a head object that keeps
    /// reporting the name it was created with while the output has another
    /// one.
    fn is_same_head(&self, other: &Self) -> bool {
        self.name == other.name
            && self.description == other.description
            && self.make == other.make
            && self.model == other.model
            && self.physical_size == other.physical_size
    }
}

/// One client's bound `zwlr_output_manager_v1` and the objects created for it.
#[derive(Debug)]
struct Manager {
    manager: ZwlrOutputManagerV1,
    /// `None` before the output exists (a manager bound by a client during
    /// `State::new`, before `headless::init_named` has run) and after the head
    /// has been retired.
    head: Option<Head>,
}

/// The head object handed to one client, and the mode objects under it.
#[derive(Debug)]
struct Head {
    /// [`Weak`] because version 3 lets a client `release` the head while
    /// keeping its manager; an update then has nowhere to go and is skipped
    /// rather than resurrecting an object the client said it was done with.
    head: Weak<ZwlrOutputHeadV1>,
    /// One entry per mode this client has been told about, keyed by the mode's
    /// *value* rather than by position: the protocol sends `mode` once per
    /// supported mode, so what matters is whether this client has already
    /// heard of a given mode, not where it sat in the output's list.
    modes: Vec<(Mode, Weak<ZwlrOutputModeV1>)>,
}

impl Manager {
    /// Brings this manager up to date and closes the batch with `done`.
    ///
    /// Returns whether it can still be kept in step. `false` means drop it:
    /// its client is gone, or an object could not be created for it and it has
    /// already been sent `finished`.
    fn apply(
        &mut self,
        dh: &DisplayHandle,
        published: Option<&HeadState>,
        current: Option<&HeadState>,
        serial: u32,
    ) -> bool {
        // A head whose identity changed is a *different* head, so the old
        // objects are retired and new ones announced rather than updated.
        // Unreachable today (see `is_same_head`), and the same branch covers
        // the two reachable shapes: no head yet, and a head that went away.
        let carried_over = match (published, current, &self.head) {
            (Some(published), Some(current), Some(_)) if published.is_same_head(current) => {
                Some(published)
            }
            _ => None,
        };
        if carried_over.is_none() {
            self.retire_head();
        }
        let Some(current) = current else {
            self.manager.done(serial);
            return true;
        };
        let Ok(client) = dh.get_client(self.manager.id()) else {
            return false;
        };
        let updated = match carried_over {
            Some(published) => self.update(dh, &client, published, current, serial),
            None => self.announce(dh, &client, current, serial),
        };
        if !updated {
            return false;
        }
        self.manager.done(serial);
        true
    }

    /// Tells this client's head and modes that they are gone, and forgets them.
    ///
    /// Innermost first: a mode object belongs to the head, so it is retired
    /// before the head that introduced it. Both become inert per the protocol;
    /// the client destroys them in its own time, and the [`Weak`]s here stop
    /// upgrading either way.
    fn retire_head(&mut self) {
        let Some(head) = self.head.take() else {
            return;
        };
        for (_, mode) in &head.modes {
            if let Ok(mode) = mode.upgrade() {
                mode.finished();
            }
        }
        if let Ok(head) = head.head.upgrade() {
            head.finished();
        }
    }

    /// Creates this client's head and every mode under it, and states the whole
    /// of `current` on them.
    fn announce(
        &mut self,
        dh: &DisplayHandle,
        client: &Client,
        current: &HeadState,
        serial: u32,
    ) -> bool {
        let version = self.manager.version();
        let created = client.create_resource::<ZwlrOutputHeadV1, _, State>(dh, version, HeadData);
        let Ok(head) = created else {
            return self.give_up("zwlr_output_head_v1", serial);
        };
        self.manager.head(&head);
        head.name(current.name.clone());
        head.description(current.description.clone());
        if current.physical_size.0 > 0 && current.physical_size.1 > 0 {
            head.physical_size(current.physical_size.0, current.physical_size.1);
        }
        if version >= 2 {
            head.make(current.make.clone());
            head.model(current.model.clone());
        }
        let mut modes = Vec::with_capacity(current.modes.len());
        for mode in &current.modes {
            let Some(object) = create_mode(dh, client, &head, version, *mode, current) else {
                // Recorded before giving up so `give_up` can retire it: the
                // head has already been introduced and half described, and the
                // client is owed a `finished` on it rather than a partial head
                // it will never hear about again.
                self.head = Some(Head {
                    head: head.downgrade(),
                    modes,
                });
                return self.give_up("zwlr_output_mode_v1", serial);
            };
            modes.push((*mode, object.downgrade()));
        }
        // `enabled` before the four properties it makes meaningful, which the
        // protocol says are "only sent if the output is enabled". scoot's one
        // output is always enabled: there is no disabled state to reach, and
        // no request that could ask for one.
        head.enabled(1);
        if let Some(mode) = mode_object(&modes, current.current_mode) {
            head.current_mode(&mode);
        }
        head.position(current.position.x, current.position.y);
        head.transform(current.transform.into());
        head.scale(current.scale);
        if version >= 4 {
            // True, not a placeholder: scoot has no VRR support on any
            // backend, so adaptive sync is off and nothing can turn it on.
            head.adaptive_sync(AdaptiveSyncState::Disabled);
        }
        self.head = Some(Head {
            head: head.downgrade(),
            modes,
        });
        true
    }

    /// Sends only what changed between `published` and `current`.
    fn update(
        &mut self,
        dh: &DisplayHandle,
        client: &Client,
        published: &HeadState,
        current: &HeadState,
        serial: u32,
    ) -> bool {
        // Taken out so the mode list can be extended while `self` stays free
        // for `give_up`; put back on every path that keeps the manager.
        let Some(mut entry) = self.head.take() else {
            return true;
        };
        let Ok(head) = entry.head.upgrade() else {
            // The client released the head but kept the manager. Its mode
            // objects are kept as they are: they are inert to this client
            // already, and re-announcing a head it asked to be rid of is not
            // something the protocol offers.
            self.head = Some(entry);
            return true;
        };
        // New modes first, so `current_mode` below can name one of them.
        let version = self.manager.version();
        for mode in &current.modes {
            if entry.modes.iter().any(|(known, _)| known == mode) {
                continue;
            }
            let Some(object) = create_mode(dh, client, &head, version, *mode, current) else {
                self.head = Some(entry);
                return self.give_up("zwlr_output_mode_v1", serial);
            };
            entry.modes.push((*mode, object.downgrade()));
        }
        if current.current_mode != published.current_mode {
            // Skipped when the client released the object for the mode that
            // just became current: it said it was done with that object, and
            // a second object for the same mode would break the protocol's
            // "sent once per supported mode".
            if let Some(mode) = mode_object(&entry.modes, current.current_mode) {
                head.current_mode(&mode);
            }
        }
        if current.position != published.position {
            head.position(current.position.x, current.position.y);
        }
        if current.transform != published.transform {
            head.transform(current.transform.into());
        }
        if current.scale != published.scale {
            head.scale(current.scale);
        }
        self.head = Some(entry);
        true
    }

    /// Closes an incomplete batch and retires this manager, for the one
    /// failure that can happen mid-announcement: `create_resource` fails
    /// only when the client is already gone by the time this runs
    /// (`Handle::create_object` errors solely from `get_client_mut` missing
    /// the client; the server's own id allocation for a new object never
    /// fails). Object-id exhaustion is not a real failure mode here -- this
    /// exists for the client that disconnects mid-batch.
    ///
    /// The head goes first (it may have been half described, and the client is
    /// owed a `finished` on it rather than a partial head nothing will ever
    /// correct), then `done`, so a client waiting for one before it redraws is
    /// left out of date rather than waiting forever, then `finished` so it
    /// knows nothing more is coming. `finished` is a destructor event, so
    /// nothing may be sent on the manager afterwards -- hence the immediate
    /// `false`, which drops this entry.
    fn give_up(&mut self, what: &str, serial: u32) -> bool {
        tracing::warn!(
            interface = what,
            "could not create an output-management object; dropping this manager"
        );
        self.retire_head();
        self.manager.done(serial);
        self.manager.finished();
        false
    }
}

/// Creates one `zwlr_output_mode_v1`, introduces it on `head` and states it.
///
/// The object is created at the *manager's* version, which is what a wayland
/// child object inherits from the parent whose event carried its `new_id` --
/// so the client and the compositor agree on the version even though
/// `zwlr_output_mode_v1` maxes out at 3 while the manager maxes out at 4.
/// Nothing at mode version 4 exists to send or receive, so the extra number is
/// inert; this matches what wlroots itself does.
fn create_mode(
    dh: &DisplayHandle,
    client: &Client,
    head: &ZwlrOutputHeadV1,
    version: u32,
    mode: Mode,
    current: &HeadState,
) -> Option<ZwlrOutputModeV1> {
    let object = client
        .create_resource::<ZwlrOutputModeV1, _, State>(dh, version, ModeData)
        .ok()?;
    head.mode(&object);
    object.size(mode.size.w, mode.size.h);
    // "Only sent if the mode has a fixed refresh rate" -- so a zero (or
    // nonsensical negative) rate means "unknown" and is left unsaid rather
    // than advertised as 0 Hz.
    if mode.refresh > 0 {
        object.refresh(mode.refresh);
    }
    if current.preferred_mode == Some(mode) {
        object.preferred();
    }
    Some(object)
}

/// The live object a client has for `mode`, if it has one.
///
/// `None` covers both "never created" and "the client released it", which the
/// callers treat the same way: there is nothing to name in a `current_mode`
/// event.
fn mode_object(
    modes: &[(Mode, Weak<ZwlrOutputModeV1>)],
    mode: Option<Mode>,
) -> Option<ZwlrOutputModeV1> {
    let mode = mode?;
    modes
        .iter()
        .find(|(known, _)| *known == mode)?
        .1
        .upgrade()
        .ok()
}

impl State {
    /// Brings every bound manager up to date with the output.
    ///
    /// Called from the two sites that change the output's state
    /// (`headless::init_named` and [`State::resize_output`], the only callers
    /// of `headless.rs`'s `set_mode`) and from a fresh bind. Costs one
    /// [`HeadState`] read and a compare when nothing changed, and sends
    /// nothing at all in that case -- no `done`, no serial bump.
    pub(super) fn refresh_output_heads(&mut self) {
        let current = self.output.as_ref().map(HeadState::of);
        if self.output_management.published == current {
            return;
        }
        let dh = self.display_handle.clone();
        let management = &mut self.output_management;
        // Wrapping: the protocol's serial is a `uint` with no ordering
        // requirement beyond "a new one per change", and scoot refuses every
        // configuration whatever serial it names, so a wrap after 4 billion
        // mode changes costs nothing. It must not panic in a debug build,
        // which a plain `+= 1` eventually would.
        management.serial = management.serial.wrapping_add(1);
        let serial = management.serial;
        // Moved out so the per-manager loop can borrow `management` mutably.
        let published = management.published.take();
        management
            .managers
            .retain_mut(|manager| manager.apply(&dh, published.as_ref(), current.as_ref(), serial));
        management.published = current;
    }

    /// Builds a freshly bound manager's whole world: the head, its modes, and
    /// the `done` that makes it one atomic picture.
    fn announce_output_heads(
        &mut self,
        dh: &DisplayHandle,
        client: &Client,
        manager: ZwlrOutputManagerV1,
    ) {
        // First, so `published` is what the output says right now. Everything
        // below is built from `published` rather than from the `Output`
        // directly, which is what keeps this manager in lockstep with the ones
        // already bound: the next refresh diffs from a snapshot this client
        // really was sent.
        self.refresh_output_heads();
        let serial = self.output_management.serial;
        let mut entry = Manager {
            manager,
            head: None,
        };
        let announced = match self.output_management.published.as_ref() {
            Some(published) => entry.announce(dh, client, published, serial),
            // No output yet -- only reachable before `headless::init_named`
            // has run. A bare `done` is the honest answer: the client is up to
            // date, there is simply nothing to be up to date about, and the
            // head arrives in its own batch once the output exists.
            None => true,
        };
        if !announced {
            // Counted at bind but never registered, so the claim is given
            // back: the client is gone, and a leak here would be a counter
            // that only grows.
            self.bind_budget
                .release_bind(&client.id(), &entry.manager.id());
            return;
        }
        entry.manager.done(serial);
        self.output_management.managers.push(entry);
    }
}

/// User data on the `zwlr_output_manager_v1` global itself.
struct ManagerGlobalData;

/// ...and on a manager a client has bound.
struct ManagerData;

/// User data on a `zwlr_output_head_v1`.
///
/// Deliberately empty: which output a head stands for is [`Manager::head`],
/// not a field on the object. Storing it here would be a second copy of the
/// same fact, free to drift from the list the events are computed against.
struct HeadData;

/// User data on a `zwlr_output_mode_v1`. Empty for the same reason.
struct ModeData;

impl GlobalDispatch2<ZwlrOutputManagerV1, State> for ManagerGlobalData {
    fn bind(
        &self,
        state: &mut State,
        dh: &DisplayHandle,
        client: &Client,
        resource: New<ZwlrOutputManagerV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        let manager = data_init.init(resource, ManagerData);
        if state.bind_budget.refuse_bind(client, &manager.id()) {
            // Over the shared per-client budget (see `bind_budget.rs`):
            // deferred, not sent here -- `finished` is a destructor event,
            // and sending one inside `bind` panics wayland-backend's bind
            // epilogue. Not counted, not registered, so no later refresh
            // walks it. The serial is the current one, unwound by no refresh:
            // this manager will never build a configuration, so it is purely
            // informational.
            let serial = state.output_management.serial;
            state.defer_bind_refusal(super::bind_budget::RefusedBind::OutputManager(
                manager, serial,
            ));
            return;
        }
        state.announce_output_heads(dh, client, manager);
    }
}

impl Dispatch2<ZwlrOutputManagerV1, State> for ManagerData {
    fn request(
        &self,
        state: &mut State,
        client: &Client,
        manager: &ZwlrOutputManagerV1,
        request: zwlr_output_manager_v1::Request,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            zwlr_output_manager_v1::Request::CreateConfiguration { id, serial: _ } => {
                // The serial is ignored on purpose. It exists so a compositor
                // can `cancelled` a configuration built against stale state;
                // this one refuses every configuration whatever its serial, so
                // checking it would only change *which* refusal a client got,
                // and `cancelled` invites a retry loop that can never succeed.
                data_init.init(id, configuration::ConfigurationData::default());
            }
            zwlr_output_manager_v1::Request::Stop => {
                // Released synchronously rather than left to `destroyed`: a
                // stop-and-rebind in one batch must see the freed slot without
                // waiting for post-batch cleanup. Idempotent with the
                // `destroyed` release -- `finished` queues it, and removing an
                // absent id is a no-op.
                state.bind_budget.release_bind(&client.id(), &manager.id());
                // Unregistered first: `finished` is a destructor event, so the
                // object is gone as soon as it is sent (wayland-backend
                // removes it from the client's object map and queues its
                // `destroyed` callback), and nothing may try to send to it
                // afterwards. `destroyed` runs this same `retain` later, which
                // is idempotent.
                //
                // The client's head and mode objects are deliberately left
                // alone: the protocol's teardown is stop, wait for `finished`,
                // then destroy what it still holds.
                state
                    .output_management
                    .managers
                    .retain(|entry| entry.manager != *manager);
                manager.finished();
            }
            // The request enums are `#[non_exhaustive]`; an opcode this
            // version does not define never reaches here (wayland-backend
            // rejects it first), so there is nothing to do but ignore it --
            // and certainly not panic, which is what `unreachable!()` would
            // make of a future protocol version.
            _ => {}
        }
    }

    fn destroyed(&self, state: &mut State, client: ClientId, manager: &ZwlrOutputManagerV1) {
        // The path an ordinary client disconnect takes, and the only thing
        // that stops a dead client's entry from being walked on every refresh
        // -- and the path its budget claim takes back, including for a bare
        // destroy with no `stop` before it.
        state.bind_budget.release_bind(&client, &manager.id());
        state
            .output_management
            .managers
            .retain(|entry| entry.manager != *manager);
    }
}

impl Dispatch2<ZwlrOutputHeadV1, State> for HeadData {
    /// A head has exactly one request, the version-3 `release` destructor, and
    /// nothing to do for it: wayland-backend destroys the object itself, and
    /// the [`Weak`] held for it simply stops upgrading -- which every send site
    /// here already handles, because a client may release a head while keeping
    /// its manager.
    fn request(
        &self,
        _state: &mut State,
        _client: &Client,
        _head: &ZwlrOutputHeadV1,
        _request: zwlr_output_head_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
    }
}

impl Dispatch2<ZwlrOutputModeV1, State> for ModeData {
    /// Same as the head above: one `release` destructor, nothing to do.
    fn request(
        &self,
        _state: &mut State,
        _client: &Client,
        _mode: &ZwlrOutputModeV1,
        _request: zwlr_output_mode_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
    }
}
