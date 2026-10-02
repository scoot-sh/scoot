//! The window-title module: the focused window's title, click to focus.
//!
//! A `wlr-foreign-toplevel-management-v1` client. The compositor announces
//! one handle per window, carrying `title`, `app_id`, `output_enter` and a
//! `state` array with the `activated` bit, closed by `done`. The module
//! shows each output the title of the window activated on it, and sends
//! `activate` for a click on it (or `close` for a middle click, when the
//! config allows it).
//!
//! ## Why only the wlr protocol
//!
//! The ticket names both window-list protocols, but only the wlr one can
//! drive this module: `ext-foreign-toplevel-list-v1` has no `activated`
//! state, no output events and no requests, and the two protocols share no
//! client-visible key (the ext list's `identifier` names a window id the
//! wlr handle never carries), so a bar cannot correlate their handles.
//! The daemon binds only the wlr manager, and only while this module is
//! placed, like the workspaces module and its manager.
//!
//! ## Batching: applied at once, drawn once
//!
//! Events mutate the shared state and bump a generation right away; there
//! is no staged half to draw wrong (a title without its focus is still the
//! truth of that batch). [`Module::on_dispatch`] reports the move once per
//! loop turn however many events arrived, so a retitle flood is one redraw
//! per turn, and a title-only change past the first in
//! [`TITLE_INTERVAL`] is held for the flush timer instead (focus, output
//! and close changes always draw at once: interactive latency is not
//! capped, only progress readouts are). The timerfd exists only while a
//! flush is held, so an idle module owns no fd.
//!
//! ## Fixed bounds, no allocation past start-up
//!
//! Toplevels live in a fixed array ([`MAX_TOPLEVELS`]), titles and app ids
//! in fixed bytes, so events, frames and clicks allocate nothing. Past the
//! bound the extras are left out (said once on stderr). The 256-byte view
//! bound ([`super::MAX_TEXT`]) still caps a hostile title, control
//! characters are stripped before they reach the view (the glyph cache
//! never sees them), and the tooltip carries the full title for `query`.
//!
//! ## Truncation
//!
//! The module's span is capped at `max-width` logical pixels
//! ([`Module::max_width`]), so the title yields the bar to the other
//! modules; longer text is drawn cut with an ellipsis by
//! [`Module::custom_draw`], measured in device pixels, never counted in
//! characters.

use std::cell::RefCell;
use std::fmt::Write;
use std::io;
use std::os::fd::{AsFd, BorrowedFd};
use std::rc::Rc;
use std::time::{Duration, Instant};

use rustix::event::PollFlags;
use rustix::time::{
    Itimerspec, TimerfdClockId, TimerfdFlags, Timespec, timerfd_create, timerfd_settime,
};
use wayland_client::Proxy;
use wayland_client::protocol::wl_output::WlOutput;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_protocols_wlr::foreign_toplevel::v1::client::zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1;
use wayland_protocols_wlr::foreign_toplevel::v1::client::zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1;

use super::{
    ActionSpec, ArgKind, CustomDraw, Init, Input, InvokeError, MAX_TEXT, Module, OutputView,
    Sources, Update, View,
};
use crate::action::{Action, ModuleAction, Trigger};
use crate::print::warn;

#[cfg(test)]
mod tests;

/// The id `--left`, `--center` and `--right` name it by.
pub const ID: &str = "window-title";

/// The actions a binding may name (`activate`, `close`).
pub const ACTIONS: &[ActionSpec] = &[
    ActionSpec {
        name: "activate",
        arg: ArgKind::None,
    },
    ActionSpec {
        name: "close",
        arg: ArgKind::None,
    },
];

/// Toplevels tracked at most: past it the extras are left out.
const MAX_TOPLEVELS: usize = 64;
/// Title bytes kept per toplevel: the view's bound, so what is stored is
/// what can be shown.
const MAX_TITLE: usize = MAX_TEXT;
/// App-id bytes kept per toplevel: app ids are short (`org.foo.Bar`);
/// longer ones are cut.
const MAX_APP_ID: usize = 64;
/// Outputs one window is remembered on: windows stand on one output;
/// more is room for moves mid-batch.
const MAX_WINDOW_OUTPUTS: usize = 4;
/// Output names kept at most, in bytes: `wl_output` names are short
/// (`DP-1`, `HEADLESS-1`); longer ones are cut.
const MAX_NAME: usize = 64;

