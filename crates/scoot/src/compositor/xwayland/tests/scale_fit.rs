//! The X scale's bound (`xwayland/scale.rs`, "Which scale"): X draws at
//! `ceil([output] scale)` only while the whole output layout, times that,
//! fits X's 16-bit coordinates; past it, at the largest integer that fits.
//! A layout too wide at its integer would put every X window past X pixel
//! 32767 where X cannot address it -- clamped on the wire, off the X
//! screen for `QueryPointer`, and wrong for anything X does in root
//! coordinates (a menu it places, an XDND position, `USPosition`).
//!
//! The live tests here run at `[output] scale = 2` on outputs 8192 physical
//! pixels wide (4096 logical, 8192 X pixels at X scale 2), so each X size
//! is an exact integer and a handful of outputs crosses the bound. The
//! hermetic ones pin the chooser itself, edges included.

use std::time::{Duration, Instant};

use scoot_core::OutputId;
use x11rb::protocol::xproto::{ConnectionExt as _, Window as XWindow};

use super::live::{Live, Shape, live_shaped};
use super::scale::{managed_at, scaled, toolkit_scale, x_rect, x_screen};
use super::x11::XClient;
use crate::compositor::test_support::capture_logs;
use crate::compositor::xwayland::scale::{Bounds, XScale, fit_x_scale};

/// An output 8192 by 400 physical pixels: 4096 by 200 logical at scale 2.
const WIDE: (i32, i32) = (8192, 400);

/// The fixture at `[output] scale = 2`: the 400-square primary (200
/// logical) and `extra` outputs to its right.
fn wide_live(test: &str, extra: &'static [(i32, i32)]) -> Option<Live> {
    live_shaped(
        test,
        Shape {
            scale: 2.0,
            extra,
            ..Shape::default()
        },
    )
}

/// The logical right edge of the layout: the primary's 200 plus 4096 per
/// wide output.
fn right_edge(wide: i32) -> i32 {
    200 + 4096 * wide
}

/// The X server's answer to `QueryPointer`: the pointer's root position,
/// and every window on the path from the root down to the deepest one under
/// it (a managed window sits inside the window manager's frame).
fn under_pointer(x: &XClient) -> (i16, i16, Vec<XWindow>) {
    let query = |window| {
        x.conn
            .query_pointer(window)
            .expect("a pointer query")
            .reply()
            .expect("the pointer")
    };
    let at_root = query(x.root);
    let mut path = Vec::new();
    let mut window = at_root.child;
    // Bounded: a window tree is finite, and no real one is this deep.
    for _ in 0..16 {
        if window == x11rb::NONE {
            break;
        }
        path.push(window);
        window = query(window).child;
    }
    (at_root.root_x, at_root.root_y, path)
}

/// Moves the pointer to the logical centre of output `id`'s area, so the
/// next window opens there.
fn point_at_output(live: &mut Live, id: OutputId) {
    let area = {
        let state = &live.fixture.state;
        let output = state.outputs.get(id).expect("the output").clone();
        state
            .space
            .output_geometry(&output)
            .expect("the output is placed")
    };
    live.fixture.state.pointer_move(
        f64::from(area.loc.x + area.size.w / 2),
        f64::from(area.loc.y + area.size.h / 2),
    );
}

/// Five wide outputs beside the primary: 20680 logical pixels, which at
/// the output's integer scale (2) would be an X screen 41360 wide -- past
/// X's 32767. X draws at 1 instead, where the whole layout fits: the X
/// screen is the logical layout, no toolkit is told a scale (as in any
/// scale-1 session), and a window on the far output -- logical x past
/// 16383, which at 2 the wire clamp pinned at X 32766 -- sits in X at
/// exactly its logical place, where `QueryPointer` finds it.
#[test]
fn a_layout_too_wide_for_x_at_its_integer_draws_x_at_1() {
    let Some(mut live) = wide_live(
        "a_layout_too_wide_for_x_at_its_integer_draws_x_at_1",
        &[WIDE; 5],
    ) else {
        return;
    };
    let right = right_edge(5);
    let screen = x_screen(&live.x);
    let settings = toolkit_scale(&live.x);
    assert_eq!(
        (live.fixture.state.x11_scale(), screen, settings),
        (1, (right as u16, 200), (None, None, None)),
        "(X scale, X screen, XSETTINGS) for a layout {right} logical pixels wide"
    );

    point_at_output(&mut live, OutputId(6));
    let (xid, _, placement) = managed_at(&mut live, 1);
    assert!(
        placement.rect.x > i32::from(i16::MAX) / 2,
        "the window is not far enough out to test the bound: {:?}",
        placement.rect
    );
    assert_eq!(x_rect(&live, xid), scaled(placement.rect, 1));

    let (cx, cy) = (
        placement.rect.x + placement.rect.w / 2,
        placement.rect.y + placement.rect.h / 2,
    );
    live.fixture
        .state
        .pointer_move(f64::from(cx), f64::from(cy));
    live.drain();
    let (root_x, root_y, path) = under_pointer(&live.x);
    assert_eq!(
        (i32::from(root_x), i32::from(root_y)),
        (cx, cy),
        "the X server has the pointer elsewhere"
    );
    assert!(
        path.contains(&xid),
        "QueryPointer does not find the window under the pointer: {path:?}"
    );
}

