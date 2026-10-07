//! The cursor on an overlay plane, where the CRTC has no cursor plane.
//!
//! # Why
//!
//! Smithay tries the primary plane only for the bottom element of a frame,
//! and only when nothing above it was rendered. On a CRTC with no cursor
//! plane (Apple Silicon's `apple,dcp`) the pointer is rendered, so while it is
//! visible a fullscreen window under it never gets the primary plane, and
//! every frame composites (`Asahi.md`, Tests 5 and 14:
//! 29-33 jiffies per 10 s for a fullscreen video against 13-14 with the pointer
//! hidden). Putting the cursor on an overlay plane leaves nothing rendered
//! above the window.
//!
//! Smithay's overlay assignment takes `Kind::Cursor` elements, but only those
//! whose storage can become a framebuffer, and the drawn shapes
//! (`cursor.rs`'s [`MemoryRenderBuffer`]s, the theme's and the fallback ones)
//! cannot. So on such a CRTC [`CursorPlanes::back`] swaps each drawn-shape
//! element of the frame for a [`PlaneCursorElement`]: the same pixels in a
//! `LINEAR` `Argb8888` dma-buf, answered as `UnderlyingStorage::Dmabuf` (a
//! scoot-sh Smithay fork variant, `docs/forks.md`). Smithay then tests it on
//! an overlay plane like any candidate and composites it if the test fails.
//!
//! A client's own cursor surface is left alone: its buffer is the client's,
//! and a client dma-buf can already ride an overlay. An shm one composites,
//! as before.
//!
//! # Padding
//!
//! The dma-buf (and the memory twin drawn when it composites) is padded with
//! transparent pixels on the right and bottom, to [`plane_size`]: at least
//! [`MIN_SIDE`] on both sides and a width a multiple of [`PITCH_PIXELS`].
//! `apple,dcp` rejects a plane whose destination is smaller than 32x32 (its
//! firmware crashes on one, so the driver refuses it in the atomic check) and a
//! pitch not 64-byte aligned. A 24 px cursor would otherwise always fail the
//! test. The hotspot is the top-left corner's offset, so padding there would
//! move it; padding right and bottom does not.
//!
//! # Cost
//!
//! Per frame: a hash lookup per drawn cursor element (one, in practice) and
//! the swap. The dma-buf is built once per distinct image, the first frame
//! that shows it: a padded copy, a GBM allocation and a write. Every
//! image after that is cached. The cache is bounded ([`MAX_IMAGES`]):
//! a config reload makes new images under new ids, and the old ones are
//! dropped when the bound is hit.

use std::collections::HashMap;

use smithay::backend::allocator::dmabuf::{Dmabuf, DmabufFlags};
use smithay::backend::allocator::gbm::{GbmBufferFlags, GbmDevice};
use smithay::backend::allocator::{Fourcc, Modifier};
use smithay::backend::drm::DrmDeviceFd;
use smithay::backend::renderer::element::memory::{
    MemoryRenderBuffer, MemoryRenderBufferRenderElement,
};
use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::{ImportAll, ImportMem, Renderer, Texture};
use smithay::utils::{Physical, Rectangle, Scale, Size, Transform};

use super::elements::Elements;
use crate::compositor::cursor::CursorElement;
use crate::compositor::cursor::plane::PlaneCursorElement;

#[cfg(test)]
mod tests;

/// The smallest plane side `apple,dcp` accepts, in pixels (see the module
/// doc's padding section).
const MIN_SIDE: i32 = 32;

/// Pixels per 64 bytes at 4 bytes a pixel: the width granularity that keeps
/// a tightly packed `LINEAR` pitch 64-byte aligned.
const PITCH_PIXELS: i32 = 16;

/// The largest image side this will put on a plane. Far above any cursor
/// (`Appearance::MAX_CURSOR_SIZE`, a theme's largest nominal size); it only
/// keeps a corrupt theme image from becoming a huge allocation.
const MAX_SIDE: i32 = 1024;