/// Title-only changes draw at most this often; focus, output and close
/// changes always draw at once.
const TITLE_INTERVAL: Duration = Duration::from_millis(100);

/// The default `max-width`, in logical pixels: about half a 1080p bar, so
/// the title yields the rest to the other modules.
pub const DEFAULT_MAX_WIDTH: u32 = 480;
/// The most `max-width` takes, in logical pixels.
pub const MAX_MAX_WIDTH: u32 = 4096;

/// The module's options. The link is plumbing, not configuration: a fresh
/// daemon run's shared toplevel state, held here so the daemon's Wayland
/// dispatch and this module see the same one. Equal by construction, so
/// command-line parsing still compares.
#[derive(Debug, Clone)]
pub struct Settings {
    pub link: Link,
    pub show_app_id: bool,
    pub max_width: u32,
    pub placeholder: String,
    pub allow_close: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            link: Link::default(),
            show_app_id: false,
            max_width: DEFAULT_MAX_WIDTH,
            placeholder: String::new(),
            allow_close: false,
        }
    }
}

impl PartialEq for Settings {
    fn eq(&self, other: &Self) -> bool {
        // The link is plumbing, equal by construction.
        self.show_app_id == other.show_app_id
            && self.max_width == other.max_width
            && self.placeholder == other.placeholder
            && self.allow_close == other.allow_close
    }
}

impl Eq for Settings {}

/// One daemon run's toplevel state, shared between the module and the
/// daemon's Wayland dispatch.
#[derive(Debug, Clone, Default)]
pub struct Link(pub(crate) Rc<RefCell<Shared>>);

/// Everything the protocol told the bar: one entry per announced
/// toplevel, applied at once (no staging: a title without its focus is
/// still the truth of that batch).
#[derive(Debug)]
pub struct Shared {
    manager: Option<ZwlrForeignToplevelManagerV1>,
    /// False after the manager's `finished`: no more events will come, and
    /// no request may be sent on it (the server destroyed its side; a
    /// request would be a protocol error). The committed view stays as the
    /// last thing known.
    live: bool,
    /// Bumped at every change; [`WindowTitle::seen_gen`] trails it.
    generation: u64,
    /// Bumped when focus, outputs or lifetime changed (not on a bare
    /// title): [`WindowTitle`] draws those at once and caps only titles.
    focus_gen: u64,
    toplevels: [Tl; MAX_TOPLEVELS],
    toplevels_len: usize,
    /// The seat to activate on: the daemon keeps it while one is bound.
    seat: Option<WlSeat>,
    /// Output names seen on `wl_output`, by proxy identity: a window's
    /// `output_enter` is matched against them at view time, whichever event
    /// came first.
    known: [KnownName; 8],
    known_len: usize,
    /// Said once: past a bound.
    said_no_room: bool,
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            manager: None,
            live: false,
            generation: 0,
            focus_gen: 0,
            toplevels: std::array::from_fn(|_| Tl::default()),
            toplevels_len: 0,
            seat: None,
            known: std::array::from_fn(|_| KnownName::default()),
            known_len: 0,
            said_no_room: false,
        }
    }
}

impl Shared {
    /// Whether the manager was bound: the daemon bound it, or the
    /// compositor lacks the protocol.
    pub(crate) fn has_manager(&self) -> bool {
        self.manager.is_some()
    }
}

/// One toplevel: what the view needs, plus the handle `activate` and
/// `close` act on.
#[derive(Debug, Clone)]
struct Tl {
    handle: Option<ZwlrForeignToplevelHandleV1>,
    title: [u8; MAX_TITLE],
    title_len: usize,
    app_id: [u8; MAX_APP_ID],
    app_id_len: usize,
    outputs: [Standing; MAX_WINDOW_OUTPUTS],
    outputs_len: usize,
    activated: bool,
    fullscreen: bool,
}

