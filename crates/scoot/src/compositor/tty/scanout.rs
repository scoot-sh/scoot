//! The GPU scanout presenter: `DrmCompositor` over a GBM swapchain.
//!
//! The alternative to [`dumb`](super::dumb), and the reason this crate has a
//! `gpu-scanout` Cargo feature at all -- everything reachable from here needs
//! Smithay's `backend_gbm`, which is a link-time dependency on libgbm (see
//! the crate's `Cargo.toml`). Without the feature this module does not exist
//! and `--tty` has exactly the dumb-buffer tier it always had.
//!
//! # What this replaces rather than wraps
//!
//! `DrmCompositor` owns an allocator, a swapchain, buffer ages, page flip and
//! modeset, *and its own `OutputDamageTracker`*. So on this tier none of the
//! dumb tier's machinery is called at all -- not `buffers.rs`'s `BufferPool`
//! or its per-slot ages, not `flip_tracker.rs`, and not `Backend`'s own
//! damage tracker. After the presenter split they are not merely unused here
//! but unreachable: they live behind [`DumbPresenter`](super::dumb::DumbPresenter),
//! which this tier's [`Presenter`](super::Presenter) variant does not hold.
//!
//! One piece is deliberately *shared* rather than replaced, against the
//! letter of the staging note: [`PresentRetries`](super::present_retry::PresentRetries).
//! A refused `queue_frame` is the same hazard here as a refused `page_flip`
//! there -- nothing is in flight, so no completion event will ever retry the
//! frame, and the screen stays stale until unrelated damage arrives -- and
//! the counter is a pure bounded-retry arithmetic with no dumb-buffer
//! coupling whatsoever. Duplicating it would have been worse engineering than
//! reusing it.
//!
//! # Which flip is which
//!
//! `flip_tracker.rs`'s job here is done by `DrmCompositor` itself:
//! [`queue_frame`](DrmCompositor::queue_frame) takes a `user_data` and
//! [`frame_submitted`](DrmCompositor::frame_submitted) hands that exact value
//! back when the vblank for *that* frame arrives. So the session-lock blank
//! confirmation (`session_lock.rs`, and
//! `docs/backlog/resolved/session-lock-vblank-confirm-done.md`) rides on the
//! kernel's own pairing rather than on "at most one flip is out" -- which is
//! not true here, since `queue_frame` may queue behind a pending flip instead
//! of refusing it.
//!
//! The one residual corner is the same one the dumb tier accepts and
//! documents: a *stale* completion for a flip that was issued before a pause
//! or a modeset, arriving after a fresh frame is already pending, is handed
//! back as the fresh frame's number and can confirm a lock up to one vblank
//! early. It needs a discard, a re-present and a still-queued stale
//! completion all inside the lock window, and its worst case is exactly the
//! pre-vblank-confirmation behaviour rather than a new failure mode.
//! Confirming *late* is always safe (the fallback deadline bounds it);
//! confirming early is the one that matters, and one vblank is the bound.

use std::error::Error;
use std::rc::Rc;

