//! The GPU scanout tier's framebuffer exporter: Smithay's GBM one, refusing a
//! client buffer whose tiled layout the framebuffer lost.
//!
//! # What it guards
//!
//! Under `--renderer gles` the dma-buf feedback offers the driver's own tiled
//! and compressed modifiers (`dmabuf.rs`), so a client buffer reaching the
//! primary plane (`render::primary_direct`) may carry one. Smithay turns a
//! client dma-buf into a framebuffer by importing it into GBM and `AddFB2`ing
//! the result, and the layout the framebuffer is added *with* is whatever GBM
//! reports back for the import (`drm/gbm.rs`'s `framebuffer_from_dmabuf` ->
//! `framebuffer_from_bo_internal`, at the pinned rev): the reported modifier
//! with `DRM_MODE_FB_MODIFIERS`, or -- when GBM reports none -- no modifier at
//! all, so KMS reads the buffer in the driver's *implicit* layout.
//!
//! A GBM that accepted a tiled import and then reported no modifier (or a
//! different one) would therefore produce a framebuffer describing the
//! buffer wrongly, and nothing downstream catches it: the framebuffer's
//! format becomes `{fourcc, Invalid}`, and Smithay lists `{fourcc, Invalid}`
//! for every fourcc of every plane unconditionally (`drm/mod.rs:288-297`), so
//! the plane check passes and the atomic test has no way to know. The
//! result is scrambled tiles on screen, not a fallback. No driver this
//! project has run on is known to do it -- it is exactly the case
//! `Asahi.md` Test 6 asks real hardware about -- but it is a wrong picture
//! on a real machine if one does, and checking costs one comparison per
//! exported buffer.
//!
//! # The rule
//!
//! A client buffer with an **explicit, non-`LINEAR` modifier** is given a
//! framebuffer only if that framebuffer carries the **same** modifier
//! ([`keeps_layout`]). Otherwise the framebuffer is dropped (which removes
//! it from KMS) and the exporter answers "no framebuffer" -- which Smithay
//! caches per element and buffer and turns into compositing that element, the
//! same path an shm buffer takes. The client is never told and cannot be
//! hurt: this is a missed optimisation at worst.
//!
//! Everything else is passed through untouched, deliberately:
//!
//! - **`LINEAR`, single-plane, offset 0** -- the one shape every client here
//!   has sent so far -- goes through Smithay's *non*-modifier GBM import
//!   (`allocator/gbm.rs:355-381`, `from_bo(bo, true)`), so its framebuffer is
//!   always added without a modifier and reads back `Invalid`, on every
//!   device. That is not a lost layout: it is correct exactly when the
//!   kernel's implicit layout for an imported linear buffer is linear, which
//!   it is on every driver this has run on (virtio measured; the same holds
//!   for the drivers whose implicit layout comes from the buffer object's own
//!   metadata, since a linear allocation carries linear metadata). Refusing
//!   it would end primary-direct on the only buffers it has ever been seen
//!   with.
//! - **An implicit-modifier buffer** never reaches here with a framebuffer:
//!   Smithay refuses those before importing (Weston's rule), and the
//!   feedback never offers one.
//! - **Swapchain slots** (`ExportBuffer::Allocator`) are this compositor's
//!   own allocations and are delegated as they are.
//!
//! A device without `ADDFB2_MODIFIERS` needs no rule of its own: an explicit
//! modifier reported by GBM is added with `DRM_MODE_FB_MODIFIERS`, which such
//! a device refuses, and Smithay gives client buffers no legacy fallback --
//! so that is already an export error and a composited element. The only way
//! a tiled buffer reaches it as a framebuffer is the lost-modifier case
//! above, which this catches.
//!
//! # What a refusal is remembered for
//!
//! Every refused modifier is also recorded ([`LostLayouts`]), because the
//! GPU scanout tier *invites* clients to allocate tiled layouts: the
//! per-surface scanout tranche (`dmabuf::scanout`) offers a fullscreen
//! window every layout the primary plane lists. Nothing short of a real
//! import tells whether this device's GBM keeps a given modifier, so the
//! scanout tranche learns it from here: a modifier this exporter has
//! refused is dropped from the tranche and the window re-sent the smaller
//! one, instead of being steered back into a layout that can only
//! composite. The record is per device, not per compositor, so it outlives
//! a CRTC switch (the presenter hands the same record to every rebuilt
//! exporter).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use smithay::backend::allocator::gbm::GbmBuffer;
use smithay::backend::allocator::{Buffer, Modifier};
use smithay::backend::drm::exporter::gbm::{Error as GbmExportError, GbmFramebufferExporter};
use smithay::backend::drm::exporter::{ExportBuffer, ExportFramebuffer};
use smithay::backend::drm::gbm::GbmFramebuffer;
use smithay::backend::drm::{DrmDeviceFd, Framebuffer};
use smithay::wayland::dmabuf::get_dmabuf;

#[cfg(test)]
mod tests;