impl Default for Tl {
    fn default() -> Self {
        Self {
            handle: None,
            title: [0; MAX_TITLE],
            title_len: 0,
            app_id: [0; MAX_APP_ID],
            app_id_len: 0,
            outputs: std::array::from_fn(|_| Standing::default()),
            outputs_len: 0,
            activated: false,
            fullscreen: false,
        }
    }
}

impl Tl {
    fn title(&self) -> &str {
        std::str::from_utf8(&self.title[..self.title_len]).unwrap_or("")
    }

    fn app_id(&self) -> &str {
        std::str::from_utf8(&self.app_id[..self.app_id_len]).unwrap_or("")
    }
}

/// One output a window stands on: whose output (by proxy identity, for
/// `output_leave` to find it) and what it is called (copied at
/// `output_enter` when known, backfilled when the `wl_output` name
/// arrives later, so the view matches by name either way).
#[derive(Debug, Clone)]
struct Standing {
    output: Option<WlOutput>,
    name: [u8; MAX_NAME],
    name_len: usize,
}

impl Default for Standing {
    fn default() -> Self {
        Self {
            output: None,
            name: [0; MAX_NAME],
            name_len: 0,
        }
    }
}

impl Standing {
    fn name(&self) -> &str {
        std::str::from_utf8(&self.name[..self.name_len]).unwrap_or("")
    }

    fn set_name(&mut self, name: &str) {
        store(&mut self.name, &mut self.name_len, name);
    }
}

/// One output's name, kept by proxy identity.
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
    fn name(&self) -> &str {
        std::str::from_utf8(&self.name[..self.name_len]).unwrap_or("")
    }

    fn set_name(&mut self, name: &str) {
        store(&mut self.name, &mut self.name_len, name);
    }
}

/// Copies `text` into fixed bytes, cut at a character boundary.
fn store<const N: usize>(into: &mut [u8; N], len: &mut usize, text: &str) {
    let bytes = text.as_bytes();
    let room = bytes.len().min(N);
    into[..room].copy_from_slice(&bytes[..room]);
    let mut cut = room;
    while cut > 0 && std::str::from_utf8(&into[..cut]).is_err() {
        cut -= 1;
    }
    *len = cut;
}

/// Destroys a toplevel handle once.
fn destroy(toplevel: &Tl) {
    if let Some(handle) = toplevel.handle.as_ref().filter(|h| h.is_alive()) {
        handle.destroy();
    }
}

impl Shared {
    fn no_room(&mut self, what: &str) {
        if !self.said_no_room {
            self.said_no_room = true;
            warn(format_args!(
                "scootbar: window-title: past its bound ({what}); leaving the rest out"
            ));
        }
    }

