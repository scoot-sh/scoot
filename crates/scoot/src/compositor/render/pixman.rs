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
