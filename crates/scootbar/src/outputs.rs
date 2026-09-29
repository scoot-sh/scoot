//! What scootbar knows about each output, and where its bar surface
//! stands: a pure model, with no Wayland objects and no I/O, so every
//! ordering of events can be tested without a compositor. The Wayland glue
//! (`daemon::surfaces`) turns events into calls here and carries out the
//! [`Effect`]s they return; the loop asks each output for its [`Plan`]
//! once per turn and draws what it says.
//!
//! This is scootbg's per-output lifecycle (`crates/scootbg/src/outputs.rs`),
//! reused rather than reinvented, and trimmed to what a bar needs: no
//! wallpaper choices, no waiting replies, no `xdg_output`. It is the
//! Wayland scaffolding `docs/scootbar/backlog/extract-scootui.md` names as
//! the likely shared piece, so the two copies are kept close in shape;
//! scootbg is not refactored here.
//!
//! ## One output's life
//!
//! 1. **Bound.** A `wl_output` global is bound, then a `wl_display.sync`
//!    is sent. Its properties arrive *staged*: `wl_output` applies a batch
//!    atomically at `done`.
//! 2. **Settled.** The sync's callback fires: every event answering the
//!    bind was sent before it, so the output's scale and mode are as known
//!    as they will be, and [`Output::settled`] returns [`Effect::Create`].
//!    An output that never sends `done` (a v1 `wl_output` has no such
//!    event) is settled all the same, with what it did send.
//! 3. **Surface.** One `top`-layer surface per output moves through
//!    [`Surface`]: `Pending` (created, committed with no buffer, no
//!    configure yet), `Configured`, and on the compositor's `closed`,
//!    `Closed` (destroyed; re-created after one more round trip, once) or,
//!    closed a second time over the output's life, `GaveUp`, for that
//!    output only, until it is replugged.
//! 4. **Removed.** The glue drops the entry whatever state it is in;
//!    events still in flight for it then find no entry and are dropped.
//!
//! ## Drawing
//!
//! The surface is drawn when it is configured and its [`Frame`] (logical
//! size and scale) differs from the one it was last committed with, and
//! only then: that is the whole damage rule while the bar is one color. A
//! `configure` that changes nothing still needs a commit, so the ack takes
//! effect ([`Plan::Commit`]). Many events in one read (a scale change
//! arrives as several) are one plan, since the loop asks after dispatching
//! them all.
//!
//! ## Ids
//!
//! Each bound output gets an [`OutputId`] that is never reused for the
//! daemon's life (registry names can be: a compositor may hand a replugged
//! monitor its old global name). Round-trip callbacks carry the id and look
//! the output up again when they land.

#[cfg(test)]
mod tests;

use crate::bar::Bar;
use crate::density::{Preferred, Scale};

/// Identifies one bound output for the daemon's whole life; never reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputId(u64);

/// A width and height, in whatever unit the context says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

/// Whether `wl_output.transform` swaps the output's axes: its logical
/// width is then its mode's height. The only part of the transform a bar
/// needs (for a compositor that configures a width of 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Rotation {
    #[default]
    Upright,
    Sideways,
}

/// An output's applied properties: what the last `done` made current.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Info {
    /// `wl_output.name` (v4), for messages.
    pub name: Option<String>,
    /// The current mode, in device pixels.
    pub mode: Option<Size>,
    /// `wl_output.scale`: an integer, the fractional scale rounded up.
    pub scale: u32,
    pub rotation: Rotation,
}

impl Default for Info {
    fn default() -> Self {
        Self {
            name: None,
            mode: None,
            scale: 1,
            rotation: Rotation::Upright,
        }
    }
}

impl Info {
    /// The current mode rotated as the output is: its size in device
    /// pixels, the way its surfaces are laid out.
    pub fn device(&self) -> Option<Size> {
        let mode = self.mode?;
        Some(match self.rotation {
            Rotation::Upright => mode,
            Rotation::Sideways => Size {
                width: mode.height,
                height: mode.width,
            },
        })
    }
}

