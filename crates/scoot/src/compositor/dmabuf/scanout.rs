//! Per-surface dma-buf feedback with a **scanout tranche**: steering a
//! fullscreen client into a buffer layout the display can scan out directly.
//!
//! # Why this exists
//!
//! Under `--renderer gles` the default feedback is the driver's whole import
//! set, tiled and compressed layouts included ([`driver_tranche`](super::driver_tranche)).
//! A client picks from it by *render* preference, which on a real GPU is
//! commonly a layout the display cannot scan out -- so a fullscreen window
//! that could go primary-direct (`render::primary_direct`) composites
//! instead, for no reason the client could have known. `zwp_linux_dmabuf_v1`
//! (v4+) lets a compositor tell one *surface* more: a feedback whose first
//! tranche is flagged `scanout` and names the display device, listing what
//! the plane can show. Smithay's anvil does the same at the pinned rev
//! (`anvil/src/udev.rs`, `get_surface_dmabuf_feedback`).
//!
//! Only the GPU scanout tier (`--tty --renderer gles`, `gpu-scanout` build)
//! has a plane to describe, so only it builds one; every other backend and
//! tier is untouched.
//!
//! # What the scanout tranche holds ([`scanout_tranche`])
//!
//! The entries of the **advertised default table** the primary plane would
//! accept, and nothing else. Starting from the advertised table, not from
//! the renderer's raw set, is what keeps the module doc's promise with teeth
//! intact without re-arguing it: every pair here is already one the default
//! feedback offers every client, so it already imports -- a client that
//! allocates from the scanout tranche and then composites (the plane refused
//! it after all, a capture stream started, a notification popped up) is
//! imported exactly like any other. The per-surface feedback's format table
//! and main tranche are the default's, entry for entry (it is built from the
//! same [`DmabufFeedbackBuilder`], [`DefaultFeedback`]); the scanout tranche
//! only reorders preference. Smithay validates a buffer's fourcc against the
//! global's own table, which this cannot change.
//!
//! "Would accept" mirrors what the primary plane is actually asked, traced at
//! the pinned rev. The primary exports a client buffer with
//! `allow_opaque_fallback` (`try_assign_primary_plane` -> `element_config(..,
//! true)`), so the framebuffer's fourcc is the *opaque* variant
//! (`get_opaque`: `AR24` -> `XR24`), and the plane's list is checked for
//! `{opaque fourcc, framebuffer modifier}`. Which modifier the framebuffer
//! has depends on how the buffer is imported:
//!
//! - **An explicit tiled or compressed modifier** is imported with modifiers
//!   and added with that modifier, so the plane must list
//!   `{opaque, modifier}` -- which it only does where the device has
//!   `ADDFB2_MODIFIERS` and an `IN_FORMATS` property naming it
//!   (`drm/mod.rs`'s `plane_formats`). Minus any modifier this device's GBM
//!   has been seen to lose (`tty::layout_exporter::LostLayouts`):
//!   the layout exporter refuses those, so offering one could only ever
//!   composite.
//! - **`LINEAR`, single-plane** is imported *without* modifiers
//!   (`allocator/gbm.rs`, `import_to`'s non-modifier arm) and added without
//!   one, so its framebuffer is `{opaque, Invalid}` on every device -- and
//!   Smithay lists `{fourcc, Invalid}` for every fourcc of every plane.
//!   Where the plane names explicit modifiers for that fourcc, `LINEAR` is
//!   offered only if one of them is `LINEAR`: the plane has said what it
//!   takes, and a no-modifier framebuffer of a linear buffer on a plane that
//!   did not say `LINEAR` is a `TEST_ONLY` failure waiting to happen. Where
//!   it names none (no `IN_FORMATS` -- the dev VM's virtio-gpu), `LINEAR` is
//!   offered for a single-plane packed fourcc (`get_bpp` knows it): that is
//!   exactly the path PR #228 measured going primary-direct there, and the
//!   implicit-linear assumption it rests on is `tty/layout_exporter.rs`'s.
//! - **`LINEAR`, multi-plane** takes the modifier arm, so it needs an
//!   explicit `{opaque, LINEAR}` like any other explicit modifier.
//! - **`Modifier::Invalid`** is never offered. The default table never holds
//!   it (see [`driver_tranche`](super::driver_tranche)), and Smithay refuses
//!   to scan out an implicit-modifier client buffer anyway (Weston's rule,
//!   `framebuffer_from_wayland_buffer`).
//!
//! What the plane's list cannot say -- scaling, a crop, a size that does not
//! cover the output -- the atomic `TEST_ONLY` commit still judges, per
//! buffer. The tranche is a hint that makes going direct *possible*; it
//! never makes a buffer go direct that would not have.
//!
//! An empty tranche (a plane that lists nothing the renderer imports) means
//! no scanout feedback at all: nothing is sent, and every surface keeps the
//! default.
//!
//! # Who gets it, and when ([`ScanoutFeedback`])
//!
//! The **root surface of the fullscreen window covering the output**, while
//! `render::primary_direct` judges the output eligible -- the same judgement
//! that hands the frame the direct flag set, shared rather than re-derived.
//! That judgement includes whether Smithay would try the covering window's
//! own element for the primary at all (its rule 6, mirroring Smithay's
//! walk), so a client Smithay would never scan out -- an alpha buffer with
//! no opaque region over a grey background, or over any wallpaper -- is not
//! asked to reallocate into a scannable layout it could never use.
//!
//! **Known cost, accepted in review:** a window becomes eligible only once
//! its buffer spans the output, i.e. after the client has redrawn at the
//! fullscreen size, so a client that acts on the feedback reallocates twice
//! on entering fullscreen (once for the size, once for the layout) -- one
//! extra buffer set per fullscreen entry, never per frame.
//! Subsurfaces are not steered: the window's root is what a fullscreen GL
//! client or game renders into, and a subsurface cannot reach the primary
//! unless everything drawn over the root is already a plane of its own.
//!
//! Sent on a **change**, never per frame: Smithay's
//! [`SurfaceDmabufFeedbackState::set_feedback`] re-sends only when the
//! feedback differs from what the surface holds, and the tracker only calls
//! it when the target changes. A client reallocates on every feedback change
//! it acts on, so flapping would cost it real work:
//!
//! - **The covering window changes** (it leaves fullscreen, unmaps, another
//!   window covers, the output shows another workspace): the old window is
//!   reverted to the default feedback at once. It is not what the output
//!   shows any more, and it is usually about to be resized anyway.
//! - **The same window still covers, but the frame is not eligible** (the
//!   session locked, a capture stream started, a translucent element): it
//!   keeps the scanout feedback, and is reverted on the first frame drawn
//!   once that has lasted [`REVERT_HOLD`]. `Screencopy::streaming` counts one
//!   ext-image-copy request as streaming for a second, so a shell refreshing
//!   a thumbnail every few seconds would otherwise flap the client's
//!   allocations on every refresh. The check is frame-driven, with no timer
//!   of its own: an output that stops drawing (a still lock screen) keeps
//!   the scanout feedback until its next frame. Keeping a scannable layout
//!   longer is harmless either way: every pair in the tranche imports.
//! - **Something composited above the window** (an `overlay` notification,
//!   a popup) is not an eligibility rule -- `render::primary_direct` leaves
//!   it to Smithay, which simply composites that frame -- so it does not
//!   revert anything. It is transient by nature, and the window goes direct
//!   again the moment it is gone, in the layout this feedback asked for.
//!
//! A client that asks for surface feedback *after* its window went
//! fullscreen is answered with the scanout feedback straight away
//! (`DmabufHandler::new_surface_feedback`, which Smithay calls once per
//! surface, on its first request). A client that never asks, binds a
//! pre-feedback version, or ignores what it is told is unaffected: the
//! scanout feedback promises nothing the default does not.
//!
//! # Cost
//!
//! Built once per plane set, off the frame path's steady state: at the first
//! eligible frame, and again only when the key moves ([`FormatsKey`]: a CRTC
//! switch, or a modifier newly refused by the exporter). A mode change keeps
//! the plane and so the tranche. Per frame, the tracker compares one cached
//! key and one weak surface handle, and reads the clock only while it is
//! holding a transient revert -- no allocation.