use smithay::backend::allocator::Format as DrmFormat;
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::allocator::format::FormatSet;
use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice};
use smithay::backend::allocator::{Fourcc, Modifier};
use smithay::backend::drm::compositor::{DrmCompositor, FrameFlags, PrimaryPlaneElement};
use smithay::backend::drm::exporter::gbm::{GbmFramebufferExporter, NodeFilter};
use smithay::backend::drm::{DrmDeviceFd, DrmSurface, PlaneInfo, Planes};
use smithay::backend::renderer::element::{Id, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::utils::Buffer as ClientBuffer;
use smithay::backend::renderer::{Bind, Color32F, Renderer, Texture};
use smithay::output::{Output, OutputModeSource};
use smithay::reexports::drm::control::{Mode, crtc, plane};
use smithay::utils::{Buffer, Rectangle, Size, Transform};

use super::layout_exporter::{LayoutKeepingExporter, LostLayouts};
use super::present_retry::{self, PresentRetries};
use crate::compositor::dmabuf::scanout::FormatsKey;
use crate::compositor::drm_syncobj::ExplicitBuffers;
use crate::compositor::drm_syncobj::release_hold::ReleaseHold;
use crate::compositor::render::{CursorInFrame, Plane};

/// The concrete `DrmCompositor` this backend drives.
///
/// `u64` is the per-frame user data: the flip sequence number the
/// session-lock wait matches on (see this module's doc). `DrmDeviceFd` is the
/// cursor-plane device parameter, which is live only where the CRTC has a
/// cursor plane -- [`ScanoutPresenter::build`] passes `gbm: None` where it
/// has none, disabling the cursor plane outright, exactly the construction
/// this tier had before the cursor step existed.
type Compositor = DrmCompositor<GbmAllocator<DrmDeviceFd>, LayoutKeepingExporter, u64, DrmDeviceFd>;

/// The per-frame plane assignment for a frame whose primary may go direct:
/// every plane Smithay can drive -- cursor, overlay, and the primary plane
/// handed to a client buffer *whatever its format* (`ANY`).
///
/// Which frames get it is not this constant's decision: [`frame_flags`]
/// hands it only to a frame `render::primary_direct` judged eligible (a
/// fullscreen window covering the output, unlocked, nothing translucent or
/// rounded in the frame, no capture stream on the output) and not armed by
/// a capture ([`ForceComposite`]). Every other frame gets
/// [`COMPOSITE_FLAGS`], with no primary bit at all -- so `ANY` never reaches
/// an arbitrary bottom window, and not even the format-matching
/// `ALLOW_PRIMARY_PLANE_SCANOUT` does (it used to, on every frame, lock
/// frames included; see `docs/backlog/resolved/gpu-primary-direct-format-gate-done.md`).
///
/// # Why `ANY`, and why it is safe on an eligible frame
///
/// Traced at the pinned rev, `try_assign_primary_plane` has no element-kind
/// test; without `ANY` its gate is `slot.format() != element_config.properties.format`,
/// a whole-`Format` comparison (fourcc *and* modifier) between the swapchain
/// slot and the framebuffer the exporter made from the client buffer. The
/// primary path exports with `allow_opaque_fallback`, so the client
/// framebuffer is the opaque fourcc (`Xrgb8888`) while the swapchain is
/// `Argb8888` (the first entry of [`COLOR_FORMATS`]) -- unequal for every
/// buffer a client can send here. The modifier may differ as well on a
/// device that takes modifiers (the client's `LINEAR` against an implicit
/// swapchain), but not on the dev VM's virtio-gpu: it has no `IN_FORMATS`
/// and no `ADDFB2_MODIFIERS`, so the swapchain is `Invalid` (measured
/// `Testing Formats: [AR24, Invalid]`) and the client framebuffer is added
/// without a modifier and comes back `Invalid` too (Smithay's trace names it
/// `XR24`/`Invalid`). There only the fourcc differs.
///
/// So putting `Xrgb8888` first in `COLOR_FORMATS` might match on virtio --
/// untried: rendering into an `Xrgb8888` swapchain, `render::read_back`'s
/// ARGB assumption and the test commit were never exercised -- and would
/// still leave modifier-capable devices unmatched. What rules it out is
/// that it changes the format of *every* composited frame on every device
/// to serve the one frame shape that may go direct. `ANY` changes nothing
/// for a composited frame, so it is the lift taken.
///
/// Skipping the comparison does not hand KMS a buffer described wrongly:
///
/// - **The framebuffer carries its own fourcc, and the layout it is added
///   with is what GBM reports for the import** (`element_config` ->
///   `framebuffer_from_wayland_buffer` -> `framebuffer_from_dmabuf`); the
///   swapchain's format is never applied to it. Three shapes, traced at the
///   pinned rev:
///   - *Implicit* (`Invalid`): refused outright before any import (Weston's
///     rule), and `zwp_linux_dmabuf_v1` never offers it
///     (`dmabuf::driver_tranche`).
///   - *`LINEAR`, single-plane, offset 0* -- every buffer this path has been
///     seen with: Smithay imports it through GBM's **non**-modifier call and
///     forces the result implicit (`allocator/gbm.rs:355-381`,
///     `from_bo(bo, true)`), so it is `AddFB2`'d **without** a modifier on
///     every device, not only virtio, and comes back `Invalid`. KMS then
///     reads it in the driver's implicit layout for an imported buffer. That
///     is right exactly when that layout is linear for a linear allocation,
///     which holds on every driver this has run on (virtio measured) and on
///     drivers whose implicit layout is the buffer object's own metadata. It
///     is the one assumption the `LINEAR` path rests on, and it predates the
///     full-format feedback.
///   - *An explicit tiled or compressed modifier* (offered under GLES since
///     the feedback became the driver's own set): imported with modifiers,
///     and `AddFB2`'d with `DRM_MODE_FB_MODIFIERS` and whatever modifier GBM
///     reports. A device without `ADDFB2_MODIFIERS` refuses that call, and
///     client buffers get no legacy fallback, so the element composites.
///     **If GBM reports the modifier wrongly or not at all**, the framebuffer
///     would be added with that wrong layout, its format would read
///     `{fourcc, Invalid}` (or the wrong modifier) -- and `{fourcc, Invalid}`
///     is in every plane's list, because Smithay adds it for every plane
///     fourcc unconditionally (`drm/mod.rs:288-297`), so the plane check below
///     would *pass* and the screen would show scrambled tiles, not a
///     fallback. That is why the exporter is not Smithay's bare one:
///     [`LayoutKeepingExporter`] drops any client framebuffer that did not
///     keep the client's explicit modifier, and the element composites
///     (`tty/layout_exporter.rs`, rule pinned there). No driver is known to
///     misreport; `Asahi.md` Test 6 asks real hardware.
///
///   A multi-plane YUV buffer takes the modifier path and reaches the primary
///   only where the plane lists that fourcc (next bullet); Smithay treats it
///   as opaque (`has_alpha` knows no YUV fourcc), which is what it is. What
///   the comparison `ANY` skips protected is only "the primary shows the
///   same format it composites in", not "KMS reads the buffer right".
/// - **The plane still has to take that exact format.** `try_assign_plane`
///   refuses unless `plane.formats.contains(element format)`, fourcc and
///   modifier, before any commit is built -- `ANY` skips the swapchain
///   comparison, not the plane's own format list.
/// - **The atomic `TEST_ONLY` commit judges the rest** -- scaling, a source
///   crop, a buffer transform (Smithay refuses a non-`Normal` transform
///   outright on a plane without a `rotation` property), a destination
///   smaller than the CRTC. A refusal is cached per element and the element
///   composites; a frame-level test failure falls back the same way
///   (`test_state_complete`'s error arm).
/// - **The one real difference, alpha, cannot show on the primary.** The
///   opaque fallback means the display ignores the client's alpha channel.
///   Smithay only tries the primary for the *bottom* visible element, with
///   everything above it on its own plane, and only when that element is
///   opaque and covers the whole output *or* the clear colour is
///   black/transparent. Opaque, the alpha channel is 1 wherever it is shown;
///   over black, a premultiplied pixel composited over the clear colour *is*
///   its own RGB -- exactly what scanning it out ignoring alpha shows. The
///   frame-level eligibility also refuses any element with a sub-1.0
///   alpha, so no plane-alpha property is ever asked to stand in for it.
///
/// So what the ticket named as `ANY`'s risk -- a driver accepting a
/// mismatched format and showing it with the wrong alpha -- does not apply
/// to an opaque bottom element: the format is the buffer's own, and alpha
/// is not visible there.
///
/// # Overlay and cursor
///
/// The overlay bit's reachable effect is deliberately narrow. Smithay's
/// `try_assign_overlay_plane` only considers elements of kind
/// `ScanoutCandidate` or `Cursor`, and this tree constructs neither for any
/// window surface -- every surface element is built `Kind::Unspecified`
/// (`render/elements.rs`, both call sites), only cursor elements are
/// `Kind::Cursor`, and `Rounded` forwards its inner kind unchanged. So no
/// window can ride an overlay plane until something is marked a scanout
/// candidate (`docs/backlog/core/gpu-overlay-window-candidates.md`). What the bit
/// does today is let the *cursor* ride an overlay where a CRTC has overlays
/// but no cursor plane (the cursor plane is still tried first), which
/// captures reconcile like any plane-assigned cursor (`render::scanout`,
/// `render::capture_cursor`).
///
/// Pinned below, with [`COMPOSITE_FLAGS`] and [`frame_flags`]'s three rows.
const DIRECT_FLAGS: FrameFlags =
    FrameFlags::ALLOW_SCANOUT.union(FrameFlags::ALLOW_PRIMARY_PLANE_SCANOUT_ANY);

/// The flags for every frame that must land whole in the swapchain slot:
/// the cursor and overlay bits only, both primary bits out.
///
/// The frame a capture is about to read ([`ForceComposite`]) and every
/// frame `render::primary_direct` did not judge eligible -- a locked one,
/// one with no fullscreen window covering the output, one with a capture
/// stream running. The cursor may still ride its plane: the slot is then
/// recorded without it, and a capture that asked for the pointer re-renders
/// its region (`render::capture_cursor`) rather than this frame being made
/// to composite it.
const COMPOSITE_FLAGS: FrameFlags = composite_only(DIRECT_FLAGS);

/// The flags for one frame.
///
/// [`DIRECT_FLAGS`] only when the frame is eligible *and* not armed by a
/// capture; [`COMPOSITE_FLAGS`] otherwise. The arming always wins: a
/// capture is about to read the slot this frame draws into, so it must
/// draw into it whatever the eligibility says. Pure, so every row is
/// pinnable without a DRM device.
fn frame_flags(force_composite: bool, allow_primary_direct: bool) -> FrameFlags {
    if allow_primary_direct && !force_composite {
        DIRECT_FLAGS
    } else {
        COMPOSITE_FLAGS
    }
}

/// `flags` with every bit that lets a client buffer take the primary plane
/// removed.
///
/// *Both* bits: Smithay's `try_assign_primary_plane` proceeds when the flags
/// intersect either (`ALLOW_PRIMARY_PLANE_SCANOUT |
/// ALLOW_PRIMARY_PLANE_SCANOUT_ANY` at the pinned rev), so dropping only the
/// first would still let an `ANY` frame go direct -- the frame a capture is
/// about to read, silently off the swapchain.
const fn composite_only(flags: FrameFlags) -> FrameFlags {
    flags.difference(
        FrameFlags::ALLOW_PRIMARY_PLANE_SCANOUT.union(FrameFlags::ALLOW_PRIMARY_PLANE_SCANOUT_ANY),
    )
}

/// The capture fix's arming: one composite-only frame, bought by a capture
/// that is about to read the swapchain slot.
///
/// Its own type rather than a bare `bool` on the presenter so that the exact
/// code [`ScanoutPresenter::render_and_queue`] runs -- take the arming, turn
/// it into this frame's flags -- is pinnable without a DRM device (see this
/// module's tests and `render::scanout`'s capture-sequence test). Armed
/// only by `State::ensure_scanout_capture_current` (through
/// [`ScanoutPresenter::arm_force_composite`]); *taken*, never read, so one
/// arming buys exactly one composite frame however many frames look.
///
/// A frame that never reaches the presenter -- no `DrmCompositor`, a render
/// that headed elsewhere, a session holding no DRM master -- leaves it
/// armed, costing at most one composite frame later. That is the safe
/// direction to fail in: a spare composite frame is a little CPU, a missing
/// one is a stale capture.
#[derive(Debug, Default)]
pub(crate) struct ForceComposite {
    armed: bool,
}

impl ForceComposite {
    /// Arms the next frame to composite whole.
    pub(crate) fn arm(&mut self) {
        self.armed = true;
    }

    /// Takes the arming and answers this frame's flags: the composite-only
    /// set exactly once after [`arm`](Self::arm) whatever
    /// `allow_primary_direct` says, and otherwise the direct set if and only
    /// if the frame is eligible (see [`frame_flags`]).
    pub(crate) fn take_flags(&mut self, allow_primary_direct: bool) -> FrameFlags {
        frame_flags(std::mem::take(&mut self.armed), allow_primary_direct)
    }
}

/// Which client buffers the framebuffer exporter may turn into DRM
/// framebuffers for direct scanout: all of them, subject to the checks that
/// actually decide it (below).
///
/// `NodeFilter` is compared, at the pinned rev, against `Dmabuf::node()` --
/// and on this tree that is `None` for essentially every client buffer, so
/// `Node(..)` would admit nothing either. The node is only ever set on a
/// *client* dma-buf by the client's own `set_sampling_device` request
/// (`zwp_linux_buffer_params_v1`, since v6 -- `wayland/dmabuf/dispatch.rs`),
/// which the clients measured on this compositor do not send: quickshell
/// binds v5 and Mesa's EGL queues v4 (`dmabuf.rs`), neither of which even
/// has the request. The other writer, `MultiRenderer`'s import path, is not
/// on this tree: scoot imports through a single `GlesRenderer`, whose EGL
/// import never sets it (only EGL *export* does). `Node(render_node)` works
/// for Smithay's anvil precisely because anvil's `MultiRenderer` stamps
/// every imported buffer; copied here it would have been as inert as `None`.
/// And `DrmNode` equality includes the node *type*, so even a buffer that
/// did carry a hint would carry the render node `dmabuf.rs` advertises as
/// `main_device`, never the primary node the GBM device is opened on.
///
/// `All` is safe because the node was never what decided whether a buffer
/// can be scanned out -- the device is. Everything after this filter runs
/// against the scanout device itself and falls back to compositing on any
/// refusal, with no path back to the client:
///
/// - Only buffers the renderer already imported reach here at all (a
///   refused import is `failed`/`create_immed`-fatal at `dmabuf_imported`,
///   before any element exists), and on this tier the renderer's EGL display
///   was made on this very GBM device (`render::scanout::ScanoutBackend::new`).
/// - `framebuffer_from_wayland_buffer` refuses a buffer with no explicit
///   modifier (Weston's rule: an implicit layout is not safe to hand to KMS)
///   and returns no framebuffer for a non-dma-buf one (shm, single-pixel),
///   then `gbm_bo_import`s onto the scanout device and `AddFB2`s it.
/// - Any of those failing is an `Err` in `element_config`, cached per
///   element and buffer so it is not retried every frame, and the element
///   composites as it always has. Nothing on that path touches the client's
///   `wl_buffer`, sends it an event or posts an error: a refused framebuffer
///   cannot disconnect a client.
/// - A client framebuffer that did not keep the client's explicit tiled
///   modifier is dropped by the wrapping [`LayoutKeepingExporter`] and the
///   element composites (see `DIRECT_FLAGS` for why the plane check alone
///   would not catch it).
/// - A framebuffer that exists still has to pass the atomic `TEST_ONLY`
///   commit before any plane takes it (`try_assign_plane`).
///
/// This also settles the split render/display topology without a special
/// case (Apple Silicon: AGX's `renderD128` renders, `apple,dcp`'s `card2`
/// scans out, one GBM device on the display card serves allocator, exporter
/// and EGL -- `06-gpu-pipeline.md`). There `Backend::render_node()` can only
/// answer AGX's `renderD128` -- `card2` has no render node of its own, so
/// the GBM rung is `None`, and whether the EGL device or `dmabuf.rs`'s path
/// ladder answers, `renderD128` is the only render node on the machine --
/// which is not the device the exporter imports onto. With `All` that
/// mismatch is irrelevant: the import onto `card2` is what is tried, and
/// refused cleanly if the display cannot take the buffer.
///
/// What the widening reaches, stated so nobody has to re-derive it: a
/// covering fullscreen window's buffer on the primary plane (only on frames
/// [`DIRECT_FLAGS`] is handed to), no window on
/// an overlay (no element is `Kind::ScanoutCandidate`), and no change to the
/// cursor plane (it renders into buffers of its own through its own
/// exporter, `NodeFilter::None` inside Smithay). The one newly reachable
/// assignment is a *client cursor surface* whose buffer is a dma-buf riding
/// an overlay plane where the cursor plane could not take it. The worse
/// variant of that: where a CRTC has an overlay plane with a zpos *below*
/// the primary, Smithay may put an *opaque* element there as an underlay and
/// punch a transparent hole in the primary above it, so the swapchain slot a
/// capture reads holds a transparent cut-out where the cursor is. Captures
/// handle both: the frame records the footprint of every cursor element that
/// rode an overlay (`render_and_queue`'s `CursorInFrame::on_overlay`, read
/// off `overlay_elements`), and every capture re-renders that footprint --
/// with the cursor when it asked for the pointer, without it when it did not
/// (`render::capture_cursor`) -- so no capture shows the hole. It needs an
/// opaque dma-buf cursor surface and underlay-capable hardware, neither seen
/// here (virtio has no overlay plane; pinned with a synthetic record); see
/// `docs/backlog/resolved/capture-cursor-parity-done.md`.
const EXPORTER_FILTER: NodeFilter = NodeFilter::All;

/// Colour formats offered to `DrmCompositor::new`, in order. `Argb8888` first
/// because it is the format every read-back consumer in this compositor
/// already expects (see `render::read_back`); `Xrgb8888` is the same layout
/// with the alpha channel ignored, which some planes prefer.
const COLOR_FORMATS: [Fourcc; 2] = [Fourcc::Argb8888, Fourcc::Xrgb8888];

/// What one frame on this tier did, for `render::draw_frame_scanout` to turn
/// into a [`FrameOutcome`](crate::compositor::render::FrameOutcome).
pub(crate) struct ScanoutFrame {
    /// Whether a frame reached the renderer at all. Only such a frame may
    /// confirm a pending session lock.
    pub(crate) drew: bool,
    /// The flip this frame was queued on, if one was queued. `None` for a
    /// frame with no damage (nothing needed to go out, and the screen already
    /// shows this content) and for a refused commit.
    pub(crate) flip: Option<u64>,
    /// Whether this frame's content actually changed anything on screen --
    /// what `State::frame_serial` counts.
    pub(crate) damaged: bool,
    /// The element whose client buffer the primary plane scanned out
    /// directly on this frame, instead of a composite in the swapchain slot
    /// -- `None` when the frame composited. Two readers, one meaning ("the
    /// primary went direct, with this element"):
    ///
    /// - the capture recording: only a direct frame owes it a
    ///   `note_direct`, because the slot it would otherwise record was never
    ///   drawn into;
    /// - presentation feedback: that element's surface is the one whose
    ///   `wp_presentation_feedback.presented` carries `zero_copy`
    ///   (`presentation_time.rs`).
    ///
    /// Always `None` for an undamaged or failed frame (nothing reached any
    /// plane), and read by `render::draw_frame_scanout` right after this
    /// returns. An `Id` clone is a reference-count bump, not an allocation.
    pub(crate) primary_direct: Option<Id>,
}

/// `DrmCompositor`, plus what is needed to rebuild it on a different CRTC and
/// to bound a refusal streak.
pub(crate) struct ScanoutPresenter {
    compositor: Compositor,
    /// The GBM device the swapchain allocates from, the framebuffer exporter
    /// registers with, and the renderer's EGL display was made on. Kept so a
    /// CRTC switch can rebuild the compositor without re-deriving it.
    gbm: GbmDevice<DrmDeviceFd>,
    /// The renderer's importable dma-buf formats, as
    /// `DrmCompositor::new` intersects them with the plane's. Kept for the
    /// same reason as `gbm`.
    ///
    /// Not the same set as the one `zwp_linux_dmabuf_v1` advertises to
    /// clients, and it never reaches it: these are the formats the renderer
    /// can *render into* for scanout (`dmabuf_render_formats`), which
    /// `DrmCompositor::new` requires in order to pick a swapchain format at
    /// all. What a client is offered is what the renderer can *import*
    /// (`dmabuf_texture_formats`, external-only layouts included and
    /// implicit-modifier entries resolved), derived in `dmabuf.rs`.
    renderer_formats: Vec<DrmFormat>,
    /// Where the compositor reads the mode, scale and transform from. Kept
    /// so a CRTC switch can rebuild the compositor against the same source
    /// -- the real `wl_output` once [`track_output`](Self::track_output) has
    /// run, which is before any frame is drawn. Without this, rebuilding
    /// would have to be handed an `Output` from a call site that has no
    /// business holding one.
    mode_source: OutputModeSource,
    /// The next flip's sequence number. Monotonic for the process's life;
    /// `u64` wrap is not a correctness question (one flip per vblank would
    /// take hundreds of millions of years), the same stance `flip_tracker.rs`
    /// takes.
    next_flip: u64,
    /// How many consecutive commits the kernel has refused -- bounds the
    /// timer-driven retries (see `present_retry.rs`).
    retries: PresentRetries,
    /// Whether the last frame was a refused commit owed a timer-driven retry.
    /// Set only by [`arm_retry`](Self::arm_retry), and *taken* -- never read
    /// -- so it fires exactly once however many places look.
    ///
    /// Two takers here, unlike the dumb tier's one, and the difference is
    /// real rather than sloppy: a refusal from
    /// [`render_and_queue`](Self::render_and_queue) is owed to the render
    /// tail (`take_retry_render`), while a refusal from
    /// [`frame_submitted`](Self::frame_submitted) happens inside a completion
    /// event, where the render tail will not run again unless something asks
    /// for it -- so that one takes the flag itself and reports it as
    /// "re-render" to `drm_event`. Either way exactly one `request_render`
    /// results, and the bound in `present_retry.rs` counts both.
    retry_armed: bool,
    /// Whether the next frame that reaches the presenter must composite
    /// whole into the swapchain slot instead of allowing primary direct
    /// scanout. Armed only by [`arm_force_composite`](Self::arm_force_composite),
    /// taken by [`render_and_queue`](Self::render_and_queue) -- see
    /// [`ForceComposite`] for the one-arming-one-frame contract.
    ///
    /// Armed by `State::ensure_scanout_capture_current` just before the
    /// render whose pixels a capture is about to read -- together with an
    /// `invalidate_scanout` (which forces the full damage a static screen
    /// would otherwise draw nothing on) only when there is no recording to
    /// refresh, or when a plain forced frame recorded nothing.
    force_composite: ForceComposite,
    /// Whether the swapchain's slots have been freed since the render path
    /// last looked. Set by every path that frees slots -- the ones that
    /// rebuild or resize the swapchain, and also a failed `render_frame`,
    /// which resets the swapchain internally before returning its error --
    /// taken by `render::draw_frame_scanout`, which uses it to drop the
    /// dma-bufs it exported from those slots (see
    /// `render::scanout::ScanoutBackend::forget_slots`). One writer set, one
    /// taker, so the two sides cannot disagree about whether a cached export
    /// still names a live buffer.
    slots_dropped: bool,
    /// How many KMS cursor planes the compositor may assign the cursor to.
    /// Zero where the CRTC has none -- the graceful fallback, where the
    /// cursor stays composited into the primary plane exactly as before this
    /// step. Read once at startup for the log line that says which it is.
    cursor_planes: usize,
    /// How many KMS overlay planes the compositor may assign elements to.
    /// Zero where the CRTC has none -- the graceful fallback, byte-identical
    /// by construction: with no planes Smithay's overlay assignment exits
    /// before touching anything. Read alongside `cursor_planes` for the same
    /// startup log line. (Even where non-zero, no window element can be
    /// assigned to one -- see [`DIRECT_FLAGS`]'s overlay section -- so this
    /// count decides cursor-sized consequences only.)
    overlay_planes: usize,
    /// The DRM device's hardware cursor size, as passed to
    /// [`ScanoutPresenter::new`]. Kept so a CRTC switch rebuilds the
    /// compositor with the same bound: the size is a property of the device,
    /// not of the CRTC, so it cannot have changed under us -- but the thread
    /// from here to `build` has to carry *something*, and re-reading it from
    /// the device would hand a call site that has no business holding one a
    /// `DrmDevice`.
    cursor_size: Size<u32, Buffer>,
    /// The client modifiers this device's GBM has been seen to lose, as the
    /// framebuffer exporter recorded them (`layout_exporter.rs`). One record
    /// for the device, handed to every exporter [`build`](Self::build)
    /// makes -- startup's and every CRTC switch's -- because losing a
    /// modifier is a property of the device's GBM, not of a CRTC. Read by
    /// the scanout tranche (see [`scanout_formats`](Self::scanout_formats)).
    lost: Rc<LostLayouts>,
    /// Which plane set the compositor was built on: moved by every CRTC
    /// switch ([`adopt_surface`](Self::adopt_surface)), the one path that
    /// can change the primary plane and so its format list. Half of the key
    /// the scanout tranche's cache is rebuilt on ([`FormatsKey`]); a mode
    /// change keeps the plane, so it keeps this.
    plane_epoch: u64,
    /// Explicit-sync client buffers the composited frames still in flight
    /// sampled, kept until each frame is done with them on the GPU -- so a
    /// release point is never signalled while a queued frame may still read
    /// its buffer. Fed by [`render_and_queue`](Self::render_and_queue),
    /// drained by [`frame_submitted`](Self::frame_submitted) and by every
    /// path after which a held frame can no longer be trusted to flip. See
    /// `drm_syncobj/release_hold.rs`. Always empty in a session that never
    /// saw an explicit-sync commit.
    release_hold: ReleaseHold<ClientBuffer>,
}

/// What the scanout tranche is built from on this presenter's device:
/// see [`ScanoutPresenter::scanout_formats`].
pub(crate) struct ScanoutFormats<'a> {
    /// The primary plane's own format list, as Smithay read it
    /// (`{fourcc, Invalid}` for every fourcc, plus the explicit modifiers
    /// `IN_FORMATS` names where the device has them).
    pub(crate) primary: &'a FormatSet,
    /// The client modifiers the exporter has refused on this device.
    pub(crate) lost: Vec<Modifier>,
    /// The DRM device the plane belongs to -- the tranche's
    /// `target_device`. `None` if it cannot be `stat`ed.
    pub(crate) device: Option<libc::dev_t>,
}