    fn bump(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    fn bump_focus(&mut self) {
        self.bump();
        self.focus_gen = self.focus_gen.wrapping_add(1);
    }

    pub(crate) fn set_manager(&mut self, manager: ZwlrForeignToplevelManagerV1) {
        self.manager = Some(manager);
        self.live = true;
    }

    /// The manager, for the daemon to tell it from a stale one (one it
    /// asked to `stop`, whose last events may still be on the wire).
    pub(crate) fn manager(&self) -> Option<&ZwlrForeignToplevelManagerV1> {
        self.manager.as_ref()
    }

    /// Lets the protocol go: no window-title module is placed any more
    /// (`daemon::binds`). Stops the manager (the compositor answers
    /// `finished` and sends nothing after), destroys every handle it made,
    /// and forgets what they said, so a later bind starts clean. Bumps
    /// both generations, so a module placed again draws from nothing.
    pub(crate) fn release(&mut self) {
        if let Some(manager) = self.manager.take() {
            if manager.is_alive() {
                manager.stop();
            }
        }
        self.live = false;
        for toplevel in &self.toplevels[..self.toplevels_len] {
            destroy(toplevel);
        }
        self.toplevels = std::array::from_fn(|_| Tl::default());
        self.toplevels_len = 0;
        self.seat = None;
        self.said_no_room = false;
        self.bump_focus();
    }

    /// The seat `activate` is sent on, kept by the daemon while one is
    /// bound.
    pub(crate) fn set_seat(&mut self, seat: &WlSeat) {
        if self.seat.as_ref() != Some(seat) {
            self.seat = Some(seat.clone());
        }
    }

    /// The seat is gone (or nothing needs it any more): `activate` has
    /// nothing to ride on until one is back.
    pub(crate) fn clear_seat(&mut self) {
        self.seat = None;
    }

    fn index(&self, handle: &ZwlrForeignToplevelHandleV1) -> Option<usize> {
        self.toplevels[..self.toplevels_len]
            .iter()
            .position(|toplevel| toplevel.handle.as_ref() == Some(handle))
    }

    /// A manager `toplevel` event: one entry per announced window.
    pub(crate) fn on_toplevel(&mut self, handle: ZwlrForeignToplevelHandleV1) {
        if self.index(&handle).is_some() {
            return;
        }
        if self.toplevels_len >= MAX_TOPLEVELS {
            self.no_room("toplevels");
            handle.destroy();
            return;
        }
        self.toplevels[self.toplevels_len] = Tl {
            handle: Some(handle),
            ..Tl::default()
        };
        self.toplevels_len += 1;
        self.bump_focus();
    }

    pub(crate) fn on_title(&mut self, handle: &ZwlrForeignToplevelHandleV1, title: &str) {
        if let Some(index) = self.index(handle) {
            let toplevel = &mut self.toplevels[index];
            store(&mut toplevel.title, &mut toplevel.title_len, title);
            self.bump();
        }
    }

    pub(crate) fn on_app_id(&mut self, handle: &ZwlrForeignToplevelHandleV1, app_id: &str) {
        if let Some(index) = self.index(handle) {
            let toplevel = &mut self.toplevels[index];
            store(&mut toplevel.app_id, &mut toplevel.app_id_len, app_id);
            self.bump();
        }
    }

    pub(crate) fn on_output_enter(
        &mut self,
        handle: &ZwlrForeignToplevelHandleV1,
        output: WlOutput,
    ) {
        if let Some(index) = self.index(handle) {
            let known = self.output_name(&output).unwrap_or("");
            let mut name = [0u8; MAX_NAME];
            let mut name_len = 0;
            store(&mut name, &mut name_len, known);
            let toplevel = &mut self.toplevels[index];
            if toplevel.outputs[..toplevel.outputs_len]
                .iter()
                .any(|standing| standing.output.as_ref() == Some(&output))
            {
                return;
            }
            if toplevel.outputs_len >= MAX_WINDOW_OUTPUTS {
                return;
            }
            toplevel.outputs[toplevel.outputs_len] = Standing {
                output: Some(output),
                name,
                name_len,
            };
            toplevel.outputs_len += 1;
            self.bump_focus();
        }
    }

    pub(crate) fn on_output_leave(
        &mut self,
        handle: &ZwlrForeignToplevelHandleV1,
        output: &WlOutput,
    ) {
        if let Some(index) = self.index(handle) {
            let toplevel = &mut self.toplevels[index];
            if let Some(slot) = toplevel.outputs[..toplevel.outputs_len]
                .iter()
                .position(|standing| standing.output.as_ref() == Some(output))
            {
                toplevel.outputs.swap(slot, toplevel.outputs_len - 1);
                toplevel.outputs[toplevel.outputs_len - 1] = Standing::default();
                toplevel.outputs_len -= 1;
                self.bump_focus();
            }
        }
    }

    pub(crate) fn on_state(
        &mut self,
        handle: &ZwlrForeignToplevelHandleV1,
        activated: bool,
        fullscreen: bool,
    ) {
        if let Some(index) = self.index(handle) {
            let toplevel = &mut self.toplevels[index];
            if toplevel.activated != activated || toplevel.fullscreen != fullscreen {
                toplevel.activated = activated;
                toplevel.fullscreen = fullscreen;
                self.bump_focus();
            }
        }
    }

    /// A handle `closed`: gone at once (its death is the event; nothing
    /// may be sent on it after), so it is swept now rather than at a
    /// `done` that may never come.
    pub(crate) fn on_closed(&mut self, handle: &ZwlrForeignToplevelHandleV1) {
        if let Some(index) = self.index(handle) {
            destroy(&self.toplevels[index]);
            self.toplevels.swap(index, self.toplevels_len - 1);
            self.toplevels[self.toplevels_len - 1] = Tl::default();
            self.toplevels_len -= 1;
            self.bump_focus();
        }
    }

    /// The manager's `finished`: the committed view stays as the last
    /// thing known, and nothing is sent any more.
    pub(crate) fn on_finished(&mut self) {
        self.live = false;
    }

    /// The output's `name` event: kept by object for windows standing on
    /// it, and backfilled into every window standing there whose
    /// `output_enter` came first.
    pub(crate) fn note_output_name(&mut self, output: &WlOutput, name: &str) {
        if let Some(known) = self.known[..self.known_len]
            .iter_mut()
            .find(|known| known.output.as_ref() == Some(output))
        {
            known.set_name(name);
        } else if self.known_len < self.known.len() {
            self.known[self.known_len].output = Some(output.clone());
            self.known[self.known_len].set_name(name);
            self.known_len += 1;
        } else {
            self.no_room("output names");
            return;
        }
        for toplevel in &mut self.toplevels[..self.toplevels_len] {
            for standing in &mut toplevel.outputs[..toplevel.outputs_len] {
                if standing.output.as_ref() == Some(output) {
                    standing.set_name(name);
                }
            }
        }
        // A name learned late can show a window: move the focus
        // generation, so it draws at once rather than waiting out a held
        // title.
        self.bump_focus();
    }

    /// The output went away (its `wl_output` global removed): forget the
    /// object, so a replugged monitor under the same name never matches a
    /// stale entry.
    pub(crate) fn purge_output(&mut self, output: &WlOutput) {
        for toplevel in &mut self.toplevels[..self.toplevels_len] {
            let mut kept = 0;
            for index in 0..toplevel.outputs_len {
                if toplevel.outputs[index].output.as_ref() == Some(output) {
                    continue;
                }
                if index != kept {
                    toplevel.outputs.swap(index, kept);
                }
                kept += 1;
            }
            for standing in &mut toplevel.outputs[kept..toplevel.outputs_len] {
                *standing = Standing::default();
            }
            toplevel.outputs_len = kept;
        }
        if let Some(slot) = self.known[..self.known_len]
            .iter()
            .position(|known| known.output.as_ref() == Some(output))
        {
            self.known.swap(slot, self.known_len - 1);
            self.known[self.known_len - 1] = KnownName::default();
            self.known_len -= 1;
        }
        self.bump_focus();
    }

    /// The name of `output`, if its `wl_output` name event arrived.
    fn output_name(&self, output: &WlOutput) -> Option<&str> {
        self.known[..self.known_len]
            .iter()
            .find(|known| known.output.as_ref() == Some(output))
            .map(KnownName::name)
    }

    /// The activated toplevel standing on the output named `name`, if any.
    /// The first, if the compositor ever activates two at once.
    fn focused_on(&self, name: Option<&str>) -> Option<usize> {
        let name = name?;
        self.toplevels[..self.toplevels_len]
            .iter()
            .position(|toplevel| {
                toplevel.activated
                    && toplevel.outputs[..toplevel.outputs_len]
                        .iter()
                        .any(|standing| standing.name() == name)
            })
    }
}

pub fn init(settings: &super::Settings) -> Init {
    Init::Available(Box::new(WindowTitle {
        link: settings.window_title.link.clone(),
        show_app_id: settings.window_title.show_app_id,
        max_width: settings.window_title.max_width.clamp(1, MAX_MAX_WIDTH),
        placeholder: settings.window_title.placeholder.clone(),
        allow_close: settings.window_title.allow_close,
        seen_gen: 0,
        seen_focus: 0,
        last_title: None,
        flush: None,
        armed: false,
    }))
}

/// The module: an `Rc` to the shared state plus the generations last
/// drawn, and the flush timer for held-back title changes.
pub struct WindowTitle {
    link: Link,
    show_app_id: bool,
    max_width: u32,
    placeholder: String,
    allow_close: bool,
    seen_gen: u64,
    seen_focus: u64,
    /// When a title-only change was last drawn: later ones within
    /// [`TITLE_INTERVAL`] wait for the flush timer.
    last_title: Option<Instant>,
    /// Created on the first held-back change, armed while one is held.
    flush: Option<Flush>,
    armed: bool,
}

/// The flush timer: a `timerfd` armed for one relative deadline, read when
/// it fires.
struct Flush {
    fd: std::os::fd::OwnedFd,
}

impl Flush {
    fn new() -> io::Result<Self> {
        let fd = timerfd_create(
            TimerfdClockId::Monotonic,
            TimerfdFlags::CLOEXEC | TimerfdFlags::NONBLOCK,
        )?;
        Ok(Self { fd })
    }

