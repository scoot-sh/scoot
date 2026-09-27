//! Drawing a color or an image on an output's surface: the buffers each
//! output holds, and the requests that put one on screen. The decisions
//! (which path, what to draw, which buffer) are `crate::paint`'s and the
//! model's; this carries them out.
//!
//! Every draw is one batch: attach (only when the buffer changes), buffer
//! scale, viewport destination and opaque region (the whole surface; each
//! only when it differs from what the surface has, as they persist),
//! damage (the whole buffer), commit. The surface's latest `configure` was
//! acked when it arrived, before any of this, so each commit carries it.
//!
//! ## Buffers
//!
//! - **Single-pixel** (path 1): one `wl_buffer` from
//!   `create_u32_rgba_buffer` per color. A new color makes a new one; the
//!   old one is destroyed right after the commit that replaces it. It has
//!   no storage to write, so destroying it while the compositor still
//!   holds it is harmless.
//! - **Shm** (paths 2 and 3): up to [`SLOTS`] sealed-memfd buffers. A
//!   buffer is written only while free (never attached, or released:
//!   `scootbg_mem::shm`'s type state), so a change reuses a released
//!   buffer in place, else takes the other slot, else waits for a release
//!   ([`Canvas::stalled`]). A released buffer of a size no longer drawn is
//!   dropped at once rather than kept. The `wl_shm_pool` is destroyed as
//!   soon as its buffer exists: the buffer keeps the memory alive.
//!
//! - **Images**: a full-size shm buffer the worker thread rendered
//!   (`daemon::worker`), offered here ([`Canvas::offer`]) and put in a slot
//!   like a color's. A draw with no rendered buffer for the image at that
//!   size says so ([`Drew::NeedsRender`]) and the caller asks the worker.
//!   A released buffer holding an image that is no longer on screen is
//!   dropped at once: only a new render could use its slot.
//!
//! ## Clearing
//!
//! `clear` destroys the layer surface and makes a fresh one (committed with
//! no buffer, waiting for its `configure`), rather than attaching a null
//! buffer. Both are protocol-correct, but a null attach *unmaps* the layer
//! surface, which "returns to the state it had right after
//! get_layer_surface": every property is reset (Smithay resets anchor,
//! size, exclusive zone and interactivity to their defaults, and a default
//! anchor with size 0×0 is a protocol error on the next commit), so it
//! would have to be set up again anyway, and re-mapping is the path
//! compositors exercise least. A fresh surface is the path every
//! compositor already takes for scootbg at start-up.

use std::fmt;

use wayland_client::QueueHandle;
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_shm;
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;

use scootbg_mem::{Attached, ShmBuffer, ShmError};

use super::surfaces::LayerObjects;
use super::wayland::{Globals, State};
use crate::color::Color;
use crate::outputs::{OutputId, Size};
use crate::paint::{Drawn, Path, Pick, SLOTS, Slot, pick};
use crate::wallpaper::Wallpaper;

/// Why a draw failed. Reported on stderr, and the waiting reply says so.
#[derive(Debug)]
pub enum DrawError {
    /// The buffer's size overflows (a compositor asking for an absurd
    /// surface).
    TooLarge(Size, u32),
    Shm(ShmError),
    /// The path's global is not bound. Cannot happen: the path is chosen
    /// from the globals bound at start-up, which stay bound.
    Missing(&'static str),
}

impl fmt::Display for DrawError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge(size, scale) => write!(
                f,
                "a {}x{} surface at scale {scale} is too large for a buffer",
                size.width, size.height
            ),
            Self::Shm(error) => write!(f, "{error}"),
            Self::Missing(interface) => write!(f, "the compositor has no {interface}"),
        }
    }
}

/// How a draw went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drew {
    /// Committed.
    Committed,
    /// Both shm buffers are held by the compositor: nothing was sent, and
    /// [`Canvas::stalled`] asks for a redraw on the next release.
    Stalled,
    /// An image with no buffer rendered for it at this size (pixels):
    /// nothing was sent; ask the worker, and draw again once it offers one.
    NeedsRender((u32, u32)),
}

