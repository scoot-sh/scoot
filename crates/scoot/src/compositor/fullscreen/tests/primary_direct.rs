//! Which frames the GPU scanout tier may hand the primary plane to a client
//! buffer, judged over the list a real `State` gathers for a real client.
//!
//! Each assertion runs `State::primary_direct_now`, which calls the same
//! `scanout_frame_elements` `draw_frame_scanout` runs -- the gather and the
//! judgement -- with this headless session's renderer instead of the tier's
//! `GlesRenderer`. What happens *after* the judgement (Smithay's plane
//! assignment, the atomic test, the capture force) needs a DRM device and
//! is evidenced live on the dev VM (see the PR); what is pinned here is the
//! part that decides whether `ALLOW_PRIMARY_PLANE_SCANOUT_ANY` reaches the
//! frame at all.
//!
//! The buffers here are `wl_shm`, which never get a framebuffer, so none of
//! these windows could actually be scanned out -- which is fine: eligibility
//! is deliberately independent of the buffer type (a client may switch from
//! dma-buf to shm between two frames, and Smithay composites the shm one).

use crate::compositor::render::PrimaryDirect;

use super::*;

/// Maps a window, makes it fullscreen, and draws it at the size it was told.
///
/// The window declares itself opaque: its `Argb8888` buffer has an alpha
/// channel, so without an opaque region nothing covers the output opaquely,
/// and over this suite's non-black background Smithay would not try the
/// primary at all (rule 6, pinned below).
fn covering(fixture: &mut Fixture) {
    fixture.map(WINDOW_BGRA);
    fixture.done(Step::SetOpaque { window: 0 });
    fixture.configured(Step::SetFullscreen {
        window: 0,
        output: None,
    });
    fixture.done(Step::Draw {
        window: 0,
        color: WINDOW_BGRA,
    });
    assert_eq!(fixture.rect_of(0), OUTPUT, "the window covers the output");
}

/// A black background with no ring and no gap: the shape in which Smithay
/// would try the bottom window for the primary on its own (a
/// black clear colour passes its check for any element), i.e. the one in
/// which `ANY` reaching a frame without a covering window would do harm.
fn black() -> Appearance {
    Appearance {
        background_color: Color::new(0.0, 0.0, 0.0, 1.0),
        focus_ring_width: 0,
        ..appearance()
    }
}

#[test]
fn a_covering_fullscreen_window_is_eligible() {
    let mut fixture = Fixture::new();
    covering(&mut fixture);
    assert_eq!(fixture.state.primary_direct_now(), PrimaryDirect::Eligible);
}

#[test]
fn a_tiled_window_is_never_eligible_even_over_black() {
    // No covering window, no primary bit: a lone tiled window over a black
    // background would pass Smithay's own clear-colour check, and `ANY`
    // would hand it the primary whatever it is. It must not get the chance.
    for appearance in [appearance(), black()] {
        let mut fixture = Fixture::with_appearance(appearance);
        fixture.map(WINDOW_BGRA);
        assert_eq!(
            fixture.state.primary_direct_now(),
            PrimaryDirect::NotCovered
        );
    }
    // Nothing mapped at all: the same answer.
    let mut empty = Fixture::with_appearance(black());
    assert_eq!(empty.state.primary_direct_now(), PrimaryDirect::NotCovered);
}

#[test]
fn a_rounded_session_goes_direct_only_when_covered_and_never_rounded() {
    // `Rounded` forwards the unclipped buffer, so a rounded window reaching
    // the primary would lose its corners. Tiled, it is not covered; covering,
    // it is not rounded (a fullscreen window's corners are the output's) --
    // so the rounded check never refuses the frame that should go direct.
    let mut fixture = Fixture::with_appearance(Appearance {
        corner_radius: 12,
        ..black()
    });
    fixture.map(OTHER_BGRA);
    assert_eq!(
        fixture.state.primary_direct_now(),
        PrimaryDirect::NotCovered
    );
    fixture.map(WINDOW_BGRA);
    fixture.configured(Step::SetFullscreen {
        window: 1,
        output: None,
    });
    fixture.done(Step::Draw {
        window: 1,
        color: WINDOW_BGRA,
    });
    assert_eq!(fixture.state.primary_direct_now(), PrimaryDirect::Eligible);
}

#[test]
fn leaving_fullscreen_ends_eligibility() {
    let mut fixture = Fixture::new();
    covering(&mut fixture);
    assert_eq!(fixture.state.primary_direct_now(), PrimaryDirect::Eligible);
    fixture.configured(Step::UnsetFullscreen { window: 0 });
    fixture.done(Step::Draw {
        window: 0,
        color: WINDOW_BGRA,
    });
    assert_eq!(
        fixture.state.primary_direct_now(),
        PrimaryDirect::NotCovered
    );
}

