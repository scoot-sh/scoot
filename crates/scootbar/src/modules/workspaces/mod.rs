//! The workspaces module: each output's workspace numbers, the active one
//! in a pill, switching on click, scrolling through them on request.
//!
//! An `ext-workspace-v1` client. The compositor announces one group per
//! output and, per group, workspace handles with a `name`, one-element
//! `coordinates` and an `active` state bit, closing every batch with
//! `done`. The module shows each output its own group's numbers, marks the
//! active one, and sends `activate` then `commit` for the pill clicked
//! (`on_input` names the workspace, `invoke` does the sending: the same
//! path `on-click = "activate 3"`, `on-scroll-down = "next"` and an agent's
//! `invoke` take).
//!
//! ## Positions, not identities
//!
//! Handles are positions: scoot sends no `id`, renumbers past a dropped
//! workspace, and names an adopted workspace `"2 DP-1"`. So nothing is
//! remembered across batches except the last `done`'s committed state:
//! names are parsed to their leading number at event time (an adopted
//! workspace shows its number), workspaces sort by `coordinates` (not name:
//! `"10"` sorts before `"2"`), and a click acts on the committed handle,
//! never an older one. A name with no leading number (a foreign
//! compositor's free-form name) shows its 1-based position.
//!
//! ## Batching: staged, committed, never half-drawn
//!
//! Events between two `done`s mutate staged state only; `done` sorts and
//! commits it and bumps the generation, which [`Module::on_dispatch`]
//! reports. A batch without `done` therefore never redraws, and a manager
//! `finished` mid-batch drops the staged half (restoring the last commit)
//! without bumping, so it is never drawn half-updated. Window-churn floods
//! cost one redraw: any number of `done`s before the loop's next draw is
//! still one draw.
//!
//! ## Fixed bounds, no allocation past start-up
//!
//! Groups and workspaces live in fixed arrays ([`MAX_GROUPS`],
//! [`MAX_WORKSPACES`]), so events, frames and clicks allocate nothing. Past
//! either bound the extras are left out (said once on stderr); both are far
//! past anything reachable (scoot has at most 8 outputs, and a workspace
//! needs a window to exist). The 256-byte view bound
//! ([`super::MAX_TEXT`]) still caps hostile counts or names.
//!
//! ## The pill and the hit test
//!
//! The active workspace's pill is drawn by [`Module::custom_draw`] as an
//! accent fill behind its number, with the number itself in the bar's
//! background color; the rest draws as plain text. Its shape (`rect` by
//! default, `pill`, `circle`), radius and inset are [`pill`]'s.
//! [`Module::on_input`] hit-tests the items' rects, walked with
//! [`Text::advance`] exactly as the text draw walks its pen, so the rects
//! match the ink pixel for pixel; and the active pill's own drawn extent
//! ([`pill_geometry`], the one the draw uses) wins first, so a circle
//! grown past its item never turns a click on it into a neighbour's.

use std::cell::RefCell;
use std::fmt::Write;
use std::rc::Rc;

use wayland_client::Proxy;
use wayland_client::protocol::wl_output::WlOutput;
use wayland_protocols::ext::workspace::v1::client::ext_workspace_group_handle_v1::ExtWorkspaceGroupHandleV1;
use wayland_protocols::ext::workspace::v1::client::ext_workspace_handle_v1::ExtWorkspaceHandleV1;
use wayland_protocols::ext::workspace::v1::client::ext_workspace_manager_v1::ExtWorkspaceManagerV1;

use super::{
    ActionSpec, ArgKind, CustomDraw, Init, Input, InvokeError, Measure, Module, OutputView,
    Sources, Update, View,
};
pub use pill::{Pill, Shape};

use crate::action::{Action, ModuleAction, Trigger};
use crate::color::Color;
use crate::density::Scale;
use crate::paint::Span;
use crate::print::warn;
use crate::text::Text;

pub mod pill;
#[cfg(test)]
mod tests;

/// The id `--left`, `--center` and `--right` name it by.
pub const ID: &str = "workspaces";

/// What the module shows: the workspace numbers, or a dot each.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Display {
    #[default]
    Numbers,
    Dots,
}

impl Display {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "numbers" => Some(Self::Numbers),
            "dots" => Some(Self::Dots),
            _ => None,
        }
    }
}

/// The actions a binding may name (`activate 3`, `activate-position 1`,
/// `previous`, `next`).
pub const ACTIONS: &[ActionSpec] = &[
    // By the number shown: what a user means by "workspace 3".
    ActionSpec {
        name: "activate",
        arg: ArgKind::Required,
    },
    // By place in the list as drawn, 1 first: the click's own, exact even
    // where two items show the same number (an adopted `"2 DP-1"` beside a
    // native 2).
    ActionSpec {
        name: "activate-position",
        arg: ArgKind::Required,
    },
    ActionSpec {
        name: "previous",
        arg: ArgKind::None,
    },
    ActionSpec {
        name: "next",
        arg: ArgKind::None,
    },
];

/// Groups tracked at most: one per output.
const MAX_GROUPS: usize = 8;
/// Workspaces per group, staged, committed and unassigned each.
const MAX_WORKSPACES: usize = 32;

/// The most spaces `item-gap` takes between two numbers: the view's text is
/// cut at [`super::MAX_TEXT`] bytes, and at 8 spaces that still holds 28
/// single-digit workspaces (about 26 numbered up to 99).
pub const MAX_ITEM_GAP: u32 = 8;
/// Output names kept at most, in bytes: `wl_output` names are short
/// (`DP-1`, `HEADLESS-1`); longer ones are cut.
const MAX_NAME: usize = 64;

