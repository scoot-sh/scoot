//! The GPU renderer: GLES compositing into an offscreen renderbuffer.
//!
//! Opt-in and never the default -- `--renderer gles` or `[renderer] backend =
//! "gles"`, on `--headless` and `--nested` only (see
//! [`RendererKind`](crate::cli::RendererKind) and `render::resolve`). GPU-free
//! operation is a hard requirement (webtop, no-GPU boxes), so
//! [`pixman`](super::pixman) stays the default and the only pipeline that
//! needs no device at all.
//!
//! Like [`PixmanBackend`](super::pixman::PixmanBackend), what lives here is
//! only what is *specific to this renderer*: the renderer and the target it
//! draws into. Damage tracking and the framebuffer's size are
//! renderer-agnostic and stay on [`Backend`](super::Backend).
//!
//! # What this stage does and does not buy
//!
//! The frame still lands in main memory: it is composited into a
//! renderbuffer and read back with `ExportMem`, exactly as pixman's image is,
//! so every consumer downstream (`screenshot.rs`, `screencopy.rs`, both
//! presenters) is unchanged. Skipping that read-back -- scanning the GPU
//! buffer out directly through `DrmCompositor` -- is stage 3's work and only
//! applies to `--tty`. What this stage establishes is that the seam really
//! carries a second renderer and that the two draw the same pixels.
//!
//! # Which device it picks
//!
//! Hardware first, software last: [`EGLDevice::enumerate`] is sorted by
//! [`EGLDevice::is_software`] (a stable sort, so enumeration order decides
//! within each group), and the first device that yields a working renderer
//! wins. So on a real GPU the real GPU is used, and Mesa's
//! `EGL_MESA_device_software` device (llvmpipe) is the fallback for a box
//! that has none -- which is the only kind of box this project's dev VM has,
//! and the reason `--renderer gles` is testable there at all.
//!
//! Enumeration order is not trusted to be availability: a device can be
//! listed and still refuse a display, a context or a renderbuffer (a render
//! node the session may not open, a driver that will not give a GLES 2
//! context). Each candidate is therefore *tried*, with its failure logged,
//! before the next one -- and only if every one of them fails does this
//! report an error, naming each.
//!
//! Tying the choice to the DRM device `--tty` drives (`--gpu`/`[tty] gpu`, on
//! hardware where the display controller and the 3D device differ -- Apple
//! Silicon under Asahi being the motivating case) is stage 3's problem: a
//! renderer whose frames are read back to main memory does not have to live
//! on the scanout device, and one whose frames are scanned out directly does.

use std::error::Error;
use std::path::PathBuf;

use smithay::backend::allocator::Fourcc;
use smithay::backend::egl::{EGLContext, EGLDevice, EGLDisplay};
use smithay::backend::renderer::gles::{GlesRenderbuffer, GlesRenderer};
use smithay::backend::renderer::{Bind, Offscreen};

/// GLES, and the offscreen renderbuffer it composites into.
///
/// The renderbuffer is `Argb8888` for the same reason pixman's image is: it
/// is the little-endian BGRA layout every consumer of a read-back frame
/// already expects (see [`read_back`](super::read_back)), so the two
/// pipelines hand out bytes in one format, not two.
pub(super) struct GlesBackend {
    pub(super) renderer: GlesRenderer,
    pub(super) buffer: GlesRenderbuffer,
}

impl GlesBackend {
    /// Builds a GLES renderer and a renderbuffer of exactly `width` x
    /// `height`, on the best EGL device that will have it (see the module
    /// doc).
    ///
    /// Fails -- rather than quietly falling back to pixman -- when no device
    /// can: the caller asked for this renderer explicitly, and a session that
    /// silently drew with the other one would make every "verified under
    /// GLES" claim untrue. `compositor::run` turns that into a startup error
    /// naming the default that needs no GPU; `State::resize_output` treats it
    /// the same way it treats pixman failing, i.e. as "the render target is
    /// not there" (see that function's doc).
    pub(super) fn new(width: i32, height: i32) -> Result<Self, Box<dyn Error>> {
        let mut candidates: Vec<EGLDevice> = EGLDevice::enumerate()
            .map_err(|error| format!("could not enumerate EGL devices: {error}"))?
            .collect();
        if candidates.is_empty() {
            return Err("no EGL device is available for the GLES renderer".into());
        }
        candidates.sort_by_key(EGLDevice::is_software);

        let mut failures = Vec::with_capacity(candidates.len());
        for device in candidates {
            let name = describe(&device);
            let software = device.is_software();
            match build(device, width, height) {
                Ok(backend) => {
                    tracing::info!(
                        device = %name,
                        software,
                        width,
                        height,
                        "the GLES renderer is up"
                    );
                    return Ok(backend);
                }
                Err(error) => {
                    tracing::warn!(
                        device = %name,
                        software,
                        %error,
                        "this EGL device could not drive the GLES renderer; trying the next one"
                    );
                    failures.push(format!("{name}: {error}"));
                }
            }
        }
        Err(format!(
            "could not build the GLES renderer on any EGL device ({}); \
             --renderer pixman, the default, needs no GPU at all",
            failures.join("; ")
        )
        .into())
    }
}

/// One candidate device, all the way to a renderbuffer that really binds.
///
/// The trailing bind is not ceremony. A renderbuffer larger than the
/// driver's `GL_MAX_RENDERBUFFER_SIZE` is *created* without complaint and
/// only fails when it is attached to a framebuffer, which on the frame path
/// would be a per-frame "could not bind the framebuffer" warning and a black
/// screen rather than a startup failure. Binding once here turns that into
/// this device's error, so the next candidate is tried and, if none works, the
/// operator is told at startup. The target is dropped immediately; smithay
/// deletes the FBO with it, and the frame path binds its own.
fn build(device: EGLDevice, width: i32, height: i32) -> Result<GlesBackend, Box<dyn Error>> {
    // SAFETY: `EGLDisplay::new`'s contract is that nothing *else* in this
    // process calls `eglGetPlatformDisplay`/`eglTerminate` behind smithay's
    // back, so that smithay's own refcounting of displays stays truthful.
    // scoot only ever reaches EGL through smithay -- this function is the
    // one place in the compositor that opens a display at all.
    let display = unsafe { EGLDisplay::new(device) }?;
    let context = EGLContext::new(&display)?;
    // SAFETY: the context must not be current on another thread. It was
    // created on this thread one line ago and has never been handed
    // anywhere; `GlesRenderer` is neither `Send` nor `Sync` and lives in
    // `State`, which is single-threaded (see `state.rs`), so it cannot
    // become current on another thread later either.
    let mut renderer = unsafe { GlesRenderer::new(context) }?;
    let mut buffer = renderer.create_buffer(Fourcc::Argb8888, (width, height).into())?;
    renderer.bind(&mut buffer)?;
    Ok(GlesBackend { renderer, buffer })
}

/// A device's DRM node path for the log, or a stand-in when it has none --
/// which is exactly what Mesa's pure software device is: no node, no card.
///
/// Render node first, then the primary node, matching the order
/// `EGLDevice`'s own `EGLNativeDisplay::identifier` uses.
fn describe(device: &EGLDevice) -> String {
    device
        .render_device_path()
        .or_else(|_| device.drm_device_path())
        .as_ref()
        .map_or_else(
            |_| "no device node".to_owned(),
            |path: &PathBuf| path.display().to_string(),
        )
}