/// Properties received since the last `done`, applied atomically at it.
#[derive(Debug, Default)]
struct Staged {
    name: Option<String>,
    mode: Option<Size>,
    scale: Option<u32>,
    rotation: Option<Rotation>,
}

impl Staged {
    fn apply(&mut self, info: &mut Info) {
        if let Some(name) = self.name.take() {
            info.name = Some(name);
        }
        if let Some(mode) = self.mode.take() {
            info.mode = Some(mode);
        }
        if let Some(scale) = self.scale.take() {
            info.scale = scale;
        }
        if let Some(rotation) = self.rotation.take() {
            info.rotation = rotation;
        }
    }
}

/// Where an output's bar surface stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    /// No surface yet: the output has not settled.
    Waiting,
    /// Created and committed with no buffer; no `configure` yet.
    Pending,
    /// The compositor sent a size and scootbar acked it.
    Configured {
        /// The size as the compositor sent it; 0 on an axis means "yours to
        /// choose", resolved by [`Output::surface_size`].
        requested: Size,
    },
    /// The compositor closed it and it is destroyed; it is re-created once
    /// a round trip has shown the output was not removed meanwhile.
    Closed,
    /// Closed a second time: scootbar has stopped trying on this output,
    /// for the output's whole life (a compositor that configures and then
    /// closes every surface would otherwise be answered with a new one
    /// forever). A replug is a new output and starts afresh.
    GaveUp,
}

impl Surface {
    /// Whether a live layer surface exists for it.
    pub fn is_live(self) -> bool {
        matches!(self, Self::Pending | Self::Configured { .. })
    }
}

/// What the glue must do after an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum Effect {
    None,
    /// Create the layer surface and commit it with no buffer.
    Create,
    /// Ack this `configure`.
    Ack(u32),
    /// Destroy the layer surface, then send a `wl_display.sync` whose
    /// callback calls [`Output::retry`].
    DestroyAndRetry,
    /// Destroy the layer surface and say that this output is given up on.
    DestroyAndGiveUp,
}

/// What a draw is at: everything that decides the buffer's pixels while the
/// bar is one color. Two equal frames are the same pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    /// The surface's size, in logical pixels.
    pub size: Size,
    pub scale: Scale,
}

/// What the loop must do for an output now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    Nothing,
    /// Commit the surface as it is, so an acked `configure` takes effect.
    Commit,
    /// Draw this frame and commit it.
    Draw(Frame),
}

/// One output, as scootbar sees it.
#[derive(Debug)]
pub struct Output {
    id: OutputId,
    /// The `wl_registry` name of its `wl_output` global.
    global: u32,
    info: Info,
    staged: Staged,
    /// A `done` has been received at least once.
    done: bool,
    /// The settle callback has fired.
    settled: bool,
    surface: Surface,
    /// The compositor has closed a surface on this output before.
    closed_once: bool,
    /// The scales the compositor asked the live surface to be drawn at.
    /// Kept when the surface is re-created: they describe the output, and
    /// the new surface's own events replace them.
    preferred: Preferred,
    /// A `configure` was acked and the surface not committed since.
    unacked: bool,
    /// What the live surface was last committed with; `None` while nothing
    /// is attached. Gone with the surface.
    shown: Option<Frame>,
    /// A draw at this frame failed (said once on stderr); not tried again
    /// until the frame changes, so a failure (a buffer too large for
    /// `wl_shm`, out of memory) cannot loop.
    failed: Option<Frame>,
}

impl Output {
    pub fn id(&self) -> OutputId {
        self.id
    }

    #[cfg(test)]
    pub fn info(&self) -> &Info {
        &self.info
    }

    #[cfg(test)]
    pub fn surface(&self) -> Surface {
        self.surface
    }

    /// The scale to draw at: the best the compositor said
    /// ([`Preferred::scale`]).
    pub fn scale(&self) -> Scale {
        self.preferred.scale(self.info.scale)
    }

