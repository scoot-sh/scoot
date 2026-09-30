//! Drawing a bar on an output's surface: the buffers each output holds, and
//! the requests that put one on screen.
//!
//! ## Buffers: a pooled double buffer
//!
//! Up to [`SLOTS`] sealed-memfd `wl_shm` buffers per output
//! (`scootbg_mem::ShmBuffer`, each with its own pool), made on first need
//! and reused in place. A buffer is written only while the compositor does
//! not hold it (never attached, or released since), so a redraw reuses a
//! released buffer, else takes the other slot, else waits for a release
//! ([`Drew::Stalled`]; the release wakes the loop, which draws then). A
//! static bar needs one buffer: the second is made only if a redraw comes
//! while the first is still held. A buffer whose size is no longer the
//! surface's is dropped once released, so a resize does not keep the old
//! size's memory. The memfd is closed as soon as its pool exists (the
//! compositor has its own copy, the mapping keeps the memory).
//!
//! Each buffer keeps a [`Record`] of what its pixels show, so a redraw
//! paints only what differs from it (`crate::render::paint`): after one
//! module changed, only that module's span, even in a buffer that missed a
//! draw while the compositor held it.
//!
//! ## One draw
//!
//! One batch: attach, the viewport's destination or the buffer scale, the
//! opaque region and the input region (each only when it differs from what
//! the surface has, as they persist), damage, commit. With a viewporter every buffer, at any
//! scale, is attached at buffer scale 1 and sized by the viewport's
//! destination (the surface's logical size); without one, the scale is an
//! integer and goes to `set_buffer_scale`. **Damage is only what changed**
//! from what the surface shows (`crate::render::damage`): a new size, scale
//! or layout damages the whole bar, a module's new view only its span.

use std::fmt;

use wayland_client::QueueHandle;
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_shm;
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::protocol::wl_surface::WlSurface;

use scootbg_mem::{ShmBuffer, ShmError};

use super::Content;
use super::surfaces::LayerObjects;
use super::wayland::{Globals, State};
use crate::density::Scale;
use crate::modules::OutputView;
use crate::outputs::{Frame, OutputId, Size};
use crate::paint::{self, Span};
use crate::region;
use crate::render::{self, Record, Scene};

#[cfg(test)]
mod tests;

/// Buffers per output: one on screen, one to draw the next frame in.
pub const SLOTS: usize = 2;

/// Why a draw failed. Said once on stderr per streak; the model retries
/// the same frame a few times, then waits for a new one
/// ([`crate::outputs::Output::draw_failed`]).
#[derive(Debug)]
pub enum DrawError {
    /// The buffer's size overflows (a compositor asking for an absurd
    /// surface).
    TooLarge(Frame),
    Shm(ShmError),
}

impl fmt::Display for DrawError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge(frame) => write!(
                f,
                "a {}x{} bar at scale {} is too large for a buffer",
                frame.size.width, frame.size.height, frame.scale
            ),
            Self::Shm(error) => write!(f, "{error}"),
        }
    }
}

/// How a draw went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drew {
    Committed,
    /// Every buffer is held by the compositor: nothing was sent. The next
    /// release lets the loop draw.
    Stalled,
}

/// One buffer: its memory, the pool over it, the `wl_buffer`, and what its
/// pixels show.
#[derive(Debug)]
struct Slot {
    shm: ShmBuffer,
    pool: WlShmPool,
    buffer: WlBuffer,
    state: SlotState,
    record: Record,
}

/// What [`pick`] needs to know about a slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotState {
    pub dims: (u32, u32),
    /// Attached and not released since: the compositor may be reading.
    pub held: bool,
    /// The frame its pixels were last painted at (their [`Record`] has the
    /// rest).
    pub painted: Option<Frame>,
}