use std::time::{Duration, Instant};

use smithay::backend::allocator::format::{FormatSet, get_bpp, get_opaque};
use smithay::backend::allocator::{Format, Fourcc, Modifier};
use smithay::reexports::wayland_protocols::wp::linux_dmabuf::zv1::server::zwp_linux_dmabuf_feedback_v1::TrancheFlags;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Resource, Weak};
use smithay::wayland::compositor::with_states;
use smithay::wayland::dmabuf::{DmabufFeedback, SurfaceDmabufFeedbackState};

use scoot_core::OutputId;

use super::DefaultFeedback;
use crate::compositor::State;
use crate::compositor::tty::scanout::ScanoutFormats;

#[cfg(test)]
mod tests;

/// How long the window covering an output may stay ineligible for a
/// transient reason -- locked, streamed, translucent -- before the next
/// drawn frame steers it back to the default feedback. Longer than
/// `Screencopy`'s one-second stream window, so a single capture request
/// never reverts anything.
pub(crate) const REVERT_HOLD: Duration = Duration::from_secs(2);

/// The versions of `zwp_linux_dmabuf_feedback_v1` the scanout tranche is
/// sent to: every version that has feedback objects at all (v4 onward;
/// Smithay advertises the global at 6). Anvil's range.
const SCANOUT_TRANCHE_VERSIONS: std::ops::RangeInclusive<u32> = 4..=6;

