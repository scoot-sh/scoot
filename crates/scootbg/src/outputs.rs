//! What scootbg knows about each output, and where its wallpaper surface
//! stands: a pure model, with no Wayland objects and no I/O, so every
//! ordering of events can be tested without a compositor. The Wayland glue
//! (`daemon::surfaces`) turns events into calls here and carries out the
//! [`Effect`]s they return.
//!
//! ## One output's life
//!
//! 1. **Bound.** A `wl_output` global is bound (and, when it is older than
//!    v4 and so has no `name`, an `zxdg_output_v1` is asked for it), then a
//!    `wl_display.sync` is sent. Its properties arrive *staged*: `wl_output`
//!    applies a batch atomically at `done`, so nothing staged is visible
//!    until then.
//! 2. **Settled.** The sync's callback fires. Every event the compositor
//!    sent in answer to the bind (and to `get_xdg_output`) was sent before
//!    it, so the output's identity is now as known as it will ever be:
//!    [`Output::settled`] returns [`Effect::Create`]. An output that never
//!    sends `done` (a v1 `wl_output` has no such event, and a buggy
//!    compositor might skip it) is settled all the same, with what it did
//!    send.
//! 3. **Surface.** One background layer surface per output moves through
//!    [`Surface`]: `Pending` (created, first commit sent, no configure yet),
//!    `Configured`, and on the compositor's `closed`, `Closed` (destroyed;
//!    re-created after one more round trip, once) or, closed a second time
//!    over the output's life, `GaveUp`, for that output only, until it is
//!    replugged.
//! 4. **Removed.** The glue drops the entry whatever state it is in, before
//!    `done`, mid-configure, or while closed; events still in flight for it
//!    then find no entry and are dropped.
//!
//! ## Ids
//!
//! Each bound output gets an [`OutputId`] that is never reused for the
//! daemon's life (registry names can be: a compositor may hand a replugged
//! monitor its old global name). Anything that answers later (the settle
//! and retry callbacks, and an image decoded on the worker thread,
//! `daemon::images`) carries the id and looks the output up again when it
//! lands. An output removed meanwhile is simply not found, and the result
//! is dropped.

#[cfg(test)]
mod paint_tests;
#[cfg(test)]
mod scale_tests;
#[cfg(test)]
mod tests;

use crate::density::{Buffer, Preferred, Scale};
use crate::paint::{Drawn, Plan};
use crate::waiters::Progress;
use crate::wallpaper::Wallpaper;

/// Identifies one bound output for the daemon's whole life; never reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputId(u64);

/// A width and height, in whatever unit the context says. Serialized as
/// `{"width":W,"height":H}` in `query` replies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

/// `wl_output.transform`: how the output is rotated or flipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Transform {
    #[default]
    Normal,
    Rotate90,
    Rotate180,
    Rotate270,
    Flipped,
    Flipped90,
    Flipped180,
    Flipped270,
}

impl Transform {
    /// Whether the output's logical width is its mode's height.
    pub fn swaps_axes(self) -> bool {
        matches!(
            self,
            Self::Rotate90 | Self::Rotate270 | Self::Flipped90 | Self::Flipped270
        )
    }

    /// The name `query` reports.
    pub fn name(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Rotate90 => "90",
            Self::Rotate180 => "180",
            Self::Rotate270 => "270",
            Self::Flipped => "flipped",
            Self::Flipped90 => "flipped-90",
            Self::Flipped180 => "flipped-180",
            Self::Flipped270 => "flipped-270",
        }
    }
}

/// An output's applied properties: what the last `done` made current.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Info {
    /// `wl_output.name` (v4), else `zxdg_output_v1.name`.
    pub name: Option<String>,
    /// `wl_output.description` (v4), else `zxdg_output_v1.description`.
    pub description: Option<String>,
    /// The current mode, in device pixels.
    pub mode: Option<Size>,
    /// `wl_output.scale`: an integer, the fractional scale rounded up.
    pub scale: u32,
    pub transform: Transform,
    /// `zxdg_output_v1.logical_size`, only bound for pre-v4 outputs.
    pub xdg_logical: Option<Size>,
}