impl ScanoutPresenter {
    /// Builds the scanout tier on `surface`, at `size`.
    ///
    /// `size` is the mode's own size: the mode source starts
    /// [`Static`](OutputModeSource::Static) because the `wl_output` does not
    /// exist yet at this point in startup (`tty::init` runs before
    /// `headless::init_named`). [`track_output`](Self::track_output) swaps it
    /// for the real one the moment there is one, and nothing renders in
    /// between.
    ///
    /// `cursor_size` is the DRM device's own hardware cursor size
    /// (`DrmDevice::cursor_size` at the call site): the buffer size bound
    /// Smithay renders the cursor plane into. It is only read where a cursor
    /// plane exists -- without one the value is stored and never consulted.
    pub(super) fn new(
        surface: DrmSurface,
        gbm: GbmDevice<DrmDeviceFd>,
        renderer_formats: Vec<DrmFormat>,
        cursor_size: Size<u32, Buffer>,
        size: (i32, i32),
    ) -> Result<Self, Box<dyn Error>> {
        let mode_source = OutputModeSource::Static {
            size: size.into(),
            scale: 1.0.into(),
            transform: Transform::Normal,
        };
        let planes = surface_planes(&surface);
        let cursor_planes = planes.cursor.len();
        let overlay_planes = planes.overlay.len();
        let lost = Rc::new(LostLayouts::default());
        let compositor = Self::build(
            &planes,
            surface,
            &gbm,
            &renderer_formats,
            cursor_size,
            mode_source.clone(),
            &lost,
        )?;
        Ok(Self {
            compositor,
            gbm,
            renderer_formats,
            mode_source,
            next_flip: 0,
            retries: PresentRetries::new(),
            retry_armed: false,
            force_composite: ForceComposite::default(),
            slots_dropped: false,
            cursor_planes,
            overlay_planes,
            cursor_size,
            lost,
            plane_epoch: 0,
            release_hold: ReleaseHold::default(),
        })
    }

