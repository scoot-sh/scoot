//! The module API: the contract every module is written against.
//!
//! A module turns some source of events (a timer, a socket, a netlink fd)
//! into a small declarative [`View`]: text, an optional icon, a state
//! [`Class`] and an optional tooltip. It never touches pixels; the bar
//! measures, lays out and draws (`crate::render`).
//!
//! ## The life of a module
//!
//! 1. **`init`** ([`Spec::init`]) probes and returns [`Init::Available`]
//!    with the module, or [`Init::Unavailable`] saying why. An unavailable
//!    module registers no fds and takes no space, so absent hardware costs
//!    nothing. `init` must not block: a module whose set-up is slow (a bus
//!    connection, a scan) starts it without waiting, returns `Available`
//!    with an empty view, and fills it in from [`Module::on_ready`] when the
//!    answer comes. An empty view takes no space, so the module joins the
//!    bar when ready and never delays the first frame.
//! 2. **[`Module::sources`]** names the fds it wants polled, and for what.
//!    A timer is an fd too (a `timerfd`). Asked on every turn of the loop,
//!    so the set may change at any time; the fds stay the module's own.
//! 3. **[`Module::on_ready`]** handles one ready source and says whether
//!    the view changed ([`Update`]). Only then is the module's area redrawn,
//!    on every output.
//! 4. **[`Module::view`]** fills a [`View`] for one output. It is asked
//!    after an update and when an output's size or scale changes, never per
//!    frame. `output` lets a per-output module (workspaces) differ; most
//!    ignore it.
//!
//! Modules are built once at start-up as trait objects (the only allocation
//! they cost the loop), and a redraw is a handful of virtual calls.
//!
//! ## Adding one
//!
//! One file in this directory, one line in [`REGISTRY`], one Cargo feature
//! in `Cargo.toml` (so a build can leave it out), and tests through
//! [`harness`]. Pointer input (`on_input`) arrives with
//! `docs/scootbar/backlog/pointer-and-interactions.md`, as a method with a
//! default, so no module written before it changes.

use std::fmt::{self, Write};
use std::os::fd::BorrowedFd;

use rustix::event::{PollFd, PollFlags};

use crate::layout::{Layout, Section};

#[cfg(feature = "clock")]
pub mod clock;
#[cfg(test)]
pub mod harness;

#[cfg(test)]
mod tests;

/// A module, once available.
pub trait Module {
    /// Adds each fd this module wants polled now, with the events it wants
    /// (`PollFlags::IN`, mostly). A module's `n`th added source is `n` in
    /// [`Module::on_ready`].
    fn sources<'fd>(&'fd self, sources: &mut Sources<'_, 'fd>);

    /// Source `source` is ready with `events`. Returns whether the view
    /// changed; a module that cannot tell says it did.
    fn on_ready(&mut self, source: usize, events: PollFlags) -> Update;

    /// Fills `view` (cleared by the caller) for `output`.
    fn view(&self, output: &OutputView<'_>, view: &mut View);
}

/// What `init` found.
pub enum Init {
    Available(Box<dyn Module>),
    /// Why not, for one line on stderr.
    Unavailable(String),
}

/// What handling a source did to the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum Update {
    Unchanged,
    Changed,
}

/// A module in the [`REGISTRY`]: its id (what `--left`, `--center` and
/// `--right` name) and how to start it.
pub struct Spec {
    pub id: &'static str,
    pub init: fn(&Settings) -> Init,
}

/// Every module this build has, one line each.
pub const REGISTRY: &[Spec] = &[
    #[cfg(feature = "clock")]
    Spec {
        id: clock::ID,
        init: clock::init,
    },
];

/// The registry entry for `id`.
pub fn find(id: &str) -> Option<&'static Spec> {
    REGISTRY.iter().find(|spec| spec.id == id)
}

/// A started module, where the layout put it.
pub struct Placed {
    pub section: Section,
    pub module: Box<dyn Module>,
    /// Bumped each time the module reports a changed view: the render
    /// compares it with what each output shows (`crate::render`).
    pub revision: u64,
}

impl Placed {
    /// Hands source `source` to the module, and bumps the revision if the
    /// view changed.
    pub fn ready(&mut self, source: usize, events: PollFlags) {
        if self.module.on_ready(source, events) == Update::Changed {
            self.revision = self.revision.wrapping_add(1);
        }
    }
}

/// Starts every module `layout` places, in its order. An unavailable one
/// is said on stderr (through `warn`) and left out: it takes no space and
/// no fd.
pub fn start(
    layout: &Layout,
    settings: &Settings,
    warn: &mut dyn FnMut(&str, &str),
) -> Vec<Placed> {
    let mut placed = Vec::new();
    for (section, id) in layout.placed() {
        // The command line refuses unknown ids; skipped all the same.
        let Some(spec) = find(id) else {
            continue;
        };
        match (spec.init)(settings) {
            Init::Available(module) => placed.push(Placed {
                section,
                module,
                revision: 0,
            }),
            Init::Unavailable(why) => warn(spec.id, &why),
        }
    }
    placed
}

