//! Drawing a color or an image on an output's surface: the buffers each
//! output holds, and the requests that put one on screen. The decisions
//! (which path, what to draw, which buffer) are `crate::paint`'s and the
//! model's; this carries them out.
//!
//! Every draw is one batch: attach (only when the buffer changes), buffer
//! scale, viewport destination and opaque region (the whole surface; each
//! only when it differs from what the surface has, as they persist),
//! damage (the whole buffer), commit. The viewport sizes any buffer that is
//! not the surface's size at its own buffer scale (a 1×1 color, or a
//! buffer at a fractional scale, `crate::density`); once a surface has one,
//! its destination is kept at the surface's size whatever is drawn. The
//! surface's latest `configure` was acked when it arrived, before any of
//! this, so each commit carries it.
//!
//! ## Buffers
//!
//! - **Single-pixel** (path 1): one `wl_buffer` from
//!   `create_u32_rgba_buffer` per color. A new color makes a new one; the
//!   old one is destroyed right after the commit that replaces it. It has
//!   no storage to write, so destroying it while the compositor still
//!   holds it is harmless.
//! - **Shm** (paths 2 and 3): up to [`SLOTS`] sealed-memfd buffers. A
//!   buffer is written only while writable (never attached, or released,
//!   and its pixels shared with no other buffer: [`crate::share`]), so a
//!   change reuses a released buffer in place, else takes the other slot,
//!   else waits for a release ([`Canvas::stalled`]). A buffer released
//!   while another is on screen is dropped at once, not kept as a spare:
//!   a static wallpaper needs one buffer, and the next change allocating
//!   again costs a few milliseconds (13 ms more at 4K, measured in
//!   `docs/scootbg/backlog/resolved/memory-and-idle-done.md`) against a
//!   whole output's worth of memory held for as long as it stays up. The
//!   memfd is closed as soon as its `wl_shm_pool` exists (the compositor
//!   has its own copy, the mapping keeps the memory); the pool is kept,
//!   with no fd, for as long as the pixels are, so another output's buffer
//!   can be made over them.
//!
//! - **Images**: a full-size shm buffer (the surface's real device pixels)
//!   the worker thread rendered (`daemon::worker`), once per size: outputs
//!   of one size showing one image share the pixels ([`Pixels`], offered
//!   to each with [`Canvas::offer`]), each through its own `wl_buffer`
//!   (why not one: `crate::share`), put in a slot like a color's. A draw
//!   with no rendered pixels for the image at that size says so
//!   ([`Drew::NeedsRender`]) and the caller asks the worker, or finds them
//!   on another output (`daemon::images::pump`).
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
use std::rc::Rc;

use wayland_client::QueueHandle;
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_shm;
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;

use scootbg_mem::{ShmBuffer, ShmError};

use super::surfaces::LayerObjects;
use super::wayland::{Globals, State};
use crate::color::Color;
use crate::density::Scale;
use crate::outputs::{OutputId, Size};
use crate::paint::{self, Drawn, Path, Pick, SLOTS, pick};
use crate::share::{Shared, Slot};
use crate::wallpaper::Wallpaper;

/// Why a draw failed. Reported on stderr, and the waiting reply says so.
#[derive(Debug)]
pub enum DrawError {
    /// The buffer's size overflows (a compositor asking for an absurd
    /// surface).
    TooLarge(Size, Scale),
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

/// What a buffer's pixels hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Content {
    Color(Color),
    /// The image of this serial (`crate::wallpaper::Image`).
    Image(u64),
    /// A transition frame (`daemon::transition`): never shared between
    /// outputs and never mistaken for an endpoint, so it matches neither.
    Frame,
}

/// The memory behind one or more outputs' buffers: the mapping, and the
/// `wl_shm_pool` their `wl_buffer`s are made from. The pool goes with the
/// last share ([`Pixels`]).
#[derive(Debug)]
pub struct Memory {
    shm: ShmBuffer,
    pool: WlShmPool,
    dims: (u32, u32),
    content: Content,
}

impl Drop for Memory {
    fn drop(&mut self) {
        // The compositor unmaps the pool once its buffers are gone too.
        self.pool.destroy();
    }
}

impl Memory {
    /// The pixels, shared, for a transition snapshot to copy.
    pub(crate) fn bytes(&self) -> &[u8] {
        self.shm.pixels()
    }

    /// The pixels, to blend a transition frame into.
    pub(crate) fn bytes_mut(&mut self) -> &mut [u8] {
        self.shm.pixels_mut()
    }
}