/// How many images the cache keeps before it starts over. Above
/// `Shape::COUNT` plus every theme shape a session uses, so it is reached only
/// after config reloads have left old images behind.
const MAX_IMAGES: usize = 64;

/// Where a plane image's pixels go: a scanout-capable `LINEAR` `Argb8888`
/// dma-buf. A trait so the cache is testable without a GPU.
pub(crate) trait Upload {
    /// A dma-buf of `width` x `height` holding `pixels` (tightly packed,
    /// `width * 4` bytes a row), or `None` when the device cannot make one.
    fn upload(&mut self, pixels: &[u8], width: u32, height: u32) -> Option<Dmabuf>;
}

/// [`Upload`] onto the scanout device's GBM.
pub(crate) struct GbmUpload {
    gbm: GbmDevice<DrmDeviceFd>,
}

impl GbmUpload {
    pub(crate) fn new(gbm: GbmDevice<DrmDeviceFd>) -> Self {
        Self { gbm }
    }
}

impl Upload for GbmUpload {
    fn upload(&mut self, pixels: &[u8], width: u32, height: u32) -> Option<Dmabuf> {
        // `LINEAR` by usage flag rather than by modifier list: the dma-buf
        // below names `Modifier::Linear` explicitly, which is what the
        // exporter requires (an implicit layout is never handed to KMS), and
        // the flag is what makes that claim true.
        let mut bo = self
            .gbm
            .create_buffer_object::<()>(
                width,
                height,
                Fourcc::Argb8888,
                GbmBufferFlags::SCANOUT | GbmBufferFlags::LINEAR,
            )
            .inspect_err(|error| {
                tracing::debug!(%error, width, height, "cursor plane: no scanout buffer");
            })
            .ok()?;
        let row = usize::try_from(width).ok()?.checked_mul(4)?;
        let written = bo
            .map_mut(0, 0, width, height, |map| {
                let stride = usize::try_from(map.stride()).ok()?;
                let target = map.buffer_mut();
                for (y, source) in pixels.chunks_exact(row).enumerate() {
                    let start = y.checked_mul(stride)?;
                    target
                        .get_mut(start..start.checked_add(row)?)?
                        .copy_from_slice(source);
                }
                Some(())
            })
            .inspect_err(|error| {
                tracing::debug!(%error, "cursor plane: could not map the scanout buffer");
            })
            .ok()
            .flatten();
        written?;
        let fd = bo.fd().ok()?;
        let mut builder = Dmabuf::builder(
            (i32::try_from(width).ok()?, i32::try_from(height).ok()?),
            Fourcc::Argb8888,
            Modifier::Linear,
            DmabufFlags::empty(),
        );
        builder.add_plane(fd, bo.offset(0), bo.stride());
        builder.build()
    }
}

/// One drawn image, ready for a plane: the padded pixels in memory (what a
/// composited frame draws) and in a dma-buf (what the plane scans out).
struct PlaneImage {
    memory: MemoryRenderBuffer,
    dmabuf: Dmabuf,
}

/// The plane images for one output's drawn cursor shapes, keyed by the source
/// buffer's [`Id`]. `None` remembers an image that cannot go on a plane (a
/// format or size this does not handle, a failed upload), so it is not
/// retried every frame.
pub(crate) struct CursorPlanes<U = GbmUpload> {
    upload: U,
    images: HashMap<Id, Option<PlaneImage>>,
}

impl<U: Upload> CursorPlanes<U> {
    pub(crate) fn new(upload: U) -> Self {
        Self {
            upload,
            images: HashMap::new(),
        }
    }