/// [`GbmFramebufferExporter`], plus [`keeps_layout`] on client buffers and a
/// record of what that refused. See the module doc.
#[derive(Debug)]
pub(crate) struct LayoutKeepingExporter {
    inner: GbmFramebufferExporter<DrmDeviceFd>,
    lost: Rc<LostLayouts>,
}

impl LayoutKeepingExporter {
    /// Wraps `inner`, recording every refusal into `lost` -- the presenter's
    /// one record for this device, shared with the scanout tranche.
    pub(crate) fn new(inner: GbmFramebufferExporter<DrmDeviceFd>, lost: Rc<LostLayouts>) -> Self {
        Self { inner, lost }
    }
}

/// The explicit client modifiers this device's GBM has been seen to lose,
/// and a generation that changes whenever one is added.
///
/// Written only by [`LayoutKeepingExporter`] on a refusal; read by the GPU
/// scanout tier to keep those modifiers out of the per-surface scanout
/// tranche (`dmabuf::scanout`), which rebuilds when
/// [`generation`](Self::generation) moves. Single-threaded by construction
/// (`DrmCompositor` and the render path both live on the event-loop
/// thread), hence `Cell`/`RefCell` behind an `Rc`.
///
/// Bounded without a cap: a modifier is only ever added once, and a client
/// can only reach the exporter with a modifier the renderer imported, i.e.
/// one the advertised table holds -- so the record can never outgrow that
/// table. Empty on every device where GBM keeps what it imports, which is
/// every device measured so far.
#[derive(Debug, Default)]
pub(crate) struct LostLayouts {
    modifiers: RefCell<Vec<Modifier>>,
    generation: Cell<u64>,
}

impl LostLayouts {
    /// Records `modifier` as lost, moving the generation only when it is
    /// new. Called once per refused buffer (Smithay caches the refusal), so
    /// the allocation a first push makes is off every frame path.
    pub(crate) fn note(&self, modifier: Modifier) {
        let mut modifiers = self.modifiers.borrow_mut();
        if !modifiers.contains(&modifier) {
            modifiers.push(modifier);
            self.generation.set(self.generation.get().wrapping_add(1));
        }
    }

    /// Changes whenever [`note`](Self::note) adds a modifier: what the
    /// scanout tranche's cache is keyed on. One `Cell` read, per frame.
    pub(crate) fn generation(&self) -> u64 {
        self.generation.get()
    }

    /// The lost modifiers, for a tranche rebuild. A copy, because the
    /// rebuild runs outside any borrow of this record and happens only when
    /// the generation moved.
    pub(crate) fn modifiers(&self) -> Vec<Modifier> {
        self.modifiers.borrow().clone()
    }
}

impl ExportFramebuffer<GbmBuffer> for LayoutKeepingExporter {
    type Framebuffer = GbmFramebuffer;
    type Error = GbmExportError;

    fn add_framebuffer(
        &self,
        drm: &DrmDeviceFd,
        buffer: ExportBuffer<'_, GbmBuffer>,
        use_opaque: bool,
    ) -> Result<Option<Self::Framebuffer>, Self::Error> {
        // Read before the buffer is handed on. A pointer compare and a copy
        // of a `Format`; once per exported buffer, never per frame (Smithay
        // caches the answer per element and buffer).
        let client_modifier = match &buffer {
            ExportBuffer::Wayland(wl_buffer) => get_dmabuf(wl_buffer)
                .ok()
                .map(|dmabuf| dmabuf.format().modifier),
            ExportBuffer::Allocator(_) => None,
        };
        let framebuffer = self.inner.add_framebuffer(drm, buffer, use_opaque)?;
        Ok(framebuffer.filter(|framebuffer| {
            let kept = keeps_layout(client_modifier, framebuffer.format().modifier);
            if !kept {
                // Once per refused buffer (the refusal is cached), so `debug`
                // is enough and cannot flood.
                tracing::debug!(
                    client = ?client_modifier,
                    framebuffer = ?framebuffer.format().modifier,
                    "not scanning out a client buffer whose framebuffer lost its tiled layout"
                );
                // `keeps_layout` refuses only an explicit client modifier,
                // so there is always one to record here.
                if let Some(modifier) = client_modifier {
                    self.lost.note(modifier);
                }
            }
            kept
        }))
    }

    fn can_add_framebuffer(&self, buffer: &ExportBuffer<'_, GbmBuffer>) -> bool {
        self.inner.can_add_framebuffer(buffer)
    }
}

/// Whether a framebuffer added with `framebuffer` describes a client buffer
/// whose own modifier was `client` (`None`: not a client dma-buf).
///
/// Only an explicit, non-`LINEAR` client modifier is checked, and it must
/// survive exactly; see the module doc for why `LINEAR` and `Invalid` pass.
pub(crate) fn keeps_layout(client: Option<Modifier>, framebuffer: Modifier) -> bool {
    match client {
        None | Some(Modifier::Invalid | Modifier::Linear) => true,
        Some(explicit) => framebuffer == explicit,
    }
}