    /// The one `DrmCompositor::new` call, shared by startup and by a CRTC
    /// switch so the two cannot configure it differently.
    ///
    /// The exporter admits client buffers ([`EXPORTER_FILTER`]): which node
    /// filter, why `All` rather than a node, and what that reaches are all
    /// on that constant. Which *frames* may then hand the primary plane to
    /// one of those buffers is decided per frame, not here -- see
    /// [`frame_flags`] and `render::primary_direct` -- and the capture
    /// consequence of a direct frame (the swapchain slot a capture reads was
    /// not drawn into) is covered by `render::scanout`'s mark and force.
    fn build(
        planes: &Planes,
        surface: DrmSurface,
        gbm: &GbmDevice<DrmDeviceFd>,
        renderer_formats: &[DrmFormat],
        cursor_size: Size<u32, Buffer>,
        mode_source: OutputModeSource,
        lost: &Rc<LostLayouts>,
    ) -> Result<Compositor, Box<dyn Error>> {
        // `RENDERING | SCANOUT`: the buffers are both drawn into by GLES and
        // handed to the CRTC, so they must satisfy both.
        let allocator = GbmAllocator::new(
            gbm.clone(),
            GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
        );
        // Shared by startup and every CRTC switch, so a rebuilt compositor
        // cannot admit a different set of client buffers than the first one
        // did. See `EXPORTER_FILTER` for the choice and what it reaches.
        // Wrapped: a client buffer whose tiled layout the framebuffer would
        // lose is refused rather than scanned out scrambled (see
        // `layout_exporter`).
        let exporter = LayoutKeepingExporter::new(
            GbmFramebufferExporter::new(gbm.clone(), EXPORTER_FILTER),
            Rc::clone(lost),
        );
        // `Some` only where a cursor plane exists to drive with it. Without
        // one the cursor state -- its pixman renderer, its `CURSOR | WRITE`
        // buffer pool -- would be allocated and then never consulted (Smithay
        // finds no plane to claim and composites the cursor as before), so
        // `None` keeps the no-cursor-plane construction behaving exactly as
        // the old `None` + `(64, 64)` one did: with `cursor_state` absent,
        // `try_assign_cursor_plane` returns before ever reading the stored
        // `cursor_size` (traced at the pinned rev), so the device's real size
        // sitting in that field is behaviorally inert -- the graceful
        // fallback is structural, not a flag something has to remember to
        // check per frame.
        let cursor_gbm = if planes.cursor.is_empty() {
            None
        } else {
            Some(gbm.clone())
        };
        let compositor = Compositor::new(
            mode_source,
            surface,
            Some(planes.clone()),
            allocator,
            exporter,
            COLOR_FORMATS,
            renderer_formats.to_vec(),
            cursor_size,
            cursor_gbm,
        )?;
        Ok(compositor)
    }