/// Which plane set and lost-modifier record a scanout tranche was built
/// from. The cache is rebuilt when this moves -- see
/// `ScanoutPresenter::scanout_formats_key`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FormatsKey {
    /// Moved by every CRTC switch.
    pub(crate) planes: u64,
    /// Moved by every modifier the exporter newly refuses.
    pub(crate) lost: u64,
}

/// The scanout tranche: every entry of `advertised` (the default table, in
/// its order) that the primary plane, whose format list is `plane`, would
/// take as a client framebuffer -- minus any explicit modifier in `lost`.
/// See the module doc for the rule, entry by entry.
///
/// Pure, and built only when the plane set changes, so the quadratic scan
/// of `plane` in the `LINEAR` arm (a few dozen fourccs, a few modifiers
/// each) never reaches a frame.
pub(crate) fn scanout_tranche(
    advertised: &[Format],
    plane: &FormatSet,
    lost: &[Modifier],
) -> Vec<Format> {
    advertised
        .iter()
        .copied()
        .filter(|format| plane_takes(*format, plane, lost))
        .collect()
}

/// Whether a client buffer of `format` would pass the primary plane's
/// format check, per the module doc.
fn plane_takes(format: Format, plane: &FormatSet, lost: &[Modifier]) -> bool {
    // What the framebuffer's fourcc is: the primary exports with the opaque
    // fallback, so an alpha format is added as its opaque twin.
    let code = get_opaque(format.code).unwrap_or(format.code);
    match format.modifier {
        Modifier::Invalid => false,
        Modifier::Linear => {
            if plane.contains(&Format {
                code,
                modifier: Modifier::Linear,
            }) {
                return true;
            }
            // No explicit answer for this fourcc at all (no `IN_FORMATS`):
            // only the no-modifier import of a single-plane packed buffer,
            // whose framebuffer is `{code, Invalid}`, can pass.
            single_plane(format.code)
                && plane.contains(&Format {
                    code,
                    modifier: Modifier::Invalid,
                })
                && !plane
                    .iter()
                    .any(|listed| listed.code == code && listed.modifier != Modifier::Invalid)
        }
        explicit => {
            !lost.contains(&explicit)
                && plane.contains(&Format {
                    code,
                    modifier: explicit,
                })
        }
    }
}

/// Whether `code` is a single-plane packed format -- one Smithay imports into
/// GBM without modifiers when it is `LINEAR` at offset 0.
///
/// Smithay's own format table (`get_bpp`) lists the packed RGB formats plus
/// two planar YUV 4:2:0 ones, `Yuv420` and `Nv12`, at 12 bits per pixel; the
/// planar ones are exactly those whose pixel is not a whole number of bytes.
/// A fourcc the table does not know (`YUYV`, `P010`, ...) answers `false`,
/// which only ever withholds a `LINEAR` offer on a plane with no
/// `IN_FORMATS` -- never makes one. Pinned in the tests.
fn single_plane(code: Fourcc) -> bool {
    get_bpp(code).is_some_and(|bits| bits % 8 == 0)
}

