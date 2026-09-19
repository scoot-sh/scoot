//! Where layer surfaces draw, and what space they take away from windows.
//!
//! Pixel assertions throughout, against a deliberately garish
//! [`appearance`]: the ordering bug this feature could most easily have
//! shipped -- a wallpaper drawn on top of the focus ring -- looks completely
//! correct at the type level.

use super::*;

// -------------------------------------------------------------------------
// Nothing changes when nothing uses the protocol
// -------------------------------------------------------------------------

/// The whole feature is invisible to a session with no layer surfaces: the
/// usable area is the output, and the window and its ring draw exactly where
/// they did before any of this existed. The baseline every other test's
/// numbers are read against.
#[test]
fn a_session_with_no_layer_surfaces_is_unchanged() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    assert_eq!(fixture.usable(), WHOLE);
    let rect = fixture.window_rect();
    assert_eq!((rect.x, rect.y), (GAP, GAP));

    let pixels = fixture.render();
    assert_pixel(&pixels, rect.x, rect.y, WINDOW_BGRA, "the window");
    assert_pixel(&pixels, rect.x - RING, rect.y, RING_BGRA, "its focus ring");
    assert_pixel(
        &pixels,
        CANVAS - 1,
        CANVAS - 1,
        BACKGROUND_BGRA,
        "the background",
    );
}

// -------------------------------------------------------------------------
// The render stack
// -------------------------------------------------------------------------

/// `msg outputs` reports the bar-reserved usable area, not just the full
/// output: with no bar they coincide, and a 30px bar narrows `usable`
/// while `rect` stays whole.
#[test]
fn outputs_reports_the_bar_reserved_usable_area() {
    use scoot_ipc::{Request, Response};

    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);

    let Response::Outputs { outputs } = fixture.state.handle_request(Request::Outputs) else {
        panic!("outputs should answer with outputs");
    };
    assert_eq!(outputs.len(), 1, "one output in these tests");
    assert_eq!(
        outputs[0].rect,
        scoot_ipc::Rect {
            x: 0,
            y: 0,
            width: CANVAS,
            height: CANVAS,
        },
        "rect is the whole output"
    );
    assert_eq!(
        outputs[0].usable, outputs[0].rect,
        "with no bar reserved, usable is the whole output"
    );

    fixture.run(Step::CreateLayer(LayerSpec::bar(30)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });

    let Response::Outputs { outputs } = fixture.state.handle_request(Request::Outputs) else {
        panic!("outputs should answer with outputs");
    };
    assert_eq!(
        outputs[0].rect,
        scoot_ipc::Rect {
            x: 0,
            y: 0,
            width: CANVAS,
            height: CANVAS,
        },
        "a bar reserves from usable, never from the output itself"
    );
    assert_eq!(
        outputs[0].usable,
        scoot_ipc::Rect {
            x: 0,
            y: 30,
            width: CANVAS,
            height: CANVAS - 30,
        },
        "usable is the output minus the bar's 30px strip"
    );
    fixture.disconnect_client();
}

/// A `top` layer surface covers a window, which is the whole point of the
/// layer: a bar is not something a maximized window is allowed to hide.
#[test]
fn a_top_layer_surface_draws_in_front_of_a_window() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    // No exclusive zone, so the window keeps its place and the bar overlaps
    // it -- exactly the case where "who is in front" is observable.
    fixture.run(Step::CreateLayer(LayerSpec {
        exclusive_zone: 0,
        ..LayerSpec::bar(30)
    }));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });

    let rect = fixture.window_rect();
    assert_eq!(
        (rect.x, rect.y),
        (GAP, GAP),
        "the window must not have moved"
    );
    let pixels = fixture.render();
    // Inside the window's own rectangle, but in the bar's rows.
    assert_pixel(&pixels, rect.x + 5, 15, BAR_BGRA, "the bar over the window");
    // ...and below the bar, the window itself.
    assert_pixel(&pixels, rect.x + 5, 35, WINDOW_BGRA, "the window below it");
}

