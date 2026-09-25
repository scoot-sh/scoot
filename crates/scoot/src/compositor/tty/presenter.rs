//! How a rendered frame reaches one CRTC: the two presenter tiers behind one
//! enum, split out of `mod.rs` once `--tty` began driving one presenter per
//! connector (see `head.rs`). Nothing here changed in that split -- each
//! [`Head`](super::head::Head) owns one of these exactly as the whole
//! backend used to.

use smithay::backend::drm::DrmDevice;
use smithay::reexports::drm::control::{Mode, crtc};

use super::buffers::BufferPool;
use super::dumb::DumbPresenter;
#[cfg(feature = "gpu-scanout")]
use super::scanout;

/// How a rendered frame reaches the CRTC, and the one place the two tiers
/// are told apart.
///
/// The split is deliberately at the *presenter*, not at a flag: the
/// dumb-buffer tier's pool, per-slot ages and single-in-flight flip tracker
/// live inside [`DumbPresenter`] and are simply not reachable from the
/// scanout variant, and the scanout tier's swapchain and
/// `OutputDamageTracker` are not reachable from the dumb one. That is
/// stronger than "the GPU path does not call them": it is a type error to.
///
/// Both variants are boxed. Neither is small (a `BufferPool` with its two
/// mappings, a `DrmCompositor` with its swapchain and element tables), and
/// `Presenter` is built once at startup and never moved again -- so a pointer
/// each costs one allocation at startup and nothing per frame, while an
/// unboxed pair would make every `Tty` carry the larger of the two whichever
/// tier it is on.
pub(super) enum Presenter {
    /// CPU-composited frames memcpy'd into DRM dumb buffers -- the default,
    /// and the only tier that needs no GPU stack at all. See `dumb.rs`.
    Dumb(Box<DumbPresenter>),
    /// GLES-composited frames scanned out of a GBM swapchain by
    /// `DrmCompositor`, with no read-back at all. `--renderer gles` under
    /// `--tty`, only in a build carrying the `gpu-scanout` feature. Boxed
    /// because `DrmCompositor` is a large value and `Tty` is moved into
    /// `State` at startup. See `scanout.rs`.
    #[cfg(feature = "gpu-scanout")]
    Gpu(Box<scanout::ScanoutPresenter>),
}

impl Presenter {
    /// The CRTC being driven -- what a `DrmEvent::VBlank` is matched against,
    /// and whose gamma LUT `zwlr_gamma_control_v1` reports.
    pub(super) fn crtc(&self) -> crtc::Handle {
        match self {
            Self::Dumb(dumb) => dumb.crtc(),
            #[cfg(feature = "gpu-scanout")]
            Self::Gpu(gpu) => gpu.crtc(),
        }
    }

    /// The DRM surface being driven, for the hotplug path's connector and
    /// mode moves (`hotplug.rs`'s `set_pending`), which are identical on both
    /// tiers -- a surface is a surface.
    pub(super) fn surface(&self) -> &smithay::backend::drm::DrmSurface {
        match self {
            Self::Dumb(dumb) => dumb.surface(),
            #[cfg(feature = "gpu-scanout")]
            Self::Gpu(gpu) => gpu.surface(),
        }
    }

    /// Settles whatever flip was outstanding, returning whether a render
    /// should be re-triggered and the completed flip's number for the
    /// session-lock wait (see `session_lock.rs`).
    ///
    /// The two tiers answer the "which flip" question differently and that is
    /// the point: the dumb tier tracks its own single in-flight number
    /// (`flip_tracker.rs`), while `DrmCompositor` hands back the `user_data`
    /// the kernel paired with the frame that actually reached scan-out.
    pub(super) fn settle_flip(&mut self) -> (bool, Option<u64>) {
        match self {
            Self::Dumb(dumb) => dumb.flip_settled(),
            #[cfg(feature = "gpu-scanout")]
            Self::Gpu(gpu) => gpu.frame_submitted(),
        }
    }

    /// Takes whether the last frame was a refused commit owed a timer-driven
    /// retry (see `present_retry.rs`, shared by both tiers).
    pub(super) fn take_retry_render(&mut self) -> bool {
        match self {
            Self::Dumb(dumb) => dumb.take_retry_render(),
            #[cfg(feature = "gpu-scanout")]
            Self::Gpu(gpu) => gpu.take_retry_render(),
        }
    }

