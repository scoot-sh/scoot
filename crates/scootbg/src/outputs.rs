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
//!    re-created after one more round trip, once) or, closed a second time,
//!    `GaveUp`, for that output only.
//! 4. **Removed.** The glue drops the entry whatever state it is in, before
//!    `done`, mid-configure, or while closed; events still in flight for it
//!    then find no entry and are dropped.
//!
//! ## Ids
//!
//! Each bound output gets an [`OutputId`] that is never reused for the
//! daemon's life (registry names can be: a compositor may hand a replugged
//! monitor its old global name). Anything that answers later, the settle
//! and retry callbacks today and an image decoded on a worker thread in
//! images-decode-and-fit.md, carries the id and looks the output up again
//! when it lands. An output removed meanwhile is simply not found, and the
//! result is dropped.

#[cfg(test)]
mod tests;

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
    /// The output's size in logical pixels, when it can be known.
    ///
    /// The compositor's own answer when it gave one (`xdg_output`'s logical
    /// size, bound only for outputs older than v4). Otherwise the current
    /// mode, rotated by the transform, divided by the integer scale and
    /// rounded down, which is what wlroots computes. That is exact at
    /// integer scales. At a fractional scale `wl_output` reports the scale
    /// rounded up, so this comes out smaller than the real logical size.
    ///
    /// [`Output::logical`] prefers the configured surface's size to this.
    /// Here it is the fallback for a `configure` of 0: a surface anchored to all
    /// four edges gets its real size from the compositor, and every
    /// compositor checked (scoot, sway) sends it. Before the output has
    /// reported a mode there is nothing to derive from, and the answer is
    /// `None`: the surface waits for a later `configure` or `done` rather
    /// than guess.
    pub fn logical(&self) -> Option<Size> {
        if let Some(logical) = self.xdg_logical {
            return Some(logical);
        }
        let mode = self.mode?;
        let (width, height) = if self.transform.swaps_axes() {
            (mode.height, mode.width)
        } else {
            (mode.width, mode.height)
        };
        // `scale` is never 0 (see `stage_scale`); `max` keeps it so.
        let scale = self.scale.max(1);
        Some(Size {
            width: width / scale,
            height: height / scale,
        })
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    /// No surface yet: the output has not settled.
    Waiting,
    /// Created and committed with no buffer; no `configure` yet.
    Pending,
    /// The compositor sent a size and scootbg acked it. A buffer attached
    /// and committed now (solid-colour.md) maps the surface.
    Configured {
        /// The last `configure`'s serial, already acked.
        serial: u32,
        /// The size as the compositor sent it; 0 on an axis means "yours to
        /// choose", resolved by [`Output::surface_size`].
        requested: Size,
    },
    /// The compositor closed it and it is destroyed; it is re-created once
    /// a round trip has shown the output was not removed meanwhile.
    Closed,
    /// Closed a second time: scootbg has stopped trying on this output.
    GaveUp,
}

impl Surface {
    /// The name `query` reports.
    pub fn name(self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Pending => "pending",
            Self::Configured { .. } => "configured",
            Self::Closed => "closed",
            Self::GaveUp => "gave-up",
        }
    }

    /// Whether a live layer surface exists for it.
    pub fn is_live(self) -> bool {
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
}

impl Output {
    pub fn id(&self) -> OutputId {
        self.id
    }

    pub fn info(&self) -> &Info {
        &self.info
    }

    pub fn surface(&self) -> Surface {
        self.surface
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
    /// ([`Output::surface_size`]) reads the new properties as they are.
    /// Redrawing at a new scale is hidpi-fractional-scale.md.
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

    /// A `configure` on the live surface: remember it, and ack it.
    pub fn configure(&mut self, serial: u32, width: u32, height: u32) -> Effect {
        if !self.surface.is_live() {
            // No live surface: a stale event, already handled by the
            // glue's object check. Nothing to ack.
            return Effect::None;
        }
        self.surface = Surface::Configured {
            serial,
            requested: Size { width, height },
        };
        Effect::Ack(serial)
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
    /// at any scale), else [`Info::logical`], which undershoots at a
    /// fractional scale.
    pub fn logical(&self) -> Option<Size> {
        self.surface_size().or_else(|| self.info.logical())
    }

    /// The size to draw the surface at, in logical pixels: the configured
    /// size, with an axis of 0 taken from the output's logical size (see
    /// [`Info::logical`]). `None` while not configured, or while a 0 axis
    /// has nothing to resolve against yet.
    pub fn surface_size(&self) -> Option<Size> {
        let Surface::Configured { requested, .. } = self.surface else {
            return None;
        };
        if requested.width != 0 && requested.height != 0 {
            return Some(requested);
        }
        let logical = self.info.logical()?;
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

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.list.len()
    }
}