/// The slot to draw `frame` (a buffer of `dims`) in, given each slot's
/// state (`None`: empty): a free one already showing `frame`, else a free
/// one of the right size, else an empty one, else any free one (replaced
/// at the new size); `None` when the compositor holds them all.
pub fn pick(
    slots: impl Iterator<Item = Option<SlotState>> + Clone,
    frame: Frame,
    dims: (u32, u32),
) -> Option<usize> {
    let free = |slot: &Option<SlotState>| slot.is_some_and(|s| !s.held);
    let find = |wanted: &dyn Fn(&Option<SlotState>) -> bool| slots.clone().position(|s| wanted(&s));
    find(&|s| free(s) && s.is_some_and(|s| s.painted == Some(frame)))
        .or_else(|| find(&|s| free(s) && s.is_some_and(|s| s.dims == dims)))
        .or_else(|| find(&|s| s.is_none()))
        .or_else(|| find(&free))
}

impl Slot {
    fn new(
        globals: &Globals,
        qh: &QueueHandle<State>,
        id: OutputId,
        dims: (u32, u32),
        modules: usize,
        format: wl_shm::Format,
    ) -> Result<Self, ShmError> {
        let mut shm = ShmBuffer::new(dims.0, dims.1)?;
        let geometry = shm.geometry();
        let Some(fd) = shm.fd() else {
            // A fresh buffer has its fd; this cannot happen.
            return Err(ShmError::Io(std::io::Error::other(
                "the buffer's memfd is already closed",
            )));
        };
        let pool = globals.shm.create_pool(fd, geometry.len, qh, ());
        // The queued request holds its own copy of the fd until it is sent,
        // and the compositor its own after that.
        shm.close_fd();
        let buffer = pool.create_buffer(
            0,
            geometry.width,
            geometry.height,
            geometry.stride,
            format,
            qh,
            id,
        );
        Ok(Self {
            shm,
            pool,
            buffer,
            state: SlotState {
                dims,
                held: false,
                painted: None,
            },
            record: Record::new(modules),
        })
    }

    /// Destroys the buffer and the pool; the mapping goes with `shm`. Safe
    /// while the compositor holds the buffer (a surface destroyed under
    /// it): it has its own mapping of the memfd, which the seals keep
    /// whole, and this side never writes the pages again.
    fn destroy(self) {
        self.buffer.destroy();
        self.pool.destroy();
    }
}

/// One output's buffers.
#[derive(Debug)]
pub struct Canvas {
    slots: [Option<Slot>; SLOTS],
    /// The buffer size of the last draw: a released buffer of another size
    /// is dropped.
    current: Option<(u32, u32)>,
    /// What the surface shows (its last commit), for damage.
    shown: Record,
    /// The spans to damage, reused draw to draw.
    damage: Vec<Span>,
    /// How many modules the bar has: each record's length.
    modules: usize,
}

impl Canvas {
    pub fn new(modules: usize) -> Self {
        Self {
            slots: [None, None],
            current: None,
            shown: Record::new(modules),
            damage: Vec::with_capacity(modules),
            modules,
        }
    }

    /// What the surface shows.
    pub fn shown(&self) -> &Record {
        &self.shown
    }

    /// Destroys every buffer (the surface is gone, or going), and forgets
    /// what it showed.
    pub fn clear(&mut self) {
        for slot in &mut self.slots {
            if let Some(slot) = slot.take() {
                slot.destroy();
            }
        }
        self.current = None;
        self.shown.reset();
    }

    /// A reloaded layout changed the module count: destroys every buffer
    /// and records the new count, so the next draw starts whole at it.
    /// (`clear` alone keeps the old count, which the next draw would read
    /// past.)
    pub fn resize(&mut self, modules: usize) {
        self.clear();
        self.modules = modules;
        self.shown = Record::new(modules);
    }