/// The module's options: none of its own. The link is plumbing, not
/// configuration: a fresh daemon run's shared workspace state, held here so
/// the daemon's Wayland dispatch and this module see the same one. Equal by
/// construction, so command-line parsing still compares.
#[derive(Debug, Clone)]
pub struct Settings {
    pub link: Link,
    pub pill: Pill,
    /// Spaces between the numbers, `1..=`[`MAX_ITEM_GAP`]: one space is about
    /// a third of the font size.
    pub item_gap: u32,
    /// The active pill's fill, `None` for the `accent` token.
    pub active_color: Option<Color>,
    /// Inactive numbers' ink, `None` for the `normal` class's token.
    pub inactive_color: Option<Color>,
    /// What the module shows: the numbers, or a dot each (`dots` draws
    /// itself, and clicks land by the dots' places).
    pub display: Display,
    /// With a `circle` pill: grow the module's own span to the disc's
    /// diameter, so a single digit is a disc at any padding.
    pub disc: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            link: Link::default(),
            pill: Pill::default(),
            item_gap: 1,
            active_color: None,
            inactive_color: None,
            display: Display::default(),
            disc: false,
        }
    }
}

impl PartialEq for Settings {
    fn eq(&self, other: &Self) -> bool {
        // The link is plumbing, equal by construction.
        self.pill == other.pill
            && self.item_gap == other.item_gap
            && self.active_color == other.active_color
            && self.inactive_color == other.inactive_color
            && self.display == other.display
            && self.disc == other.disc
    }
}

impl Eq for Settings {}

/// One daemon run's workspace state, shared between the module and the
/// daemon's Wayland dispatch.
#[derive(Debug, Clone, Default)]
pub struct Link(pub(crate) Rc<RefCell<Shared>>);

/// Everything the protocol told the bar, staged and committed.
///
/// Staged state is what the current batch says; committed is what the last
/// `done` closed. Groups are staged in place (added on sight, swept at
/// `done` when removed); workspaces are staged per group, with announced
/// but unassigned handles in `pending` until their `workspace_enter`.
#[derive(Debug, Default)]
pub struct Shared {
    manager: Option<ExtWorkspaceManagerV1>,
    /// False after the manager's `finished`: no more events will come, and
    /// no request may be sent on it (the server destroyed its side; a
    /// request would be a protocol error). The committed view stays as the
    /// last thing known.
    live: bool,
    /// Bumped at every `done`; [`Workspaces::seen`] trails it.
    generation: u64,
    groups: [Group; MAX_GROUPS],
    groups_len: usize,
    pending: [Ws; MAX_WORKSPACES],
    pending_len: usize,
    /// Output names seen on `wl_output`, by proxy identity: a group's name
    /// is applied when it enters the output, whichever event came first
    /// (scoot sends `name` before the groups' `output_enter`).
    known: [KnownName; MAX_GROUPS],
    known_len: usize,
    /// Said once: past a bound, or created nothing to show.
    said_no_room: bool,
}

impl Shared {
    /// Whether the manager was bound: the daemon bound it, or the
    /// compositor lacks the protocol.
    pub(crate) fn has_manager(&self) -> bool {
        self.manager.is_some()
    }
}

/// One output's group and its workspaces.
#[derive(Debug, Clone)]
struct Group {
    group: Option<ExtWorkspaceGroupHandleV1>,
    output: Option<WlOutput>,
    name: [u8; MAX_NAME],
    name_len: usize,
    staged: [Ws; MAX_WORKSPACES],
    staged_len: usize,
    committed: [Ws; MAX_WORKSPACES],
    committed_len: usize,
    /// `removed` since the last `done`: swept then, never drawn meanwhile.
    staged_dead: bool,
}

impl Default for Group {
    fn default() -> Self {
        Self {
            group: None,
            output: None,
            name: [0; MAX_NAME],
            name_len: 0,
            staged: std::array::from_fn(|_| Ws::default()),
            staged_len: 0,
            committed: std::array::from_fn(|_| Ws::default()),
            committed_len: 0,
            staged_dead: false,
        }
    }
}

impl Group {
    fn name(&self) -> &str {
        std::str::from_utf8(&self.name[..self.name_len]).unwrap_or("")
    }

    fn set_name(&mut self, name: &str) {
        set_name_bytes(&mut self.name, &mut self.name_len, name);
    }
}

/// One output's name, kept by proxy identity so a group entering it later
/// still learns it.
#[derive(Debug, Clone)]
struct KnownName {
    output: Option<WlOutput>,
    name: [u8; MAX_NAME],
    name_len: usize,
}

impl Default for KnownName {
    fn default() -> Self {
        Self {
            output: None,
            name: [0; MAX_NAME],
            name_len: 0,
        }
    }
}

impl KnownName {
    fn set_name(&mut self, name: &str) {
        set_name_bytes(&mut self.name, &mut self.name_len, name);
    }
}

/// Copies `name` into fixed bytes, cut at a character boundary.
fn set_name_bytes(into: &mut [u8; MAX_NAME], len: &mut usize, name: &str) {
    let bytes = name.as_bytes();
    let room = bytes.len().min(MAX_NAME);
    into[..room].copy_from_slice(&bytes[..room]);
    let mut cut = room;
    while cut > 0 && std::str::from_utf8(&into[..cut]).is_err() {
        cut -= 1;
    }
    *len = cut;
}