    /// The output as a message names it. The name comes from the
    /// compositor, so it is escaped, never printed raw to a terminal.
    pub fn label(&self) -> Label<'_> {
        Label(self.info.name.as_deref())
    }

    pub fn stage_name(&mut self, name: String) {
        self.staged.name = Some(name);
    }

    /// A `wl_output.mode` event. Only the current mode matters; a size
    /// that is not positive is a broken compositor, and ignored rather
    /// than let it reach a buffer size.
    pub fn stage_mode(&mut self, current: bool, width: i32, height: i32) {
        if let (true, Some(size)) = (current, positive(width, height)) {
            self.staged.mode = Some(size);
        }
    }

    /// A `wl_output.scale` event. A factor below 1 is a broken compositor
    /// and ignored: the scale divides.
    pub fn stage_scale(&mut self, factor: i32) {
        if let Ok(scale @ 1..) = u32::try_from(factor) {
            self.staged.scale = Some(scale);
        }
    }

    pub fn stage_rotation(&mut self, rotation: Rotation) {
        self.staged.rotation = Some(rotation);
    }

    /// `wl_output.done`: the staged properties become current. A new scale
    /// shows in the next [`Output::plan`].
    pub fn done(&mut self) {
        self.staged.apply(&mut self.info);
        self.done = true;
    }

    /// The live surface's `wp_fractional_scale_v1.preferred_scale`.
    pub fn prefer_fractional(&mut self, v120: u32) {
        self.preferred.set_fractional(v120);
    }

    /// The live surface's `wl_surface.preferred_buffer_scale`.
    pub fn prefer_buffer_scale(&mut self, factor: i32) {
        self.preferred.set_buffer_scale(factor);
    }

    /// The settle callback: the output is as known as it will be, so its
    /// surface is created.
    pub fn settled(&mut self) -> Effect {
        if self.settled {
            return Effect::None;
        }
        self.settled = true;
        if !self.done {
            // No `done` came (a v1 output, or a broken compositor): use
            // what was sent rather than wait forever.
            self.staged.apply(&mut self.info);
        }
        if self.surface == Surface::Waiting {
            self.made();
            Effect::Create
        } else {
            Effect::None
        }
    }

    /// A new surface, committed with no buffer, waits for its first
    /// `configure`.
    fn made(&mut self) {
        self.surface = Surface::Pending;
        self.unacked = false;
        self.shown = None;
        self.failed = None;
    }

    /// A `configure` on the live surface: remember its size, and ack it.
    /// A new size is also a new chance for a draw that failed.
    pub fn configure(&mut self, serial: u32, width: u32, height: u32) -> Effect {
        if !self.surface.is_live() {
            // No live surface: a stale event, already filtered by the
            // glue's object check. Nothing to ack.
            return Effect::None;
        }
        self.surface = Surface::Configured {
            requested: Size { width, height },
        };
        self.unacked = true;
        self.failed = None;
        Effect::Ack(serial)
    }

    /// `closed` on the live surface: retry once, then give up.
    pub fn closed(&mut self) -> Effect {
        if !self.surface.is_live() {
            return Effect::None;
        }
        self.shown = None;
        self.unacked = false;
        if self.closed_once {
            self.surface = Surface::GaveUp;
            Effect::DestroyAndGiveUp
        } else {
            self.closed_once = true;
            self.surface = Surface::Closed;
            Effect::DestroyAndRetry
        }
    }

    /// The retry callback after a `closed`: the output survived the round
    /// trip, so its surface is created again.
    pub fn retry(&mut self) -> Effect {
        if self.surface == Surface::Closed {
            self.made();
            Effect::Create
        } else {
            Effect::None
        }
    }

    /// The size to draw the surface at, in logical pixels: the configured
    /// size, with an axis of 0 taken from the bar (its height) or the
    /// output (its width, less the side margins). `None` while not
    /// configured, or while a 0 width has no mode to resolve against.
    pub fn surface_size(&self, bar: &Bar) -> Option<Size> {
        let Surface::Configured { requested } = self.surface else {
            return None;
        };
        let width = match requested.width {
            0 => bar.width_on(self.scale().logical(self.info.device()?).width),
            width => width,
        };
        let height = match requested.height {
            0 => bar.height,
            height => height,
        };
        Some(Size { width, height })
    }

    /// What to do for this output now (see the module docs).
    pub fn plan(&self, bar: &Bar) -> Plan {
        let Some(size) = self.surface_size(bar) else {
            return Plan::Nothing;
        };
        let want = Frame {
            size,
            scale: self.scale(),
        };
        if self.shown != Some(want) && self.failed != Some(want) {
            return Plan::Draw(want);
        }
        // The same pixels, or a draw that failed at this frame: an acked
        // `configure` still needs a commit to take effect, on a surface
        // that is mapped (an unmapped one has nothing to apply it to).
        if self.unacked && self.shown.is_some() {
            Plan::Commit
        } else {
            Plan::Nothing
        }
    }

    /// The surface was committed with `frame` attached.
    pub fn drew(&mut self, frame: Frame) {
        if self.surface.is_live() {
            self.shown = Some(frame);
            self.unacked = false;
            self.failed = None;
        }
    }

    /// The surface was committed as it was ([`Plan::Commit`]).
    pub fn committed(&mut self) {
        self.unacked = false;
    }

    /// Drawing `frame` failed; see the field.
    pub fn draw_failed(&mut self, frame: Frame) {
        self.failed = Some(frame);
    }
}