    fn arm(&self) {
        let millis = TITLE_INTERVAL.as_millis() as i64;
        let spec = Itimerspec {
            it_interval: Timespec {
                tv_sec: 0,
                tv_nsec: 0,
            },
            it_value: Timespec {
                tv_sec: millis / 1000,
                tv_nsec: (millis % 1000) * 1_000_000,
            },
        };
        // A monotonic timerfd armed relative cannot fail usefully here;
        // the expiry is advisory (the next event flushes anyway).
        let _ = timerfd_settime(&self.fd, rustix::time::TimerfdTimerFlags::empty(), &spec);
    }

    fn read(&self) {
        let mut expirations = [0u8; 8];
        let _ = rustix::io::read(&self.fd, &mut expirations);
    }
}

impl AsFd for Flush {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

/// Writes `text` without control characters: they draw as nothing, and
/// never reach the glyph cache.
fn push_sanitized(into: &mut impl Write, text: &str) {
    for c in text.chars().filter(|c| !c.is_control()) {
        let _ = into.write_char(c);
    }
}

impl WindowTitle {
    /// The module's text for `output`: the focused window's title (and app
    /// id, if configured), the app id alone when the title is empty, or
    /// the placeholder when none is focused. Sanitized: no control
    /// characters.
    fn write_text(&self, output: &OutputView<'_>, view: &mut View) {
        let shared = self.link.0.borrow();
        let Some(index) = shared.focused_on(output.name) else {
            push_sanitized(view.text_mut(), &self.placeholder);
            return;
        };
        let toplevel = &shared.toplevels[index];
        let title = toplevel.title();
        let app_id = toplevel.app_id();
        if title.is_empty() {
            push_sanitized(view.text_mut(), app_id);
        } else {
            push_sanitized(view.text_mut(), title);
            if self.show_app_id && !app_id.is_empty() {
                let _ = view.text_mut().write_str(" - ");
                push_sanitized(view.text_mut(), app_id);
            }
        }
    }

