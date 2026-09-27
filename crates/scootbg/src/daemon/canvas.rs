//! Drawing a color on an output's surface: the buffers each output holds,
//! and the requests that put one on screen. The decisions (which path,
//! what to draw, which buffer) are `crate::paint`'s and the model's; this
//! carries them out.
//!
//! Every draw is one batch: attach (only when the buffer changes), buffer
//! scale or viewport destination, damage (the whole buffer), opaque region
//! (the whole surface), commit. The surface's latest `configure` was acked when it
//! arrived, before any of this, so each commit carries it.
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
    color: Color,
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
        Ok(Self {
            buffer,
            mem: Mem::Free(memory),
            dims,
            color,
        })
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
    /// Path 1: the single-pixel buffer on the surface, and its color.
    pixel: Option<(WlBuffer, Color)>,
    /// Paths 2 and 3.
    slots: [Option<ShmSlot>; SLOTS],
    /// The slot on the surface now, if any (it may be held or released).
    current: Option<usize>,
    /// A draw waits for a buffer to be released.
    pub stalled: bool,
}

impl fmt::Debug for Canvas {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Canvas")
            .field("pixel", &self.pixel.as_ref().map(|(_, color)| color))
            .field("slots", &self.slots.iter().filter(|s| s.is_some()).count())
            .field("current", &self.current)
            .field("stalled", &self.stalled)
            .finish()
    }
}

impl Canvas {
    /// Draws `target` on `layer`'s surface and commits, or reports a stall.
    pub fn show(
        &mut self,
        globals: &Globals,
        qh: &QueueHandle<State>,
        id: OutputId,
        layer: &mut LayerObjects,
        target: Drawn,
    ) -> Result<Drew, DrawError> {
        let path = globals.path;
        let dims = target
            .buffer_dims(path)
            .ok_or(DrawError::TooLarge(target.size, target.scale))?;
        let surface = layer.surface.clone();
        // What replaces the old single-pixel buffer, destroyed after the
        // commit.
        let mut retired = None;
        match path {
            Path::SinglePixel => {
                if self.pixel.as_ref().map(|(_, color)| *color) != Some(target.color) {
                    let manager = globals
                        .single_pixel
                        .as_ref()
                        .ok_or(DrawError::Missing("wp_single_pixel_buffer_manager_v1"))?;
                    let [r, g, b, a] = target.color.single_pixel();
                    let buffer = manager.create_u32_rgba_buffer(r, g, b, a, qh, id);
                    surface.attach(Some(&buffer), 0, 0);
                    retired = self
                        .pixel
                        .replace((buffer, target.color))
                        .map(|(old, _)| old);
                }
            }
            Path::ViewportShm | Path::FullShm => {
                let on_screen = self.current.and_then(|i| self.slots.get(i)?.as_ref());
                let unchanged =
                    on_screen.is_some_and(|slot| slot.dims == dims && slot.color == target.color);
                if !unchanged {
                    let Some((index, slot)) =
                        self.take_slot(globals, qh, id, dims, target.color)?
                    else {
                        self.stalled = true;
                        return Ok(Drew::Stalled);
                    };
                    surface.attach(Some(&slot.buffer), 0, 0);
                    self.put(index, slot.attached());
                    self.current = Some(index);
                }
                if path == Path::FullShm {
                    surface.set_buffer_scale(clamp(target.scale));
                }
            }
        }
        if path.uses_viewport() {
            let viewporter = globals
                .viewporter
                .as_ref()
                .ok_or(DrawError::Missing("wp_viewporter"))?;
            let viewport = layer
                .viewport
                .get_or_insert_with(|| viewporter.get_viewport(&surface, qh, ()));
            viewport.set_destination(clamp(target.size.width), clamp(target.size.height));
        }
        // All of it, every time: a draw is rare, and a new buffer scale or
        // viewport with the same buffer changes every pixel on screen too.
        surface.damage_buffer(0, 0, clamp(dims.0), clamp(dims.1));
        // Opaque everywhere: the compositor need draw nothing beneath it.
        let region = globals.compositor.create_region(qh, ());
        region.add(0, 0, clamp(target.size.width), clamp(target.size.height));
        surface.set_opaque_region(Some(&region));
        region.destroy();
        surface.commit();
        if let Some(old) = retired {
            old.destroy();
        }
        self.stalled = false;
        Ok(Drew::Committed)
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
                        color: written,
                    }) => {
                        if written != color {
                            fill(&mut memory, color);
                        }
                        let slot = ShmSlot {
                            buffer,
                            mem: Mem::Free(memory),
                            dims,
                            color,
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
    /// buffer that is not on screen and not the size on screen is dropped,
    /// so an old size does not linger. Returns whether a stalled draw
    /// should be retried.
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
        let stale = Some(index) != self.current && current_dims.is_some_and(|d| d != slot.dims);
        if stale {
            slot.destroy();
        } else {
            self.put(index, slot);
        }
        self.stalled
    }

    /// The surface is gone (closed, or replaced by `clear`): nothing of
    /// this is on screen any more. Shm buffers stay for reuse (the
    /// compositor releases those it held with the surface); the
    /// single-pixel one is no use.
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