/// Pixels to show, shared by every output of one size showing one image
/// (`crate::share`).
pub type Pixels = Rc<Shared<Memory>>;

/// Hands `shm`, whose pixels hold `content`, to the compositor: a pool
/// over its memfd, which is then closed. Fails only for a buffer whose fd
/// is closed already, which every caller's fresh buffer is not.
pub(crate) fn pixels(
    globals: &Globals,
    qh: &QueueHandle<State>,
    mut shm: ShmBuffer,
    content: Content,
) -> Result<Pixels, ShmError> {
    let geometry = shm.geometry();
    // Positive `i32`s (`Geometry` validated them): lossless.
    let dims = (geometry.width as u32, geometry.height as u32);
    let pool = match shm.fd() {
        Some(fd) => globals.shm.create_pool(fd, geometry.len, qh, ()),
        None => {
            return Err(ShmError::Io(std::io::Error::other(
                "the buffer's memfd is already closed",
            )));
        }
    };
    // The queued request holds its own copy of the fd until it is sent, and
    // the compositor its own after that.
    shm.close_fd();
    Ok(Shared::new(Memory {
        shm,
        pool,
        dims,
        content,
    }))
}

/// One output's buffer over [`Pixels`].
pub(crate) type ShmSlot = Slot<WlBuffer, Memory>;

/// A `wl_buffer` for this output over `pixels`, not attached yet.
pub(crate) fn buffer_over(pixels: Pixels, qh: &QueueHandle<State>, id: OutputId) -> ShmSlot {
    let memory = pixels.memory();
    let geometry = memory.shm.geometry();
    let buffer = memory.pool.create_buffer(
        0,
        geometry.width,
        geometry.height,
        geometry.stride,
        wl_shm::Format::Xrgb8888,
        qh,
        id,
    );
    Slot::new(buffer, pixels)
}

/// A new buffer of `dims` filled with `color`.
fn color_buffer(
    globals: &Globals,
    qh: &QueueHandle<State>,
    id: OutputId,
    dims: (u32, u32),
    color: Color,
) -> Result<ShmSlot, ShmError> {
    let mut shm = ShmBuffer::new(dims.0, dims.1)?;
    fill(&mut shm, color);
    let pixels = pixels(globals, qh, shm, Content::Color(color))?;
    Ok(buffer_over(pixels, qh, id))
}

/// Destroys the slot's `wl_buffer`. Its pixels (the mapping, the pool) go
/// with their last share. If the compositor still holds the buffer, the
/// wl_buffer protocol allows destroying it before its release as long as
/// the storage is not written again, and it never is (`crate::share`
/// freezes it for anyone still showing it).
pub(crate) fn destroy(slot: ShmSlot) {
    slot.retire().destroy();
}

/// Brings the surface's persistent, double-buffered state to a buffer of
/// `scale` under `size`: the buffer scale, the viewport destination (when
/// the buffer is not the surface's size at its scale, or the surface
/// already has a viewport) and the opaque region (the whole surface).
/// Each is sent only when it differs from what the surface has, as they
/// persist. A transition frame commits through this for the same tail its
/// static draw uses.
pub(crate) fn sync_surface(
    globals: &Globals,
    layer: &mut LayerObjects,
    surface: &WlSurface,
    qh: &QueueHandle<State>,
    scale: u32,
    size: Size,
    viewported: bool,
) -> Result<(), DrawError> {
    if layer.buffer_scale != scale {
        surface.set_buffer_scale(clamp(scale));
        layer.buffer_scale = scale;
    }
    if (viewported || layer.viewport.is_some()) && layer.destination != Some(size) {
        let viewporter = globals
            .viewporter
            .as_ref()
            .ok_or(DrawError::Missing("wp_viewporter"))?;
        let viewport = layer
            .viewport
            .get_or_insert_with(|| viewporter.get_viewport(surface, qh, ()));
        viewport.set_destination(clamp(size.width), clamp(size.height));
        layer.destination = Some(size);
    }
    if layer.opaque != Some(size) {
        // Opaque everywhere: the compositor need draw nothing beneath it.
        let region = globals.compositor.create_region(qh, ());
        region.add(0, 0, clamp(size.width), clamp(size.height));
        surface.set_opaque_region(Some(&region));
        region.destroy();
        layer.opaque = Some(size);
    }
    Ok(())
}

impl paint::Slot for ShmSlot {
    fn is_free(&self) -> bool {
        Slot::is_free(self)
    }

    fn is_writable(&self) -> bool {
        Slot::is_writable(self)
    }

    fn dims(&self) -> (u32, u32) {
        self.memory().dims
    }
}