    /// The full title for the tooltip: what the ellipsis may cut.
    fn write_tooltip(&self, output: &OutputView<'_>, view: &mut View) {
        let shared = self.link.0.borrow();
        let Some(index) = shared.focused_on(output.name) else {
            return;
        };
        let toplevel = &shared.toplevels[index];
        let title = toplevel.title();
        let app_id = toplevel.app_id();
        if title.is_empty() && app_id.is_empty() {
            return;
        }
        // The tooltip is carried for tooltips (not drawn yet) and for
        // `query`: the whole title, uncut by the span.
        if title.is_empty() {
            push_sanitized(view.tooltip_mut(), app_id);
        } else {
            push_sanitized(view.tooltip_mut(), title);
            if self.show_app_id && !app_id.is_empty() {
                let _ = view.tooltip_mut().write_str(" - ");
                push_sanitized(view.tooltip_mut(), app_id);
            }
        }
    }

    /// Holds back a title-only change for the flush timer: within
    /// [`TITLE_INTERVAL`] of the last title draw it costs no redraw.
    /// `true` when held (the timer is armed).
    fn hold_back(&mut self) -> bool {
        if self.flush.is_none() {
            match Flush::new() {
                Ok(flush) => self.flush = Some(flush),
                // No fd to arm (out of descriptors): draw at once rather
                // than hold what nothing would flush.
                Err(_) => return false,
            }
        }
        if let Some(flush) = &self.flush {
            flush.arm();
            self.armed = true;
            true
        } else {
            false
        }
    }
}

/// What an `invoke`d action asks of the title.
enum Op {
    Activate,
    Close,
}

impl Module for WindowTitle {
    /// The flush timer, while a title change is held for it; nothing
    /// otherwise (this source is event-driven, no polling).
    fn sources<'fd>(&'fd self, sources: &mut Sources<'_, 'fd>) {
        if self.armed {
            if let Some(flush) = &self.flush {
                sources.add(flush.as_fd(), PollFlags::IN);
            }
        }
    }