/// Builds the per-surface scanout feedback: `default`'s own builder with the
/// scanout tranche in front of its main tranche, `target_device` naming
/// `device` (the display device the plane belongs to), flagged `scanout`.
///
/// `None` -- and nothing is ever sent -- when the tranche is empty, when the
/// device could not be named, or when the format-table memfd could not be
/// created; each is logged here, once per build. Built only when the plane
/// set changes (see the module doc's cost section).
pub(crate) fn build(
    default: &DefaultFeedback,
    plane: &FormatSet,
    lost: &[Modifier],
    device: Option<libc::dev_t>,
) -> Option<DmabufFeedback> {
    let tranche = scanout_tranche(&default.formats, plane, lost);
    if tranche.is_empty() {
        tracing::info!(
            "dmabuf feedback: the primary plane takes none of the advertised formats; \
             no scanout tranche, every surface keeps the default feedback"
        );
        return None;
    }
    let Some(device) = device else {
        tracing::warn!(
            "dmabuf feedback: the display device could not be named; no scanout tranche"
        );
        return None;
    };
    // `info!` once per plane set: the line a "why does my fullscreen game
    // composite" report needs, with the table itself at `debug`.
    tracing::info!(
        pairs = tranche.len(),
        lost = lost.len(),
        device,
        "dmabuf feedback: scanout tranche for fullscreen windows"
    );
    tracing::debug!(table = ?tranche, "dmabuf scanout tranche");
    match default
        .builder
        .clone()
        .add_preference_tranche(
            device,
            TrancheFlags::Scanout,
            tranche,
            SCANOUT_TRANCHE_VERSIONS,
        )
        .build()
    {
        Ok(feedback) => Some(feedback),
        Err(error) => {
            tracing::warn!(%error, "dmabuf scanout feedback could not be built; not steering");
            None
        }
    }
}

/// What one [`ScanoutFeedback::steer`] call did. Returned so the frame path
/// can log a transition and the tests can pin which arm ran; never stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Steer {
    /// Nothing to do: no target and none wanted, or no scanout feedback to
    /// steer with.
    Idle,
    /// The target already holds the scanout feedback and still should.
    Kept,
    /// A new target was sent the scanout feedback (and any previous one
    /// reverted).
    Sent,
    /// The target still covers the output but is not eligible; it keeps the
    /// scanout feedback until [`REVERT_HOLD`] has passed.
    Holding,
    /// The target was sent the default feedback back.
    Reverted,
}

/// The scanout feedback one output's covering window is steered with, and
/// which surface currently holds it. See the module doc.
#[derive(Default)]
pub(crate) struct ScanoutFeedback {
    /// The feedback built for the current plane set, and the key it was
    /// built for. `feedback` is `None` when nothing can be steered (see
    /// [`build`]); that is cached too, so a failure is not retried per frame.
    built: Option<(FormatsKey, Option<DmabufFeedback>)>,
    /// The surface the scanout feedback was last sent to -- the covering
    /// window's root surface. Weak: a surface destroyed, or a client gone,
    /// while it holds the feedback is simply forgotten.
    target: Option<Weak<WlSurface>>,
    /// When the target, still covering, was first seen ineligible. `None`
    /// while it is eligible or there is no target.
    held_since: Option<Instant>,
}

impl ScanoutFeedback {
    /// Whether the cached feedback was built for anything but `key`, i.e.
    /// whether the frame path must call [`install`](Self::install) before
    /// steering. One comparison, per frame.
    pub(crate) fn needs_build(&self, key: FormatsKey) -> bool {
        self.built.as_ref().is_none_or(|(built, _)| *built != key)
    }

    /// Caches `feedback` as the scanout feedback for `key`, and moves a
    /// current target onto it: the rebuilt one if there is one (Smithay
    /// re-sends only if it differs), the default if the new plane set left
    /// nothing to steer with.
    pub(crate) fn install(
        &mut self,
        key: FormatsKey,
        feedback: Option<DmabufFeedback>,
        default: Option<&DmabufFeedback>,
    ) {
        match (&feedback, self.live_target()) {
            (Some(scanout), Some(surface)) => send(&surface, scanout),
            (None, Some(surface)) => {
                if let Some(default) = default {
                    send(&surface, default);
                }
                self.target = None;
                self.held_since = None;
            }
            (_, None) => {}
        }
        self.built = Some((key, feedback));
    }

