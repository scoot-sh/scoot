//! A drawn cursor shape backed by a dma-buf an overlay plane can scan out.
//!
//! On a CRTC with no cursor plane (Apple Silicon's `apple,dcp`), the pointer
//! is an ordinary element. Smithay may put a `Kind::Cursor` element on an
//! overlay plane, but only one whose storage can become a DRM framebuffer, and
//! a [`MemoryRenderBuffer`](smithay::backend::renderer::element::memory::MemoryRenderBuffer)
//! cannot. Composited, the cursor is a rendered element above everything, so
//! a fullscreen window under it never gets the primary plane (Smithay tries
//! the primary only for the bottom element, and only when nothing above it
//! was rendered).
//!
//! [`PlaneCursorElement`] is the same drawn shape with a dma-buf copy of its
//! pixels beside it. It draws exactly as the memory element it wraps does (a
//! composited frame, a capture's re-render), and answers
//! [`UnderlyingStorage::Dmabuf`] to Smithay's plane assignment, which takes
//! it through the exporter like a client's dma-buf `wl_buffer`. Both are built
//! by `render::cursor_plane`, the one place that knows whether this output
//! can use one.

use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::backend::renderer::{ImportMem, Renderer};
use smithay::utils::user_data::UserDataMap;
use smithay::utils::{Buffer, Physical, Point, Rectangle, Scale, Transform};

/// A cursor shape that draws from memory and scans out from a dma-buf holding
/// the same pixels. See the module doc.
///
/// Every [`Element`] answer is the wrapped memory element's, so the damage
/// tracker, the capture footprint (`render::capture_cursor`) and Smithay's
/// plane assignment all see one geometry, and the plane shows exactly what a
/// composited frame would. Only [`RenderElement::underlying_storage`] differs.
#[derive(Debug)]
pub struct PlaneCursorElement<R: Renderer> {
    inner: MemoryRenderBufferRenderElement<R>,
    /// Same pixels, size and format as `inner`'s buffer. A clone is a
    /// reference-count bump: the frame path never allocates for it.
    dmabuf: Dmabuf,
}

impl<R: Renderer> PlaneCursorElement<R> {
    /// Pairs a memory element with the dma-buf holding the same pixels.
    ///
    /// The caller guarantees the pairing (`render::cursor_plane` builds both
    /// from one pixel copy); nothing here can check it per frame without
    /// reading the dma-buf back.
    pub fn new(inner: MemoryRenderBufferRenderElement<R>, dmabuf: Dmabuf) -> Self {
        Self { inner, dmabuf }
    }
}

impl<R: Renderer> Element for PlaneCursorElement<R> {
    fn id(&self) -> &Id {
        self.inner.id()
    }

    fn current_commit(&self) -> CommitCounter {
        self.inner.current_commit()
    }

    fn location(&self, scale: Scale<f64>) -> Point<i32, Physical> {
        self.inner.location(scale)
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        self.inner.src()
    }

    fn transform(&self) -> Transform {
        self.inner.transform()
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.inner.geometry(scale)
    }

    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        self.inner.damage_since(scale, commit)
    }

    fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        self.inner.opaque_regions(scale)
    }

    fn alpha(&self) -> f32 {
        self.inner.alpha()
    }

    fn kind(&self) -> Kind {
        // Always a cursor: `scanout::overlay_rode` and the capture record
        // (`CursorInFrame::of`) tell the cursor from a window by this alone,
        // and Smithay offers overlays only to cursors and scanout candidates.
        Kind::Cursor
    }
}

impl<R> RenderElement<R> for PlaneCursorElement<R>
where
    R: Renderer + ImportMem,
    R::TextureId: 'static,
{
    fn draw(
        &self,
        frame: &mut R::Frame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), R::Error> {
        self.inner
            .draw(frame, src, dst, damage, opaque_regions, cache)
    }

    fn underlying_storage(&self, _renderer: &mut R) -> Option<UnderlyingStorage<'_>> {
        Some(UnderlyingStorage::Dmabuf(&self.dmabuf))
    }
}