/// Each module's own options, from the command line (a config file's
/// sections later). A module reads only its own.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Settings {
    #[cfg(feature = "clock")]
    pub clock: clock::Settings,
}

/// What a module may know about the output it is asked to show on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputView<'a> {
    /// `wl_output.name` (`DP-1`), if the compositor sent one.
    pub name: Option<&'a str>,
}

/// A state class: how the bar colors a module's view (through the theme's
/// tokens, `crate::theme`). A module never picks colors itself. The clock
/// is always `Normal`; the others are the contract's, for the modules that
/// follow it (battery, volume), and tested through the render.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(dead_code)]
pub enum Class {
    #[default]
    Normal,
    Warn,
    Urgent,
    Muted,
}

/// The most text a view holds, in bytes; more is cut at a character
/// boundary. Bounds what an untrusted string (a window title) can cost.
pub const MAX_TEXT: usize = 256;

/// A module's declarative output for one output. Reused: the bar clears
/// and refills the same one, so its strings allocate once, at most
/// [`MAX_TEXT`] bytes each.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct View {
    text: Bounded,
    tooltip: Bounded,
    icon: Option<char>,
    class: Class,
}

impl View {
    pub fn clear(&mut self) {
        self.text.0.clear();
        self.tooltip.0.clear();
        self.icon = None;
        self.class = Class::Normal;
    }

    /// The text, to write into with `write!`; cut at [`MAX_TEXT`] bytes.
    pub fn text_mut(&mut self) -> &mut impl Write {
        &mut self.text
    }

    /// The tooltip (none while empty), cut at [`MAX_TEXT`] bytes. Drawn
    /// once `docs/scootbar/backlog/tooltips.md` lands. (The rest of this
    /// block is the contract the clock does not use: icon, class, tooltip.)
    #[allow(dead_code)]
    pub fn tooltip_mut(&mut self) -> &mut impl Write {
        &mut self.tooltip
    }

    /// A glyph drawn before the text, from the same font.
    #[allow(dead_code)]
    pub fn set_icon(&mut self, icon: Option<char>) {
        self.icon = icon;
    }

    #[allow(dead_code)]
    pub fn set_class(&mut self, class: Class) {
        self.class = class;
    }

    pub fn text(&self) -> &str {
        &self.text.0
    }

    #[allow(dead_code)]
    pub fn tooltip(&self) -> &str {
        &self.tooltip.0
    }

    pub fn icon(&self) -> Option<char> {
        self.icon
    }

    pub fn class(&self) -> Class {
        self.class
    }

    /// Whether it shows nothing: it then takes no space.
    pub fn is_empty(&self) -> bool {
        self.text.0.is_empty() && self.icon.is_none()
    }
}

/// A string that stops growing at [`MAX_TEXT`] bytes: a write past it is
/// cut at the last character that fits, and reports an error so a
/// formatter stops there.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Bounded(String);

impl Write for Bounded {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let room = MAX_TEXT.saturating_sub(self.0.len());
        if s.len() <= room {
            self.0.push_str(s);
            return Ok(());
        }
        let mut cut = room;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        self.0.push_str(s.get(..cut).unwrap_or(""));
        Err(fmt::Error)
    }
}

/// The fds the loop polls this turn, filled by each module's
/// [`Module::sources`]. A fixed array on the loop's stack: polling
/// allocates nothing.
pub struct Sources<'s, 'fd> {
    fds: &'s mut [PollFd<'fd>],
    /// For each fd: the module that added it and its index there.
    owners: &'s mut [(usize, usize)],
    len: &'s mut usize,
    module: usize,
    next: usize,
}

impl<'s, 'fd> Sources<'s, 'fd> {
    /// For the loop: `fds` and `owners` from index `*len` on are free, and
    /// what `module` adds lands there.
    pub fn new(
        fds: &'s mut [PollFd<'fd>],
        owners: &'s mut [(usize, usize)],
        len: &'s mut usize,
        module: usize,
    ) -> Self {
        Self {
            fds,
            owners,
            len,
            module,
            next: 0,
        }
    }

    /// Polls `fd` for `events`. Past the loop's capacity (`MAX_POLL` fds in
    /// all, far more than the registry's modules add) the fd is not polled,
    /// and this returns `false`.
    pub fn add(&mut self, fd: BorrowedFd<'fd>, events: PollFlags) -> bool {
        let index = *self.len;
        let source = self.next;
        self.next += 1;
        let (Some(slot), Some(owner)) = (self.fds.get_mut(index), self.owners.get_mut(index))
        else {
            return false;
        };
        *slot = PollFd::from_borrowed_fd(fd, events);
        *owner = (self.module, source);
        *self.len = index + 1;
        true
    }
}