    /// Points the compositor's mode/scale/transform at the real `wl_output`,
    /// so a later mode change or scale change is followed without a second
    /// place having to remember to mirror it.
    ///
    /// Called once, by `headless::init_named`, immediately after the output
    /// is created -- which is the only moment at which this is both possible
    /// (the output exists) and still free (no frame has been drawn).
    pub(super) fn track_output(&mut self, output: &Output) {
        self.mode_source = output.into();
        self.compositor
            .set_output_mode_source(self.mode_source.clone());
    }

    /// The CRTC this presenter drives.
    pub(super) fn crtc(&self) -> crtc::Handle {
        self.compositor.crtc()
    }

    /// How many KMS cursor planes the cursor may ride on. Zero is the
    /// fallback -- the cursor stays in the primary plane -- and is read once
    /// at startup for the log line that says which it is.
    pub(super) fn cursor_planes(&self) -> usize {
        self.cursor_planes
    }

    /// How many KMS overlay planes elements may ride on. Zero is the
    /// fallback -- everything stays composited -- and is read alongside
    /// [`cursor_planes`](Self::cursor_planes) for the same startup log line.
    pub(super) fn overlay_planes(&self) -> usize {
        self.overlay_planes
    }

    /// The surface being driven, for the hotplug path's connector/mode moves.
    pub(super) fn surface(&self) -> &DrmSurface {
        self.compositor.surface()
    }

    /// What the per-surface scanout tranche is keyed on: the plane set and
    /// the lost-modifier record. Allocation-free and syscall-free -- two
    /// integer reads -- because `render::draw_frame_scanout` asks it every
    /// frame to decide whether the cached tranche is still current.
    pub(crate) fn scanout_formats_key(&self) -> FormatsKey {
        FormatsKey {
            planes: self.plane_epoch,
            lost: self.lost.generation(),
        }
    }