/// One workspace: what the view and the hit test need, plus the handle a
/// click acts on (from the last `done`, never older).
#[derive(Debug, Clone, Default)]
struct Ws {
    handle: Option<ExtWorkspaceHandleV1>,
    /// The name's leading number; 0 when it had none (shown as its
    /// position instead).
    number: u32,
    /// The first `coordinates` element; 0 when never sent (arrival order
    /// then, the sort being stable).
    coord: u32,
    active: bool,
}

/// Destroys a workspace handle once (staged and committed hold clones of
/// one handle, and a destroyed one is no longer alive).
fn destroy_workspace(ws: &Ws) {
    if let Some(handle) = ws.handle.as_ref().filter(|h| h.is_alive()) {
        handle.destroy();
    }
}

/// The leading ASCII digits of a workspace name (`"2 DP-1"` shows `2`); 0
/// when there are none.
fn parse_number(name: &str) -> u32 {
    let digits = name.bytes().take_while(u8::is_ascii_digit).count().min(9);
    name[..digits].parse().unwrap_or(0)
}

/// The first `coordinates` element, native byte order, as the protocol
/// carries a `uint` array; 0 for a short or empty array.
fn parse_coord(coordinates: &[u8]) -> u32 {
    coordinates
        .get(..4)
        .and_then(|four| four.try_into().ok())
        .map(u32::from_ne_bytes)
        .unwrap_or(0)
}

impl Shared {
    fn no_room(&mut self, what: &str) {
        if !self.said_no_room {
            self.said_no_room = true;
            warn(format_args!(
                "scootbar: workspaces: past its bound ({what}); leaving the rest out"
            ));
        }
    }

    pub(crate) fn set_manager(&mut self, manager: ExtWorkspaceManagerV1) {
        self.manager = Some(manager);
        self.live = true;
    }

    /// The manager, for the daemon to tell it from a stale one (one it
    /// asked to `stop`, whose last events may still be on the wire).
    pub(crate) fn manager(&self) -> Option<&ExtWorkspaceManagerV1> {
        self.manager.as_ref()
    }

    /// Lets the protocol go: no workspaces module is placed any more
    /// (`daemon::binds`). Stops the manager (the compositor answers
    /// `finished` and sends nothing after), destroys every handle it made,
    /// and forgets what they said, so a later bind starts clean. The output
    /// names stay: they come from `wl_output`, not from this protocol.
    /// Bumps the generation, so a module placed again draws from nothing.
    pub(crate) fn release(&mut self) {
        if let Some(manager) = self.manager.take() {
            if manager.is_alive() {
                manager.stop();
            }
        }
        self.live = false;
        for ws in self.pending[..self.pending_len].iter() {
            destroy_workspace(ws);
        }
        for group in &mut self.groups[..self.groups_len] {
            for ws in group.staged[..group.staged_len]
                .iter()
                .chain(&group.committed[..group.committed_len])
            {
                destroy_workspace(ws);
            }
            if let Some(handle) = group.group.as_ref().filter(|h| h.is_alive()) {
                handle.destroy();
            }
        }
        self.groups = std::array::from_fn(|_| Group::default());
        self.groups_len = 0;
        self.pending = std::array::from_fn(|_| Ws::default());
        self.pending_len = 0;
        self.said_no_room = false;
        self.generation = self.generation.wrapping_add(1);
    }

    fn group_index(&self, group: &ExtWorkspaceGroupHandleV1) -> Option<usize> {
        self.groups[..self.groups_len]
            .iter()
            .position(|entry| entry.group.as_ref() == Some(group))
    }

    /// The staged workspace `handle`, wherever it is (unassigned or in a
    /// group), for the `name`/`coordinates`/`state` events, which may arrive
    /// before its `workspace_enter`.
    fn staged_mut(&mut self, handle: &ExtWorkspaceHandleV1) -> Option<&mut Ws> {
        if let Some(found) = self.pending[..self.pending_len]
            .iter_mut()
            .find(|ws| ws.handle.as_ref() == Some(handle))
        {
            return Some(found);
        }
        self.groups[..self.groups_len]
            .iter_mut()
            .flat_map(|group| group.staged[..group.staged_len].iter_mut())
            .find(|ws| ws.handle.as_ref() == Some(handle))
    }

    /// A `workspace_group` event: staged on sight, committed at `done`.
    pub(crate) fn on_group(&mut self, group: ExtWorkspaceGroupHandleV1) {
        if self.groups_len >= MAX_GROUPS {
            self.no_room("groups");
            return;
        }
        // A group re-announced (a bind race): keep the one place.
        if let Some(index) = self.group_index(&group) {
            self.groups[index].staged_dead = false;
            return;
        }
        self.groups[self.groups_len] = Group {
            group: Some(group),
            ..Group::default()
        };
        self.groups_len += 1;
    }

    /// A group's `removed`: swept at the next `done`, so the batch it
    /// closes still draws whole.
    pub(crate) fn on_group_removed(&mut self, group: &ExtWorkspaceGroupHandleV1) {
        if let Some(index) = self.group_index(group) {
            self.groups[index].staged_dead = true;
        }
    }

    /// A group's `output_enter`: whose output it stands on, by proxy
    /// identity (a replugged monitor is a new object, never a stale name).
    /// The name is whatever [`Shared::note_output_name`] last said for that
    /// object, whenever it arrived relative to this event.
    pub(crate) fn on_output_enter(&mut self, group: &ExtWorkspaceGroupHandleV1, output: WlOutput) {
        if let Some(index) = self.group_index(group) {
            // The name kept for this object, if its `name` event already
            // arrived (copied out first: no allocation on this path).
            let mut remembered = [0u8; MAX_NAME];
            let mut remembered_len = 0;
            let mut found = false;
            for known in &self.known[..self.known_len] {
                if known.output.as_ref() == Some(&output) {
                    remembered = known.name;
                    remembered_len = known.name_len;
                    found = true;
                    break;
                }
            }
            let entry = &mut self.groups[index];
            entry.output = Some(output);
            if found {
                entry.name = remembered;
                entry.name_len = remembered_len;
            }
        }
    }