    /// Swaps every drawn-shape cursor element in `elements` for its
    /// plane-backed twin (see the module doc). An element whose image cannot
    /// be built stays as it is and composites, as before, and so does one
    /// whose twin would show less than [`MIN_SIDE`] either way on the
    /// output (`frame`: its scale and physical size): the driver clips a
    /// plane to the screen and then refuses it, so offering it would only
    /// cost a failed `TEST_ONLY` commit every frame the pointer sits at an
    /// edge (and `apple,dcp` logs a warning the first time).
    pub(super) fn back<R>(
        &mut self,
        renderer: &mut R,
        elements: &mut [Elements<R>],
        frame: (Scale<f64>, Size<i32, Physical>),
    ) where
        R: Renderer + ImportAll + ImportMem,
        R::TextureId: Texture + Send + Clone + 'static,
    {
        for slot in elements.iter_mut() {
            let Elements::Cursor(CursorElement::Fallback(element)) = &*slot else {
                continue;
            };
            let (scale, size) = frame;
            // Off-output first, before the image is built: every output's
            // frame carries the cursor element wherever the pointer is, and
            // building (allocating, uploading and caching a dma-buf twin)
            // for a cursor that is not on this output fills each idle
            // output's cache with images it never shows. Zero overlap skips;
            // any overlap falls through to the exact twin check after the
            // build. Costs one geometry call per cursor element per frame.
            if !overlaps(element.geometry(scale), size) {
                continue;
            }
            // The rounded origin is where the memory element draws (its
            // geometry's location is the location it was built at, rounded),
            // so the twin draws on the same pixels.
            let location = element.geometry(Scale::from(1.0)).loc.to_f64();
            let Some(image) = self.image(renderer, element) else {
                continue;
            };
            let inner = match MemoryRenderBufferRenderElement::from_buffer(
                renderer,
                location,
                &image.memory,
                None,
                None,
                None,
                Kind::Cursor,
            ) {
                Ok(inner) => inner,
                Err(error) => {
                    tracing::warn!(%error, "cursor plane: could not build the element");
                    continue;
                }
            };
            let (scale, size) = frame;
            if !fits(inner.geometry(scale), size) {
                continue;
            }
            let twin = PlaneCursorElement::new(inner, image.dmabuf.clone());
            *slot = Elements::Cursor(CursorElement::Plane(twin));
        }
    }

    /// The plane image for `element`'s buffer, built on first sight.
    fn image<R>(
        &mut self,
        renderer: &mut R,
        element: &MemoryRenderBufferRenderElement<R>,
    ) -> Option<&PlaneImage>
    where
        R: Renderer + ImportMem,
        R::TextureId: 'static,
    {
        let id = element.id();
        if !self.images.contains_key(id) {
            if self.images.len() >= MAX_IMAGES {
                // Starting over is enough: the images in use are rebuilt the
                // next frame they show. A dma-buf on screen right now stays
                // alive in the compositor's frame state until it is replaced.
                self.images.clear();
            }
            let image = build(&mut self.upload, renderer, element);
            if image.is_none() {
                tracing::debug!(?id, "cursor plane: this image stays composited");
            }
            self.images.insert(id.clone(), image);
        }
        self.images.get(id)?.as_ref()
    }

    /// How many images are cached, built or refused. Tests only.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.images.len()
    }

    /// Drops every cached image, built or refused. Called on a cursor config
    /// reload ([`Cursor::rebuild`]): the new images arrive under new ids, so
    /// without this the old ones sit in the cache until the bound turns it
    /// over -- up to [`MAX_IMAGES`] stale dma-bufs per output.
    pub(crate) fn clear(&mut self) {
        self.images.clear();
    }
}

/// Builds the plane image for one drawn element, or `None` for anything this
/// does not handle: not a memory buffer, not `Argb8888`, a buffer scale or
/// transform other than 1 and normal (the twin is built at scale 1, normal,
/// so anything else would draw differently), a size out of range, a failed
/// upload.
fn build<U, R>(
    upload: &mut U,
    renderer: &mut R,
    element: &MemoryRenderBufferRenderElement<R>,
) -> Option<PlaneImage>
where
    U: Upload,
    R: Renderer + ImportMem,
    R::TextureId: 'static,
{
    if element.transform() != Transform::Normal {
        return None;
    }
    let Some(UnderlyingStorage::Memory(memory)) = element.underlying_storage(renderer) else {
        return None;
    };
    if memory.format() != Fourcc::Argb8888 {
        return None;
    }
    let size = memory.size();
    // At scale 1 a buffer-scale-1 element is exactly its buffer's size.
    if element.geometry(Scale::from(1.0)).size != Size::from((size.w, size.h)) {
        return None;
    }
    let (width, height) = plane_size(size.w, size.h)?;
    let pixels = pad(
        &memory[..],
        memory.stride(),
        (size.w, size.h),
        (width, height),
    )?;
    let dmabuf = upload.upload(&pixels, width.unsigned_abs(), height.unsigned_abs())?;
    let memory = MemoryRenderBuffer::from_slice(
        &pixels,
        Fourcc::Argb8888,
        (width, height),
        1,
        Transform::Normal,
        None,
    );
    Some(PlaneImage { memory, dmabuf })
}