    /// Everything the scanout tranche is built from on this device. Only
    /// called when [`scanout_formats_key`](Self::scanout_formats_key) moved
    /// (startup, a CRTC switch, a newly lost modifier), never per frame: it
    /// copies the lost list and `fstat`s the device fd.
    pub(crate) fn scanout_formats(&self) -> ScanoutFormats<'_> {
        let surface = self.compositor.surface();
        ScanoutFormats {
            primary: &surface.plane_info().formats,
            lost: self.lost.modifiers(),
            device: surface.device_fd().dev_id().ok(),
        }
    }

    /// Composites `elements` into a swapchain slot -- or hands the primary
    /// plane to one of them direct -- and queues the result for scan-out.
    ///
    /// `on_frame` is handed the GBM buffer the frame landed in, and only when
    /// the frame actually drew something *into the swapchain*: a damaged
    /// frame whose primary went direct reports that in the returned
    /// [`ScanoutFrame::primary_direct`](ScanoutFrame) instead, so
    /// `render::draw_frame_scanout` can mark the capture recording rather
    /// than leaving it pointing at a slot that was never drawn into. A frame
    /// with no damage deliberately calls neither: the previous frame is still
    /// what is on screen, so the recorded buffer must stay the previous one.
    ///
    /// Which of the two it may be comes from [`frame_flags`]: a frame armed
    /// by [`arm_force_composite`](Self::arm_force_composite) composites
    /// whole, and so does every frame the caller did not judge eligible
    /// (`allow_primary_direct` false -- see `render::primary_direct`). Only an
    /// eligible, unarmed frame lets Smithay try the primary plane, and even
    /// then Smithay decides: it may still composite (an element above the
    /// candidate that no plane took, a failed `TEST_ONLY` commit, a buffer
    /// with no framebuffer), which is why the outcome is reported back
    /// rather than assumed from the flags: `allow_primary_direct` is what the
    /// frame *may* do, [`ScanoutFrame::primary_direct`](ScanoutFrame) what it
    /// *did*, and only the second may mark the capture recording.
    ///
    /// An empty frame is the *normal* no-damage case, not a failure: nothing
    /// is queued, no retry is armed and no warning is logged, exactly as the
    /// dumb tier simply never calls `present` when `render_output` reports no
    /// damage.
    ///
    /// `on_frame` is also handed what that slot holds of the cursor
    /// ([`CursorInFrame`]), read off the `DrmCompositor`'s own answer about
    /// which elements it put on a plane -- its cursor element and its
    /// overlay (and underlay) elements -- rather than guessed from which
    /// planes exist. `frame` is the output's `(scale, physical size)`, which
    /// the cursor elements' geometry is measured and clamped in.
    ///
    /// `explicit` names the client buffers whose latest commit carried
    /// explicit-sync points: a composited frame holds every one of them it
    /// sampled until the frame is done on the GPU (see the `release_hold`
    /// field). Empty -- one length check per frame -- in every session
    /// without an explicit-sync client.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render_and_queue<R, E>(
        &mut self,
        renderer: &mut R,
        elements: &[E],
        clear_color: Color32F,
        allow_primary_direct: bool,
        frame: (f64, (i32, i32)),
        explicit: &ExplicitBuffers,
        mut on_frame: impl FnMut(&smithay::backend::allocator::gbm::GbmBuffer, CursorInFrame),
    ) -> ScanoutFrame
    where
        R: Renderer + Bind<Dmabuf>,
        R::TextureId: Texture + 'static,
        E: RenderElement<R>,
    {
        let flags = self.force_composite.take_flags(allow_primary_direct);
        let result = match self
            .compositor
            .render_frame(renderer, elements, clear_color, flags)
        {
            Ok(result) => result,
            Err(error) => {
                // Smithay frees the swapchain on its way out of this
                // error: `render_frame`'s own `Err` arm calls
                // `self.swapchain.reset_buffers()` before returning
                // (`drm/compositor/mod.rs:2343` at the pinned rev), and
                // that sets every slot to `Default::default()`
                // (`allocator/swapchain.rs:234`), dropping the
                // `Arc<InternalSlot>`s the export pool is keyed on.
                //
                // So this path frees slots exactly like a rebuild does,
                // and has to say so. Without this the next `acquire()`
                // allocates identically-sized slots that the allocator
                // will happily place at the addresses just freed, a
                // cached export aliases one by pointer, and `note_frame`
                // serves a *stale* dma-buf -- so every IPC screenshot
                // and `ext-image-copy-capture-v1` frame shows an old
                // screen until something else rebuilds the swapchain.
                // For an agent driving this compositor that is acting on
                // a screen that is not there, which is worse than the
                // failed frame that caused it.
                self.slots_dropped = true;
                tracing::warn!(%error, "could not render the frame for scanout");
                return ScanoutFrame {
                    drew: false,
                    flip: None,
                    damaged: false,
                    primary_direct: None,
                };
            }
        };
        let damaged = !result.is_empty;
        // The capture contract's branch: a damaged frame lives either in the
        // swapchain slot (recorded for captures through `on_frame`) or on
        // the primary plane direct (reported so the recording is marked, not
        // left pointing at a slot this frame never drew into). An undamaged
        // frame is neither -- the screen still shows the previous frame, and
        // so must the recording.
        let primary_direct = match &result.primary_element {
            PrimaryPlaneElement::Element(element) if damaged => Some(element.id().clone()),
            _ => None,
        };
        if damaged && let PrimaryPlaneElement::Swapchain(element) = &result.primary_element {
            let (scale, size) = frame;
            // Which plane took an element, told apart because only an
            // overlay can leave a hole in the slot (an underlay's hole
            // punch -- see `CursorInFrame::on_overlay`).
            let on_plane = |id: &Id| {
                if result
                    .cursor_element
                    .is_some_and(|cursor| cursor.id() == id)
                {
                    Some(Plane::Cursor)
                } else if result
                    .overlay_elements
                    .iter()
                    .any(|overlay| overlay.id() == id)
                {
                    Some(Plane::Overlay)
                } else {
                    None
                }
            };
            let cursor = CursorInFrame::of(
                elements,
                scale.into(),
                Rectangle::from_size(size.into()),
                on_plane,
            );
            on_frame(element.buffer(), cursor);
        }
        // The composited frame's render fence, for the explicit-sync release
        // hold below: only a damaged frame whose primary is the swapchain
        // sampled client buffers into it. A direct frame composited nothing,
        // and its plane buffer is kept by `DrmCompositor` itself until the
        // frame after it is on screen.
        let composite_sync = match &result.primary_element {
            PrimaryPlaneElement::Swapchain(element) if damaged && !explicit.is_empty() => {
                Some(element.sync.clone())
            }
            _ => None,
        };
        // Dropped before `queue_frame`, per `RenderFrameResult`'s own doc:
        // holding it keeps a swapchain slot out of circulation.
        drop(result);

        if !damaged {
            // Nothing changed, so nothing goes out. The screen already shows
            // this content, so this is not a skipped frame owed a retry --
            // it is the same no-op the dumb tier gets by `render_output`
            // reporting no damage and `present` never being called.
            return ScanoutFrame {
                drew: true,
                flip: None,
                damaged: false,
                primary_direct: None,
            };
        }

        let flip = self.next_flip;
        if let Some(sync) = composite_sync {
            // Every explicit buffer in the frame, sampled or not: holding one
            // the damage tracker skipped costs it at most this frame's flip,
            // and asking Smithay which elements it drew would cost a lookup
            // per element per frame for nothing.
            self.release_hold.hold(
                flip,
                sync,
                elements
                    .iter()
                    .filter_map(|element| match element.underlying_storage(renderer) {
                        Some(UnderlyingStorage::Wayland(buffer)) if explicit.contains(buffer) => {
                            Some(buffer.clone())
                        }
                        _ => None,
                    }),
            );
        }
        match self.compositor.queue_frame(flip) {
            Ok(()) => {
                self.next_flip = self.next_flip.wrapping_add(1);
                self.retries.succeeded();
                ScanoutFrame {
                    drew: true,
                    flip: Some(flip),
                    damaged: true,
                    primary_direct,
                }
            }
            Err(error) => {
                tracing::warn!(%error, "drm: queueing the frame for scanout failed");
                // This frame will never be shown, and its number is reused by
                // the retry: release what it held now.
                self.release_hold.discard_frame(flip);
                self.arm_retry();
                ScanoutFrame {
                    drew: true,
                    flip: None,
                    damaged: true,
                    primary_direct,
                }
            }
        }
    }

    /// Settles the flip a vblank just completed, returning whether a render
    /// should be re-triggered and the completed flip's number for the
    /// session-lock wait.
    ///
    /// The number comes from the kernel's own pairing: it is whatever
    /// `queue_frame` was given for the frame that just reached scan-out. A
    /// vblank with nothing pending (a stale completion for a frame a reset
    /// already discarded) answers `None` and confirms nothing.
    ///
    /// An error here is not cosmetic: `frame_submitted` has already taken the
    /// pending frame, and it failed while submitting whatever was queued
    /// behind it -- so that frame, which may be the blanked one a lock is
    /// waiting on, is gone with no completion coming. Treated exactly like a
    /// refused commit: bounded timer-driven retry, and no number, so the lock
    /// falls back to its deadline rather than confirming against a frame that
    /// never scanned out.
    ///
    /// The completed flip also releases the explicit-sync buffers held for
    /// it and for every earlier frame (see the `release_hold` field). On an
    /// error the completed frame's number is lost with it, and that frame
    /// is on screen, so everything held is released after waiting out its
    /// render instead.
    pub(super) fn frame_submitted(&mut self) -> (bool, Option<u64>) {
        match self.compositor.frame_submitted() {
            Ok(flip) => {
                if let Some(flip) = flip {
                    self.release_hold.flip_completed(flip);
                }
                (false, flip)
            }
            Err(error) => {
                tracing::warn!(%error, "drm: a queued frame could not be submitted after the vblank");
                self.release_hold.release_all();
                self.arm_retry();
                (std::mem::take(&mut self.retry_armed), None)
            }
        }
    }

    /// Records a refused commit against the bounded retry budget. The
    /// caller's own `warn!` has already said what failed; this decides
    /// whether anything is owed a retry.
    fn arm_retry(&mut self) {
        match self.retries.failed() {
            present_retry::Retry::Arm => self.retry_armed = true,
            present_retry::Retry::GiveUp => tracing::warn!(
                "drm: scanout commits keep failing; leaving the screen as-is \
                 until new damage arrives"
            ),
            present_retry::Retry::Quiet => {}
        }
    }

    /// Takes whether the last frame was a refused commit owed a timer-driven
    /// retry. Read once per frame by the render tail.
    pub(crate) fn take_retry_render(&mut self) -> bool {
        std::mem::take(&mut self.retry_armed)
    }

    /// Arms one fully-composited frame: the next frame that reaches
    /// [`render_and_queue`](Self::render_and_queue) renders with both primary
    /// direct-scanout bits off whatever its eligibility, landing whole in the
    /// swapchain slot.
    ///
    /// Armed by `State::ensure_scanout_capture_current` just before the
    /// render whose pixels a capture is about to read (IPC `screenshot` and
    /// `ext-image-copy-capture-v1` both funnel through there), paired there
    /// with an [`invalidate_scanout`](Self::invalidate_scanout) only when the
    /// forced frame needs full damage (see `render::force_needs_reset`). The cursor may still ride its
    /// plane on a forced frame -- only the primary bits are dropped -- so the
    /// forced slot may lack the cursor, or (where the plane refused it this
    /// time) hold it. Either is recorded with the slot, and the capture path
    /// re-renders the cursor's region to whatever the capture asked for (see
    /// `render::capture_cursor`), so a forced frame can neither drop the
    /// pointer from a capture that wants it nor give one two.
    pub(crate) fn arm_force_composite(&mut self) {
        self.force_composite.arm();
    }

    /// Takes whether the swapchain's slots have been freed since the render
    /// path last looked (see the field's doc).
    pub(crate) fn take_slots_dropped(&mut self) -> bool {
        std::mem::take(&mut self.slots_dropped)
    }

    /// Re-reads the CRTC's state after a session reactivation, and clears any
    /// frame the pause left pending.
    ///
    /// `reset_state` alone is what Smithay's own `DrmOutputManager::activate`
    /// does, and it is not enough here. It sets `reset_pending` (so the next
    /// frame is a full commit rather than a page flip onto state another VT
    /// may have reconfigured) but it deliberately does **not** touch
    /// `pending_frame` -- and `queue_frame` only *submits* when
    /// `pending_frame` is `None`, queueing behind it otherwise. So a flip
    /// that was still in flight when the session paused, whose vblank the
    /// kernel then never delivers, would leave every subsequent frame queued
    /// and never submitted: a permanently black screen after a VT switch
    /// back, with no error anywhere. That is precisely the failure class
    /// `docs/roadmap/05b-vt-switch-eperm.md` is about, so the drain is not
    /// belt-and-braces.
    ///
    /// The drained frame's number is discarded rather than reported: its
    /// content never reached the screen, so it must not confirm a session
    /// lock. The wait stays, owned by its fallback deadline, until the
    /// post-reactivation render records a fresh flip.
    pub(super) fn reactivate(&mut self) {
        if let Err(error) = self.compositor.reset_state() {
            tracing::warn!(%error, "could not reset drm surface state after reactivation");
        }
        if let Err(error) = self.compositor.frame_submitted() {
            // debug!, not warn!: submitting a frame prepared before the pause
            // onto a CRTC that has just been reset is expected to fail, and
            // the failure is harmless -- `frame_submitted` has already
            // cleared the pending slot, which is the whole point of the call,
            // and `reactivate` unconditionally asks for a fresh render.
            tracing::debug!(%error, "drm: a pre-pause frame could not be submitted; discarding it");
        }
        // Already released by `pause`, unless the pause never reached us (a
        // reactivation without one); either way nothing held here will be
        // shown, and no GPU-fence wait belongs on the activate path.
        self.release_hold.discard_all();
        self.invalidate_scanout();
    }

    /// The session has been paused (VT-switched away): the frames in flight
    /// may never report their vblank, so the explicit-sync buffers they hold
    /// are released now rather than kept until the switch back -- a client
    /// rendering while switched away must not run out of buffers over frames
    /// it will never see. Without waiting on their renders: those frames are
    /// not going to be shown, and a blocking fence wait does not belong on
    /// the pause path (see `drm_syncobj/release_hold.rs`).
    pub(super) fn pause(&mut self) {
        self.release_hold.discard_all();
    }

    /// The scanout bookkeeping shared by reactivation and the hotplug paths --
    /// and by the capture fix's forced composite frame when that needs full
    /// damage, not just composite flags (an empty recording, or a plain
    /// forced frame that recorded nothing).
    ///
    /// `reset_buffers` drops every swapchain slot, which is both what makes
    /// the next frame a full redraw (there is no buffer age left to trust)
    /// and what obliges the render side to drop the dma-bufs it exported from
    /// those slots -- hence `slots_dropped`. The refusal streak is reset with
    /// them: a CRTC that has just been reconfigured is new device state, and
    /// the first transient refusal on it must arm a retry rather than answer
    /// `Quiet` off a streak it never earned.
    ///
    /// `pub(crate)` rather than `pub(super)`: the capture paths
    /// (`State::ensure_scanout_capture_current` in `render.rs`) invalidate
    /// from outside the `tty` tree, for the same full-redraw reason the
    /// reactivation path does.
    pub(crate) fn invalidate_scanout(&mut self) {
        self.compositor.reset_buffers();
        self.slots_dropped = true;
        self.retries = PresentRetries::new();
    }

    /// Moves the compositor onto a new mode: the surface's pending mode and
    /// the swapchain's size together.
    ///
    /// The hotplug path has already put the mode on the *surface* (see
    /// `hotplug.rs`'s `set_pending`, which also handles the connector/mode
    /// ordering dance a connector change needs). This repeats
    /// `surface.use_mode` -- one more `TEST_ONLY` commit on a path that runs
    /// when a cable moves -- because `DrmCompositor::use_mode` is the only
    /// thing that also resizes the swapchain, and re-deriving that here from
    /// its internals would be a copy of Smithay's business.
    pub(super) fn use_mode(&mut self, mode: Mode) -> bool {
        match self.compositor.use_mode(mode) {
            Ok(()) => {
                self.invalidate_scanout();
                true
            }
            Err(error) => {
                tracing::warn!(%error, "drm: the compositor would not take the new mode");
                false
            }
        }
    }

    /// Rebuilds the compositor on a surface belonging to a different CRTC,
    /// answering whether it took.
    ///
    /// `DrmCompositor` owns its `DrmSurface` by value, so following a
    /// connector onto another CRTC means a new compositor, not a mutated one.
    /// Built first and installed only on success, which is the same property
    /// `hotplug.rs`'s `switch_crtc` relies on for the dumb tier: a failed
    /// switch leaves the live compositor -- and what is on screen -- exactly
    /// as it was, and the candidate surface drops with the failure.
    pub(super) fn adopt_surface(&mut self, surface: DrmSurface) -> bool {
        let planes = surface_planes(&surface);
        match Self::build(
            &planes,
            surface,
            &self.gbm,
            &self.renderer_formats,
            self.cursor_size,
            self.mode_source.clone(),
            &self.lost,
        ) {
            Ok(compositor) => {
                // The old compositor's in-flight frames drop with it and will
                // never be shown.
                self.release_hold.discard_all();
                self.compositor = compositor;
                // A new CRTC may come with a different primary plane, and so
                // a different format list: the scanout tranche rebuilds.
                self.plane_epoch = self.plane_epoch.wrapping_add(1);
                // Another CRTC may or may not have cursor or overlay planes
                // of its own, so both counts are re-read from the fresh plane
                // set. The cursor *size* is a property of the device rather
                // than the CRTC and rides along unchanged in
                // `self.cursor_size`.
                let cursor_planes = planes.cursor.len();
                let overlay_planes = planes.overlay.len();
                if cursor_planes != self.cursor_planes || overlay_planes != self.overlay_planes {
                    // info!, same bar as the startup line: whether the cursor
                    // rides its own plane decides what a capture sees, so a
                    // CRTC switch changing the answer must say so. The
                    // overlay count rides along so the line stays the one
                    // place that says what every plane is doing.
                    tracing::info!(
                        from = self.cursor_planes,
                        to = cursor_planes,
                        overlay_from = self.overlay_planes,
                        overlay_to = overlay_planes,
                        "drm: scanout cursor planes changed on the new crtc"
                    );
                    self.cursor_planes = cursor_planes;
                    self.overlay_planes = overlay_planes;
                }
                // The old compositor's swapchain drops with it, so every
                // dma-buf the render side exported from it is stale.
                self.slots_dropped = true;
                self.retries = PresentRetries::new();
                true
            }
            Err(error) => {
                tracing::warn!(
                    %error,
                    "drm: could not build a scanout compositor on the other crtc"
                );
                false
            }
        }
    }
}

