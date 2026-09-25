//! A fullscreen X window on the GPU scanout tier: the covering surface the
//! tier judges and steers is the one XWayland draws the window into
//! (`State::fullscreen_surface`, which answered `None` for every X window
//! before Phase 2). Only in builds with both `gpu-scanout` and `xwayland`.
//!
//! The judgement and the steering are the tier's own, run with this
//! headless session's renderer (`State::primary_direct_now`,
//! `State::steer_now` -- see `fullscreen/tests/primary_direct.rs` and
//! `scanout_feedback.rs` for what they do and do not cover). XWayland's
//! buffers here are `wl_shm`, which never get a framebuffer, so nothing is
//! actually scanned out; eligibility and steering are independent of the
//! buffer type by design.

use std::time::Instant;

use smithay::backend::allocator::format::FormatSet;
use smithay::backend::allocator::{Format, Fourcc, Modifier};
use smithay::desktop::Window;

use super::live::{RED, live};
use super::x11::Props;
use crate::compositor::dmabuf::scanout::{FormatsKey, Steer};
use crate::compositor::render::PrimaryDirect;

#[test]
fn a_fullscreen_x_window_is_judged_and_steered_like_any_covering_window() {
    let Some(mut live) =
        live("a_fullscreen_x_window_is_judged_and_steered_like_any_covering_window")
    else {
        return;
    };
    let mut props = Props::new(RED);
    props.fullscreen = true;
    let xid = live.x.map(&props);
    let id = live.managed(xid);
    live.drain();
    let primary = live.fixture.state.outputs.primary_id().expect("an output");
    assert_eq!(live.fixture.state.world.fullscreen_on(primary), Some(id));
    let surface = live
        .fixture
        .state
        .window(id)
        .and_then(Window::x11_surface)
        .and_then(smithay::xwayland::X11Surface::wl_surface)
        .expect("the X window's surface");

    // The covering surface is the X window's own.
    assert_eq!(
        live.fixture.state.fullscreen_surface(primary).as_deref(),
        Some(&surface),
        "the scanout tier does not see the fullscreen X window's surface"
    );
    // Rule 6 finds its element in that surface's tree (XWayland draws the
    // window in an opaque format, so it covers the output opaquely).
    assert_eq!(
        live.fixture.state.primary_direct_now(),
        PrimaryDirect::Eligible,
        "a fullscreen X window is not eligible to go direct"
    );
    // And the steering sends it the scanout tranche.
    let plane: FormatSet = std::iter::once(Format {
        code: Fourcc::Xrgb8888,
        modifier: Modifier::Linear,
    })
    .collect();
    live.fixture
        .state
        .install_scanout_feedback(&plane, 0xfeed, FormatsKey { planes: 1, lost: 0 });
    assert_eq!(
        live.fixture.state.steer_now(Instant::now()),
        Steer::Sent,
        "the steering did not take the fullscreen X window's surface"
    );
}