    /// `wl_buffer.release` for `buffer`: its slot may be written again, or
    /// is dropped if the surface has moved on to another size.
    pub fn released(&mut self, buffer: &WlBuffer) {
        let current = self.current;
        for entry in &mut self.slots {
            let Some(slot) = entry else {
                continue;
            };
            if &slot.buffer != buffer {
                continue;
            }
            slot.state.held = false;
            if current != Some(slot.state.dims) {
                if let Some(slot) = entry.take() {
                    slot.destroy();
                }
            }
            return;
        }
    }

    /// Draws `scene` at `frame` on `layer`'s surface and commits it.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        globals: &Globals,
        qh: &QueueHandle<State>,
        id: OutputId,
        layer: &mut LayerObjects,
        frame: Frame,
        scene: &mut Scene,
        output: &OutputView<'_>,
        content: &mut Content,
    ) -> Result<Drew, DrawError> {
        // Without a viewport the buffer must be the logical size times an
        // integer (`set_buffer_scale`; anything else is a protocol error).
        // A fraction cannot reach here without one (the fractional-scale
        // manager is bound only with a viewporter), and this makes it so
        // by construction all the same.
        let scale = match layer.viewport {
            Some(_) => frame.scale,
            None => Scale::Integer(frame.scale.integer()),
        };
        let dims = scale.buffer(frame.size).ok_or(DrawError::TooLarge(frame))?;
        let Some(index) = pick(
            self.slots.iter().map(|s| s.as_ref().map(|s| s.state)),
            frame,
            dims,
        ) else {
            return Ok(Drew::Stalled);
        };
        let Some(entry) = self.slots.get_mut(index) else {
            return Ok(Drew::Stalled);
        };
        // A free slot of the wrong size is replaced.
        if entry.as_ref().is_some_and(|slot| slot.state.dims != dims) {
            if let Some(stale) = entry.take() {
                stale.destroy();
            }
        }
        let slot = match entry {
            Some(slot) => slot,
            None => {
                // `ARGB8888` only when the style needs an alpha channel;
                // a reload that changes that clears every buffer first.
                let format = if content.style.translucent() {
                    wl_shm::Format::Argb8888
                } else {
                    wl_shm::Format::Xrgb8888
                };
                let fresh = Slot::new(globals, qh, id, dims, self.modules, format)
                    .map_err(DrawError::Shm)?;
                entry.insert(fresh)
            }
        };
        let Content {
            modules,
            text,
            style,
        } = &mut *content;
        // The frame as drawn: at `scale`, which differs from the frame's
        // own only without a viewport. Measuring, painting and damage all
        // use it, so they cannot disagree about the scale.
        let drawn = Frame {
            size: frame.size,
            scale,
        };
        let extent = Size {
            width: dims.0,
            height: dims.1,
        };
        scene.update(modules, output, text.as_ref(), style, drawn.scale, extent);
        let Some(mut canvas) = paint::Canvas::new(slot.shm.pixels_mut(), dims.0, dims.1) else {
            // The buffer was made at `dims`; this cannot happen.
            return Err(DrawError::TooLarge(frame));
        };
        render::paint(
            &mut canvas,
            &mut slot.record,
            scene,
            modules,
            text.as_mut(),
            style,
            drawn,
            output,
        );
        slot.state.painted = Some(frame);

        let surface = &layer.surface;
        surface.attach(Some(&slot.buffer), 0, 0);
        match &layer.viewport {
            Some(viewport) => {
                if layer.destination != Some(frame.size) {
                    let (width, height) = (frame.size.width, frame.size.height);
                    viewport.set_destination(logical(width), logical(height));
                    layer.destination = Some(frame.size);
                }
                set_buffer_scale(surface, &mut layer.buffer_scale, 1);
            }
            None => set_buffer_scale(surface, &mut layer.buffer_scale, scale.integer()),
        }
        let opaque = (frame.size, style.opaque_inset());
        if layer.opaque != Some(opaque) {
            set_opaque_region(globals, qh, surface, opaque);
            layer.opaque = Some(opaque);
        }
        // The input region is the rounded shape (logical pixels, so the
        // scale is not part of its key): only when the size or the
        // effective radius changes, and never for a square bar, which
        // keeps the default (the whole surface).
        let round = region::effective_radius(style.radius, frame.size.width, frame.size.height);
        let input = (frame.size, round);
        if layer.input != Some(input) {
            if round > 0 || layer.input.is_some_and(|(_, was)| was > 0) {
                set_input_region(globals, qh, surface, input);
            }
            layer.input = Some(input);
        }
        if render::damage(&mut self.shown, scene, drawn, &mut self.damage) {
            surface.damage_buffer(0, 0, logical(dims.0), logical(dims.1));
        } else {
            for span in &self.damage {
                surface.damage_buffer(logical(span.x), 0, logical(span.width), logical(dims.1));
            }
        }
        surface.commit();
        slot.state.held = true;
        self.current = Some(dims);
        // Free buffers of another size are memory the bar will not use.
        for entry in &mut self.slots {
            if entry
                .as_ref()
                .is_some_and(|s| !s.state.held && s.state.dims != dims)
            {
                if let Some(stale) = entry.take() {
                    stale.destroy();
                }
            }
        }
        Ok(Drew::Committed)
    }
}