/// What a slot's pixels hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Content {
    Color(Color),
    /// The image of this serial (`crate::wallpaper::Image`).
    Image(u64),
}

/// A buffer the worker rendered, not on screen yet.
struct Rendered {
    serial: u64,
    dims: (u32, u32),
    memory: ShmBuffer,
}

/// An shm buffer's memory: ours to write, or the compositor's to read.
enum Mem {
    Free(ShmBuffer),
    Held(Attached),
}

/// One shm buffer and its `wl_buffer`.
struct ShmSlot {
    buffer: WlBuffer,
    mem: Mem,
    dims: (u32, u32),
    /// What its pixels hold.
    content: Content,
}

impl ShmSlot {
    /// A new buffer of `dims` filled with `color`.
    fn new(
        globals: &Globals,
        qh: &QueueHandle<State>,
        id: OutputId,
        dims: (u32, u32),
        color: Color,
    ) -> Result<Self, ShmError> {
        let mut memory = ShmBuffer::new(dims.0, dims.1)?;
        fill(&mut memory, color);
        Ok(Self::wrap(globals, qh, id, memory, Content::Color(color)))
    }

    /// A `wl_buffer` for `memory`, whose pixels hold `content`.
    fn wrap(
        globals: &Globals,
        qh: &QueueHandle<State>,
        id: OutputId,
        memory: ShmBuffer,
        content: Content,
    ) -> Self {
        let geometry = memory.geometry();
        let pool = globals.shm.create_pool(memory.fd(), geometry.len, qh, ());
        let buffer = pool.create_buffer(
            0,
            geometry.width,
            geometry.height,
            geometry.stride,
            wl_shm::Format::Xrgb8888,
            qh,
            id,
        );
        // The buffer keeps the pool's memory alive; the pool object itself
        // is not needed again.
        pool.destroy();
        // Positive `i32`s (`Geometry` validated them): lossless.
        let dims = (geometry.width as u32, geometry.height as u32);
        Self {
            buffer,
            mem: Mem::Free(memory),
            dims,
            content,
        }
    }

    /// Hands it to the compositor: from here until [`ShmSlot::released`]
    /// the pixels cannot be reached.
    fn attached(self) -> Self {
        let mem = match self.mem {
            Mem::Free(memory) => Mem::Held(memory.attach()),
            held @ Mem::Held(_) => held,
        };
        Self { mem, ..self }
    }

    /// `wl_buffer.release`: ours to write again.
    fn released(self) -> Self {
        let mem = match self.mem {
            Mem::Held(attached) => Mem::Free(attached.released()),
            free @ Mem::Free(_) => free,
        };
        Self { mem, ..self }
    }

    fn destroy(self) {
        self.buffer.destroy();
        // The mapping goes with `mem`. If the compositor still holds the
        // buffer, it has its own mapping of the pool; the wl_buffer
        // protocol allows destroying before release as long as the storage
        // is not written again, and it never is.
    }
}

impl Slot for ShmSlot {
    fn is_free(&self) -> bool {
        matches!(self.mem, Mem::Free(_))
    }

    fn dims(&self) -> (u32, u32) {
        self.dims
    }
}

/// Writes `color` into every pixel.
fn fill(memory: &mut ShmBuffer, color: Color) {
    let pixel = color.xrgb8888();
    for chunk in memory.pixels_mut().chunks_exact_mut(pixel.len()) {
        chunk.copy_from_slice(&pixel);
    }
}

/// One output's buffers.
#[derive(Default)]
pub struct Canvas {
    /// Path 1: the single-pixel buffer on the surface, and its color. Never
    /// set while a slot is on screen (`current`).
    pixel: Option<(WlBuffer, Color)>,
    /// Paths 2 and 3, and images.
    slots: [Option<ShmSlot>; SLOTS],
    /// The slot on the surface now, if any (it may be held or released).
    current: Option<usize>,
    /// An image the worker rendered, waiting to go on screen.
    ready: Option<Rendered>,
    /// A draw waits for a buffer to be released.
    pub stalled: bool,
}