/// Whether a plane at `geometry` (the twin's, at the output's scale) keeps at
/// least [`MIN_SIDE`] both ways once clipped to an output of `output`
/// physical pixels. Saturating: the geometry follows the pointer, which the
/// input path clamps to the outputs, but nothing here relies on that.
fn fits(geometry: Rectangle<i32, Physical>, output: Size<i32, Physical>) -> bool {
    let visible = |start: i32, length: i32, limit: i32| {
        start
            .saturating_add(length)
            .min(limit)
            .saturating_sub(start.max(0))
    };
    visible(geometry.loc.x, geometry.size.w, output.w) >= MIN_SIDE
        && visible(geometry.loc.y, geometry.size.h, output.h) >= MIN_SIDE
}

/// Whether `geometry` (the source cursor's, at the output's scale) shows any
/// pixel on an output of `output` physical pixels: the pre-build gate in
/// [`CursorPlanes::back`]. Saturating, like [`fits`]: absurd positions from
/// a corrupt client must read as off-output, never panic.
fn overlaps(geometry: Rectangle<i32, Physical>, output: Size<i32, Physical>) -> bool {
    let visible = |start: i32, length: i32, limit: i32| {
        start
            .saturating_add(length)
            .min(limit)
            .saturating_sub(start.max(0))
    };
    visible(geometry.loc.x, geometry.size.w, output.w) > 0
        && visible(geometry.loc.y, geometry.size.h, output.h) > 0
}

/// The padded plane size for a `width` x `height` image: at least
/// [`MIN_SIDE`] both ways, and a width rounded up to [`PITCH_PIXELS`].
/// `None` for an empty or negative size, or one past [`MAX_SIDE`].
fn plane_size(width: i32, height: i32) -> Option<(i32, i32)> {
    if !(1..=MAX_SIDE).contains(&width) || !(1..=MAX_SIDE).contains(&height) {
        return None;
    }
    let width = width.max(MIN_SIDE);
    // Cannot overflow: `width <= MAX_SIDE`.
    let width = (width + PITCH_PIXELS - 1) / PITCH_PIXELS * PITCH_PIXELS;
    Some((width, height.max(MIN_SIDE)))
}

/// `source` (`stride` bytes a row, `size` pixels) copied into the top-left of
/// a transparent `padded` image, tightly packed. `None` if `source` is shorter
/// than `stride` and `size` say, or the sizes are inconsistent.
fn pad(source: &[u8], stride: i32, size: (i32, i32), padded: (i32, i32)) -> Option<Vec<u8>> {
    let (width, height) = (usize::try_from(size.0).ok()?, usize::try_from(size.1).ok()?);
    let (padded_width, padded_height) = (
        usize::try_from(padded.0).ok()?,
        usize::try_from(padded.1).ok()?,
    );
    if width > padded_width || height > padded_height {
        return None;
    }
    let stride = usize::try_from(stride).ok()?;
    let row = width.checked_mul(4)?;
    let padded_row = padded_width.checked_mul(4)?;
    if stride < row {
        return None;
    }
    let mut out = vec![0u8; padded_row.checked_mul(padded_height)?];
    for y in 0..height {
        let start = y.checked_mul(stride)?;
        let from = source.get(start..start.checked_add(row)?)?;
        let at = y * padded_row;
        out[at..at + row].copy_from_slice(from);
    }
    Some(out)
}