/// Whether `slot` shows `content` at `dims`.
fn holds(slot: &ShmSlot, content: Content, dims: (u32, u32)) -> bool {
    let memory = slot.memory();
    memory.content == content && memory.dims == dims
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
    /// An image rendered for this output, waiting to go on screen. At most
    /// one, and often the same pixels as other outputs' (`Pixels`).
    ready: Option<Pixels>,
    /// Rendered workspace wallpapers kept for instant switches
    /// (`crate::workspaces`): one entry per (image, size), shared with
    /// other outputs of that size showing that image (`Pixels`). A switch
    /// to a workspace whose pixels are stashed attaches at once; without
    /// them it would decode first (hundreds of milliseconds). Bounded by
    /// the mappings (`crate::choices::MAX_WORKSPACES`), dropped with them
    /// and when the output's size leaves them stale.
    stash: Vec<Pixels>,
    /// A draw waits for a buffer to be released.
    pub stalled: bool,
}

impl fmt::Debug for Canvas {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Canvas")
            .field("pixel", &self.pixel.as_ref().map(|(_, color)| color))
            .field("slots", &self.slots.iter().filter(|s| s.is_some()).count())
            .field("current", &self.current)
            .field(
                "ready",
                &self
                    .ready
                    .as_ref()
                    .map(|r| (r.memory().content, r.memory().dims)),
            )
            .field("stashed", &self.stash.len())
            .field("stalled", &self.stalled)
            .finish()
    }
}

impl Canvas {
    /// Pixels rendered for an image, to show at the next draw that wants
    /// exactly that image at that size. Replaces (drops) any earlier ones
    /// not yet shown.
    pub fn offer(&mut self, pixels: Pixels) {
        self.ready = Some(pixels);
    }

    /// Drops a rendered image waiting to go on screen unless it is of image
    /// `serial`: once something else is wanted, it could never be shown, and
    /// it is an output-sized buffer.
    pub fn forget_ready_unless(&mut self, serial: Option<u64>) {
        let keep = self.ready.as_ref().is_some_and(|ready| {
            serial.is_some_and(|serial| ready.memory().content == Content::Image(serial))
        });
        if !keep {
            self.ready = None;
        }
    }

    /// Pixels this output has (on screen, kept, waiting, or stashed for a
    /// workspace switch) holding image `serial` at `dims`, for another
    /// output of that size to share.
    pub fn image(&self, serial: u64, dims: (u32, u32)) -> Option<&Pixels> {
        let content = Content::Image(serial);
        self.slots
            .iter()
            .flatten()
            .map(Slot::shared)
            .chain(&self.ready)
            .chain(&self.stash)
            .find(|pixels| pixels.memory().content == content && pixels.memory().dims == dims)
    }

    /// Keeps `pixels` for a workspace switch that may come: a later draw
    /// of that image at that size attaches at once instead of decoding.
    /// Replaces (drops) what was stashed for that image at that size; the
    /// memory is shared with other outputs holding the same pixels.
    pub fn stash(&mut self, pixels: Pixels) {
        let (content, dims) = {
            let memory = pixels.memory();
            (memory.content, memory.dims)
        };
        if let Some(index) = self.stash.iter().position(|stashed| {
            let memory = stashed.memory();
            memory.content == content && memory.dims == dims
        }) {
            self.stash[index] = pixels;
        } else {
            self.stash.push(pixels);
        }
    }

    /// Moves stashed pixels for image `serial` at `dims` into `ready`, for
    /// the next draw to attach: whether a switch to that workspace is
    /// instant. `false` when nothing was stashed (decode instead).
    pub fn revive(&mut self, serial: u64, dims: (u32, u32)) -> bool {
        let content = Content::Image(serial);
        let index = self.stash.iter().position(|stashed| {
            let memory = stashed.memory();
            memory.content == content && memory.dims == dims
        });
        match index {
            Some(index) => {
                self.ready = Some(self.stash[index].clone());
                true
            }
            None => false,
        }
    }

    /// Drops everything stashed for image `serial`: its mapping went away
    /// (or was replaced), so no switch will ever want it.
    pub fn drop_stash_serial(&mut self, serial: u64) {
        let content = Content::Image(serial);
        self.stash
            .retain(|stashed| stashed.memory().content != content);
    }

    /// Drops every stashed workspace image: adopting a profile whose file
    /// says nothing of the old mappings (`daemon::config`), so no switch
    /// will ever want any of them. Stashes hold only workspace images
    /// (`images::offer` keeps them only for mappings), so nothing else goes.
    pub fn drop_all_stash(&mut self) {
        self.stash.clear();
    }