#[test]
fn unmapping_the_covering_window_ends_eligibility() {
    // The window that was on the primary goes away: the next frame has no
    // covering window and no primary bit, so the primary returns to the
    // swapchain.
    let mut fixture = Fixture::new();
    covering(&mut fixture);
    fixture.done(Step::Unmap { window: 0 });
    fixture.settle();
    assert_eq!(
        fixture.state.primary_direct_now(),
        PrimaryDirect::NotCovered
    );
}

#[test]
fn a_lock_over_a_fullscreen_window_composites() {
    // The lock screen is never left to plane assignment, and nothing from
    // behind it -- the fullscreen window stays fullscreen underneath -- can
    // be what the primary shows.
    let mut fixture = Fixture::new();
    covering(&mut fixture);
    fixture.done(Step::LockSession);
    assert!(fixture.state.session_lock.is_locked());
    assert!(fixture.state.world.is_fullscreen(fixture.id(0)));
    assert_eq!(fixture.state.primary_direct_now(), PrimaryDirect::Locked);
}

#[test]
fn a_translucent_fullscreen_window_composites_and_opaque_again_goes_direct() {
    let mut fixture = Fixture::new();
    covering(&mut fixture);
    fixture.done(Step::SetAlpha {
        window: 0,
        multiplier: u32::MAX / 2,
    });
    assert_eq!(
        fixture.state.primary_direct_now(),
        PrimaryDirect::Translucent
    );
    // Fully opaque again, set explicitly rather than unset: `u32::MAX` must
    // judge exactly like a client that never touched the protocol.
    fixture.done(Step::SetAlpha {
        window: 0,
        multiplier: u32::MAX,
    });
    assert_eq!(fixture.state.primary_direct_now(), PrimaryDirect::Eligible);
}

#[test]
fn an_overlay_notification_is_left_to_smithay() {
    // Deliberately not a refusal here: whether something composited above
    // the window stops the primary is Smithay's call (it only tries the last
    // visible element, and only once everything above it rode a plane), and
    // it composites the frame when it does. Refusing here as well would be
    // a second copy of that rule that could drift from it.
    let mut fixture = Fixture::new();
    covering(&mut fixture);
    fixture.done(Step::CreateLayer(Layer::Notification));
    assert_eq!(fixture.state.primary_direct_now(), PrimaryDirect::Eligible);
    // The bar, which the covering window hides, is not in the frame at all.
    fixture.done(Step::CreateLayer(Layer::Bar));
    assert_eq!(fixture.state.primary_direct_now(), PrimaryDirect::Eligible);
}

#[test]
fn an_alpha_window_with_no_opaque_region_is_not_tried_over_a_grey_background() {
    // The case review measured live: an `AR24` fullscreen client with no
    // opaque region, over the default (non-black) background. Smithay never
    // tries the primary for it, so the frame gets no direct flags and the
    // window is not steered toward a scannable layout.
    let mut fixture = Fixture::new();
    fixture.map(WINDOW_BGRA);
    fixture.configured(Step::SetFullscreen {
        window: 0,
        output: None,
    });
    fixture.done(Step::Draw {
        window: 0,
        color: WINDOW_BGRA,
    });
    assert_eq!(
        fixture.state.primary_direct_now(),
        PrimaryDirect::NothingOpaqueCovers
    );
    // Declaring itself opaque is what makes it a candidate. (With its next
    // buffer: Smithay recomputes a surface's opaque regions only when the
    // buffer or its view changes, which is when a real client sets one.)
    fixture.done(Step::SetOpaque { window: 0 });
    fixture.done(Step::Draw {
        window: 0,
        color: WINDOW_BGRA,
    });
    assert_eq!(fixture.state.primary_direct_now(), PrimaryDirect::Eligible);
}

#[test]
fn over_black_an_alpha_window_is_tried_without_an_opaque_region() {
    // ... and over a black background it needs nothing: Smithay's other arm.
    let mut fixture = Fixture::with_appearance(black());
    fixture.map(WINDOW_BGRA);
    fixture.configured(Step::SetFullscreen {
        window: 0,
        output: None,
    });
    fixture.done(Step::Draw {
        window: 0,
        color: WINDOW_BGRA,
    });
    assert_eq!(fixture.state.primary_direct_now(), PrimaryDirect::Eligible);
}

/// Makes the only window fullscreen and draws it in `Argb8888` (optionally
/// declaring an opaque region), over whatever layers the test mapped.
fn fullscreen_alpha(fixture: &mut Fixture, opaque_region: bool) {
    fixture.configured(Step::SetFullscreen {
        window: 0,
        output: None,
    });
    if opaque_region {
        fixture.done(Step::SetOpaque { window: 0 });
    }
    fixture.done(Step::Draw {
        window: 0,
        color: WINDOW_BGRA,
    });
}