    /// One frame's steering: `covering` is the root surface of the window
    /// covering the output (if any), `eligible` whether
    /// `render::primary_direct` judged this frame eligible, `default` the
    /// feedback to revert to. `now` is read only while a transient revert is
    /// being held. See the module doc for the rules.
    pub(crate) fn steer(
        &mut self,
        covering: Option<&WlSurface>,
        eligible: bool,
        default: Option<&DmabufFeedback>,
        now: impl FnOnce() -> Instant,
    ) -> Steer {
        if self
            .target
            .as_ref()
            .is_some_and(|target| !target.is_alive())
        {
            // Destroyed, or its client gone: nothing to revert, nothing to
            // send to.
            self.target = None;
            self.held_since = None;
        }
        let Some(scanout) = self
            .built
            .as_ref()
            .and_then(|(_, feedback)| feedback.as_ref())
        else {
            return Steer::Idle;
        };
        let holds = |surface: &WlSurface| self.target.as_ref().is_some_and(|t| t == surface);
        match covering {
            Some(surface) if eligible => {
                if holds(surface) {
                    self.held_since = None;
                    return Steer::Kept;
                }
                let scanout = scanout.clone();
                self.revert(default);
                send(surface, &scanout);
                self.target = Some(surface.downgrade());
                Steer::Sent
            }
            Some(surface) if holds(surface) => {
                let now = now();
                let since = *self.held_since.get_or_insert(now);
                if now.saturating_duration_since(since) < REVERT_HOLD {
                    return Steer::Holding;
                }
                self.revert(default);
                Steer::Reverted
            }
            _ => {
                if self.target.is_none() {
                    return Steer::Idle;
                }
                self.revert(default);
                Steer::Reverted
            }
        }
    }

    /// The scanout feedback, if `surface` is the one currently steered with
    /// it -- what a surface asking for feedback for the first time is
    /// answered with (`DmabufHandler::new_surface_feedback`).
    pub(crate) fn for_new_surface(&self, surface: &WlSurface) -> Option<DmabufFeedback> {
        if !self.target.as_ref().is_some_and(|target| target == surface) {
            return None;
        }
        self.built
            .as_ref()
            .and_then(|(_, feedback)| feedback.clone())
    }

    /// Sends the current target (if it is still alive) the default feedback
    /// and forgets it.
    fn revert(&mut self, default: Option<&DmabufFeedback>) {
        self.held_since = None;
        if let (Some(surface), Some(default)) = (self.live_target(), default) {
            send(&surface, default);
        }
        self.target = None;
    }

    fn live_target(&self) -> Option<WlSurface> {
        self.target
            .as_ref()
            .and_then(|target| target.upgrade().ok())
    }
}

/// Hands `surface` `feedback` for every `zwp_linux_dmabuf_feedback_v1` it
/// asked for. Smithay compares against what the surface already holds and
/// sends nothing when it is equal (`Arc` identity first, then content). A
/// surface that never asked has no feedback state, and nothing to send to.
fn send(surface: &WlSurface, feedback: &DmabufFeedback) {
    with_states(surface, |states| {
        if let Some(state) = SurfaceDmabufFeedbackState::from_states(states) {
            state.set_feedback(feedback);
        }
    });
}

/// One [`ScanoutFeedback`] per output that has drawn a scanout frame, in
/// `State`. An entry is made on an output's first frame (the one
/// allocation), and removed with its output ([`ScanoutFeedbacks::forget`],
/// on a `--tty` hotplug).
#[derive(Default)]
pub(crate) struct ScanoutFeedbacks {
    outputs: Vec<(OutputId, ScanoutFeedback)>,
}

impl ScanoutFeedbacks {
    /// `output`'s tracker, made on first use.
    pub(crate) fn get_mut(&mut self, output: OutputId) -> &mut ScanoutFeedback {
        let index = match self.outputs.iter().position(|(id, _)| *id == output) {
            Some(index) => index,
            None => {
                self.outputs.push((output, ScanoutFeedback::default()));
                self.outputs.len() - 1
            }
        };
        &mut self.outputs[index].1
    }