/// See [`Output::label`].
pub struct Label<'a>(Option<&'a str>);

impl std::fmt::Display for Label<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            Some(name) => write!(f, "output \"{}\"", name.escape_debug()),
            None => f.write_str("an unnamed output"),
        }
    }
}

fn positive(width: i32, height: i32) -> Option<Size> {
    match (u32::try_from(width), u32::try_from(height)) {
        (Ok(width @ 1..), Ok(height @ 1..)) => Some(Size { width, height }),
        _ => None,
    }
}

/// Every output currently bound, each with the glue's objects `O` for it
/// (`()` in tests).
#[derive(Debug)]
pub struct Outputs<O> {
    list: Vec<Entry<O>>,
    next_id: u64,
}

/// One output and its objects.
#[derive(Debug)]
pub struct Entry<O> {
    pub output: Output,
    pub objects: O,
}

impl<O> Default for Outputs<O> {
    fn default() -> Self {
        Self {
            list: Vec::new(),
            next_id: 0,
        }
    }
}

impl<O> Outputs<O> {
    /// A newly bound output, for registry global `global`. `objects` makes
    /// the glue's objects given the new id (they carry it as user data).
    pub fn add(&mut self, global: u32, objects: impl FnOnce(OutputId) -> O) -> OutputId {
        let id = OutputId(self.next_id);
        // 2^64 binds cannot happen; wrapping keeps it panic-free anyway.
        self.next_id = self.next_id.wrapping_add(1);
        self.list.push(Entry {
            output: Output {
                id,
                global,
                info: Info::default(),
                staged: Staged::default(),
                done: false,
                settled: false,
                surface: Surface::Waiting,
                closed_once: false,
                preferred: Preferred::default(),
                unacked: false,
                shown: None,
                failed: None,
            },
            objects: objects(id),
        });
        id
    }

    /// Takes the output bound from registry global `global` out, whatever
    /// state it is in. `None` for any other global.
    pub fn remove_global(&mut self, global: u32) -> Option<Entry<O>> {
        let index = self.list.iter().position(|e| e.output.global == global)?;
        Some(self.list.remove(index))
    }

    pub fn get_mut(&mut self, id: OutputId) -> Option<&mut Entry<O>> {
        self.list.iter_mut().find(|e| e.output.id == id)
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Entry<O>> {
        self.list.iter_mut()
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.list.len()
    }
}