/// The planes `DrmCompositor` may use: this surface's own primary plane,
/// plus its cursor and overlay planes.
///
/// The primary filter is not decoration. `DrmSurface::planes()` reports every
/// primary plane the CRTC could use, and `DrmCompositor` assumes the primary
/// it is given is the one the surface commits against -- handing it a
/// different one would be a plane the surface never claimed.
///
/// The cursor and overlay lists are only as complete as the kernel lets them
/// be: on paravirtualized drivers the cursor plane is hidden until the
/// session sets `CURSOR_PLANE_HOTSPOT`, which `open_device` does on the
/// device fd before `DrmDevice::new` (see its comment -- the position
/// matters, since `AtomicDrmDevice::new` snapshots the plane list once at
/// construction) -- so the snapshot here already includes it. Overlay planes
/// need no such cap: `UNIVERSAL_PLANES` (which Smithay's `DrmDevice::new`
/// sets itself) is what exposes them, and no client capability gates them.
fn surface_planes(surface: &DrmSurface) -> Planes {
    select_planes(surface.planes(), surface.plane())
}

/// Picks the planes `DrmCompositor` may use out of a CRTC's full inventory.
///
/// The primary is narrowed to the one plane the surface commits against (see
/// [`surface_planes`]); the cursor and overlay lists ride along whole, not
/// narrowed like the primary. There is no per-plane choice to make here:
/// Smithay claims one plane per frame out of the handed-over lists and falls
/// back to compositing wherever none can be claimed (oversized element,
/// failed TEST, no free plane -- all traced at the pinned rev, none wedging
/// the frame), and `DrmCompositor::new` sorts the overlay list into
/// front-to-back order itself. Narrowing would only risk dropping a plane
/// Smithay could have used. What also makes the whole-list handover safe is
/// that construction cannot newly fail because of it: the swapchain format
/// search (`find_supported_format` at the pinned rev) reads the *primary*
/// plane's formats only, so an overlay-only format gap can refuse a frame's
/// assignment but never the compositor's construction.
///
/// A CRTC with no overlay planes selects an empty overlay list, which is the
/// graceful fallback: Smithay's overlay assignment exits before touching
/// anything, `build` behaves exactly as the old primary-plus-cursor one did,
/// and the frame is byte-identical.
///
/// Split out from [`surface_planes`] so the selection is pinnable against a
/// fake inventory: everything around it needs a live DRM fd.
fn select_planes(inventory: &Planes, primary: plane::Handle) -> Planes {
    let primary: Vec<PlaneInfo> = inventory
        .primary
        .iter()
        .filter(|plane| plane.handle == primary)
        .cloned()
        .collect();
    Planes {
        primary,
        cursor: inventory.cursor.clone(),
        overlay: inventory.overlay.clone(),
    }
}

#[cfg(test)]
mod tests {
    //! The plane selection, frame flags, capture-force arming and exporter
    //! filter, decided without a DRM device.
    //!
    //! These are the parts of this module that can be exercised off real
    //! hardware: `build` needs a live `DrmSurface`, and the per-frame
    //! claim-and-fallback inside Smithay is traced in the module docs rather
    //! than re-proven here. The capture side of the force path (the
    //! recording's mark, refusal and clearing) is pinned in
    //! `render::scanout`'s tests, which drive [`super::ForceComposite`]
    //! through the same sequence. The no-cursor-plane shape --
    //! an empty cursor list in, `gbm: None` out -- is covered live on any
    //! CRTC without one; the dev VM's virtio-gpu has one, so its run proves
    //! the other shape instead.

    use std::num::NonZeroU32;

    use smithay::backend::allocator::format::FormatSet;
    use smithay::reexports::drm::control::PlaneType;

    use super::*;

    /// The plane the kernel would call id `raw`, of `type_`. Real handles,
    /// built from the `NonZeroU32` the kernel identifies a plane by -- the
    /// same construction `tty::hotplug::tests` uses for connectors.
    fn fake_plane(raw: u32, type_: PlaneType) -> PlaneInfo {
        PlaneInfo {
            handle: plane::Handle::from(NonZeroU32::new(raw).expect("plane ids start at 1")),
            type_,
            zpos: None,
            formats: std::iter::empty::<DrmFormat>().collect::<FormatSet>(),
            size_hints: None,
        }
    }

    /// A whole CRTC inventory in the shape virtio-gpu reports: one primary
    /// it commits against, a second primary it does not, one cursor plane,
    /// one overlay plane.
    fn inventory() -> (Planes, plane::Handle) {
        let primary = fake_plane(33, PlaneType::Primary).handle;
        let inventory = Planes {
            primary: vec![
                fake_plane(31, PlaneType::Primary),
                PlaneInfo {
                    handle: primary,
                    ..fake_plane(33, PlaneType::Primary)
                },
            ],
            cursor: vec![fake_plane(34, PlaneType::Cursor)],
            overlay: vec![fake_plane(35, PlaneType::Overlay)],
        };
        (inventory, primary)
    }

