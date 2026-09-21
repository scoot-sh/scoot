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
//! `EGL_MESA_device_software` device -- the one with no device node at all --
//! is the fallback for a box that has none.
//!
//! Worth knowing, because it is what the dev VM does and it looks like a
//! contradiction otherwise: `is_software()` is only `EGL_MESA_device_software`
//! being advertised, so a *device node* backed by a software driver answers
//! `false`. The VM's virtio-gpu render node (`/dev/dri/renderD128`) is picked
//! as "hardware" here and then served by Mesa's `kms_swrast`, i.e. llvmpipe.
//! That is the right outcome for this ordering -- it preferred a real device
//! node -- and it is why the numbers measured there are llvmpipe's, not a
//! GPU's.
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
use std::sync::atomic::{AtomicBool, Ordering};

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

/// Whether "the GLES renderer is up" has already been logged.
///
/// Process-global rather than a field, because the thing it describes is:
/// one compositor process runs one session, whose renderer is fixed for its
/// life (`State::renderer`), and the backend this guards is *replaced* on
/// every resize -- a field on it would reset with the rebuild it exists to
/// stay quiet about. Only ever `swap`ped to `true`, so `Relaxed` is enough:
/// nothing else is ordered against it, and the worst a race could do is log
/// the line twice at startup.
static FIRST_BUILD_LOGGED: AtomicBool = AtomicBool::new(false);

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
        // Before Smithay's first EGL touch (see `lib_loadable`): on a box
        // with no loadable libEGL that touch panics instead of failing.
        lib_loadable(LIB_EGL_SONAME).map_err(|cause| format!("{cause}{FALL_BACK_HINT}"))?;
        let mut candidates: Vec<EGLDevice> = EGLDevice::enumerate()
            .map_err(|error| format!("could not enumerate EGL devices: {error}{FALL_BACK_HINT}"))?
            .collect();
        if candidates.is_empty() {
            return Err(format!(
                "no EGL device is available for the GLES renderer{FALL_BACK_HINT}"
            )
            .into());
        }
        candidates.sort_by_key(EGLDevice::is_software);

        let mut failures = Vec::with_capacity(candidates.len());
        for device in candidates {
            let name = describe(&device);
            let software = device.is_software();
            match build(device, width, height) {
                Ok(backend) => {
                    // INFO once, DEBUG for every rebuild after -- the same
                    // once-per-session shape as `dmabuf.rs`'s first-import
                    // line, and for the same reason. `State::resize_output`
                    // comes back through here on every resize, which under
                    // `--nested` is now once per size the host configures the
                    // window to: a drag would otherwise emit this line at
                    // host frame rate, which is exactly the flood
                    // `docs/backlog/resolved/clean-disconnect-log-flood-done.md`
                    // is about. It would also be *wrong* after the first:
                    // "the GLES renderer is up" is news once, and the device
                    // it names cannot change within a session (see
                    // `State::renderer`).
                    if FIRST_BUILD_LOGGED.swap(true, Ordering::Relaxed) {
                        tracing::debug!(
                            device = %name,
                            software,
                            width,
                            height,
                            "rebuilt the GLES renderer at a new size"
                        );
                    } else {
                        tracing::info!(
                            device = %name,
                            software,
                            width,
                            height,
                            "the GLES renderer is up"
                        );
                    }
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
            "could not build the GLES renderer on any EGL device ({}){FALL_BACK_HINT}",
            failures.join("; ")
        )
        .into())
    }
}

/// Tacked onto every way [`GlesBackend::new`] can fail, because all three
/// reach the operator the same way: as `compositor::run`'s startup error, on
/// a session that has just refused to start. The one thing that is always
/// true and always actionable there is that the default renderer needs
/// nothing this one could not find.
const FALL_BACK_HINT: &str = "; --renderer pixman, the default, needs no GPU at all";

/// The `dlopen` soname Smithay loads libEGL under (see `lib_loadable`).
pub(super) const LIB_EGL_SONAME: &str = "libEGL.so.1";