/// The bound follows the layout at runtime. Three wide outputs fit X at 2
/// (12488 logical, 24976 X pixels); a fourth plugged in crosses it (16584,
/// 33168 at 2), and X drops to 1 -- the X screen, the XSETTINGS toolkits
/// read and an open X window's X geometry all follow, logged once. Taking
/// the fourth away comes back under it, and X returns to 2 the same way.
#[test]
fn a_hotplug_across_the_bound_moves_the_x_scale_both_ways() {
    let ((), logs) = capture_logs(|| {
        let Some(mut live) = wide_live(
            "a_hotplug_across_the_bound_moves_the_x_scale_both_ways",
            &[WIDE; 3],
        ) else {
            return;
        };
        let (xid, id, _) = managed_at(&mut live, 2);
        assert_eq!(
            (x_screen(&live.x), toolkit_scale(&live.x).0),
            (((right_edge(3) * 2) as u16, 400), Some(2)),
            "three wide outputs fit X at 2"
        );

        let added = crate::compositor::headless::add_output(
            &mut live.fixture.state,
            "wide-plugged",
            WIDE.0,
            WIDE.1,
        )
        .expect("a hotplugged output");
        for (scale, right, what) in [
            (1, right_edge(4), "after the fourth output was plugged in"),
            (2, right_edge(3), "after the fourth output was taken away"),
        ] {
            if scale == 2 {
                assert!(live.fixture.state.remove_output(added), "the removal");
            }
            settled_at(&mut live, xid, id, scale, (right, 200), what);
        }
    });
    assert_eq!(reductions(&logs), 1, "the reduction is logged once: {logs}");
}

/// A resized output moves the bound too: the primary resized from 400 to
/// 8192 pixels beside three wide outputs makes the layout 16384 logical
/// pixels -- 32768 at 2, one past X's edge -- and X drops to 1; resized
/// back, it returns to 2.
#[test]
fn a_resize_across_the_bound_moves_the_x_scale_both_ways() {
    let Some(mut live) = wide_live(
        "a_resize_across_the_bound_moves_the_x_scale_both_ways",
        &[WIDE; 3],
    ) else {
        return;
    };
    let (xid, id, _) = managed_at(&mut live, 2);
    for (scale, width, right, what) in [
        (1, WIDE.0, 4096 * 4, "after the primary grew"),
        (2, 400, right_edge(3), "after the primary shrank back"),
    ] {
        assert!(
            live.fixture.state.resize_output(width, 400),
            "the resize to {width}"
        );
        settled_at(&mut live, xid, id, scale, (right, 200), what);
    }
}

/// A reload of `[output] scale` that keeps its integer can still cross the
/// bound: from 2 to 1.5 the same four outputs grow from 12488 logical
/// pixels to 16653, which at 2 is past X's edge, so X drops to 1 though the
/// integer stays 2 -- and comes back to 2 on a reload back to 2.
#[test]
fn a_reload_across_the_bound_moves_the_x_scale_both_ways() {
    let Some(mut live) = wide_live(
        "a_reload_across_the_bound_moves_the_x_scale_both_ways",
        &[WIDE; 3],
    ) else {
        return;
    };
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("config.toml");
    live.fixture.state.config_path = Some(path.clone());
    let (xid, id, _) = managed_at(&mut live, 2);
    for (config, scale, what) in [("1.5", 1, "at 1.5"), ("2.0", 2, "back at 2")] {
        std::fs::write(&path, format!("[output]\nscale = {config}\n")).expect("a config file");
        let response = live
            .fixture
            .state
            .handle_request(scoot_ipc::Request::Reload);
        assert!(
            format!("{response:?}").contains("output.scale"),
            "the reload did not apply the scale: {response:?}"
        );
        assert_eq!(live.fixture.state.integer_scale, 2, "{what}");
        // The X screen in X pixels at 1.5 is what Smithay's `xdg_output`
        // rounding makes of the fractional sizes; its bound is what
        // matters here, and the window's geometry, which is exact.
        let bounds = live
            .fixture
            .state
            .outputs
            .iter()
            .filter_map(|output| live.fixture.state.space.output_geometry(output))
            .fold((0, 0), |(w, h), area| {
                (w.max(area.loc.x + area.size.w), h.max(area.size.h))
            });
        if scale == 1 {
            assert_eq!(bounds.0, 267 + 3 * 5462, "the layout at 1.5");
        }
        let want = scaled(live.placement(id).rect, scale);
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut seen;
        loop {
            live.fixture.settle();
            seen = x_rect(&live, xid);
            if seen == want || Instant::now() >= deadline {
                break;
            }
        }
        assert_eq!(seen, want, "the X window {what}");
        assert_eq!(live.fixture.state.x11_scale(), scale, "{what}");
        let (screen_w, _) = x_screen(&live.x);
        assert!(
            i32::from(screen_w) <= i32::from(i16::MAX)
                && i32::from(screen_w) >= (bounds.0 - 1) * scale,
            "{what}: an X screen {screen_w} wide for a layout {} logical pixels wide",
            bounds.0
        );
        assert_eq!(toolkit_scale(&live.x).0, Some(scale), "{what}");
    }
}

