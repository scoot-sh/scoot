//! The CPU renderer: pixman compositing into a main-memory image.
//!
//! The default behind [`Backend`](super::Backend), and the default forever --
//! GPU-free operation is a hard requirement (webtop, no-GPU boxes), not a
//! tier a GPU path supersedes. [`gles`](super::gles) is the opt-in
//! alternative; see `super`'s module doc for what the seam between them is
//! for.
//!
//! What lives here is only what is *specific to pixman*: the renderer and the
//! image it draws into. Damage tracking and the framebuffer's size are
//! renderer-agnostic and stay on [`Backend`](super::Backend), so a second
//! implementation inherits them instead of repeating them.

use std::error::Error;

use pixman::Image;
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::Offscreen;
use smithay::backend::renderer::pixman::PixmanRenderer;

use super::capture_cursor::PatchPool;
use super::{CaptureError, CaptureStage};

/// pixman, and the offscreen image it composites into.
///
/// The image is `Argb8888`, which is the same little-endian BGRA layout
/// `wl_shm`'s own `Argb8888` uses -- so the presenters' memcpy
/// (`nested.rs`, `tty/buffers.rs`) and the PNG path's swizzle
/// (`screenshot.rs`) can each say what they do about the bytes without a
/// conversion in between.
pub(super) struct PixmanBackend {
    pub(super) renderer: PixmanRenderer,
    pub(super) image: Image<'static, 'static>,
    /// What the capture path reuses between captures (see
    /// `capture_cursor::PatchPool`).
    pub(super) patch: PatchPool<PixmanRenderer, Image<'static, 'static>>,
}

impl PixmanBackend {
    /// Builds the renderer and an image of exactly `width` x `height`.
    ///
    /// Fallible in both halves and neither is theoretical: `PixmanRenderer::new`
    /// fails where pixman itself cannot be initialised, and `create_buffer`
    /// fails on an allocation this size. Callers treat a failure as "the
    /// render target is not there", never as something to substitute a
    /// different size for -- see `State::resize_output`.
    pub(super) fn new(width: i32, height: i32) -> Result<Self, Box<dyn Error>> {
        let mut renderer = PixmanRenderer::new()?;
        let image = renderer.create_buffer(Fourcc::Argb8888, (width, height).into())?;
        Ok(Self {
            renderer,
            image,
            patch: PatchPool::default(),
        })
    }
}

/// The framebuffer image's own bits as a byte slice, plus its stride in
/// bytes.
///
/// What both pixman read-backs -- the borrowed [`Backend::capture`](super::Backend::capture)
/// and the copying [`Backend::capture_into`](super::Backend::capture_into) --
/// read instead of compositing into a fresh image per call (what Smithay's
/// `ExportMem::copy_framebuffer` does at the pinned rev): the image *is* the
/// output's persistent framebuffer -- one per output, never per capture --
/// and a whole-image `Src` composite would be an identity copy of these same
/// bytes. The slice covers `stride * height` bytes; rows are `width * 4`
/// bytes each from each row start, `screencopy`'s stride derivation and
/// `capture_into`'s row loop both read it that way.
///
/// Checked rather than trusted, although the image is the backend's own: the
/// format must be the `A8R8G8B8` it was created with, the stride must cover
/// a row, and the image must be the size the backend recorded. Anything else
/// is a refusal, never an out-of-bounds read.
pub(super) fn framebuffer_bits<'a>(
    image: &'a Image<'static, 'static>,
    size: (i32, i32),
) -> Result<(&'a [u8], usize), CaptureError> {
    let (width, height) = (image.width(), image.height());
    let row = width * 4;
    let stride = image.stride();
    if !matches!(image.format(), pixman::FormatCode::A8R8G8B8)
        || stride < row
        || (width, height) != (size.0.max(0) as usize, size.1.max(0) as usize)
    {
        return Err(CaptureError::new(
            CaptureStage::Copy,
            format!("the framebuffer image is not {width}x{height} Argb8888 (stride {stride})"),
        ));
    }
    let len = stride.checked_mul(height).ok_or_else(|| {
        CaptureError::new(
            CaptureStage::Copy,
            format!("the framebuffer image is not {width}x{height} Argb8888 (stride {stride})"),
        )
    })?;
    // SAFETY: `data()` points at this image's own bits, `stride * height`
    // bytes allocated by pixman when the image was created and alive as long
    // as `image`, which the returned borrow cannot outlive. Nothing else
    // writes them while the borrow lives: captures run on the event-loop
    // thread, the frame's render finished before any capture starts, and
    // nothing renders concurrently. Each row read at both call sites is
    // `row <= stride` bytes from a row start below `len`, so every read is
    // inside the allocation.
    let bits = unsafe { image.data() }.cast::<u8>().cast_const();
    Ok((unsafe { std::slice::from_raw_parts(bits, len) }, stride))
}