impl fmt::Debug for Canvas {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Canvas")
            .field("pixel", &self.pixel.as_ref().map(|(_, color)| color))
            .field("slots", &self.slots.iter().filter(|s| s.is_some()).count())
            .field("current", &self.current)
            .field("ready", &self.ready.as_ref().map(|r| (r.serial, r.dims)))
            .field("stalled", &self.stalled)
            .finish()
    }
}

impl Canvas {
    /// A buffer the worker rendered for image `serial` at `dims`, to show
    /// at the next draw that wants exactly that. Replaces (drops) any
    /// earlier one not yet shown.
    pub fn offer(&mut self, serial: u64, dims: (u32, u32), memory: ShmBuffer) {
        self.ready = Some(Rendered {
            serial,
            dims,
            memory,
        });
    }

    /// Draws `target` on `layer`'s surface and commits, or reports why not.
    pub fn show(
        &mut self,
        globals: &Globals,
        qh: &QueueHandle<State>,
        id: OutputId,
        layer: &mut LayerObjects,
        target: &Drawn,
    ) -> Result<Drew, DrawError> {
        let path = globals.path;
        let dims = target
            .buffer_dims(path)
            .ok_or(DrawError::TooLarge(target.size, target.scale))?;
        let surface = layer.surface.clone();
        // What replaces the old single-pixel buffer, destroyed after the
        // commit.
        let mut retired = None;
        // Whether this draw attaches a buffer.
        let mut attached = true;
        let on_screen = self.current.and_then(|i| self.slots.get(i)?.as_ref());
        match &target.content {
            Wallpaper::Color(color) if path == Path::SinglePixel => {
                let shown =
                    self.current.is_none() && self.pixel.as_ref().map(|(_, c)| *c) == Some(*color);
                if !shown {
                    let manager = globals
                        .single_pixel
                        .as_ref()
                        .ok_or(DrawError::Missing("wp_single_pixel_buffer_manager_v1"))?;
                    let [r, g, b, a] = color.single_pixel();
                    let buffer = manager.create_u32_rgba_buffer(r, g, b, a, qh, id);
                    surface.attach(Some(&buffer), 0, 0);
                    retired = self.pixel.replace((buffer, *color)).map(|(old, _)| old);
                    // Shm buffers (an image's, say) are no use on this
                    // path: the free ones go now, held ones on release.
                    self.current = None;
                    self.drop_free_slots();
                } else {
                    attached = false;
                }
            }
            Wallpaper::Color(color) => {
                let content = Content::Color(*color);
                let unchanged =
                    on_screen.is_some_and(|slot| slot.dims == dims && slot.content == content);
                if !unchanged {
                    let Some((index, slot)) = self.take_slot(globals, qh, id, dims, *color)? else {
                        self.stalled = true;
                        return Ok(Drew::Stalled);
                    };
                    surface.attach(Some(&slot.buffer), 0, 0);
                    self.put(index, slot.attached());
                    self.current = Some(index);
                } else {
                    attached = false;
                }
            }
            Wallpaper::Image(image) => {
                let content = Content::Image(image.serial);
                let unchanged =
                    on_screen.is_some_and(|slot| slot.dims == dims && slot.content == content);
                // A free buffer already holding it (the surface was
                // closed and made again): shown as it is, no new decode.
                let kept = (0..SLOTS).find(|&i| {
                    self.slots
                        .get(i)
                        .and_then(Option::as_ref)
                        .is_some_and(|slot| {
                            slot.is_free() && slot.dims == dims && slot.content == content
                        })
                });
                if unchanged {
                    attached = false;
                } else if let Some(index) = kept {
                    if let Some(slot) = self.slots.get_mut(index).and_then(Option::take) {
                        surface.attach(Some(&slot.buffer), 0, 0);
                        self.put(index, slot.attached());
                        self.current = Some(index);
                        retired = self.pixel.take().map(|(old, _)| old);
                    }
                } else {
                    // Anything else waiting is stale: dropped here.
                    let Some(ready) = self
                        .ready
                        .take()
                        .filter(|r| r.serial == image.serial && r.dims == dims)
                    else {
                        return Ok(Drew::NeedsRender(dims));
                    };
                    let index = match pick(&self.slots, dims) {
                        Pick::Reuse(index) | Pick::Replace(index) | Pick::Fill(index) => index,
                        Pick::Stall => {
                            self.ready = Some(ready);
                            self.stalled = true;
                            return Ok(Drew::Stalled);
                        }
                    };
                    if let Some(old) = self.slots.get_mut(index).and_then(Option::take) {
                        old.destroy();
                    }
                    let slot = ShmSlot::wrap(globals, qh, id, ready.memory, content);
                    surface.attach(Some(&slot.buffer), 0, 0);
                    self.put(index, slot.attached());
                    self.current = Some(index);
                    retired = self.pixel.take().map(|(old, _)| old);
                }
            }
        }
        // Persistent, double-buffered state: sent only when it differs
        // from what the surface has.
        if layer.buffer_scale != target.scale {
            surface.set_buffer_scale(clamp(target.scale));
            layer.buffer_scale = target.scale;
            if !attached {
                // The same buffer at a new scale (a scale and a mode that
                // change together keep its size). The protocol applies the
                // scale at the commit either way, but Smithay-based
                // compositors (scoot's pinned fork included) read it only
                // with a newly attached buffer and would keep showing the
                // old scale; attaching the buffer on screen again costs
                // nothing and is right everywhere.
                let current = self.current.and_then(|i| self.slots.get(i)?.as_ref());
                if let Some(slot) = current {
                    surface.attach(Some(&slot.buffer), 0, 0);
                } else if let Some((buffer, _)) = &self.pixel {
                    surface.attach(Some(buffer), 0, 0);
                }
            }
        }
        // A 1×1 color is sized by the viewport; anything drawn on a surface
        // that has one keeps its destination the surface size, so the
        // buffer is shown at the size it was drawn for.
        let viewported = path.uses_viewport() && matches!(target.content, Wallpaper::Color(_));
        if (viewported || layer.viewport.is_some()) && layer.destination != Some(target.size) {
            let viewporter = globals
                .viewporter
                .as_ref()
                .ok_or(DrawError::Missing("wp_viewporter"))?;
            let viewport = layer
                .viewport
                .get_or_insert_with(|| viewporter.get_viewport(&surface, qh, ()));
            viewport.set_destination(clamp(target.size.width), clamp(target.size.height));
            layer.destination = Some(target.size);
        }
        if layer.opaque != Some(target.size) {
            // Opaque everywhere: the compositor need draw nothing beneath it.
            let region = globals.compositor.create_region(qh, ());
            region.add(0, 0, clamp(target.size.width), clamp(target.size.height));
            surface.set_opaque_region(Some(&region));
            region.destroy();
            layer.opaque = Some(target.size);
        }
        // All of it, every time: a draw is rare, and a new buffer scale or
        // viewport with the same buffer changes every pixel on screen too.
        surface.damage_buffer(0, 0, clamp(dims.0), clamp(dims.1));
        surface.commit();
        if let Some(old) = retired {
            old.destroy();
        }
        self.stalled = false;
        Ok(Drew::Committed)
    }