/// Polls until the X screen is `logical` (right and bottom edge) times
/// `scale` and window `xid` sits at its placement times `scale`, or the
/// patience runs out -- then compares, so a failure says what the X server
/// holds instead -- and checks the X scale and the toolkit settings.
fn settled_at(
    live: &mut Live,
    xid: XWindow,
    id: scoot_core::WindowId,
    scale: i32,
    logical: (i32, i32),
    what: &str,
) {
    let want = (
        ((logical.0 * scale) as u16, (logical.1 * scale) as u16),
        scaled(live.placement(id).rect, scale),
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut seen;
    loop {
        live.fixture.settle();
        seen = (x_screen(&live.x), x_rect(live, xid));
        if seen == want || Instant::now() >= deadline {
            break;
        }
    }
    assert_eq!(seen, want, "(X screen, X window) {what}");
    assert_eq!(live.fixture.state.x11_scale(), scale, "{what}");
    assert_eq!(
        toolkit_scale(&live.x),
        (Some(scale), Some(scale * 96 * 1024), Some(96 * 1024)),
        "{what}"
    );
}

/// How many times `logs` says X draws below the output scale's integer.
fn reductions(logs: &str) -> usize {
    logs.lines()
        .filter(|line| line.contains("X draws below the output scale's integer"))
        .count()
}

fn bounds(left: i32, top: i32, right: i32, bottom: i32) -> Option<Bounds> {
    Some(Bounds {
        left,
        top,
        right,
        bottom,
    })
}

/// The chooser: the largest integer up to the ceiling at which every
/// logical coordinate of the layout, times it, is an X coordinate -- the
/// right and bottom edges at most 32767 (XWayland's screen size is a
/// signed 16-bit field), the left and top at least -32768 -- and 1, marked
/// as not fitting, where not even 1 does.
#[test]
fn the_x_scale_is_the_largest_that_fits_x_coordinates() {
    let fits = |scale| XScale { scale, fits: true };
    // No output yet: nothing to bound, the ceiling as is.
    assert_eq!(fit_x_scale(2, None), fits(2));
    // A small layout keeps its integer, including a ceiling below 1.
    assert_eq!(fit_x_scale(2, bounds(0, 0, 3072, 1728)), fits(2));
    assert_eq!(fit_x_scale(0, bounds(0, 0, 100, 100)), fits(1));
    // The measured regression: 8 outputs of 3840 at 1.25, 24576 logical.
    assert_eq!(fit_x_scale(2, bounds(0, 0, 24_576, 1728)), fits(1));
    // The right edge, exact: 16383 * 2 = 32766 and 32767 * 1 fit, one more
    // does not.
    assert_eq!(fit_x_scale(2, bounds(0, 0, 16_383, 10)), fits(2));
    assert_eq!(fit_x_scale(2, bounds(0, 0, 16_384, 10)), fits(1));
    assert_eq!(fit_x_scale(1, bounds(0, 0, 32_767, 10)), fits(1));
    assert_eq!(
        fit_x_scale(1, bounds(0, 0, 32_768, 10)),
        XScale {
            scale: 1,
            fits: false
        }
    );
    // The bottom edge the same way; at 3, 10922 * 3 = 32766.
    assert_eq!(fit_x_scale(3, bounds(0, 0, 10, 10_922)), fits(3));
    assert_eq!(fit_x_scale(3, bounds(0, 0, 10, 10_923)), fits(2));
    // A negative origin: -16384 * 2 = -32768 is an X coordinate, one less
    // is not.
    assert_eq!(fit_x_scale(2, bounds(-16_384, 0, 10, 10)), fits(2));
    assert_eq!(fit_x_scale(2, bounds(-16_385, 0, 10, 10)), fits(1));
    assert_eq!(fit_x_scale(4, bounds(0, -8192, 10, 10)), fits(4));
    assert_eq!(fit_x_scale(4, bounds(0, -8193, 10, 10)), fits(3));
    assert_eq!(
        fit_x_scale(2, bounds(-32_769, 0, 10, 10)),
        XScale {
            scale: 1,
            fits: false
        }
    );
    // Degrades one step at a time, not straight to 1.
    assert_eq!(fit_x_scale(4, bounds(0, 0, 10_000, 10)), fits(3));
    // Extremes do not overflow.
    assert_eq!(
        fit_x_scale(4, bounds(i32::MIN, i32::MIN, i32::MAX, i32::MAX)),
        XScale {
            scale: 1,
            fits: false
        }
    );
    // `Bounds::of` unions the outputs' rectangles, edges exclusive.
    use smithay::utils::Rectangle;
    assert_eq!(
        Bounds::of([
            Rectangle::new((0, 0).into(), (200, 200).into()),
            Rectangle::new((-300, 50).into(), (300, 400).into()),
        ]),
        bounds(-300, 0, 200, 450)
    );
    assert_eq!(Bounds::of([]), None);
}
