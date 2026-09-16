//! What a fresh probe of the DRM device *means*, decided without one.
//!
//! [`super::plan`] is the only part of this module that can be exercised
//! off real hardware: everything around it -- `gpu::reselect`,
//! `DrmSurface::use_mode`/`set_connectors`, `BufferPool::new` -- needs a
//! live DRM file descriptor, and `drm`'s two device traits are blanket
//! implementations over `AsFd` with no seam to fake (the same reason
//! `gpu.rs`'s own tests stop at device *ordering* and never reach
//! `find_connector_and_mode`). Those paths are covered on the dev VM
//! instead; see the PR for what was run there.
//!
//! Connector handles are real ones, built from the `NonZeroU32` the kernel
//! identifies a connector by, because that is exactly what
//! `resources.connectors()` hands back -- nothing here is a stand-in type.

use std::num::NonZeroU32;

use smithay::reexports::drm::control::connector;

use super::{Plan, plan};

/// The connector the kernel would call id `raw`.
fn connector(raw: u32) -> connector::Handle {
    connector::Handle::from(NonZeroU32::new(raw).expect("connector ids start at 1"))
}

const EDP: u32 = 71;
const HDMI: u32 = 82;
const FHD: (i32, i32) = (1920, 1080);
const SMALL: (i32, i32) = (848, 480);

#[test]
fn the_same_connector_at_the_same_size_is_not_a_change() {
    // The common case by far: a `change` uevent fires for anything the
    // kernel considers a change to the card, and most of them have nothing
    // to do with which connector is lit or how big it is. Tearing the
    // display down for one would mean a visible blank every time something
    // unrelated twitched.
    assert_eq!(
        plan((connector(EDP), FHD), Some((connector(EDP), FHD))),
        Plan::Unchanged
    );
}

#[test]
fn the_same_connector_at_a_new_size_is_a_mode_change() {
    // Issue #48's first case: the vfkit window moved between a 2x and a 1x
    // screen, so virtio-gpu's `Virtual-1` now offers -- and prefers -- a
    // different size, on the same connector it always had.
    assert_eq!(
        plan((connector(EDP), FHD), Some((connector(EDP), SMALL))),
        Plan::NewMode
    );
}

#[test]
fn a_different_connector_is_a_connector_change_even_at_the_same_size() {
    // Issue #48's second case, in its awkward variant: the laptop panel was
    // unplugged (or went away) and an external monitor of *exactly* the
    // same resolution took over. Nothing about the size says anything
    // happened, but the surface still has to be moved onto the new
    // connector or it keeps driving one that is gone.
    assert_eq!(
        plan((connector(EDP), FHD), Some((connector(HDMI), FHD))),
        Plan::NewConnector
    );
}

#[test]
fn a_different_connector_at_a_different_size_is_still_one_connector_change() {
    // Both changed at once, which is the normal shape of the unplug case.
    // It must not read as a mode change on the old connector -- the mode
    // belongs to the new one, and applying it without moving the surface
    // first is what `set_pending` exists to get right.
    assert_eq!(
        plan((connector(EDP), FHD), Some((connector(HDMI), SMALL))),
        Plan::NewConnector
    );
}

#[test]
fn nothing_connected_is_its_own_answer_not_a_change_to_apply() {
    // The case that used to be a black screen until restart. It is
    // deliberately *not* folded into any of the three above: there is no
    // connector to move to and no mode to set, so the only thing to do is
    // hold the last frame and say so.
    assert_eq!(plan((connector(EDP), FHD), None), Plan::NoConnector);
}

#[test]
fn only_the_width_changing_is_still_a_mode_change() {
    // Guards the tuple comparison against being written as a comparison of
    // one dimension: an ultrawide re-probing from 2560x1080 to 3440x1080
    // keeps its height and is absolutely a new mode.
    assert_eq!(
        plan(
            (connector(EDP), (2560, 1080)),
            Some((connector(EDP), (3440, 1080)))
        ),
        Plan::NewMode
    );
}

#[test]
fn only_the_height_changing_is_still_a_mode_change() {
    assert_eq!(
        plan((connector(EDP), (1920, 1200)), Some((connector(EDP), FHD))),
        Plan::NewMode
    );
}

#[test]
fn a_zero_sized_probe_is_treated_as_a_change_like_any_other() {
    // Not reachable through `gpu::connector_mode` -- the kernel does not
    // list a 0x0 mode -- but `plan` must not be the thing that decides it
    // cannot happen. Reporting it as a change routes it into `retarget`,
    // where a real `BufferPool::new`/`use_mode` rejects it with a logged
    // error and the display stays as it was; silently reporting `Unchanged`
    // for a nonsense probe would hide that.
    assert_eq!(
        plan((connector(EDP), FHD), Some((connector(EDP), (0, 0)))),
        Plan::NewMode
    );
}