    #[test]
    fn the_primary_is_the_surfaces_own_and_nothing_else() {
        // The pre-existing filter, kept: `DrmCompositor` assumes the primary
        // it is given is the one the surface commits against.
        let (inventory, primary) = inventory();
        let selected = select_planes(&inventory, primary);
        assert_eq!(selected.primary.len(), 1);
        assert_eq!(selected.primary[0].handle, primary);
    }

    #[test]
    fn cursor_planes_ride_along_whole() {
        // No per-plane choice is made here -- Smithay claims one per frame --
        // so the whole list passes through untouched.
        let (inventory, primary) = inventory();
        let selected = select_planes(&inventory, primary);
        assert_eq!(selected.cursor.len(), 1);
        assert_eq!(selected.cursor[0].handle, inventory.cursor[0].handle);
    }

    #[test]
    fn no_cursor_plane_is_the_fallback_shape() {
        // A CRTC with no cursor plane selects an empty cursor list, which is
        // what `build` turns into `gbm: None` -- the construction this tier
        // had before the cursor step, cursor composited into the primary.
        let (mut inventory, primary) = inventory();
        inventory.cursor.clear();
        let selected = select_planes(&inventory, primary);
        assert!(selected.cursor.is_empty());
        // The primary is unaffected by the missing cursor plane.
        assert_eq!(selected.primary.len(), 1);
    }

    #[test]
    fn overlay_planes_ride_along_whole() {
        // No per-plane choice is made here -- Smithay claims one per frame
        // and falls back to compositing where none can be claimed -- so the
        // whole list passes through untouched, exactly like the cursor list.
        let (inventory, primary) = inventory();
        let selected = select_planes(&inventory, primary);
        assert_eq!(selected.overlay.len(), 1);
        assert_eq!(selected.overlay[0].handle, inventory.overlay[0].handle);
    }

    #[test]
    fn no_overlay_plane_is_the_fallback_shape() {
        // A CRTC with no overlay plane selects an empty overlay list, which
        // is what keeps `build` behaving exactly as the old
        // primary-plus-cursor construction did: Smithay's overlay assignment
        // exits before touching anything, so the frame is byte-identical.
        let (mut inventory, primary) = inventory();
        inventory.overlay.clear();
        let selected = select_planes(&inventory, primary);
        assert!(selected.overlay.is_empty());
        // Neither the primary nor the cursor list is affected by the missing
        // overlay plane.
        assert_eq!(selected.primary.len(), 1);
        assert_eq!(selected.cursor.len(), 1);
    }

    #[test]
    fn mixed_crtc_fallback_each_list_independent() {
        // A CRTC with overlays but no cursor plane (and vice versa) selects
        // each list on its own: the missing kind falls back while the
        // present kind rides along. Per-CRTC enumeration is what makes a
        // mixed pair of CRTCs correct, one `adopt_surface` at a time.
        let (mut inventory, primary) = inventory();
        inventory.cursor.clear();
        let selected = select_planes(&inventory, primary);
        assert!(selected.cursor.is_empty());
        assert_eq!(selected.overlay.len(), 1);
        assert_eq!(selected.primary.len(), 1);
    }

    /// Both bits that let a client buffer take the primary plane.
    const PRIMARY_BITS: FrameFlags =
        FrameFlags::ALLOW_PRIMARY_PLANE_SCANOUT.union(FrameFlags::ALLOW_PRIMARY_PLANE_SCANOUT_ANY);

    #[test]
    fn an_eligible_frame_may_hand_the_primary_to_any_format() {
        // Row one of the contract: an eligible, unarmed frame carries every
        // plane bit Smithay has *and* `ANY`, which is what lets a client's
        // opaque-fallback (`XR24`) framebuffer take the primary from an
        // `AR24` swapchain (see `DIRECT_FLAGS`). Losing `ANY` puts the
        // format gate back -- no fullscreen window goes direct on any
        // machine measured; losing the cursor or overlay bit regresses the
        // plane steps.
        let direct = frame_flags(false, true);
        assert_eq!(direct, DIRECT_FLAGS);
        assert!(direct.contains(FrameFlags::ALLOW_SCANOUT));
        assert!(direct.contains(PRIMARY_BITS));
        assert!(direct.contains(FrameFlags::ALLOW_OVERLAY_PLANE_SCANOUT));
        assert!(direct.contains(FrameFlags::ALLOW_CURSOR_PLANE_SCANOUT));
        // Nothing beyond the plane bits: `SKIP_CURSOR_ONLY_UPDATES` is a
        // VRR policy this tier does not have.
        assert_eq!(
            direct,
            FrameFlags::ALLOW_SCANOUT | FrameFlags::ALLOW_PRIMARY_PLANE_SCANOUT_ANY
        );
    }

    #[test]
    fn an_ineligible_frame_carries_no_primary_bit_at_all() {
        // Row two: a frame not judged eligible (locked, no covering
        // fullscreen window, a capture stream, a translucent or rounded
        // element) gets neither primary bit -- not `ANY`, which would hand
        // an arbitrary bottom window (a rounded one, over a black
        // background) the primary, and not the format-matching bit either,
        // which is no safer on a device whose swapchain happens to match.
        // The cursor and overlay bits stay: the cursor still rides its
        // plane.
        let composite = frame_flags(false, false);
        assert_eq!(composite, COMPOSITE_FLAGS);
        assert!(!composite.intersects(PRIMARY_BITS));
        assert_eq!(
            composite,
            FrameFlags::ALLOW_CURSOR_PLANE_SCANOUT | FrameFlags::ALLOW_OVERLAY_PLANE_SCANOUT
        );
    }

    #[test]
    fn a_forced_capture_frame_composites_even_when_eligible() {
        // Row three, the capture fix's: the frame a capture is about to read
        // lands whole in the swapchain slot whatever its eligibility. A
        // forced frame keeping a primary bit would let the very frame the
        // capture reads go direct -- the silently-wrong-buffer harm the
        // force exists to close, and now reachable on every eligible frame.
        for eligible in [true, false] {
            let forced = frame_flags(true, eligible);
            assert_eq!(forced, COMPOSITE_FLAGS, "eligible = {eligible}");
            assert!(!forced.intersects(PRIMARY_BITS));
        }
    }

    #[test]
    fn composite_only_drops_both_primary_bits() {
        // Smithay tries the primary plane when the flags intersect *either*
        // primary bit, so a composite set that dropped only
        // `ALLOW_PRIMARY_PLANE_SCANOUT` would still go direct under `ANY`.
        let forced = composite_only(DIRECT_FLAGS);
        assert!(!forced.intersects(PRIMARY_BITS));
        assert_eq!(
            forced,
            FrameFlags::ALLOW_CURSOR_PLANE_SCANOUT | FrameFlags::ALLOW_OVERLAY_PLANE_SCANOUT
        );
    }

    #[test]
    fn one_arming_buys_exactly_one_composite_frame() {
        // What `render_and_queue` runs, frame by frame, on an eligible
        // output: an unarmed frame may go direct, the frame right after an
        // arming may not, and the one after that may again -- the arming is
        // spent, not sticky. Arming twice before a frame still buys one frame
        // (a capture and a screencopy tick both asking is one forced frame,
        // not two). An ineligible frame does not spend the arming's purpose
        // for it either way: it composites regardless.
        let mut force = ForceComposite::default();
        assert_eq!(force.take_flags(true), DIRECT_FLAGS);
        force.arm();
        force.arm();
        assert_eq!(force.take_flags(true), COMPOSITE_FLAGS);
        assert_eq!(force.take_flags(true), DIRECT_FLAGS);
        assert_eq!(force.take_flags(false), COMPOSITE_FLAGS);
    }

    #[test]
    fn the_exporter_admits_client_buffers_that_carry_no_node() {
        // The widening, pinned against the comparison Smithay actually
        // makes: `can_add_framebuffer` asks `import_node == dmabuf.node()`,
        // and a client dma-buf's node is `None` unless the client sent
        // `set_sampling_device` (v6), which the clients measured here do not.
        // `NodeFilter::None` (what this tier had) admits nothing, and a
        // `Node(..)` filter would admit nothing either for exactly those
        // buffers -- only `All` makes the exporter reachable at all.
        use smithay::backend::drm::DrmNode;
        let unhinted: Option<DrmNode> = None;
        assert_eq!(EXPORTER_FILTER, NodeFilter::All);
        assert!(EXPORTER_FILTER == unhinted);
        assert!(NodeFilter::None != unhinted);
    }
}