    /// The flush timer fired: draw the held-back title now.
    fn on_ready(&mut self, _source: usize, _events: PollFlags) -> Update {
        if let Some(flush) = &self.flush {
            flush.read();
        }
        self.armed = false;
        self.seen_gen = self.link.0.borrow().generation;
        self.last_title = Some(Instant::now());
        Update::Changed
    }

    /// The state moved since the last turn: focus, output and close
    /// changes draw at once; a title-only change within
    /// [`TITLE_INTERVAL`] of the last title draw waits for the flush
    /// timer, so a retitle flood is at most ten redraws a second however
    /// many events it sends.
    fn on_dispatch(&mut self) -> Update {
        let (generation, focus_gen) = {
            let shared = self.link.0.borrow();
            (shared.generation, shared.focus_gen)
        };
        if generation == self.seen_gen {
            return Update::Unchanged;
        }
        if focus_gen != self.seen_focus {
            self.seen_gen = generation;
            self.seen_focus = focus_gen;
            self.last_title = Some(Instant::now());
            self.armed = false;
            return Update::Changed;
        }
        let now = Instant::now();
        if self
            .last_title
            .is_some_and(|last| now.duration_since(last) < TITLE_INTERVAL)
            && self.hold_back()
        {
            return Update::Unchanged;
        }
        self.seen_gen = generation;
        self.last_title = Some(now);
        Update::Changed
    }

    fn view(&self, output: &OutputView<'_>, view: &mut View) {
        self.write_text(output, view);
        self.write_tooltip(output, view);
    }

    /// A left click activates the focused window; a middle click closes it
    /// when the config allows closing. Anything else (no window shown
    /// here, any other input) means nothing: scroll has no default here.
    fn on_input(&self, input: &Input<'_>) -> Option<Action> {
        let trigger = match input.trigger {
            Trigger::Click => Op::Activate,
            Trigger::MiddleClick if self.allow_close => Op::Close,
            _ => return None,
        };
        let shared = self.link.0.borrow();
        // No window shown here: nothing to act on (a placeholder is not a
        // window, and a press on it must not warn).
        shared.focused_on(input.at.output.name)?;
        Some(Action::Module(ModuleAction::new(
            match trigger {
                Op::Activate => "activate",
                Op::Close => "close",
            },
            None,
        )))
    }

    /// `activate` focuses the window shown on `output`; `close` closes it,
    /// when the config allows closing. The view changes when the
    /// compositor answers, reported through [`Module::on_dispatch`], so
    /// these never report `Changed`.
    fn invoke(
        &mut self,
        output: &OutputView<'_>,
        action: &ModuleAction,
        steps: u32,
    ) -> Result<Update, InvokeError> {
        let _ = steps;
        // What was asked, checked before anything is looked up: a bad name
        // or number is an error whatever the compositor is doing.
        let op = match (&*action.name, action.arg) {
            ("activate", None) => Op::Activate,
            ("close", None) => Op::Close,
            ("activate", Some(_)) | ("close", Some(_)) => return Err(InvokeError::NoArg),
            _ => return Err(InvokeError::Unknown),
        };
        if matches!(op, Op::Close) && !self.allow_close {
            return Err(InvokeError::Refused(
                "closing is off: set window-title.allow-close = true",
            ));
        }
        let shared = self.link.0.borrow();
        let Some(index) = shared.focused_on(output.name) else {
            return Err(InvokeError::Refused("no focused window on this output"));
        };
        // A dead manager (the compositor sent `finished`) takes no
        // request: one past it is a protocol error, which would kill the
        // bar. The module shows the last thing it knew, and nothing is
        // sent.
        if !shared.live {
            return Ok(Update::Unchanged);
        }
        // `activate` rides on the seat: without one there is nothing to
        // ask with, whatever the window is doing.
        let seat = shared.seat.clone();
        if matches!(op, Op::Activate) && seat.is_none() {
            return Err(InvokeError::Refused("no seat to activate on"));
        }
        let toplevel = &shared.toplevels[index];
        let Some(handle) = toplevel.handle.clone().filter(|h| h.is_alive()) else {
            return Ok(Update::Unchanged);
        };
        match (op, seat) {
            (Op::Activate, Some(seat)) => {
                drop(shared);
                handle.activate(&seat);
            }
            (Op::Close, _) => {
                drop(shared);
                handle.close();
            }
            // Refused above: the seat is kept for `activate`.
            (Op::Activate, None) => {
                return Err(InvokeError::Refused("no seat to activate on"));
            }
        }
        Ok(Update::Unchanged)
    }

