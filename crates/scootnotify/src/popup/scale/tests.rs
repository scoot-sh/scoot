//! The scale wiring: which events redraw, and what follows a change of
//! scale. The events themselves (a `wl_surface` for a destroyed surface,
//! the output fallback) are pinned against a real compositor in
//! `tests/scale_tests.rs`; the rule and the state they drive are here.

use scootui::density::Scale;

use super::App;
use crate::config::Config;
use crate::look::{Look, ShadowStyle};

fn app() -> App {
    let mut app = App::new(Config::default(), Look::default());
    app.dirty = false;
    app.refresh_due = false;
    app
}

#[test]
fn a_change_of_the_scale_to_draw_at_asks_for_a_redraw_and_nothing_else_does() {
    let mut app = app();
    assert_eq!(app.demand.scale(), Scale::Integer(1));
    // The same scale again, at the integer the app started with: nothing.
    app.rescale(|demand| demand.prefer_buffer_scale(1));
    assert!(!app.dirty && !app.refresh_due);
    // A new one redraws, and re-lays out (the icons decode again).
    app.rescale(|demand| demand.prefer_buffer_scale(2));
    assert!(app.dirty && app.refresh_due);
    assert_eq!(app.demand.scale(), Scale::Integer(2));
    app.dirty = false;
    app.refresh_due = false;
    // A compositor re-sends the same preference on every enter.
    app.rescale(|demand| demand.prefer_buffer_scale(2));
    assert!(!app.dirty && !app.refresh_due, "a repeat is not a change");
    // The fraction wins over the integer, and is a change.
    app.rescale(|demand| demand.prefer_fractional(210));
    assert!(app.dirty);
    assert_eq!(app.demand.scale(), Scale::Fractional(210));
    app.dirty = false;
    // A lower integer the fraction already covers changes nothing drawn.
    app.rescale(|demand| demand.set_output_scale(2));
    assert!(!app.dirty, "the fraction still wins");
    // A broken zero is ignored, not drawn at.
    app.rescale(|demand| demand.prefer_fractional(0));
    app.rescale(|demand| demand.prefer_buffer_scale(0));
    assert!(!app.dirty);
    assert_eq!(app.demand.scale(), Scale::Fractional(210));
}

#[test]
fn the_integer_scale_of_the_outputs_backs_up_a_compositor_that_sends_no_preference() {
    let mut app = app();
    app.rescale(|demand| demand.set_output_scale(2));
    assert!(app.dirty);
    assert_eq!(app.demand.scale(), Scale::Integer(2));
    app.rescale(|demand| demand.set_output_scale(-3));
    assert_eq!(
        app.demand.scale(),
        Scale::Integer(2),
        "a broken factor changes nothing"
    );
}

#[test]
fn a_new_scale_rebuilds_the_shadow_and_the_layers_before_anything_paints() {
    let mut app = App::new(
        Config::default(),
        Look {
            shadow: ShadowStyle::Soft,
            ..Look::default()
        },
    );
    let logical = app.shadow.as_ref().expect("a soft shadow").pad();
    assert_eq!(app.shadow_at.as_ref().map(|s| s.pad()), Some(logical), "1x");
    app.sync_scale(Scale::Fractional(180));
    assert_eq!(app.shadow_scale, Scale::Fractional(180));
    let scaled = app.shadow_at.as_ref().expect("still a shadow").pad();
    assert!(
        scaled.bottom > logical.bottom,
        "{scaled:?} against {logical:?}"
    );
    // The layout's room is the logical one: it does not follow the scale.
    assert_eq!(app.shadow.as_ref().map(|s| s.pad()), Some(logical));
    assert_eq!(app.motion.dev().scale(), Scale::Fractional(180));
    // The same scale again is free.
    app.sync_scale(Scale::Fractional(180));
    // And back.
    app.sync_scale(Scale::Integer(1));
    assert_eq!(app.shadow_at.as_ref().map(|s| s.pad()), Some(logical));
}

#[test]
fn a_rebuilt_motion_keeps_the_scale_it_draws_at() {
    use crate::config::Anchor;
    let mut app = App::new(
        Config::default(),
        Look {
            shadow: ShadowStyle::Soft,
            ..Look::default()
        },
    );
    app.sync_scale(Scale::Fractional(150));
    // A re-theme starts the motion again: at the same scale, or its layers
    // would be drawn at 1x on a 1.25x surface.
    app.retheme(Look::default());
    assert_eq!(app.motion.dev().scale(), Scale::Fractional(150));
    assert_eq!(app.shadow_scale, Scale::Fractional(150));
    // So does a new anchor.
    let config = Config {
        anchor: Anchor::BottomLeft,
        ..Config::default()
    };
    app.apply_config(config);
    assert_eq!(app.motion.dev().scale(), Scale::Fractional(150));
}

#[test]
fn a_leaving_card_does_not_outlive_a_scale_change() {
    use std::time::Instant;

    use crate::look::{CardMotion, MotionTokens};
    use crate::view::layout::Stack;
    use crate::view::motion::Motion;
    use scootui::motion::Curve;

    let tokens = MotionTokens {
        curve: Curve::EaseOut,
        duration_ms: 400,
        bounce: 1.0,
        enter: CardMotion::Fade,
        exit: CardMotion::Fade,
    };
    let notes = [crate::view::render::tests::note(1, "Leaving", "Body")];
    let live: Vec<&crate::model::store::Notification> = notes.iter().collect();
    let text = crate::view::render::tests::make_text();
    let config = Config::default();
    let stack = crate::view::layout::layout(&live, &config, &text, 13.0, 2_000);
    let mut motion = Motion::new(&tokens, config.anchor);
    // The card arrived long ago and now closes: a ghost.
    motion.retarget(&stack, &Stack::default_empty(), Instant::now());
    assert!(motion.active(), "a ghost fades out");
    // Its picture was painted with what the notification was; a repaint at
    // a new scale would not have it. It goes.
    motion.set_scale(Scale::Fractional(180));
    assert!(
        !motion.active(),
        "no repaint of a notification that is gone"
    );
}