impl Default for Info {
    fn default() -> Self {
        Self {
            name: None,
            description: None,
            mode: None,
            scale: 1,
            transform: Transform::Normal,
            xdg_logical: None,
        }
    }
}

impl Info {
    /// The output's size in logical pixels, when it can be known from
    /// `wl_output` and `xdg_output` alone.
    ///
    /// The compositor's own answer when it gave one (`xdg_output`'s logical
    /// size, bound only for outputs older than v4). Otherwise the current
    /// mode, rotated by the transform, divided by `scale` ([`Scale::logical`]:
    /// rounded down for an integer, as wlroots does, which is exact at
    /// integer scales; up for a fraction). With `wl_output`'s own integer
    /// scale, a fractional output's scale rounded up, this comes out smaller
    /// than the real logical size; [`Output::derived_logical`] passes the
    /// surface's fractional scale when the compositor sent one.
    ///
    /// Before the output has reported a mode there is nothing to derive
    /// from, and the answer is `None`.
    pub fn logical(&self, scale: Scale) -> Option<Size> {
        if let Some(logical) = self.xdg_logical {
            return Some(logical);
        }
        let mode = self.mode?;
        let device = if self.transform.swaps_axes() {
            Size {
                width: mode.height,
                height: mode.width,
            }
        } else {
            mode
        };
        Some(scale.logical(device))
    }
}

/// Properties received since the last `done`, applied atomically at it.
#[derive(Debug, Default)]
struct Staged {
    name: Option<String>,
    description: Option<String>,
    mode: Option<Size>,
    scale: Option<u32>,
    transform: Option<Transform>,
    xdg_logical: Option<Size>,
}

impl Staged {
    /// Moves everything staged into `info`, leaving nothing staged.
    fn apply(&mut self, info: &mut Info) {
        if let Some(name) = self.name.take() {
            info.name = Some(name);
        }
        if let Some(description) = self.description.take() {
            info.description = Some(description);
        }
        if let Some(mode) = self.mode.take() {
            info.mode = Some(mode);
        }
        if let Some(scale) = self.scale.take() {
            info.scale = scale;
        }
        if let Some(transform) = self.transform.take() {
            info.transform = transform;
        }
        if let Some(logical) = self.xdg_logical.take() {
            info.xdg_logical = Some(logical);
        }
    }
}

/// Where an output's wallpaper surface stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Surface {
    /// No surface yet: the output has not settled.
    Waiting,
    /// Created and committed with no buffer; no `configure` yet.
    Pending,
    /// The compositor sent a size and scootbg acked it. A buffer attached
    /// and committed maps the surface.
    Configured {
        /// The last `configure`'s serial, already acked.
        serial: u32,
        /// The size as the compositor sent it; 0 on an axis means "yours to
        /// choose", resolved by [`Output::surface_size`].
        requested: Size,
        /// What the surface was last committed with; `None` while nothing
        /// is attached (it is not mapped). Kept across later `configure`s
        /// of the same surface, and gone with it.
        drawn: Option<Drawn>,
    },
    /// The compositor closed it and it is destroyed; it is re-created once
    /// a round trip has shown the output was not removed meanwhile.
    Closed,
    /// Closed a second time: scootbg has stopped trying on this output.
    ///
    /// For the output's whole life: the one retry is not given back after
    /// it succeeds, because a compositor that configures and then closes
    /// every surface would otherwise be answered with a new one forever.
    /// A replug is a new output (a new id) and starts afresh.
    GaveUp,
}

impl Surface {
    /// The name `query` reports.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Pending => "pending",
            Self::Configured { .. } => "configured",
            Self::Closed => "closed",
            Self::GaveUp => "gave-up",
        }
    }

    /// Whether a live layer surface exists for it.
    pub fn is_live(&self) -> bool {
        matches!(self, Self::Pending | Self::Configured { .. })
    }
}