    /// Drops `output`'s tracker, for an output that went away. The caller
    /// reverts any surface it was steering first (see
    /// `State::remove_output`), so nothing is left on a tranche built for a
    /// plane no frame will use again.
    pub(crate) fn forget(&mut self, output: OutputId) {
        self.outputs.retain(|(id, _)| *id != output);
    }

    /// Rebuilds `output`'s scanout feedback if it was built for anything
    /// but `key` -- from `formats`, which is only called then -- and moves a
    /// current target onto the rebuilt one. One comparison on every other
    /// frame. `default` is what was advertised; with none, nothing can be
    /// steered and `None` is cached.
    pub(crate) fn refresh<'a>(
        &mut self,
        output: OutputId,
        key: FormatsKey,
        default: Option<&DefaultFeedback>,
        formats: impl FnOnce() -> ScanoutFormats<'a>,
    ) {
        let feedback = self.get_mut(output);
        if !feedback.needs_build(key) {
            return;
        }
        let built = default.and_then(|default| {
            let formats = formats();
            build(default, formats.primary, &formats.lost, formats.device)
        });
        feedback.install(key, built, default.map(DefaultFeedback::feedback));
    }

    /// See [`ScanoutFeedback::for_new_surface`]; the first output steering
    /// `surface` answers (a surface is only ever one output's target).
    pub(crate) fn for_new_surface(&self, surface: &WlSurface) -> Option<DmabufFeedback> {
        self.outputs
            .iter()
            .find_map(|(_, feedback)| feedback.for_new_surface(surface))
    }
}

impl DefaultFeedback {
    /// The feedback the global advertises, which a steered surface is
    /// reverted to.
    pub(crate) fn feedback(&self) -> &DmabufFeedback {
        &self.feedback
    }
}

impl State {
    /// One frame's steering for `output` (see [`ScanoutFeedback::steer`]):
    /// the covering window is the core's fullscreen window on that output,
    /// the same one `render::primary_direct` asks about, and `eligible` is
    /// that frame's judgement. Allocation-free: two map lookups and the
    /// tracker's comparisons.
    ///
    /// The root surface of the covering window is what is steered: an xdg
    /// toplevel's, or the one XWayland associated with an X window.
    pub(crate) fn steer_scanout_feedback(
        &mut self,
        output: OutputId,
        eligible: bool,
        now: impl FnOnce() -> Instant,
    ) -> Steer {
        let default = self.dmabuf_default.as_ref().map(DefaultFeedback::feedback);
        let covering = crate::compositor::fullscreen::fullscreen_surface_in(
            &self.world,
            &self.windows,
            output,
        );
        let steer = self.scanout_feedback.get_mut(output).steer(
            covering.as_deref(),
            eligible,
            default,
            now,
        );
        if matches!(steer, Steer::Sent | Steer::Reverted) {
            // A transition, never a frame: `Kept`, `Holding` and `Idle`
            // stay silent.
            tracing::debug!(
                ?steer,
                eligible,
                "dmabuf feedback: scanout steering changed"
            );
        }
        steer
    }
}

/// The harness side of the tier's two calls: installing a scanout feedback
/// through the same [`ScanoutFeedbacks::refresh`] the frame path runs, from
/// a plane list the test chooses (a headless session has no plane), and one
/// frame's steering judged the way the tier's frame judges it.
#[cfg(test)]
impl State {
    pub(crate) fn install_scanout_feedback(
        &mut self,
        plane: &FormatSet,
        device: libc::dev_t,
        key: FormatsKey,
    ) {
        let output = self.outputs.primary_id().expect("a harness output");
        self.scanout_feedback
            .refresh(output, key, self.dmabuf_default.as_ref(), || {
                ScanoutFormats {
                    primary: plane,
                    lost: Vec::new(),
                    device: Some(device),
                }
            });
    }

    pub(crate) fn steer_now(&mut self, now: Instant) -> Steer {
        let eligible = self.primary_direct_now().allowed();
        let output = self.outputs.primary_id().expect("a harness output");
        self.steer_scanout_feedback(output, eligible, || now)
    }
}