/// The one ordering Smithay's own `space_render_elements` cannot express, and
/// the reason `render()` gathers layer surfaces itself: a full-screen
/// wallpaper belongs *behind* the focus ring, not on top of it. With the
/// elements in that fixed order this test fails with the ring pixel reading
/// as wallpaper.
#[test]
fn a_background_layer_surface_draws_behind_windows_and_the_focus_ring() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(LayerSpec::wallpaper()));
    fixture.run(Step::MapLayer {
        index: 0,
        color: WALLPAPER_BGRA,
    });

    let rect = fixture.window_rect();
    let pixels = fixture.render();
    assert_pixel(&pixels, rect.x, rect.y, WINDOW_BGRA, "the window");
    assert_pixel(
        &pixels,
        rect.x - RING,
        rect.y,
        RING_BGRA,
        "the focus ring over the wallpaper",
    );
    // Somewhere no window and no ring reaches: the wallpaper, not the
    // compositor's own background color.
    assert_pixel(
        &pixels,
        CANVAS - 1,
        CANVAS - 1,
        WALLPAPER_BGRA,
        "the wallpaper",
    );
}

/// A wallpaper explicitly asking not to be pushed around (`exclusive_zone =
/// -1`) covers the whole output and reserves nothing -- both halves of
/// `swaybg`'s behavior.
#[test]
fn a_dont_care_exclusive_zone_reserves_nothing() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(LayerSpec::wallpaper()));
    fixture.run(Step::MapLayer {
        index: 0,
        color: WALLPAPER_BGRA,
    });
    assert_eq!(fixture.usable(), WHOLE);
    assert_eq!(
        fixture.window_rect(),
        Rect::new(GAP, GAP, 94 - GAP, CANVAS - 2 * GAP)
    );
}

// -------------------------------------------------------------------------
// Exclusive zones
// -------------------------------------------------------------------------

/// The headline behavior: a bar that reserves its own height takes that
/// height away from where windows are arranged -- and the window's pixels
/// really move, not just its placement rectangle.
#[test]
fn an_exclusive_zone_moves_windows_out_of_the_way() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    let before = fixture.window_rect();
    fixture.run(Step::CreateLayer(LayerSpec::bar(30)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });

    assert_eq!(fixture.usable(), Rect::new(0, 30, CANVAS, CANVAS - 30));
    let after = fixture.window_rect();
    assert_eq!(after.y, 30 + GAP, "the window starts below the bar");
    assert_eq!(after.x, before.x, "nothing reserved horizontal space");
    assert_eq!(
        after.h,
        before.h - 30,
        "the window lost exactly the bar's height"
    );

    let pixels = fixture.render();
    assert_pixel(&pixels, CANVAS / 2, 15, BAR_BGRA, "the bar");
    assert_pixel(&pixels, after.x, after.y, WINDOW_BGRA, "the moved window");
    assert_pixel(
        &pixels,
        after.x - RING,
        after.y,
        RING_BGRA,
        "the ring around the moved window",
    );
    // The row the window used to start on is now above it: bar or background,
    // never the window.
    assert_ne!(
        pixel(&pixels, after.x, before.y),
        WINDOW_BGRA,
        "the window should no longer reach its old position"
    );
}

/// Two bars on the same edge stack: the second is placed below the first and
/// the reserved strip is the sum, which is what `LayerMap` means by
/// arranging exclusive surfaces first and shrinking the zone as it goes.
#[test]
fn two_bars_on_the_same_edge_stack_their_exclusive_zones() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(LayerSpec::bar(30)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    fixture.run(Step::CreateLayer(LayerSpec::bar(20)));
    fixture.run(Step::MapLayer {
        index: 1,
        color: WALLPAPER_BGRA,
    });

    assert_eq!(fixture.usable(), Rect::new(0, 50, CANVAS, CANVAS - 50));
    assert_eq!(fixture.window_rect().y, 50 + GAP);
    let pixels = fixture.render();
    assert_pixel(&pixels, CANVAS / 2, 15, BAR_BGRA, "the first bar");
    assert_pixel(&pixels, CANVAS / 2, 40, WALLPAPER_BGRA, "the second bar");
}

/// A layer surface that asks for a zone but never commits has not said
/// anything yet: every one of those requests is double-buffered state, and
/// nothing may be reserved on the strength of a pending one.
#[test]
fn a_layer_surface_that_never_commits_reserves_nothing() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    let before = fixture.window_rect();
    fixture.run(Step::CreateLayerWithoutCommit(LayerSpec::bar(30)));

    assert_eq!(fixture.usable(), WHOLE);
    assert_eq!(fixture.window_rect(), before);
    let pixels = fixture.render();
    assert_pixel(&pixels, CANVAS / 2, 15, BACKGROUND_BGRA, "nothing drawn");
}