    /// The session has been paused (VT-switched away).
    ///
    /// The dumb tier drops the in-flight flip's number so a late vblank
    /// cannot match a lock wait recorded after it. The scanout tier
    /// deliberately does *not*: `DrmCompositor` pairs a completion with the
    /// frame it belongs to, so a vblank that really does arrive for a frame
    /// issued before the pause confirms exactly that frame -- which did reach
    /// the screen -- and one that never arrives is cleared by the drain in
    /// [`reactivate`](Self::reactivate) instead.
    ///
    /// It does release the explicit-sync buffers its in-flight frames hold,
    /// without waiting on anything: a frame whose vblank never arrives must
    /// not keep a client's buffers until the switch back (see
    /// `ScanoutPresenter::pause`).
    pub(super) fn pause(&mut self) {
        match self {
            Self::Dumb(dumb) => dumb.discard_flip(),
            #[cfg(feature = "gpu-scanout")]
            Self::Gpu(gpu) => gpu.pause(),
        }
    }

    /// The session has been reactivated and DRM master may be back: re-read
    /// the CRTC's state and throw away every assumption about what it is
    /// showing.
    pub(super) fn reactivate(&mut self) {
        match self {
            Self::Dumb(dumb) => {
                if let Err(error) = dumb.reset_state() {
                    tracing::warn!(%error, "could not reset drm surface state after reactivation");
                }
                dumb.invalidate_scanout();
            }
            #[cfg(feature = "gpu-scanout")]
            Self::Gpu(gpu) => gpu.reactivate(),
        }
    }

    /// Allocates whatever the *new* mode needs, before anything on the DRM
    /// side is told to change.
    ///
    /// Order is load-bearing on the dumb tier and the reason this is a
    /// separate step: a failed dumb-buffer allocation then leaves the display
    /// exactly as it was and working, whereas allocating after the modeset
    /// would leave the CRTC on a mode whose frames no buffer in the pool is
    /// the right size to hold -- which `present`'s size guard drops silently,
    /// forever. `None` when only the connector changed: the existing pool is
    /// already the right size.
    ///
    /// The scanout tier has nothing to pre-allocate. `DrmCompositor` resizes
    /// its own swapchain in [`adopt_mode`](Self::adopt_mode), which has to
    /// run *after* the surface has taken the new mode, not before -- so its
    /// failure ordering is the other way round and is handled there.
    pub(super) fn new_buffers(
        &self,
        drm: &DrmDevice,
        size_changed: bool,
        width: i32,
        height: i32,
    ) -> Result<Option<BufferPool>, ()> {
        match self {
            Self::Dumb(_) if size_changed => {
                match BufferPool::new(drm.device_fd(), width, height) {
                    Ok(buffers) => Ok(Some(buffers)),
                    Err(error) => {
                        tracing::warn!(
                            %error, width, height,
                            "drm: could not allocate scanout buffers for the new mode; \
                             staying on the current one"
                        );
                        Err(())
                    }
                }
            }
            Self::Dumb(_) => Ok(None),
            #[cfg(feature = "gpu-scanout")]
            Self::Gpu(_) => Ok(None),
        }
    }

    /// Installs a pool allocated by [`new_buffers`](Self::new_buffers).
    /// Always `None`, and so always a no-op, on the scanout tier.
    pub(super) fn adopt_buffers(&mut self, buffers: Option<BufferPool>) {
        match self {
            Self::Dumb(dumb) => dumb.adopt_buffers(buffers),
            #[cfg(feature = "gpu-scanout")]
            Self::Gpu(_) => debug_assert!(buffers.is_none()),
        }
    }

    /// Follows the surface onto a new mode, answering whether it took.
    ///
    /// Nothing to do on the dumb tier: the mode lives on the surface, which
    /// `hotplug.rs`'s `set_pending` has already moved, and the pool at the
    /// new size was installed by [`adopt_buffers`](Self::adopt_buffers). The
    /// scanout tier additionally has to resize its swapchain, which is what
    /// `DrmCompositor::use_mode` does.
    #[cfg_attr(not(feature = "gpu-scanout"), allow(unused_variables))]
    pub(super) fn adopt_mode(&mut self, mode: Mode) -> bool {
        match self {
            Self::Dumb(_) => true,
            #[cfg(feature = "gpu-scanout")]
            Self::Gpu(gpu) => gpu.use_mode(mode),
        }
    }

