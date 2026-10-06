//! Night light on outputs whose CRTC has no gamma LUT but a `CTM` blob
//! property (Apple DCP on Asahi: `GAMMA_LUT` absent, `CTM` present).
//!
//! A 3x3 CTM cannot express an arbitrary LUT curve, but the night-light
//! daemons' ramps are per-channel linear scales at their default settings
//! (`wlsunset -g 1.0`: `ramp[i] = 65535 * (i / (size - 1)) * channel`),
//! which a diagonal matrix reproduces *exactly* -- and in S31.32
//! fixed point, more precisely than the 16-bit LUT entries themselves.
//! A non-default gamma bends the curve; the endpoint (white) fit below
//! still matches white and black exactly, erring only in the mid-tones.
//!
//! The matrix is pushed through its own synchronous atomic commit on the
//! session's already-open DRM device, beside Smithay's flip/modeset
//! commits, which never mention `CTM` and so never disturb it. One commit
//! per `set_gamma` -- a rare event (about one per second during a
//! minute-long dawn/dusk transition, zero otherwise) -- and nothing on
//! any frame path: no allocation, no re-render, no damage interaction.
//! Screenshots stay pre-transform (they read the framebuffer; the CTM
//! applies in the display controller after scanout), the lock screen is
//! warmed automatically (output-level state), and fullscreen direct
//! scanout keeps working (the matrix sits after plane blending).

use smithay::backend::drm::DrmDevice;
use smithay::reexports::drm::control::Device as DrmControl;
use smithay::reexports::drm::control::atomic::AtomicModeReq;
use smithay::reexports::drm::control::{AtomicCommitFlags, crtc, property};

#[cfg(test)]
mod tests;

/// Kernel `struct drm_color_ctm`: nine S31.32 fixed-point gains, row-major.
/// `repr(C)` so the blob bytes are exactly what the kernel parses: nine
/// consecutive little-endian `u64`s, 72 bytes total (see the layout test).
#[repr(C)]
pub(super) struct Ctm {
    pub(super) matrix: [u64; 9],
}

/// The pass-through matrix: every diagonal entry exactly one (S31.32
/// `1 << 32`), everything else zero. Test-only: it pins what
/// [`from_ramps`] must return for a linear ramp (see the identity test).
/// Production restores go through a linear ramp's endpoint fit instead,
/// which lands here by construction.
#[cfg(test)]
pub(super) const IDENTITY: Ctm = Ctm {
    matrix: [1 << 32, 0, 0, 0, 1 << 32, 0, 0, 0, 1 << 32],
};

/// A diagonal CTM from three `set_gamma` ramps, or `None` when they are
/// not three ramps at all (empty, ragged lengths).
///
/// Each channel's gain is its white endpoint (`ramp.last() / 65535`),
/// encoded S31.32 with integer arithmetic -- no float anywhere near this
/// path. The endpoint fit is exact at black (zero stays zero) and at
/// white by construction, and exact *everywhere* for a linear ramp, which
/// is what the daemons send at their default gamma. A bent ramp (a
/// non-default gamma) keeps exact black and white and approximates the
/// mid-tones; see the module doc.
pub(super) fn from_ramps(red: &[u16], green: &[u16], blue: &[u16]) -> Option<Ctm> {
    let (Some(&r), Some(&g), Some(&b)) = (red.last(), green.last(), blue.last()) else {
        return None;
    };
    if red.len() != green.len() || red.len() != blue.len() {
        return None;
    }
    Some(Ctm {
        matrix: [fixed(r), 0, 0, 0, fixed(g), 0, 0, 0, fixed(b)],
    })
}

/// `round(last * 2^32 / 65535)`: the whole S31.32 encoding. Halves round
/// down (the bias is `65535 / 2`, i.e. 32767); no input can overflow,
/// since the largest numerator is `65535 * 2^32`, which divided by 65535
/// is exactly `1 << 32`.
fn fixed(last: u16) -> u64 {
    (((last as u64) << 32) + 65535 / 2) / 65535
}

/// The CRTC's `CTM` blob property, if the driver exposes one. Name-matched
/// rather than assumed: a driver with a LUT never reaches here (the LUT
/// path wins), and a driver with neither answers `None`, which is the
/// existing `failed()` refusal, not a new error.
pub(super) fn prop(drm: &DrmDevice, crtc: crtc::Handle) -> Option<property::Handle> {
    let set = drm.get_properties(crtc).ok()?;
    let (ids, _) = set.as_props_and_values();
    ids.iter().copied().find(|id| {
        drm.get_property(*id)
            .is_ok_and(|info| info.name().to_bytes() == b"CTM")
    })
}

/// Push `ctm` to `crtc`'s `CTM` property in one synchronous atomic commit.
///
/// The commit carries only the CTM blob -- no mode, no planes -- so it
/// cannot disturb an in-flight flip or modeset; the kernel serializes the
/// two commits and each applies to orthogonal state. Synchronous (no
/// `NONBLOCK`, no completion event) so the blob's lifetime ends here:
/// the kernel holds its own reference to the committed state, and ours is
/// destroyed before returning either way. Any failure is the caller's to
/// turn into a `failed` event; the session keeps running.
pub(super) fn commit(
    drm: &DrmDevice,
    crtc: crtc::Handle,
    ctm_prop: property::Handle,
    ctm: &Ctm,
) -> std::io::Result<()> {
    let property::Value::Blob(blob) = drm.create_property_blob(ctm)? else {
        return Err(std::io::Error::other(
            "drm: CTM blob creation answered without a blob",
        ));
    };
    let mut req = AtomicModeReq::new();
    req.add_property(crtc, ctm_prop, property::Value::Blob(blob));
    let commit_result = drm.atomic_commit(AtomicCommitFlags::empty(), req);
    // Best-effort: the commit's outcome is what the caller reports, and a
    // leftover 72-byte kernel blob is harmless -- but there is no reason
    // to leave one behind on the happy path either.
    if let Err(error) = drm.destroy_property_blob(blob) {
        tracing::warn!(%error, "drm: could not destroy a CTM blob after commit");
    }
    commit_result?;
    Ok(())
}