/// **Deliberate, and worth pinning down**: a bar reserves its exclusive zone
/// from its *initial* commit -- the buffer-less one the protocol requires
/// before the first configure -- not from the commit that first gives it a
/// buffer. So there is a window, normally a frame or two long, where the
/// layout has made room for a bar that is not drawing yet.
///
/// That is Smithay's `LayerMap` behavior (it arranges every surface mapped
/// into it, buffer or not) and scoot takes it as-is rather than
/// reimplementing `arrange` to filter on mapped-ness. It self-heals in both
/// directions that matter: a client that unmaps has its cached state reset
/// by Smithay's own pre-commit hook, and one that dies has its surface
/// unmapped by `layer_destroyed`. See
/// `docs/backlog/resolved/layer-surface-bufferless-exclusive-zone-done.md`
/// for the case this does leave open -- a client that commits and then never attaches
/// anything holds the space for as long as it stays connected.
#[test]
fn a_bar_reserves_its_zone_from_its_initial_commit_not_its_first_buffer() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(LayerSpec::bar(30)));

    assert_eq!(fixture.usable(), Rect::new(0, 30, CANVAS, CANVAS - 30));
    // ...and nothing is drawn there yet, which is the other half of the
    // statement: reserved, not painted.
    let pixels = fixture.render();
    assert_pixel(
        &pixels,
        CANVAS / 2,
        15,
        BACKGROUND_BGRA,
        "nothing drawn yet",
    );
}

/// The open edge the pin above leaves, pinned rather than fixed: a client
/// that commits and then never attaches a buffer holds the space for as long
/// as it stays connected. There is deliberately no timeout -- a
/// mapped-but-frozen bar holds its space forever too, and no layer of this
/// compositor times out a client. What is bounded is the lifetime: the same
/// disconnect path that frees a mapped bar's space frees a buffer-less
/// one's, so a hung bar cannot outlive its client.
#[test]
fn a_bar_that_never_draws_holds_its_zone_until_it_disconnects() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(LayerSpec::bar(30)));

    assert_eq!(
        fixture.usable(),
        Rect::new(0, 30, CANVAS, CANVAS - 30),
        "held from the initial commit, with nothing ever drawn"
    );

    fixture.disconnect_client();
    assert_eq!(fixture.usable(), WHOLE);
    let pixels = fixture.render();
    assert_pixel(&pixels, CANVAS / 2, 15, BACKGROUND_BGRA, "the bar is gone");
}

/// The destroy path for a surface that never drew (PR #34 lineage):
/// `layer_destroyed` unmaps the surface out of the map and refreshes the
/// zone, so a bar destroyed between its initial commit and its first buffer
/// leaves no dead strip behind -- the same guarantee
/// `destroying_a_bar_returns_its_space` pins for a mapped bar.
#[test]
fn destroying_a_bar_that_never_drew_returns_its_space() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    let before = fixture.window_rect();
    fixture.run(Step::CreateLayer(LayerSpec::bar(30)));
    assert_ne!(fixture.window_rect(), before);

    fixture.run(Step::DestroyLayer { index: 0 });
    assert_eq!(fixture.usable(), WHOLE);
    assert_eq!(fixture.window_rect(), before);
    let pixels = fixture.render();
    assert_pixel(&pixels, CANVAS / 2, 15, BACKGROUND_BGRA, "the bar is gone");
}