    /// A group's `output_leave`: stands on nothing until entered again.
    pub(crate) fn on_output_leave(&mut self, group: &ExtWorkspaceGroupHandleV1, output: &WlOutput) {
        if let Some(index) = self.group_index(group) {
            let entry = &mut self.groups[index];
            if entry.output.as_ref() == Some(output) {
                entry.output = None;
                entry.name_len = 0;
            }
        }
    }

    /// The output's `name` event: kept by object for groups entering it
    /// later, and applied to every group standing on it now.
    pub(crate) fn note_output_name(&mut self, output: &WlOutput, name: &str) {
        if let Some(known) = self.known[..self.known_len]
            .iter_mut()
            .find(|known| known.output.as_ref() == Some(output))
        {
            known.set_name(name);
        } else if self.known_len < MAX_GROUPS {
            self.known[self.known_len].output = Some(output.clone());
            self.known[self.known_len].set_name(name);
            self.known_len += 1;
        } else {
            self.no_room("output names");
        }
        for entry in &mut self.groups[..self.groups_len] {
            if entry.output.as_ref() != Some(output) {
                continue;
            }
            entry.set_name(name);
        }
    }

    /// The output went away under the group (its `wl_output` global
    /// removed): forget the object, so a replugged monitor under the same
    /// name never matches this group's stale entry.
    pub(crate) fn purge_output(&mut self, output: &WlOutput) {
        for entry in &mut self.groups[..self.groups_len] {
            if entry.output.as_ref() == Some(output) {
                entry.output = None;
                entry.name_len = 0;
                entry.staged_dead = true;
            }
        }
        if let Some(slot) = self.known[..self.known_len]
            .iter()
            .position(|known| known.output.as_ref() == Some(output))
        {
            self.known.swap(slot, self.known_len - 1);
            self.known[self.known_len - 1] = KnownName::default();
            self.known_len -= 1;
        }
    }

    /// A manager `workspace` event: announced, unassigned until its group's
    /// `workspace_enter`.
    pub(crate) fn on_workspace(&mut self, handle: ExtWorkspaceHandleV1) {
        if self.pending_len >= MAX_WORKSPACES {
            self.no_room("workspaces");
            return;
        }
        self.pending[self.pending_len] = Ws {
            handle: Some(handle),
            ..Ws::default()
        };
        self.pending_len += 1;
    }

    /// A group's `workspace_enter`: takes the announced handle into the
    /// group's staged list.
    pub(crate) fn on_workspace_enter(
        &mut self,
        group: &ExtWorkspaceGroupHandleV1,
        handle: &ExtWorkspaceHandleV1,
    ) {
        let Some(index) = self.group_index(group) else {
            return;
        };
        let Some(slot) = self.pending[..self.pending_len]
            .iter()
            .position(|ws| ws.handle.as_ref() == Some(handle))
        else {
            return;
        };
        let entry = &mut self.groups[index];
        if entry.staged_len >= MAX_WORKSPACES {
            self.no_room("workspaces");
            return;
        }
        self.pending.swap(slot, self.pending_len - 1);
        let ws = std::mem::take(&mut self.pending[self.pending_len - 1]);
        self.pending_len -= 1;
        entry.staged[entry.staged_len] = ws;
        entry.staged_len += 1;
    }

    /// A group's `workspace_leave`: out of the staged list.
    pub(crate) fn on_workspace_leave(
        &mut self,
        group: &ExtWorkspaceGroupHandleV1,
        handle: &ExtWorkspaceHandleV1,
    ) {
        let Some(index) = self.group_index(group) else {
            return;
        };
        let entry = &mut self.groups[index];
        if let Some(slot) = entry.staged[..entry.staged_len]
            .iter()
            .position(|ws| ws.handle.as_ref() == Some(handle))
        {
            entry.staged.swap(slot, entry.staged_len - 1);
            entry.staged_len -= 1;
        }
    }

    pub(crate) fn on_workspace_name(&mut self, handle: &ExtWorkspaceHandleV1, name: &str) {
        if let Some(ws) = self.staged_mut(handle) {
            ws.number = parse_number(name);
        }
    }

    pub(crate) fn on_workspace_coordinates(
        &mut self,
        handle: &ExtWorkspaceHandleV1,
        coordinates: &[u8],
    ) {
        if let Some(ws) = self.staged_mut(handle) {
            ws.coord = parse_coord(coordinates);
        }
    }

    pub(crate) fn on_workspace_state(&mut self, handle: &ExtWorkspaceHandleV1, active: bool) {
        if let Some(ws) = self.staged_mut(handle) {
            ws.active = active;
        }
    }

    /// A workspace `removed`: gone from wherever it was staged.
    pub(crate) fn on_workspace_removed(&mut self, handle: &ExtWorkspaceHandleV1) {
        if let Some(slot) = self.pending[..self.pending_len]
            .iter()
            .position(|ws| ws.handle.as_ref() == Some(handle))
        {
            self.pending.swap(slot, self.pending_len - 1);
            self.pending_len -= 1;
        }
        for group in &mut self.groups[..self.groups_len] {
            if let Some(slot) = group.staged[..group.staged_len]
                .iter()
                .position(|ws| ws.handle.as_ref() == Some(handle))
            {
                group.staged.swap(slot, group.staged_len - 1);
                group.staged_len -= 1;
            }
        }
    }