/// A session with a wallpaper, a window mapped over it.
fn over_wallpaper(appearance: Appearance, opaque_wallpaper: bool) -> Fixture {
    let mut fixture = Fixture::with_appearance(appearance);
    fixture.done(Step::CreateLayer(Layer::Wallpaper {
        opaque: opaque_wallpaper,
    }));
    fixture.map(WINDOW_BGRA);
    fixture
}

#[test]
fn over_an_opaque_wallpaper_an_alpha_window_is_not_what_smithay_tries() {
    // Review's R1: the wallpaper under the window is opaque and spans the
    // output, so Smithay's walk ends there and it -- not the window -- is
    // the element it would try. Grey and black alike.
    for appearance in [appearance(), black()] {
        let mut fixture = over_wallpaper(appearance, true);
        fullscreen_alpha(&mut fixture, false);
        assert_eq!(
            fixture.state.primary_direct_now(),
            PrimaryDirect::NotTheWindow
        );
    }
}

#[test]
fn over_a_transparent_wallpaper_an_alpha_window_is_never_tried() {
    // Grey: nothing is opaque over the output, so Smithay tries nothing.
    let mut fixture = over_wallpaper(appearance(), false);
    fullscreen_alpha(&mut fixture, false);
    assert_eq!(
        fixture.state.primary_direct_now(),
        PrimaryDirect::NothingOpaqueCovers
    );
    // Black: Smithay would try the last element -- the wallpaper.
    let mut fixture = over_wallpaper(black(), false);
    fullscreen_alpha(&mut fixture, false);
    assert_eq!(
        fixture.state.primary_direct_now(),
        PrimaryDirect::NotTheWindow
    );
}

#[test]
fn an_opaque_region_puts_the_window_in_front_of_any_wallpaper() {
    // Declared opaque, the window ends the walk itself, whatever lies
    // beneath and whatever the background.
    for appearance in [appearance(), black()] {
        for opaque_wallpaper in [true, false] {
            let mut fixture = over_wallpaper(appearance.clone(), opaque_wallpaper);
            fullscreen_alpha(&mut fixture, true);
            assert_eq!(
                fixture.state.primary_direct_now(),
                PrimaryDirect::Eligible,
                "opaque wallpaper = {opaque_wallpaper}"
            );
        }
    }
}

#[test]
fn an_opaque_format_buffer_needs_no_opaque_region() {
    // The commonest real covering window: an `Xrgb8888` buffer, no opaque
    // region. Smithay treats a buffer with no alpha channel as opaque whole.
    for opaque_wallpaper in [None, Some(true), Some(false)] {
        let mut fixture = Fixture::new();
        if let Some(opaque) = opaque_wallpaper {
            fixture.done(Step::CreateLayer(Layer::Wallpaper { opaque }));
        }
        fixture.map(WINDOW_BGRA);
        fixture.configured(Step::SetFullscreen {
            window: 0,
            output: None,
        });
        fixture.done(Step::DrawXrgb { window: 0 });
        assert_eq!(
            fixture.state.primary_direct_now(),
            PrimaryDirect::Eligible,
            "wallpaper = {opaque_wallpaper:?}"
        );
    }
}

/// Prints what `render::primary_direct::judge` costs per frame on the
/// shapes a fullscreen session sits in: an opaque window, an alpha window
/// over an opaque wallpaper (the walk goes one element further), and a
/// window declaring its opacity in 16 rectangles, none of which covers it
/// alone (the subtraction path). Run by hand:
///
/// ```text
/// cargo test --release -p scoot --bin scoot --features gpu-scanout judge_cost -- --ignored --nocapture
/// ```
#[test]
#[ignore = "prints per-frame timings for a human; asserts nothing"]
fn judge_cost() {
    const ROUNDS: u32 = 200_000;
    let mut opaque = Fixture::new();
    covering(&mut opaque);
    let mut wallpaper = over_wallpaper(appearance(), true);
    fullscreen_alpha(&mut wallpaper, false);
    let mut striped = Fixture::new();
    striped.map(WINDOW_BGRA);
    striped.configured(Step::SetFullscreen {
        window: 0,
        output: None,
    });
    striped.done(Step::SetOpaqueStripes {
        window: 0,
        stripes: 16,
    });
    striped.done(Step::Draw {
        window: 0,
        color: WINDOW_BGRA,
    });
    for (label, fixture) in [
        ("opaque window", &mut opaque),
        ("alpha window over an opaque wallpaper", &mut wallpaper),
        ("window opaque in 16 stripes", &mut striped),
    ] {
        let (verdict, each) = fixture.state.judge_cost(ROUNDS);
        println!("judge, {label}: {each:?} per frame ({verdict:?}, {ROUNDS} rounds)");
    }
}