/// `wl_surface.set_opaque_region` for a surface of `size` (logical) that is
/// opaque `inset` pixels in from every edge (`Style::opaque_inset`): all of
/// it, or a cross of two rectangles that leaves the corner squares out
/// (their pixels are partly transparent), or none of it (`None`: the
/// compositor blends the whole surface).
fn set_opaque_region(
    globals: &Globals,
    qh: &QueueHandle<State>,
    surface: &WlSurface,
    (size, inset): (Size, Option<u32>),
) {
    let Some(inset) = inset else {
        surface.set_opaque_region(None);
        return;
    };
    let (width, height) = (size.width, size.height);
    // A surface too small for its corners keeps none opaque, as the paint
    // draws it square only when the corners do not fit: be conservative.
    let inset = inset.min(width / 2).min(height / 2);
    let region = globals.compositor.create_region(qh, ());
    if inset == 0 {
        region.add(0, 0, logical(width), logical(height));
    } else {
        region.add(
            0,
            logical(inset),
            logical(width),
            logical(height - 2 * inset),
        );
        region.add(
            logical(inset),
            0,
            logical(width - 2 * inset),
            logical(height),
        );
    }
    surface.set_opaque_region(Some(&region));
    region.destroy();
}

/// `wl_surface.set_input_region` for a surface of `size` (logical) whose
/// corners are cut by `radius` ([`region::input_rects`]): the bar's rounded
/// shape, or (`radius` 0) the default, the whole surface.
fn set_input_region(
    globals: &Globals,
    qh: &QueueHandle<State>,
    surface: &WlSurface,
    (size, radius): (Size, u32),
) {
    if radius == 0 {
        surface.set_input_region(None);
        return;
    }
    let mut rects = Vec::with_capacity(2 * radius as usize + 1);
    region::input_rects(size.width, size.height, radius, &mut rects);
    let region = globals.compositor.create_region(qh, ());
    for rect in rects {
        region.add(
            logical(rect.x),
            logical(rect.y),
            logical(rect.width),
            logical(rect.height),
        );
    }
    surface.set_input_region(Some(&region));
    region.destroy();
}

/// `wl_surface.set_buffer_scale`, sent only when it changes (`current` is
/// what the surface has).
fn set_buffer_scale(surface: &WlSurface, current: &mut u32, scale: u32) {
    if *current != scale {
        surface.set_buffer_scale(logical(scale));
        *current = scale;
    }
}

/// A size for a protocol `int`. Every size here is bounded far below
/// `i32::MAX` (the bar's height by `bar::MAX_HEIGHT`, a buffer's sides by
/// `ShmBuffer`'s own `i32` check, a width by the compositor that sent it as
/// a `uint` it meant as a size); saturate rather than wrap all the same.
fn logical(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}