    /// The manager's `done`: the batch is whole, so dead groups are swept,
    /// every staged list is committed (sorted by coordinates, stably), and
    /// the generation moves: one `done`, at most one redraw.
    pub(crate) fn on_done(&mut self) {
        let mut kept = 0;
        for index in 0..self.groups_len {
            if self.groups[index].staged_dead {
                continue;
            }
            if index != kept {
                self.groups.swap(index, kept);
            }
            kept += 1;
        }
        self.groups_len = kept;
        for group in &mut self.groups[..self.groups_len] {
            group.committed[..group.staged_len].clone_from_slice(&group.staged[..group.staged_len]);
            group.committed_len = group.staged_len;
            group.committed[..group.committed_len].sort_by_key(|ws| ws.coord);
        }
        self.generation = self.generation.wrapping_add(1);
    }

    /// The manager's `finished` mid-batch: the staged half is dropped
    /// (restored from the commit), so it is never drawn; the generation
    /// does not move, so nothing redraws. No request may be sent after.
    pub(crate) fn on_finished(&mut self) {
        self.live = false;
        self.pending_len = 0;
        for group in &mut self.groups[..self.groups_len] {
            group.staged[..group.committed_len]
                .clone_from_slice(&group.committed[..group.committed_len]);
            group.staged_len = group.committed_len;
            group.staged_dead = false;
        }
    }

    /// The committed group for the output named `name`, if it has
    /// workspaces to show.
    fn committed_for(&self, name: Option<&str>) -> Option<usize> {
        let name = name?;
        self.groups[..self.groups_len]
            .iter()
            .position(|group| group.name() == name && group.committed_len > 0)
    }
}

pub fn init(settings: &super::Settings) -> Init {
    Init::Available(Box::new(Workspaces {
        link: settings.workspaces.link.clone(),
        pill: settings.workspaces.pill,
        item_gap: settings.workspaces.item_gap.clamp(1, MAX_ITEM_GAP),
        active_color: settings.workspaces.active_color,
        inactive_color: settings.workspaces.inactive_color,
        display: settings.workspaces.display,
        disc: settings.workspaces.disc,
        seen: 0,
    }))
}

/// The module: an `Rc` to the shared state plus the generation last drawn.
pub struct Workspaces {
    link: Link,
    pill: Pill,
    /// Spaces between two numbers in the view's text.
    item_gap: u32,
    active_color: Option<Color>,
    inactive_color: Option<Color>,
    display: Display,
    disc: bool,
    seen: u64,
}

impl Workspaces {
    /// The committed workspace `(number, active)` pairs for `output`, in
    /// the order the view shows them. A workspace the protocol never named
    /// (`number` 0) shows its 1-based position.
    fn items(&self, output: &OutputView<'_>, shown: &mut [(u32, bool)], count: &mut usize) {
        let shared = self.link.0.borrow();
        let Some(index) = shared.committed_for(output.name) else {
            *count = 0;
            return;
        };
        let group = &shared.groups[index];
        let len = group.committed_len.min(shown.len());
        for (index, (slot, ws)) in shown.iter_mut().zip(&group.committed[..len]).enumerate() {
            *slot = (shown_number(ws, index) as u32, ws.active);
        }
        *count = len;
    }

    fn write_view(&self, output: &OutputView<'_>, view: &mut View) {
        let mut items = [(0u32, false); MAX_WORKSPACES];
        let mut count = 0;
        self.items(output, &mut items, &mut count);
        for (index, &(number, _)) in items[..count].iter().enumerate() {
            if index > 0 {
                for _ in 0..self.item_gap {
                    let _ = view.text_mut().write_char(' ');
                }
            }
            // Dots are drawn by `custom_draw`, never as text: the cell is
            // one ordinary character, so measuring and the hit test walk
            // the dots' own places.
            if self.display == Display::Dots {
                let _ = view.text_mut().write_char('o');
            } else {
                let _ = write!(view.text_mut(), "{number}");
            }
        }
    }