/// What the glue must do after an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum Effect {
    /// Nothing.
    None,
    /// Create the layer surface and commit it with no buffer.
    Create,
    /// Ack this `configure`.
    Ack(u32),
    /// Destroy the layer surface, then send a `wl_display.sync` whose
    /// callback calls [`Output::retry`].
    DestroyAndRetry,
    /// Destroy the layer surface and say loudly that this output is given
    /// up on.
    DestroyAndGiveUp,
}

/// One output, as scootbg sees it.
#[derive(Debug)]
pub struct Output {
    id: OutputId,
    /// The `wl_registry` name of its `wl_output` global.
    global: u32,
    info: Info,
    staged: Staged,
    /// A `done` has been received at least once.
    done: bool,
    /// The settle callback has fired (see the module docs).
    settled: bool,
    surface: Surface,
    /// The compositor has closed a surface on this output before.
    closed_once: bool,
    /// The generation of the last `set` or `clear` that targeted this
    /// output, 0 before any (see `crate::waiters`).
    stamp: u64,
    /// Drawing what it should show failed; not retried until a new
    /// request targets it or the compositor reconfigures it, so a failure
    /// (a buffer too large for `wl_shm`, out of memory) cannot loop.
    failed: bool,
    /// The scales the compositor asked the wallpaper surface to be drawn
    /// at (`wp_fractional_scale_v1`, `wl_surface.preferred_buffer_scale`),
    /// from the live surface's events only (the glue drops a stale
    /// surface's). Kept when the surface is re-created: they describe the
    /// output it is on, and the new surface's own events replace them,
    /// before its first `configure` on every compositor checked.
    preferred: Preferred,
}

impl Output {
    pub fn id(&self) -> OutputId {
        self.id
    }

    pub fn info(&self) -> &Info {
        &self.info
    }

    pub fn surface(&self) -> &Surface {
        &self.surface
    }

    /// The scale to draw the surface at: the best the compositor said
    /// ([`Preferred::scale`]), else `wl_output.scale`.
    pub fn scale(&self) -> Scale {
        self.preferred.scale(self.info.scale)
    }

    /// The full-size buffer for the surface now (an image's, or a color's
    /// on the full-size path): its configured size at [`Output::scale`].
    /// `None` while it has no size, or if that overflows. The one place an
    /// image's buffer size is decided (`daemon::change::image_dims` reads
    /// it; the draw reads the same [`Scale::buffer`] through `Drawn`).
    pub fn full_buffer(&self) -> Option<Buffer> {
        self.scale().buffer(self.surface_size()?)
    }

    /// `wp_fractional_scale_v1.preferred_scale` on the live surface.
    /// Returns whether it changed (then the surface needs redrawing).
    pub fn prefer_fractional(&mut self, v120: u32) -> bool {
        self.preferred.set_fractional(v120)
    }

    /// `wl_surface.preferred_buffer_scale` on the live surface. Returns
    /// whether it changed.
    pub fn prefer_buffer_scale(&mut self, factor: i32) -> bool {
        self.preferred.set_buffer_scale(factor)
    }