    /// Drops every buffer the compositor is not holding.
    fn drop_free_slots(&mut self) {
        for place in &mut self.slots {
            if place.as_ref().is_some_and(Slot::is_free) {
                if let Some(slot) = place.take() {
                    slot.destroy();
                }
            }
        }
    }

    /// A free buffer holding `color` at `dims`, taken out of its slot
    /// (reused and refilled, or newly allocated, as [`pick`] says) for the
    /// caller to attach and [`put`](Self::put) back; `None` when both are
    /// held.
    fn take_slot(
        &mut self,
        globals: &Globals,
        qh: &QueueHandle<State>,
        id: OutputId,
        dims: (u32, u32),
        color: Color,
    ) -> Result<Option<(usize, ShmSlot)>, DrawError> {
        let index = match pick(&self.slots, dims) {
            Pick::Stall => return Ok(None),
            Pick::Reuse(index) => {
                // `pick` names a filled, free slot; anything else falls
                // through to a new buffer rather than panic.
                match self.slots.get_mut(index).and_then(Option::take) {
                    Some(ShmSlot {
                        buffer,
                        mem: Mem::Free(mut memory),
                        dims,
                        content,
                    }) => {
                        if content != Content::Color(color) {
                            fill(&mut memory, color);
                        }
                        let slot = ShmSlot {
                            buffer,
                            mem: Mem::Free(memory),
                            dims,
                            content: Content::Color(color),
                        };
                        return Ok(Some((index, slot)));
                    }
                    Some(other) => other.destroy(),
                    None => {}
                }
                index
            }
            Pick::Replace(index) => {
                if let Some(old) = self.slots.get_mut(index).and_then(Option::take) {
                    old.destroy();
                }
                index
            }
            Pick::Fill(index) => index,
        };
        let slot = ShmSlot::new(globals, qh, id, dims, color).map_err(DrawError::Shm)?;
        Ok(Some((index, slot)))
    }