    /// A dot per committed workspace: the active one filled like the
    /// pill, the rest dim (or the configured state colors). One fill
    /// each, at most [`MAX_WORKSPACES`]: the hit test walks the same
    /// cells the view's text makes, so clicks land on the dots.
    fn draw_dots(&self, ctx: &mut CustomDraw<'_, '_>, group: &Group) -> bool {
        let committed = &group.committed[..group.committed_len];
        if committed.is_empty() {
            return false;
        }
        let full = ctx.view.text();
        let metrics = ctx.text.metrics(ctx.em);
        let line = (metrics.ascent - metrics.descent).ceil().max(0.0) as u32;
        let (top, bottom) = self.pill.rows(ctx.canvas.height(), line, ctx.scale);
        let middle = top / 2 + bottom / 2;
        let gap = (ctx.text.advance(' ', ctx.em) * self.item_gap as f32)
            .round()
            .max(0.0) as u32;
        for (index, ws) in committed.iter().enumerate() {
            let Some((start, end)) =
                item_span(ctx.text, full, ctx.em, i64::from(ctx.padding), index)
            else {
                continue;
            };
            let width = end.saturating_sub(start);
            // No taller than the line, and never into the next dot.
            let diameter = line.min(width.saturating_add(gap)).max(1);
            let center = start / 2 + end / 2;
            let radius = diameter / 2;
            let x = center.saturating_sub(radius);
            let y = middle.saturating_sub(radius);
            let color = if ws.active {
                if ctx.hovered {
                    ctx.theme.hover
                } else {
                    self.active_color.unwrap_or(ctx.theme.accent)
                }
            } else {
                self.inactive_color.unwrap_or(ctx.theme.dim)
            };
            ctx.canvas.fill_pill(
                Span { x, width: diameter },
                y,
                y.saturating_add(diameter),
                u32::MAX,
                color,
            );
        }
        true
    }
    /// How much wider than its text the module measures with `disc`: the
    /// disc's diameter less the active number's width (the rest of the
    /// text stays), so the circle the pill grows into fits the span. 0
    /// without `disc`, without a circle, showing dots, or with no active
    /// workspace (nothing is drawn to fit).
    fn disc_extra(&self, measure: &Measure<'_>) -> u32 {
        if !self.disc || self.pill.shape != Shape::Circle || self.display != Display::Numbers {
            return 0;
        }
        let mut items = [(0u32, false); MAX_WORKSPACES];
        let mut count = 0;
        self.items(&measure.output, &mut items, &mut count);
        let Some(active) = items[..count].iter().position(|&(_, active)| active) else {
            return 0;
        };
        let full = measure.view.text();
        let Some((start, end)) = item_span(measure.text, full, measure.em, 0, active) else {
            return 0;
        };
        let metrics = measure.text.metrics(measure.em);
        let line = (metrics.ascent - metrics.descent).ceil().max(0.0) as u32;
        let rows = self.pill.rows(measure.height, line, measure.scale);
        rows.1
            .saturating_sub(rows.0)
            .saturating_sub(end.saturating_sub(start))
    }
}

/// What an `invoke`d action asks of the workspaces.
enum Op {
    /// Switch to the first workspace showing this number.
    Activate(i32),
    /// Switch to the workspace at this place in the list as drawn, 1 first.
    Position(i32),
    /// Move the active one by the step count: forward or back.
    Step(bool),
}

/// The number workspace `ws`, at position `index`, shows as: the leading
/// number of its name, or its 1-based position when the protocol never
/// named it. What `activate N` means too.
fn shown_number(ws: &Ws, index: usize) -> i32 {
    let number = if ws.number == 0 {
        (index as u32).saturating_add(1)
    } else {
        ws.number
    };
    i32::try_from(number).unwrap_or(i32::MAX)
}

/// One item's device-pixel span within the view's text, walked exactly as
/// [`Text::draw`] walks its pen (from `x0`, stepping each character's
/// advance), so the bounds match the ink. A run of spaces (`item-gap`) is one
/// gap between two items, not several. `None` past the last item.
fn item_span(text: &Text, full: &str, em: f32, x0: i64, want: usize) -> Option<(u32, u32)> {
    let mut pen = x0 as f32;
    let mut index = 0;
    let mut start = pen;
    let mut in_gap = false;
    for c in full.chars() {
        if c == ' ' {
            if !in_gap {
                if index == want {
                    return Some((pixels(start), pixels(pen)));
                }
                index += 1;
                in_gap = true;
            }
            pen += text.advance(' ', em);
            start = pen;
        } else if !c.is_control() {
            in_gap = false;
            pen += text.advance(c, em);
        }
    }
    (index == want).then(|| (pixels(start), pixels(pen)))
}

fn pixels(value: f32) -> u32 {
    value.round().max(0.0) as u32
}

/// The item whose pill holds `x`: each item's span ([`item_span`]) padded
/// by half the module padding, `x` in device pixels from the content's
/// start (past the padding). `None` in a gap or past the items.
fn hit_index(text: &Text, full: &str, em: f32, pad: u32, count: usize, x: u32) -> Option<usize> {
    let step = pad / 2;
    for item in 0..count {
        let Some((start, end)) = item_span(text, full, em, i64::from(pad), item) else {
            break;
        };
        if (start.saturating_sub(step)..end.saturating_add(step)).contains(&x) {
            return Some(item);
        }
    }
    None
}

/// Where the active item's pill goes, span-relative device pixels: its
/// horizontal extent and rows. What the draw paints and the hit test
/// honors, so they cannot disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Geometry {
    lo: u32,
    hi: u32,
    top: u32,
    bottom: u32,
}

/// Item `active`'s pill in a module span `span_width` wide on a bar
/// `height` tall, everything device pixels at `scale`. `None` when there
/// is nothing to paint (no such item, an empty extent).
#[allow(clippy::too_many_arguments)]
fn pill_geometry(
    pill: &Pill,
    text: &Text,
    full: &str,
    em: f32,
    padding: u32,
    active: usize,
    span_width: u32,
    height: u32,
    scale: Scale,
) -> Option<Geometry> {
    let (start, end) = item_span(text, full, em, i64::from(padding), active)?;
    let side = padding / 2;
    let lo = start.saturating_sub(side);
    let hi = end.saturating_add(side).min(span_width);
    if hi <= lo {
        return None;
    }
    // What the pill may grow into: up to the neighbours' ink, but never
    // less than the item's own padded extent.
    let before = active
        .checked_sub(1)
        .and_then(|n| item_span(text, full, em, i64::from(padding), n))
        .map_or(0, |(_, end)| end);
    let after = item_span(text, full, em, i64::from(padding), active + 1)
        .map_or(span_width, |(start, _)| start);
    let room = (lo.min(before), hi.max(after.min(span_width)));
    let metrics = text.metrics(em);
    let line = (metrics.ascent - metrics.descent).ceil().max(0.0) as u32;
    let (top, bottom) = pill.rows(height, line, scale);
    let (lo, hi) = pill.extent((lo, hi), room, (top, bottom));
    Some(Geometry {
        lo,
        hi,
        top,
        bottom,
    })
}

