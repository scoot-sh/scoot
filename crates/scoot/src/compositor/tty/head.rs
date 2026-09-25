//! One driven connector: the unit `--tty` multiplies when it drives more
//! than one screen (milestone 19, phase E).
//!
//! Before multi-output, [`Tty`](super::Tty) *was* one of these: it held the
//! connector, the presenter (and through it the CRTC and its surface) and
//! the mode size directly. They moved here unchanged in meaning, one set per
//! connector, and `Tty` kept what is true of the whole device and session no
//! matter how many screens are lit -- the libseat session, the DRM device,
//! whether DRM master is held (`active`) and whether the session is paused
//! (`session_paused`). Those two stay device-wide on purpose: master is per
//! open file, not per CRTC, and a VT switch pauses every screen at once.
//!
//! What each field means is exactly what the old `Tty` field of the same
//! name meant, scoped to this head's connector -- see their docs. The one new
//! field is [`output`](Head::output), the id the rest of the compositor knows
//! this screen by, which is how every per-output path (render, capture,
//! gamma, the session-lock vblank wait) finds *its* head rather than
//! assuming there is one.

use scoot_core::OutputId;
use smithay::reexports::drm::control::connector;

use super::presenter::Presenter;
use crate::compositor::output_identity::OutputIdentity;

/// One connector this backend drives, and everything that drives it.
pub(super) struct Head {
    /// The output this head presents, once it has been registered with the
    /// compositor (`Tty::attach`). `None` only in the window between
    /// `tty::init` building the head and `compositor::run` creating its
    /// `wl_output`, which no frame, vblank or client request can fall into
    /// (the event loop has not started); a head that fails to register is
    /// dropped (`Tty::retain_attached`) rather than left unattached.
    pub(super) output: Option<OutputId>,
    /// The connector this head drives. Startup picks it (`gpu::find_all`)
    /// and a hotplug can move the *primary* head onto another one when its
    /// own goes away (`hotplug.rs`), which is the only other write site.
    ///
    /// The presenter's `DrmSurface` has its own `pending_connectors()`, and
    /// the two agree by construction -- every write here happens right after
    /// the matching `set_connectors` succeeded. This field exists because the
    /// surface's answer is a set (Smithay supports several connectors per
    /// CRTC; this backend deliberately drives exactly one per head), and
    /// re-deriving "the connector" from a set would mean inventing a rule for
    /// a case that cannot arise.
    pub(super) connector: connector::Handle,
    /// The connector's name (`eDP-1`, `DP-1`), for log lines. The
    /// `wl_output` carries its own copy, fixed at creation.
    pub(super) name: String,
    /// Which monitor this head was built for: the connector name plus the
    /// EDID summary where one could be read (see
    /// [`OutputIdentity`](crate::compositor::output_identity::OutputIdentity)).
    /// Fixed when the head is built, like the `wl_output` name above -- a
    /// hotplug that moves this head onto another connector (the single-output
    /// fallback) does not rewrite it, and neither does the output registered
    /// from it. Written only in `build_head`, read when the output is
    /// created (`StartupHead`, hotplug's `Change::Added`).
    pub(super) identity: OutputIdentity,
    /// How this head gets a rendered frame onto its CRTC. See
    /// [`Presenter`] for the two tiers.
    pub(super) presenter: Presenter,
    /// The mode size this head's CRTC is scanning out, in physical pixels
    /// -- `present`'s size guard, and what a hotplug compares a fresh probe
    /// against. Written only where the presenter has actually moved to the
    /// size (startup, and `hotplug.rs`'s `retarget`/`switch_crtc`).
    pub(super) width: i32,
    pub(super) height: i32,
}

impl Head {
    /// Whether this head presents `id`. The per-frame lookup every
    /// output-keyed `Tty` accessor goes through: one `Option` compare per
    /// head, over at most `MAX_OUTPUTS` heads, with no allocation.
    pub(super) fn presents(&self, id: OutputId) -> bool {
        self.output == Some(id)
    }
}