/// Whether `soname` can be `dlopen`ed right now.
///
/// Smithay reaches libEGL through `libloading` behind a `LazyLock` whose
/// miss handler is `.expect("Failed to load LibEGL")` (`ffi.rs:148` at the
/// pinned rev) -- the only `Library::new(...).expect(...)` in either the EGL
/// or the GLES backend, checked in source. So on a box with no loadable
/// libEGL, the first EGL touch panics instead of returning the per-candidate
/// startup error this tier was designed to report (gh #177: the packaged
/// `--renderer gles` died exactly there, before device enumeration ever
/// ran). Both GLES tiers probe here first and report the designed error when
/// there is nothing to load. `catch_unwind` is not an alternative: the
/// workspace's release profile sets `panic = "abort"`, which turns that
/// panic into a process abort no handler can intercept.
///
/// Once per session startup at most (both callers run before the session
/// exists), so the one `CString` allocation is off every hot path by
/// construction. `libc` is already a direct dependency, so this adds none.
pub(super) fn lib_loadable(soname: &str) -> Result<(), Box<dyn Error>> {
    let name =
        std::ffi::CString::new(soname).map_err(|_| format!("{soname}: invalid library name"))?;
    // SAFETY: `dlopen`/`dlclose`/`dlerror` are plain C calls with no
    // Rust-side invariants to uphold; the `CString` outlives the call, the
    // handle is closed on this thread before returning, and the `dlerror`
    // text (if any) is copied out before any further dl* call could
    // overwrite it.
    unsafe {
        libc::dlerror(); // Clear any stale error an earlier call left behind.
        let handle = libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
        if handle.is_null() {
            let text = libc::dlerror();
            // `dlerror` may itself report nothing (or a non-UTF8 byte
            // string); either way the operator gets a loud startup error
            // naming the soname, never a backtrace.
            let detail = if text.is_null() {
                "unknown loader error".to_owned()
            } else {
                std::ffi::CStr::from_ptr(text)
                    .to_string_lossy()
                    .into_owned()
            };
            return Err(format!("could not load {soname}: {detail}").into());
        }
        libc::dlclose(handle);
    }
    Ok(())
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

/// The DRM **render node** `renderer`'s EGL display is on, or `None` when EGL
/// cannot name one.
///
/// What `zwp_linux_dmabuf_v1`'s `main_device` wants (see `dmabuf.rs`): the
/// device a client should allocate the buffers this renderer will import
/// against. Asked of the renderer's own display rather than guessed from a
/// path, because the guess is wrong exactly where it matters -- a machine with
/// two GPUs has more than one render node, and a client that allocates on the
/// other one hands over a dma-buf this renderer cannot import, which
/// `create_immed` turns into a disconnect.
///
/// `None` has three honest causes and no error among them: Mesa's pure
/// software device has no DRM node at all, a display may not carry
/// `EGL_EXT_device_query`, and a device may carry neither
/// `EGL_EXT_device_drm_render_node` nor `EGL_EXT_device_drm`.
/// [`try_get_render_node`](EGLDevice::try_get_render_node) already folds the
/// second DRM extension into the first and converts a primary node to its
/// render node, so this is one call rather than the ladder `describe` below
/// still needs for a log line. The caller falls back to the path ladder.
pub(super) fn render_node(renderer: &GlesRenderer) -> Option<libc::dev_t> {
    let display = renderer.egl_context().display();
    let device = EGLDevice::device_for_display(display)
        .inspect_err(|error| {
            tracing::debug!(%error, "this EGL display cannot name its device");
        })
        .ok()?;
    let node = device
        .try_get_render_node()
        .inspect_err(|error| {
            tracing::debug!(%error, "this EGL device cannot name a DRM render node");
        })
        .ok()??;
    Some(node.dev_id())
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The probe passes wherever the suite runs: like `render/tests.rs`'s
    /// GLES tests, this assumes a loadable libEGL on any machine running
    /// the suite (the dev VM serves llvmpipe; CI resolves a software EGL)
    /// -- and fails loudly rather than skipping where that breaks.
    #[test]
    fn libegl_loads_where_the_suite_runs() {
        assert!(lib_loadable(LIB_EGL_SONAME).is_ok());
    }

    /// The gh #177 shape: a box with no such library gets a loud `Err`
    /// naming the soname -- what both GLES tiers turn into the designed
    /// startup error -- and never a panic. A bogus soname keeps Smithay's
    /// own `LazyLock` untouched, so this runs safely alongside the GLES
    /// suites under both runners.
    #[test]
    fn a_missing_library_is_a_named_error_not_a_panic() {
        let missing = "libscoot-probe-no-such-library.so.1";
        let error = lib_loadable(missing).expect_err("a missing library must fail the probe");
        let text = error.to_string();
        assert!(
            text.contains("could not load"),
            "a loud load failure, not a backtrace: {text}"
        );
        assert!(text.contains(missing), "the error names the soname: {text}");
    }
}