    /// Installs a surface built on a different CRTC, answering whether it
    /// took (see `hotplug.rs`'s `switch_crtc`).
    ///
    /// Infallible on the dumb tier -- a surface is a surface. The scanout
    /// tier has to rebuild its whole `DrmCompositor`, since that owns the
    /// surface by value, and that rebuild can fail; it builds the replacement
    /// before dropping the live one, so a refusal leaves what is on screen
    /// exactly as it was.
    pub(super) fn adopt_surface(&mut self, surface: smithay::backend::drm::DrmSurface) -> bool {
        match self {
            Self::Dumb(dumb) => {
                dumb.adopt_surface(surface);
                true
            }
            #[cfg(feature = "gpu-scanout")]
            Self::Gpu(gpu) => gpu.adopt_surface(surface),
        }
    }

    /// Starts the refused-commit streak over, because the CRTC underneath is
    /// new device state (see `present_retry.rs`).
    pub(super) fn reset_retries(&mut self) {
        match self {
            Self::Dumb(dumb) => dumb.reset_retries(),
            #[cfg(feature = "gpu-scanout")]
            Self::Gpu(_) => {
                // `ScanoutPresenter::invalidate_scanout`, which every caller
                // of this runs in the same breath, already does it -- the
                // swapchain and the streak are reset together there because
                // on that tier they are void for the same reason.
            }
        }
    }

    /// Whether a completion event is certainly still owed for a flip this
    /// presenter issued. Dumb tier: its single in-flight flip. Scanout tier:
    /// `false` -- `DrmCompositor` tracks its queue privately, so this cannot
    /// be answered with certainty there, and answering `true` when unsure
    /// could let the guard eat a live head's real vblank and freeze it. What
    /// that leaves on the scanout tier is the residual `scanout.rs`'s module
    /// doc already accepts: a stale completion reaching a new compositor
    /// with a frame pending settles that frame up to one vblank early (and
    /// one with nothing pending answers `None`, harmlessly).
    pub(super) fn flip_in_flight(&self) -> bool {
        match self {
            Self::Dumb(dumb) => dumb.flip_in_flight(),
            #[cfg(feature = "gpu-scanout")]
            Self::Gpu(_) => false,
        }
    }

    /// The renderer this tier composites with: GLES on the scanout tier,
    /// pixman on the dumb one.
    pub(super) fn renderer(&self) -> crate::cli::RendererKind {
        match self {
            Self::Dumb(_) => crate::cli::RendererKind::Pixman,
            #[cfg(feature = "gpu-scanout")]
            Self::Gpu(_) => crate::cli::RendererKind::Gles,
        }
    }

    /// Which tier this is, for the one startup log line that says so. A
    /// string rather than a bool because it is read by a person grepping a
    /// log, and "gpu"/"dumb" answers the question `scanout=false` only hints
    /// at.
    pub(super) fn tier(&self) -> &'static str {
        match self {
            Self::Dumb(_) => "dumb",
            #[cfg(feature = "gpu-scanout")]
            Self::Gpu(_) => "gpu",
        }
    }

    /// Points the scanout tier's `DrmCompositor` at this head's real
    /// `wl_output`, so a mode or scale change is followed without a second
    /// place having to mirror it. A no-op on the dumb tier, which reads the
    /// mode size off its [`Head`](super::head::Head) instead. See
    /// `tty::attach`, the one caller.
    pub(super) fn track_output(&mut self, output: &smithay::output::Output) {
        match self {
            Self::Dumb(_) => {}
            #[cfg(feature = "gpu-scanout")]
            Self::Gpu(gpu) => gpu.track_output(output),
        }
        let _ = output;
    }

    /// Everything about what the CRTC is showing is void -- a modeset, a
    /// connector move, or a display that came back after going away.
    pub(super) fn invalidate_scanout(&mut self) {
        match self {
            Self::Dumb(dumb) => dumb.invalidate_scanout(),
            #[cfg(feature = "gpu-scanout")]
            Self::Gpu(gpu) => gpu.invalidate_scanout(),
        }
    }
}