/// The zone stays ordinary double-buffered state after the first buffer: a
/// mapped bar that commits a smaller zone gives the space back, and windows
/// move back into it. The bar itself keeps drawing where it is -- the zone
/// is a reservation against windows, not the surface's own geometry -- so a
/// bar that drops its zone ends up drawn over the window underneath it,
/// exactly the `exclusive_zone = 0` overlap
/// `a_top_layer_surface_draws_in_front_of_a_window` pins from the other end.
#[test]
fn a_mapped_bar_may_drop_its_zone_after_its_first_buffer() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    let before = fixture.window_rect();
    fixture.run(Step::CreateLayer(LayerSpec::bar(30)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    assert_eq!(fixture.usable(), Rect::new(0, 30, CANVAS, CANVAS - 30));

    fixture.run(Step::SetLayerExclusiveZone { index: 0, zone: 0 });

    assert_eq!(fixture.usable(), WHOLE);
    assert_eq!(fixture.window_rect(), before);
    let pixels = fixture.render();
    assert_pixel(&pixels, CANVAS / 2, 15, BAR_BGRA, "the bar still draws");
    assert_pixel(
        &pixels,
        before.x + 5,
        15,
        BAR_BGRA,
        "the bar over the window that moved back underneath it",
    );
}

/// Destroying a bar gives its space back, immediately -- a launcher that
/// reserved space and quit must not leave a dead strip behind.
#[test]
fn destroying_a_bar_returns_its_space() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    let before = fixture.window_rect();
    fixture.run(Step::CreateLayer(LayerSpec::bar(30)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    assert_ne!(fixture.window_rect(), before);

    fixture.run(Step::DestroyLayer { index: 0 });
    assert_eq!(fixture.usable(), WHOLE);
    assert_eq!(fixture.window_rect(), before);
    let pixels = fixture.render();
    assert_pixel(&pixels, CANVAS / 2, 15, BACKGROUND_BGRA, "the bar is gone");
    assert_pixel(
        &pixels,
        before.x,
        before.y,
        WINDOW_BGRA,
        "the window is back",
    );
}

/// ...and so does a client that simply dies with its bar mapped, which is
/// the case a compositor actually meets (a crashed `waybar`). The surface is
/// torn down implicitly, in an order nothing here controls.
#[test]
fn a_client_that_disconnects_returns_its_bars_space() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(LayerSpec::bar(30)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    assert_eq!(fixture.usable(), Rect::new(0, 30, CANVAS, CANVAS - 30));

    fixture.disconnect_client();
    // The window went with the client too, so only the zone is assertable --
    // which is the thing that would otherwise be stuck forever.
    assert_eq!(fixture.usable(), WHOLE);
    let pixels = fixture.render();
    assert_pixel(&pixels, CANVAS / 2, 15, BACKGROUND_BGRA, "the bar is gone");
}

// -------------------------------------------------------------------------
// Output changes
// -------------------------------------------------------------------------

/// Resizing the output re-anchors every layer surface against the new mode
/// and re-derives the zone from it, rather than leaving a bar sized for the
/// old screen or a reservation measured against it.
#[test]
fn resizing_the_output_re_arranges_bars_and_the_zone() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(LayerSpec::bar(30)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });

    let smaller = CANVAS / 2;
    fixture.state.resize_output(smaller, smaller);
    fixture.settle();

    assert_eq!(
        fixture.usable(),
        Rect::new(0, 30, smaller, smaller - 30),
        "the zone should follow the new mode"
    );
    let rect = fixture.window_rect();
    assert_eq!(rect.y, 30 + GAP);
    assert!(
        rect.right() <= smaller,
        "the window should fit the new mode: {rect:?}"
    );
}

/// ...and the clients are told, not just the core.
///
/// The test above reads `World::arrange()`, which recomputes from the core's
/// new area whether or not anything pushed that arrangement onto the windows
/// -- so it passes even if every client is still sized for the old mode and
/// hanging off the edge of the screen. `State::requested` is the difference:
/// only `apply()` writes it, on the same pass that sends each toplevel its
/// `configure`.
///
/// This is why `resize_output` ends in `apply()` rather than
/// `request_render()`. It used to end in the latter, which was harmless while
/// its only caller was `--nested`'s first host configure (once per process,
/// before any window has mapped) and became a real bug the moment `--tty`
/// started resizing dynamically on DRM hotplug: `refresh_layer_zone`, the one
/// thing in there that can reach `apply()`, returns early whenever the zone
/// did not move -- which is the common case (no bar mapped at all, or a bar
/// whose exclusive zone is unchanged).
#[test]
fn resizing_the_output_reconfigures_the_windows_on_it() {
    // Deliberately no layer surface: with one mapped, a changed exclusive
    // zone can reach `apply()` through `refresh_layer_zone` and hide the
    // regression this is guarding.
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);

    let configured = |fixture: &Fixture| {
        *fixture
            .state
            .requested
            .values()
            .next()
            .expect("the mapped window was configured at some size")
    };
    // The baseline, so a resize that changed nothing cannot pass below by
    // having been wrong in the same way twice: mapping alone already
    // configures the window at the size the core laid it out at.
    let before = configured(&fixture);
    let before_rect = fixture.window_rect();
    assert_eq!(before, Size::new(before_rect.w, before_rect.h));

    let smaller = CANVAS / 2;
    fixture.state.resize_output(smaller, smaller);
    fixture.settle();

    let after = configured(&fixture);
    assert_ne!(
        after, before,
        "the window is still configured for the old mode"
    );
    let rect = fixture.window_rect();
    assert_eq!(
        after,
        Size::new(rect.w, rect.h),
        "the size the client was configured at should be the one the core laid out"
    );
}