    /// The output as a message to stderr names it. The name comes from the
    /// compositor, so it is escaped (`escape_debug`: control characters,
    /// quotes and backslashes), never printed raw to a terminal.
    pub fn label(&self) -> Label<'_> {
        Label(self.info.name.as_deref())
    }

    #[cfg(test)]
    pub fn has_seen_done(&self) -> bool {
        self.done
    }

    pub fn stage_name(&mut self, name: String) {
        self.staged.name = Some(name);
    }

    pub fn stage_description(&mut self, description: String) {
        self.staged.description = Some(description);
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

    pub fn stage_transform(&mut self, transform: Transform) {
        self.staged.transform = Some(transform);
    }

    /// `zxdg_output_v1.logical_size`, not positive ignored as for modes.
    pub fn stage_xdg_logical(&mut self, width: i32, height: i32) {
        if let Some(size) = positive(width, height) {
            self.staged.xdg_logical = Some(size);
        }
    }

    /// `wl_output.done`, or `zxdg_output_v1.done` (sent by v1 and v2 of
    /// that protocol, which apply its own events atomically): the staged
    /// properties become current.
    ///
    /// No surface change is needed: a configured surface's size fallback
    /// ([`Output::surface_size`]) reads the new properties as they are. The
    /// glue redraws a configured surface one round trip later (a new
    /// `wl_output.scale` is a new buffer scale where the compositor sent no
    /// better one).
    pub fn done(&mut self) {
        self.staged.apply(&mut self.info);
        self.done = true;
    }

    /// The settle callback: the output's identity is as known as it will
    /// be, so its surface is created.
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
            self.surface = Surface::Pending;
            Effect::Create
        } else {
            Effect::None
        }
    }

    /// A `configure` on the live surface: remember it, and ack it. What a
    /// configured surface shows stays; the glue redraws it at the new size
    /// ([`Output::plan`]). A new size is also a new chance for a draw that
    /// failed.
    pub fn configure(&mut self, serial: u32, width: u32, height: u32) -> Effect {
        let drawn = match &mut self.surface {
            Surface::Configured { drawn, .. } => drawn.take(),
            Surface::Pending => None,
            // No live surface: a stale event, already handled by the
            // glue's object check. Nothing to ack.
            Surface::Waiting | Surface::Closed | Surface::GaveUp => return Effect::None,
        };
        self.surface = Surface::Configured {
            serial,
            requested: Size { width, height },
            drawn,
        };
        self.failed = false;
        Effect::Ack(serial)
    }

    /// The serial of the `configure` the live surface was last given, while
    /// it is configured.
    pub fn configured_serial(&self) -> Option<u32> {
        match &self.surface {
            Surface::Configured { serial, .. } => Some(*serial),
            _ => None,
        }
    }

    /// The glue replaced the live surface with a fresh one, committed with
    /// no buffer (`clear`): it waits for its first `configure` again.
    pub fn recreated(&mut self) {
        if self.surface.is_live() {
            self.surface = Surface::Pending;
        }
    }

    /// A `set` or `clear` of generation `stamp` targets this output. An
    /// image lands after it is decoded, maybe after newer requests stamped
    /// the output, so the stamp only ever grows.
    pub fn want(&mut self, stamp: u64) {
        self.stamp = self.stamp.max(stamp);
        self.failed = false;
    }

    /// The generation of the last request that targeted it.
    pub fn stamp(&self) -> u64 {
        self.stamp
    }

    /// The surface was committed with `drawn`.
    pub fn drew(&mut self, drawn: Drawn) {
        if let Surface::Configured { drawn: slot, .. } = &mut self.surface {
            *slot = Some(drawn);
        }
    }

    /// Drawing failed; see the field.
    pub fn draw_failed(&mut self) {
        self.failed = true;
    }

    /// What is on screen: what the configured surface was last committed
    /// with.
    pub fn shows(&self) -> Option<&Wallpaper> {
        match &self.surface {
            Surface::Configured { drawn, .. } => drawn.as_ref().map(|d| &d.content),
            _ => None,
        }
    }

    /// What to do so the surface shows `wanted`, drawn at `scale` (see
    /// `paint::scale_for`). Only a configured surface can be drawn on; the
    /// others show nothing and wait for their `configure`.
    pub fn plan(&self, wanted: Option<&Wallpaper>, scale: Scale) -> Plan {
        let Surface::Configured { drawn, .. } = &self.surface else {
            return Plan::Nothing;
        };
        if self.failed {
            return Plan::Nothing;
        }
        let Some(content) = wanted else {
            return if drawn.is_some() {
                Plan::Clear
            } else {
                Plan::Nothing
            };
        };
        let Some(size) = self.surface_size() else {
            return Plan::Nothing;
        };
        let shown = drawn
            .as_ref()
            .is_some_and(|d| &d.content == content && d.size == size && d.scale == scale);
        if shown {
            Plan::Nothing
        } else {
            Plan::Show(Drawn {
                content: content.clone(),
                size,
                scale,
            })
        }
    }

    /// Whether it shows `wanted` (at `scale`), for a waiting reply.
    pub fn progress(&self, wanted: Option<&Wallpaper>, scale: Scale) -> Progress {
        if self.failed {
            return Progress::Failed;
        }
        let shown = match &self.surface {
            // Nothing will ever be shown there: nothing to wait for.
            Surface::GaveUp => return Progress::Done,
            Surface::Configured { drawn, .. } => {
                drawn.is_some() == wanted.is_some() && self.plan(wanted, scale) == Plan::Nothing
            }
            // No configured surface shows nothing, which is right only if
            // nothing is wanted.
            Surface::Waiting | Surface::Pending | Surface::Closed => wanted.is_none(),
        };
        if shown {
            Progress::Done
        } else {
            Progress::Waiting
        }
    }

    /// `closed` on the live surface: retry once, then give up.
    pub fn closed(&mut self) -> Effect {
        if !self.surface.is_live() {
            return Effect::None;
        }
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
            self.surface = Surface::Pending;
            Effect::Create
        } else {
            Effect::None
        }
    }

    /// The output's size in logical pixels, as well as it is known: the
    /// configured surface's size once there is one (the surface covers the
    /// whole output, so the compositor's size for it is the output's, exact
    /// at any scale), else [`Output::derived_logical`].
    pub fn logical(&self) -> Option<Size> {
        self.surface_size().or_else(|| self.derived_logical())
    }

    /// The logical size worked out from the output's properties
    /// ([`Info::logical`]) at [`Output::scale`]: with the surface's
    /// fractional scale once the compositor sent it, so 1600×1000 at 1.5 is
    /// 1067×667, not `wl_output`'s 2 and 800×500. Within a pixel of the
    /// compositor's own figure (they round differently).
    ///
    /// It sizes a surface only for a `configure` of 0 on an axis: a surface
    /// anchored to all four edges gets its real size from the compositor,
    /// and every compositor checked (scoot, sway) sends it.
    pub fn derived_logical(&self) -> Option<Size> {
        self.info.logical(self.scale())
    }

    /// The size to draw the surface at, in logical pixels: the configured
    /// size, with an axis of 0 taken from the output's logical size (see
    /// [`Output::derived_logical`]). `None` while not configured, or while a 0 axis
    /// has nothing to resolve against yet.
    pub fn surface_size(&self) -> Option<Size> {
        let Surface::Configured { requested, .. } = &self.surface else {
            return None;
        };
        let requested = *requested;
        if requested.width != 0 && requested.height != 0 {
            return Some(requested);
        }
        let logical = self.derived_logical()?;
        Some(Size {
            width: if requested.width == 0 {
                logical.width
            } else {
                requested.width
            },
            height: if requested.height == 0 {
                logical.height
            } else {
                requested.height
            },
        })
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
                stamp: 0,
                failed: false,
                preferred: Preferred::default(),
            },
            objects: objects(id),
        });
        id
    }

    /// Takes the output bound from registry global `global` out, whatever
    /// state it is in. `None` for any other global.
    pub fn remove_global(&mut self, global: u32) -> Option<Entry<O>> {
        let index = self.list.iter().position(|e| e.output.global == global)?;
        // `remove`, not `swap_remove`: `query` lists outputs in the order
        // they appeared.
        Some(self.list.remove(index))
    }

    pub fn get_mut(&mut self, id: OutputId) -> Option<&mut Entry<O>> {
        self.list.iter_mut().find(|e| e.output.id == id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Entry<O>> {
        self.list.iter()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Entry<O>> {
        self.list.iter_mut()
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.list.len()
    }
}