/// The item a press at `x` (span-relative device pixels) means: the active
/// one when `x` is on its pill (`active`: its index and drawn extent), else
/// the item whose padded rect holds it ([`hit_index`]).
fn hit_target(
    text: &Text,
    full: &str,
    em: f32,
    pad: u32,
    count: usize,
    x: u32,
    active: Option<(usize, Geometry)>,
) -> Option<usize> {
    if let Some((item, geometry)) = active {
        if (geometry.lo..geometry.hi).contains(&x) {
            return Some(item);
        }
    }
    hit_index(text, full, em, pad, count, x)
}

impl Module for Workspaces {
    fn sources<'fd>(&'fd self, _sources: &mut Sources<'_, 'fd>) {}

    fn on_ready(&mut self, _source: usize, _events: rustix::event::PollFlags) -> Update {
        Update::Unchanged
    }

    /// A `done` since the last turn: the committed view moved.
    fn on_dispatch(&mut self) -> Update {
        let generation = self.link.0.borrow().generation;
        if generation == self.seen {
            return Update::Unchanged;
        }
        self.seen = generation;
        Update::Changed
    }

    fn view(&self, output: &OutputView<'_>, view: &mut View) {
        self.write_view(output, view);
    }

    /// A click on one of this output's pills means `activate-position` for
    /// the place it landed on. Anything else (no group here, past the items,
    /// the active pill itself, any other input) means nothing: a scroll
    /// has no default here (`previous` and `next` are for a binding).
    fn on_input(&self, input: &Input<'_>) -> Option<Action> {
        if input.trigger != Trigger::Click {
            return None;
        }
        let ctx = input.at;
        let shared = self.link.0.borrow();
        let index = shared.committed_for(ctx.output.name)?;
        let group = &shared.groups[index];
        // Relative to the span's start, like `ctx.x`: the content begins
        // past the padding.
        let full = ctx.view.text();
        // Dots draw no pill: their cells are the targets, so the
        // numbers' pill geometry must not win first (a grown circle
        // reaches past the active cell into the gaps).
        if self.display == Display::Dots {
            let hit = hit_index(
                ctx.text,
                full,
                ctx.em,
                ctx.padding,
                group.committed_len,
                ctx.x,
            )?;
            let ws = &group.committed[hit];
            if ws.active {
                return None;
            }
            let position = i32::try_from(hit).unwrap_or(i32::MAX).saturating_add(1);
            return Some(Action::Module(ModuleAction::new(
                "activate-position",
                Some(position),
            )));
        }
        let active = group.committed[..group.committed_len]
            .iter()
            .position(|ws| ws.active)
            .and_then(|item| {
                let geometry = pill_geometry(
                    &self.pill,
                    ctx.text,
                    full,
                    ctx.em,
                    ctx.padding,
                    item,
                    ctx.span_width,
                    ctx.height,
                    ctx.scale,
                )?;
                Some((item, geometry))
            });
        let hit = hit_target(
            ctx.text,
            full,
            ctx.em,
            ctx.padding,
            group.committed_len,
            ctx.x,
            active,
        )?;
        let ws = &group.committed[hit];
        if ws.active {
            // Already there: the compositor would no-op, so ask for nothing.
            return None;
        }
        // By position, not number: two items can show one number, and the
        // click is for the one it landed on.
        let position = i32::try_from(hit).unwrap_or(i32::MAX).saturating_add(1);
        Some(Action::Module(ModuleAction::new(
            "activate-position",
            Some(position),
        )))
    }

    /// `activate N` switches to the workspace showing number `N` on
    /// `output` (the first, if two show it), `activate-position N` to the
    /// `N`th as drawn; `previous` and `next` move the active one by `steps`
    /// places, stopping at the ends (no wrap). The view changes when the
    /// compositor answers with a `done`, reported through
    /// [`Module::on_dispatch`], so these never report `Changed`.
    fn invoke(
        &mut self,
        output: &OutputView<'_>,
        action: &ModuleAction,
        steps: u32,
    ) -> Result<Update, InvokeError> {
        // What was asked, checked before anything is looked up: a bad name
        // or number is an error whatever the compositor is doing.
        let op = match (&*action.name, action.arg) {
            ("activate", Some(number)) => Op::Activate(number),
            ("activate-position", Some(position)) => Op::Position(position),
            ("activate" | "activate-position", None) => return Err(InvokeError::NeedsArg),
            ("previous" | "next", Some(_)) => return Err(InvokeError::NoArg),
            ("previous", None) => Op::Step(false),
            ("next", None) => Op::Step(true),
            _ => return Err(InvokeError::Unknown),
        };
        let shared = self.link.0.borrow();
        let Some(group_index) = shared.committed_for(output.name) else {
            return Err(InvokeError::Refused("no workspaces on this output"));
        };
        let group = &shared.groups[group_index];
        let committed = &group.committed[..group.committed_len];
        let target = match op {
            Op::Activate(number) => committed
                .iter()
                .enumerate()
                .position(|(index, ws)| shown_number(ws, index) == number)
                .ok_or(InvokeError::Refused("no workspace has that number here"))?,
            Op::Position(position) => usize::try_from(position)
                .ok()
                .and_then(|position| position.checked_sub(1))
                .filter(|&index| index < committed.len())
                .ok_or(InvokeError::Refused(
                    "no workspace is at that position here",
                ))?,
            Op::Step(forward) => {
                let Some(active) = committed.iter().position(|ws| ws.active) else {
                    return Ok(Update::Unchanged);
                };
                let steps = steps.max(1) as usize;
                if forward {
                    active
                        .saturating_add(steps)
                        .min(committed.len().saturating_sub(1))
                } else {
                    active.saturating_sub(steps)
                }
            }
        };
        // The action was valid; whether anything can be sent is next. A dead
        // manager (the compositor sent `finished`) takes no request: one
        // past it is a protocol error, which would kill the bar. The
        // module shows the last thing it knew, and nothing is sent.
        let (Some(manager), true) = (shared.manager.clone(), shared.live) else {
            return Ok(Update::Unchanged);
        };
        let ws = &committed[target];
        if ws.active {
            // Already there: the compositor would no-op, so send nothing.
            return Ok(Update::Unchanged);
        }
        let Some(handle) = ws.handle.clone() else {
            return Ok(Update::Unchanged);
        };
        // `Removed` sweeps the handle from staged at once but from
        // committed only at the next `done`: the pill is still drawn
        // meanwhile, and a press landing on it (or a button queued ahead
        // of the `done` in the same dispatch) must not `activate` a
        // destroyed handle — a protocol error, which would kill the bar.
        // Past a `done` staged and committed agree as sets (committed is
        // staged sorted, so the check is membership, never position), and
        // the other staged mutations keep every committed handle staged,
        // so steady-state clicks are unchanged.
        if !group.staged[..group.staged_len]
            .iter()
            .any(|staged| staged.handle.as_ref() == Some(&handle))
        {
            return Ok(Update::Unchanged);
        }
        drop(shared);
        handle.activate();
        manager.commit();
        Ok(Update::Unchanged)
    }

    /// `{"active": 2, "workspaces": [1, 2, 3]}` for `output`: the shown
    /// numbers and the active one's (`null` when none is); `None` before
    /// the compositor has said anything for this output.
    fn value(&self, output: &OutputView<'_>) -> Option<serde_json::Value> {
        let mut items = [(0u32, false); MAX_WORKSPACES];
        let mut count = 0;
        self.items(output, &mut items, &mut count);
        let shown = items.get(..count)?;
        if shown.is_empty() {
            return None;
        }
        let active = shown
            .iter()
            .find(|(_, active)| *active)
            .map(|(number, _)| *number);
        Some(serde_json::json!({
            "active": active,
            "workspaces": shown.iter().map(|(number, _)| *number).collect::<Vec<_>>(),
        }))
    }

    /// Clicking switches workspaces with no binding at all.
    fn handles_input(&self) -> bool {
        true
    }

    /// The pill is the module's own look; it tints with the `hover` token
    /// like a bound module, through [`Module::custom_draw`].
    fn tints_on_hover(&self) -> bool {
        true
    }

    /// With `disc`, the span the pill grows into (see [`Workspaces::disc_extra`]).
    fn span_extra(&self, measure: &Measure<'_>) -> u32 {
        self.disc_extra(measure)
    }

    /// The pill behind the active workspace: an accent fill over its item
    /// span, the number itself in the bar's background, the rest as plain
    /// text; `false` (the plain draw) when there is nothing to mark.
    fn custom_draw(&self, ctx: &mut CustomDraw<'_, '_>) -> bool {
        let shared = self.link.0.borrow();
        let Some(index) = shared.committed_for(ctx.output.name) else {
            return false;
        };
        let group = &shared.groups[index];
        if self.display == Display::Dots {
            let drawn = self.draw_dots(ctx, group);
            drop(shared);
            return drawn;
        }
        let active = group.committed[..group.committed_len]
            .iter()
            .position(|ws| ws.active);
        let Some(active) = active else {
            return false;
        };
        let full = ctx.view.text();
        let x0 = i64::from(ctx.span.x) + i64::from(ctx.padding);
        let Some(geometry) = pill_geometry(
            &self.pill,
            ctx.text,
            full,
            ctx.em,
            ctx.padding,
            active,
            ctx.span.width,
            ctx.canvas.height(),
            ctx.scale,
        ) else {
            return false;
        };
        let pill = Span {
            x: ctx.span.x.saturating_add(geometry.lo),
            width: geometry.hi - geometry.lo,
        };
        // The borrows end here: the draws below take the canvas and the
        // text, not the shared state.
        drop(shared);
        // Hover wins over the configured color, which wins over the
        // accent: the tint is the old one unless something says otherwise.
        let fill = if ctx.hovered {
            ctx.theme.hover
        } else {
            self.active_color.unwrap_or(ctx.theme.accent)
        };
        ctx.canvas.fill_pill(
            pill,
            geometry.top,
            geometry.bottom,
            self.pill.radius(ctx.scale),
            fill,
        );
        let background = ctx.theme.background;
        let ink = self
            .inactive_color
            .unwrap_or(ctx.theme.class(ctx.view.class()));
        let x = ctx.span.x;
        ctx.text.draw(
            ctx.canvas,
            None,
            full,
            ctx.em,
            x0,
            ctx.baseline,
            background,
            pill,
        );
        if pill.x > x {
            ctx.text.draw(
                ctx.canvas,
                None,
                full,
                ctx.em,
                x0,
                ctx.baseline,
                ink,
                Span {
                    x,
                    width: pill.x - x,
                },
            );
        }
        if pill.end() < ctx.span.end() {
            ctx.text.draw(
                ctx.canvas,
                None,
                full,
                ctx.em,
                x0,
                ctx.baseline,
                ink,
                Span {
                    x: pill.end(),
                    width: ctx.span.end() - pill.end(),
                },
            );
        }
        true
    }
}