    /// `{"title": ..., "app_id": ..., "fullscreen": ...}` for `output`:
    /// what is shown, as data; `None` when no window is shown.
    fn value(&self, output: &OutputView<'_>) -> Option<serde_json::Value> {
        let shared = self.link.0.borrow();
        let index = shared.focused_on(output.name)?;
        let toplevel = &shared.toplevels[index];
        let mut title = String::new();
        let mut app_id = String::new();
        push_sanitized(&mut title, toplevel.title());
        push_sanitized(&mut app_id, toplevel.app_id());
        Some(serde_json::json!({
            "title": title,
            "app_id": app_id,
            "fullscreen": toplevel.fullscreen,
        }))
    }

    /// The span never grows past `max-width` logical pixels: longer titles
    /// are cut with an ellipsis, so the title yields the bar to the other
    /// modules.
    fn max_width(&self) -> Option<u32> {
        Some(self.max_width)
    }

    /// A click activates with no binding at all.
    fn handles_input(&self) -> bool {
        true
    }

    /// Cuts a title longer than its span with an ellipsis, measured in
    /// device pixels; a title that fits draws the plain way (returning
    /// `false`). The color follows the plain draw (accent under the
    /// pointer, else the class), so truncating never changes the look.
    fn custom_draw(&self, ctx: &mut CustomDraw<'_, '_>) -> bool {
        let full = ctx.view.text();
        if full.is_empty() {
            return false;
        }
        let padding = ctx.padding;
        let available = ctx.span.width.saturating_sub(padding.saturating_mul(2));
        if ctx.text.measure(None, full, ctx.em) <= available {
            return false;
        }
        const ELLIPSIS: char = '…';
        let ellipsis = ctx.text.measure(None, "…", ctx.em);
        if ellipsis > available {
            // Not even the ellipsis fits: leave the span blank.
            return true;
        }
        // The longest prefix that leaves room for the ellipsis. Advances
        // are fractional and each glyph starts rounded, so the clip is the
        // backstop for the last pixel.
        let budget = available - ellipsis;
        let mut kept = [0u8; MAX_TEXT];
        let mut len = 0usize;
        let mut pen = 0.0f32;
        for c in full.chars().filter(|c| !c.is_control()) {
            let next = pen + ctx.text.advance(c, ctx.em);
            if next.round() > budget as f32 {
                break;
            }
            let width = c.len_utf8();
            if len + width + ELLIPSIS.len_utf8() > kept.len() {
                break;
            }
            c.encode_utf8(&mut kept[len..]);
            len += width;
            pen = next;
        }
        ELLIPSIS.encode_utf8(&mut kept[len..]);
        len += ELLIPSIS.len_utf8();
        let shown = std::str::from_utf8(&kept[..len]).unwrap_or("");
        let color = if ctx.hovered {
            ctx.theme.accent
        } else {
            ctx.theme.class(ctx.view.class())
        };
        ctx.text.draw(
            ctx.canvas,
            None,
            shown,
            ctx.em,
            i64::from(ctx.span.x) + i64::from(padding),
            ctx.baseline,
            color,
            ctx.span,
        );
        true
    }
}