    /// Puts a slot back where it was taken from.
    fn put(&mut self, index: usize, slot: ShmSlot) {
        match self.slots.get_mut(index) {
            Some(place) => {
                if let Some(old) = place.replace(slot) {
                    old.destroy();
                }
            }
            // `index` came from `pick`, which stays below `SLOTS`.
            None => slot.destroy(),
        }
    }

    /// `wl_buffer.release` for `buffer`: the slot is free again. A free
    /// buffer not on screen is dropped when nothing could reuse it: an old
    /// size, an image while something else is on screen (only a new render
    /// could fill its slot), or anything while a single-pixel buffer is on
    /// screen. An image's buffer released because its surface went (closed
    /// by the compositor) is kept, and shown again on the new surface
    /// without decoding. Returns whether a stalled draw should be retried.
    pub fn released(&mut self, buffer: &WlBuffer) -> bool {
        let Some(index) = self
            .slots
            .iter()
            .position(|slot| slot.as_ref().is_some_and(|s| &s.buffer == buffer))
        else {
            return false;
        };
        let current_dims = self
            .current
            .and_then(|i| self.slots.get(i)?.as_ref())
            .map(|slot| slot.dims);
        let Some(slot) = self.slots.get_mut(index).and_then(Option::take) else {
            return false;
        };
        let slot = slot.released();
        let stale = Some(index) != self.current
            && (self.pixel.is_some()
                || (matches!(slot.content, Content::Image(_)) && self.current.is_some())
                || current_dims.is_some_and(|d| d != slot.dims));
        if stale {
            slot.destroy();
        } else {
            self.put(index, slot);
        }
        self.stalled
    }

    /// The surface is gone (closed, or replaced by `clear`): nothing of
    /// this is on screen any more. Shm buffers stay for reuse (the
    /// compositor releases those it held with the surface), and so does a
    /// rendered image not yet shown; the single-pixel buffer is no use.
    pub fn surface_gone(&mut self) {
        if let Some((buffer, _)) = self.pixel.take() {
            buffer.destroy();
        }
        self.current = None;
        self.stalled = false;
    }

    /// Nothing is to be shown for now (`clear`): every buffer goes.
    pub fn clear(&mut self) {
        self.surface_gone();
        self.ready = None;
        for slot in &mut self.slots {
            if let Some(slot) = slot.take() {
                slot.destroy();
            }
        }
    }
}

/// A size for a request that takes `int32`. Sizes come from the compositor
/// as `u32`; anything past `i32::MAX` is nonsense, clamped rather than let
/// it wrap negative.
fn clamp(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

/// The viewport goes before its surface (`LayerObjects::destroy`).
pub fn destroy_viewport(viewport: Option<WpViewport>) {
    if let Some(viewport) = viewport {
        viewport.destroy();
    }
}
