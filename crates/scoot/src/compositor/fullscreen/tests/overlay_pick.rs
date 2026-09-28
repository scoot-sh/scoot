//! The overlay-candidate pick through a real `State` (`render::overlay_candidate`).
//!
//! Each assertion runs `State::overlay_pick_now`, which gathers the list
//! `draw_frame_scanout` gathers and refreshes the pick from it -- the
//! judgement and the front-to-back choice, with this headless session's
//! renderer instead of the tier's `GlesRenderer`. What happens *after* the
//! pick (the mark, Smithay's plane assignment, the capture force) needs a
//! DRM device with an overlay plane and is evidenced live on the Asahi box
//! (see the PR); what is pinned here is which window the tier would mark,
//! and which frames refuse one.
//!
//! The buffers here are `wl_shm`, which never get a framebuffer, so none of
//! these windows could actually ride an overlay -- which is fine: the pick
//! is deliberately independent of the buffer type (a client may switch from
//! dma-buf to shm between two frames, and Smithay composites the shm one),
//! exactly like primary-direct eligibility.

use crate::compositor::decorations::Appearance;

use super::*;

/// The picked window's core id, or `None` when the frame refuses a pick.
fn pick(fixture: &mut Fixture) -> Option<WindowId> {
    fixture.state.overlay_pick_now()
}

#[test]
fn a_lone_tiled_window_is_picked() {
    let mut fixture = Fixture::new();
    fixture.map(WINDOW_BGRA);
    assert_eq!(pick(&mut fixture), Some(fixture.id(0)));
}

#[test]
fn nothing_mapped_picks_nothing() {
    let mut fixture = Fixture::new();
    assert_eq!(pick(&mut fixture), None);
}

#[test]
fn a_covering_fullscreen_window_picks_nothing_then_picks_again() {
    // Covered outputs are the primary plane's domain: the covering window
    // is not a pick, and tiled windows behind it are occluded.
    let mut fixture = Fixture::new();
    fixture.map(WINDOW_BGRA);
    assert_eq!(pick(&mut fixture), Some(fixture.id(0)));
    fixture.configured(Step::SetFullscreen {
        window: 0,
        output: None,
    });
    fixture.done(Step::Draw {
        window: 0,
        color: WINDOW_BGRA,
    });
    assert_eq!(pick(&mut fixture), None);
    fixture.configured(Step::UnsetFullscreen { window: 0 });
    fixture.done(Step::Draw {
        window: 0,
        color: WINDOW_BGRA,
    });
    assert_eq!(pick(&mut fixture), Some(fixture.id(0)));
}

#[test]
fn closing_the_pick_moves_to_the_next_window() {
    // At most one window, and the pick follows what is still there: closing
    // the picked window moves the mark, it does not strand it.
    let mut fixture = Fixture::new();
    fixture.map(WINDOW_BGRA);
    fixture.map(OTHER_BGRA);
    let first = pick(&mut fixture).expect("a pick with two windows");
    let other = [fixture.id(0), fixture.id(1)]
        .into_iter()
        .find(|id| *id != first)
        .expect("two windows");
    let index = if first == fixture.id(0) { 0 } else { 1 };
    fixture.done(Step::Unmap { window: index });
    fixture.settle();
    assert_eq!(pick(&mut fixture), Some(other));
}

#[test]
fn a_rounded_session_picks_nothing() {
    // `Rounded` forwards the unclipped buffer, so a rounded window riding an
    // overlay would lose its corners. The shared refusal clears the pick --
    // the same rule that keeps rounded windows off the primary plane.
    let mut fixture = Fixture::with_appearance(Appearance {
        corner_radius: 12,
        ..appearance()
    });
    fixture.map(WINDOW_BGRA);
    assert_eq!(pick(&mut fixture), None);
}

#[test]
fn a_translucent_window_picks_nothing_and_opaque_again_picks() {
    let mut fixture = Fixture::new();
    fixture.map(WINDOW_BGRA);
    assert!(pick(&mut fixture).is_some());
    fixture.done(Step::SetAlpha {
        window: 0,
        multiplier: u32::MAX / 2,
    });
    assert_eq!(pick(&mut fixture), None);
    fixture.done(Step::SetAlpha {
        window: 0,
        multiplier: u32::MAX,
    });
    assert_eq!(pick(&mut fixture), Some(fixture.id(0)));
}

#[test]
fn a_lock_over_a_tiled_window_picks_nothing() {
    // The lock screen is never left to plane assignment, and nothing from
    // behind it can ride one.
    let mut fixture = Fixture::new();
    fixture.map(WINDOW_BGRA);
    assert!(pick(&mut fixture).is_some());
    fixture.done(Step::LockSession);
    assert!(fixture.state.session_lock.is_locked());
    assert_eq!(pick(&mut fixture), None);
}

#[test]
fn bars_wallpaper_and_notifications_do_not_become_the_pick() {
    // Layer surfaces are surface elements too, but no window owns them: the
    // pick stays the tiled window under them, whatever the layers hold.
    let mut fixture = Fixture::new();
    fixture.done(Step::CreateLayer(Layer::Bar));
    fixture.done(Step::CreateLayer(Layer::Wallpaper { opaque: true }));
    fixture.done(Step::CreateLayer(Layer::Notification));
    fixture.map(WINDOW_BGRA);
    assert_eq!(pick(&mut fixture), Some(fixture.id(0)));
}

/// Prints what one frame's overlay-candidate work costs on the shapes a
/// tiled session sits in: a lone window (the pick), two windows with layers
/// above (the pick walks further), and a rounded session (the refusal).
/// Run by hand:
///
/// ```text
/// cargo test --release -p scoot --bin scoot --features gpu-scanout overlay_pick_cost -- --ignored --nocapture
/// ```
#[test]
#[ignore = "prints per-frame timings for a human; asserts nothing"]
fn overlay_pick_cost() {
    const ROUNDS: u32 = 200_000;
    let mut lone = Fixture::new();
    lone.map(WINDOW_BGRA);
    let mut layered = Fixture::new();
    layered.done(Step::CreateLayer(Layer::Bar));
    layered.done(Step::CreateLayer(Layer::Notification));
    layered.map(WINDOW_BGRA);
    layered.map(OTHER_BGRA);
    let mut rounded = Fixture::with_appearance(Appearance {
        corner_radius: 12,
        ..appearance()
    });
    rounded.map(WINDOW_BGRA);
    for (label, fixture) in [
        ("lone window", &mut lone),
        ("two windows under layers", &mut layered),
        ("rounded session", &mut rounded),
    ] {
        let (picked, each) = fixture.state.overlay_pick_cost(ROUNDS);
        println!("overlay pick, {label}: {each:?} per frame ({picked:?}, {ROUNDS} rounds)");
    }
}