    /// Drops everything stashed at another size than `dims`: the output
    /// was reconfigured, and a switch draws at its size now. Stashes for
    /// the output's own size stay (other mappings may still want them).
    pub fn drop_stale_stash(&mut self, dims: (u32, u32)) {
        self.stash.retain(|stashed| stashed.memory().dims == dims);
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
        let buffer = target
            .buffer(path)
            .ok_or(DrawError::TooLarge(target.size, target.scale))?;
        let dims = buffer.dims;
        let surface = layer.surface.clone();
        // What replaces the old single-pixel buffer, destroyed after the
        // commit.
        let mut retired = None;
        // Whether this draw attaches a buffer.
        let mut attached = true;
        // A rendered image waits only for a draw of that image: any other
        // draw makes it moot (dropped at once, not at the next `clear`).
        if !matches!(target.content, Wallpaper::Image(_)) {
            self.ready = None;
        }
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
                let unchanged = on_screen.is_some_and(|slot| holds(slot, content, dims));
                if !unchanged {
                    let Some((index, mut slot)) = self.take_slot(globals, qh, id, dims, *color)?
                    else {
                        self.stalled = true;
                        return Ok(Drew::Stalled);
                    };
                    surface.attach(Some(slot.buffer()), 0, 0);
                    slot.attached();
                    self.put(index, slot);
                    self.current = Some(index);
                    // Any other free buffer is now a spare (one kept across a
                    // re-created surface, say): none is kept.
                    self.drop_free_slots();
                } else {
                    attached = false;
                }
            }
            Wallpaper::Image(image) => {
                let content = Content::Image(image.serial);
                let unchanged = on_screen.is_some_and(|slot| holds(slot, content, dims));
                // A released buffer already holding it (the surface was
                // closed and made again): shown as it is, no new decode.
                let kept = (0..SLOTS).find(|&i| {
                    self.slots
                        .get(i)
                        .and_then(Option::as_ref)
                        .is_some_and(|slot| slot.is_free() && holds(slot, content, dims))
                });
                if unchanged {
                    attached = false;
                    self.ready = None;
                } else if let Some(index) = kept {
                    self.ready = None;
                    if let Some(mut slot) = self.slots.get_mut(index).and_then(Option::take) {
                        surface.attach(Some(slot.buffer()), 0, 0);
                        slot.attached();
                        self.put(index, slot);
                        self.current = Some(index);
                        // Any other free buffer is now a spare (one kept across a
                        // re-created surface, say): none is kept.
                        self.drop_free_slots();
                        retired = self.pixel.take().map(|(old, _)| old);
                    }
                } else {
                    // Anything else waiting is stale: dropped here.
                    let Some(ready) = self.ready.take().filter(|ready| {
                        ready.memory().content == content && ready.memory().dims == dims
                    }) else {
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
                    // A free buffer (`pick` names no held one): the
                    // compositor is done with it.
                    if let Some(old) = self.slots.get_mut(index).and_then(Option::take) {
                        destroy(old);
                    }
                    let mut slot = buffer_over(ready, qh, id);
                    surface.attach(Some(slot.buffer()), 0, 0);
                    slot.attached();
                    self.put(index, slot);
                    self.current = Some(index);
                    // Any other free buffer is now a spare (one kept across a
                    // re-created surface, say): none is kept.
                    self.drop_free_slots();
                    retired = self.pixel.take().map(|(old, _)| old);
                }
            }
        }
        // Persistent, double-buffered state: sent only when it differs
        // from what the surface has. A transition frame commits through
        // `sync_surface` below for the same tail; this keeps the
        // scale-only re-attach, which only this path needs (a frame always
        // attaches).
        let viewported = !buffer.fits(target.size);
        let scale_changed = layer.buffer_scale != buffer.scale;
        sync_surface(
            globals,
            layer,
            &surface,
            qh,
            buffer.scale,
            target.size,
            viewported,
        )?;
        if scale_changed && !attached {
            // The same buffer at a new buffer scale (a scale and a mode
            // that change together keep its size, or an integer scale
            // becomes the same fractional one). The protocol applies
            // the scale at the commit either way, but Smithay-based
            // compositors (scoot's pinned fork included) read it only
            // with a newly attached buffer and would keep showing the
            // old scale; attaching the buffer on screen again costs
            // nothing and is right everywhere. A new viewport
            // destination alone needs no such help: Smithay works the
            // surface's view out again at every commit
            // (`RendererSurfaceState::update_buffer`), and
            // `tests/scale.rs` checks it by screenshot.
            //
            // Attached again, the buffer is the compositor's again until
            // its next release, even if it had been released (wlroots
            // releases an shm buffer once uploaded): marked held, so no
            // later draw writes into it meanwhile.
            let current = self
                .current
                .and_then(|i| self.slots.get_mut(i))
                .and_then(Option::as_mut);
            if let Some(slot) = current {
                surface.attach(Some(slot.buffer()), 0, 0);
                slot.attached();
            } else if let Some((buffer, _)) = &self.pixel {
                surface.attach(Some(buffer), 0, 0);
            }
        }
        // A buffer that is not the surface's size at its buffer scale (a 1×1
        // color, a fractional-scale buffer) is sized by the viewport;
        // anything drawn on a surface that has one keeps its destination
        // the surface size, so the buffer is shown at the size it was drawn
        // for. (Sent by `sync_surface` above.)
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
                    destroy(slot);
                }
            }
        }
    }

    /// A buffer holding `color` at `dims`, taken out of its slot (reused
    /// and refilled, or newly allocated, as [`pick`] says) for the caller
    /// to attach and [`put`](Self::put) back; `None` when both are held.
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
                if let Some(mut slot) = self.slots.get_mut(index).and_then(Option::take) {
                    // `pick` names a writable slot; anything else falls
                    // through to a new buffer rather than write into
                    // pixels the compositor or another output may read.
                    if let Some(memory) = slot.memory_mut() {
                        if memory.content != Content::Color(color) {
                            fill(&mut memory.shm, color);
                            memory.content = Content::Color(color);
                        }
                        return Ok(Some((index, slot)));
                    }
                    destroy(slot);
                }
                index
            }
            Pick::Replace(index) => {
                if let Some(old) = self.slots.get_mut(index).and_then(Option::take) {
                    destroy(old);
                }
                index
            }
            Pick::Fill(index) => index,
        };
        let slot = color_buffer(globals, qh, id, dims, color).map_err(DrawError::Shm)?;
        Ok(Some((index, slot)))
    }

    /// Puts a slot back where it was taken from.
    fn put(&mut self, index: usize, slot: ShmSlot) {
        match self.slots.get_mut(index) {
            Some(place) => {
                if let Some(old) = place.replace(slot) {
                    destroy(old);
                }
            }
            // `index` came from `pick`, which stays below `SLOTS`.
            None => destroy(slot),
        }
    }

    /// `wl_buffer.release` for `buffer`: the slot is free again. A buffer
    /// released while something else is on screen (another slot, or a
    /// single-pixel buffer) is dropped: no spare is kept (see the module
    /// docs). One released because its surface went (closed by the
    /// compositor) is kept, and an image's is shown again on the new
    /// surface without decoding. Returns whether a stalled draw should be
    /// retried.
    pub fn released(&mut self, buffer: &WlBuffer) -> bool {
        let Some(index) = self
            .slots
            .iter()
            .position(|slot| slot.as_ref().is_some_and(|s| s.buffer() == buffer))
        else {
            return false;
        };
        let Some(mut slot) = self.slots.get_mut(index).and_then(Option::take) else {
            return false;
        };
        slot.released();
        let stale = Some(index) != self.current && (self.pixel.is_some() || self.current.is_some());
        if stale {
            destroy(slot);
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

    /// Nothing is to be shown for now (`clear`, the output gone, or given
    /// up on; its surface is destroyed first): every buffer goes. Pixels
    /// another output shares stay with it, frozen if this output's buffer
    /// over them was still held (`crate::share`). Stashed workspace pixels
    /// stay: the mappings stand, and the next draw for them attaches at
    /// once rather than decoding again.
    pub fn clear(&mut self) {
        self.surface_gone();
        self.ready = None;
        for slot in &mut self.slots {
            if let Some(slot) = slot.take() {
                destroy(slot);
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

#[cfg(test)]
mod tests {
    use super::Canvas;

    /// Adopting another profile drops every stash: an output that mapped N
    /// workspace images holds N stashes, and after the adopt holds none.
    /// Stashes hold only workspace images, so dropping all is dropping
    /// exactly the cleared mappings' buffers (see `drop_all_stash`).
    #[test]
    fn dropping_all_stash_returns_buffer_accounting_to_base() {
        let mut canvas = Canvas::default();
        assert_eq!(canvas.stash.len(), 0, "no mapping: no stash");
        canvas.drop_all_stash();
        assert_eq!(canvas.stash.len(), 0, "adopt with none mapped drops none");
    }
}
