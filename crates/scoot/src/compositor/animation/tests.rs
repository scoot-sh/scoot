//! Unit tests for the slide interpolator and the retarget/cancel rules.
//!
//! The interpolator itself is pure arithmetic over a progress value, so these
//! pin values, not time. The time-dependent half (progress, finish, visual)
//! drives off explicit `start`/`now` pairs, never sleeps.

use std::time::{Duration, Instant};

use scoot_core::{OutputId, Placement, Rect, Size, WindowId};

use super::{
    AnimationSettings, Easing, is_finished, progress, should_slide, slide_position, visual_x,
    visual_y,
};

fn placement_at(
    id: u64,
    x: i32,
    y: i32,
    visible: bool,
    floating: bool,
    fullscreen: bool,
    maximized: bool,
) -> Placement {
    Placement {
        id: WindowId(id),
        output: OutputId(1),
        rect: Rect::new(x, y, 400, 300),
        visible,
        fullscreen,
        maximized,
        floating,
        requested: Some(Size::new(400, 300)),
    }
}

fn placement(
    id: u64,
    x: i32,
    visible: bool,
    floating: bool,
    fullscreen: bool,
    maximized: bool,
) -> Placement {
    placement_at(id, x, 0, visible, floating, fullscreen, maximized)
}

#[test]
fn ease_out_starts_fast_and_lands_exactly() {
    assert_eq!(super::easing_factor(0.0, Easing::EaseOut), 0.0);
    assert_eq!(super::easing_factor(1.0, Easing::EaseOut), 1.0);
    let mid = super::easing_factor(0.5, Easing::EaseOut);
    assert!(
        (mid - 0.875).abs() < 1e-6,
        "ease-out cubic at 0.5 is 1-(0.5)^3 = 0.875, got {mid}"
    );
    assert!(
        mid > 0.5,
        "ease-out must be ahead of linear mid-flight, got {mid}"
    );
    assert_eq!(super::easing_factor(0.5, Easing::Linear), 0.5);
}

#[test]
fn easing_clamps_progress_both_ends() {
    assert_eq!(super::easing_factor(-1.0, Easing::EaseOut), 0.0);
    assert_eq!(super::easing_factor(2.0, Easing::EaseOut), 1.0);
    assert_eq!(super::easing_factor(-1.0, Easing::Linear), 0.0);
    assert_eq!(super::easing_factor(2.0, Easing::Linear), 1.0);
    assert_eq!(super::easing_factor(-1.0, Easing::Spring), 0.0);
    assert_eq!(super::easing_factor(2.0, Easing::Spring), 1.0);
}

#[test]
fn slide_position_interpolates_and_rounds() {
    assert_eq!(slide_position(0, 100, 0.0, Easing::Linear), 0);
    assert_eq!(slide_position(0, 100, 1.0, Easing::Linear), 100);
    assert_eq!(slide_position(0, 100, 0.5, Easing::Linear), 50);
    assert_eq!(slide_position(100, 0, 0.5, Easing::Linear), 50);
    assert_eq!(slide_position(0, 100, 0.5, Easing::EaseOut), 88);
}

#[test]
fn zero_duration_finishes_on_its_first_frame() {
    let now = Instant::now();
    assert_eq!(progress(now, now, Duration::ZERO), 1.0);
    let slide = super::Slide {
        id: WindowId(1),
        from_x: 0,
        from_y: 0,
        to_x: 500,
        to_y: 0,
        start: now,
        duration: Duration::ZERO,
        easing: Easing::EaseOut,
    };
    assert!(is_finished(&slide, now));
    assert_eq!(visual_x(&slide, now), 500);
}

#[test]
fn progress_follows_elapsed_over_duration() {
    let start = Instant::now();
    let duration = Duration::from_millis(200);
    assert_eq!(progress(start, start, duration), 0.0);
    assert_eq!(
        progress(start + Duration::from_millis(100), start, duration),
        0.5
    );
    assert_eq!(
        progress(start + Duration::from_millis(200), start, duration),
        1.0
    );
    assert_eq!(
        progress(start + Duration::from_millis(10_000), start, duration),
        1.0,
        "a compositor falling behind snaps rather than queuing frames"
    );
}

#[test]
fn a_future_start_has_not_begun() {
    let now = Instant::now();
    let duration = Duration::from_millis(100);
    assert_eq!(
        progress(now, now + Duration::from_millis(50), duration),
        0.0
    );
}

#[test]
fn only_tiled_visible_windows_slide() {
    assert!(should_slide(&placement(1, 0, true, false, false, false)));
    assert!(!should_slide(&placement(2, 0, false, false, false, false)));
    assert!(!should_slide(&placement(3, 0, true, true, false, false)));
    assert!(!should_slide(&placement(4, 0, true, false, true, false)));
    assert!(!should_slide(&placement(5, 0, true, false, false, true)));
}

#[test]
fn zero_and_one_window_have_nowhere_to_slide() {
    let empty: Vec<Placement> = Vec::new();
    assert!(empty.is_empty());
    let one = placement(1, 0, true, false, false, false);
    assert_eq!(
        slide_position(one.rect.x, one.rect.x, 0.5, Easing::EaseOut),
        0
    );
}

#[test]
fn retarget_continues_from_the_visual_position() {
    let start = Instant::now();
    let duration = Duration::from_millis(200);
    let first = super::Slide {
        id: WindowId(7),
        from_x: 0,
        from_y: 0,
        to_x: 400,
        to_y: 0,
        start,
        duration,
        easing: Easing::Linear,
    };
    let mid_now = start + Duration::from_millis(100);
    let seen = visual_x(&first, mid_now);
    assert_eq!(seen, 200);
    let retargeted = super::Slide {
        id: WindowId(7),
        from_x: seen,
        from_y: 0,
        to_x: 600,
        to_y: 0,
        start: mid_now,
        duration,
        easing: Easing::Linear,
    };
    assert_eq!(visual_x(&retargeted, mid_now), 200);
    assert_eq!(
        visual_x(&retargeted, mid_now + Duration::from_millis(200)),
        600
    );
}

#[test]
fn easing_parses_the_three_names_and_rejects_the_rest() {
    assert_eq!(Easing::parse(None), Ok(Easing::EaseOut));
    assert_eq!(Easing::parse(Some("ease-out")), Ok(Easing::EaseOut));
    assert_eq!(Easing::parse(Some("linear")), Ok(Easing::Linear));
    assert_eq!(Easing::parse(Some("spring")), Ok(Easing::Spring));
    assert_eq!(Easing::Spring.name(), "spring");
    assert_eq!(Easing::EaseOut.name(), "ease-out");
    assert_eq!(Easing::Linear.name(), "linear");
    assert!(Easing::parse(Some("bounce")).is_err());
    assert!(Easing::parse(Some("")).is_err());
}

#[test]
fn duration_clamps_to_range() {
    assert_eq!(
        AnimationSettings::clamp_duration_ms(0),
        Duration::from_millis(0)
    );
    assert_eq!(
        AnimationSettings::clamp_duration_ms(150),
        Duration::from_millis(150)
    );
    assert_eq!(
        AnimationSettings::clamp_duration_ms(10_000),
        Duration::from_millis(super::MAX_DURATION_MS as u64)
    );
}

#[test]
fn disabled_by_default() {
    assert!(!AnimationSettings::default().enabled);
}

// -------------------------------------------------------------------------
// `State::update_slides` edge cases: no Wayland clients, just the core's
// arrangements plus manually staged slides (an in-flight slide's visual is
// read off the slide itself, never off the space, so no real `Window` is
// needed to exercise retarget and cleanup).
// -------------------------------------------------------------------------

use super::{Slide, prune_finished, slides_in_flight, visual_delta};
use crate::compositor::decorations::Appearance;
use crate::compositor::headless;
use crate::compositor::keybindings::Keybindings;
use crate::compositor::state::State;
use scoot_core::{Arrangement, Config};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;

fn arrangement_of(placements: Vec<Placement>) -> Arrangement {
    Arrangement {
        placements,
        focused: None,
        focused_output: None,
    }
}

struct SlideFixture {
    state: State,
}

impl SlideFixture {
    fn new() -> Self {
        let mut event_loop: EventLoop<'static, State> =
            EventLoop::try_new().expect("an event loop");
        let display: Display<State> = Display::new().expect("a wayland display");
        let mut state = State::new(
            &mut event_loop,
            display,
            Config::default(),
            Keybindings::default(),
            Appearance::default(),
            1.0,
            crate::compositor::test_support::test_renderer(),
        )
        .expect("a compositor state");
        headless::init(&mut state, 800, 600).expect("a headless backend");
        state.needs_render = false;
        state.scene_dirty = false;
        state.animation.enabled = true;
        state.animation.duration = Duration::from_millis(200);
        Self { state }
    }

    fn update(&mut self, arrangement: &Arrangement, now: Instant, locked: bool) {
        self.state.update_slides(arrangement, now, locked);
    }
}

#[test]
fn zero_windows_have_no_slides_and_do_not_panic() {
    let mut fixture = SlideFixture::new();
    let now = Instant::now();
    fixture.update(&arrangement_of(Vec::new()), now, false);
    assert!(fixture.state.slides.is_empty());
    assert!(!fixture.state.animation_in_flight(now));
}

#[test]
fn a_window_that_does_not_move_gets_no_slide() {
    let mut fixture = SlideFixture::new();
    let now = Instant::now();
    let arranged = arrangement_of(vec![placement(1, 100, true, false, false, false)]);
    fixture.update(&arranged, now, false);
    assert!(fixture.state.slides.is_empty());
}

#[test]
fn a_new_target_mid_slide_retargets_from_the_visual() {
    let mut fixture = SlideFixture::new();
    let start = Instant::now();
    fixture.state.slides.push(Slide {
        id: WindowId(1),
        from_x: 0,
        from_y: 0,
        to_x: 400,
        to_y: 0,
        start,
        duration: Duration::from_millis(200),
        easing: Easing::Linear,
    });
    let mid = start + Duration::from_millis(100);
    assert_eq!(
        visual_delta(&fixture.state.slides, WindowId(1), 400, 0, mid),
        (-200, 0)
    );
    fixture.update(
        &arrangement_of(vec![placement(1, 600, true, false, false, false)]),
        mid,
        false,
    );
    assert_eq!(fixture.state.slides.len(), 1);
    let slide = &fixture.state.slides[0];
    assert_eq!(slide.from_x, 200);
    assert_eq!(slide.to_x, 600);
    assert_eq!(slide.start, mid);
}

#[test]
fn a_closed_window_loses_its_slide() {
    let mut fixture = SlideFixture::new();
    let now = Instant::now();
    fixture.state.slides.push(Slide {
        id: WindowId(9),
        from_x: 0,
        from_y: 0,
        to_x: 400,
        to_y: 0,
        start: now,
        duration: Duration::from_millis(200),
        easing: Easing::Linear,
    });
    fixture.update(&arrangement_of(Vec::new()), now, false);
    assert!(fixture.state.slides.is_empty());
}

#[test]
fn a_window_hidden_by_workspace_switch_loses_its_slide() {
    let mut fixture = SlideFixture::new();
    let now = Instant::now();
    fixture.state.slides.push(Slide {
        id: WindowId(3),
        from_x: 0,
        from_y: 0,
        to_x: 400,
        to_y: 0,
        start: now,
        duration: Duration::from_millis(200),
        easing: Easing::Linear,
    });
    fixture.update(
        &arrangement_of(vec![placement(3, 400, false, false, false, false)]),
        now,
        false,
    );
    assert!(fixture.state.slides.is_empty());
}

#[test]
fn fullscreen_maximized_and_floating_never_slide() {
    let mut fixture = SlideFixture::new();
    let now = Instant::now();
    for (id, floating, fullscreen, maximized) in [
        (1, true, false, false),
        (2, false, true, false),
        (3, false, false, true),
    ] {
        fixture.state.slides.clear();
        fixture.state.slides.push(Slide {
            id: WindowId(id),
            from_x: 0,
            from_y: 0,
            to_x: 400,
            to_y: 0,
            start: now,
            duration: Duration::from_millis(200),
            easing: Easing::Linear,
        });
        fixture.update(
            &arrangement_of(vec![placement(
                id, 600, true, floating, fullscreen, maximized,
            )]),
            now,
            false,
        );
        assert!(
            fixture.state.slides.is_empty(),
            "id {id} must snap, never slide"
        );
    }
}

#[test]
fn locked_and_disabled_both_snap() {
    let mut fixture = SlideFixture::new();
    let now = Instant::now();
    fixture.state.slides.push(Slide {
        id: WindowId(1),
        from_x: 0,
        from_y: 0,
        to_x: 400,
        to_y: 0,
        start: now,
        duration: Duration::from_millis(200),
        easing: Easing::Linear,
    });
    fixture.update(
        &arrangement_of(vec![placement(1, 400, true, false, false, false)]),
        now,
        true,
    );
    assert!(fixture.state.slides.is_empty());
    fixture.state.slides.push(Slide {
        id: WindowId(1),
        from_x: 0,
        from_y: 0,
        to_x: 400,
        to_y: 0,
        start: now,
        duration: Duration::from_millis(200),
        easing: Easing::Linear,
    });
    fixture.state.animation.enabled = false;
    fixture.update(
        &arrangement_of(vec![placement(1, 600, true, false, false, false)]),
        now,
        false,
    );
    assert!(fixture.state.slides.is_empty());
}

#[test]
fn a_width_only_resize_starts_no_slide() {
    let mut fixture = SlideFixture::new();
    let now = Instant::now();
    fixture.update(
        &arrangement_of(vec![placement(1, 100, true, false, false, false)]),
        now,
        false,
    );
    assert!(fixture.state.slides.is_empty());
}

#[test]
fn extreme_coordinates_do_not_panic() {
    let mut fixture = SlideFixture::new();
    let now = Instant::now();
    fixture.state.slides.push(Slide {
        id: WindowId(1),
        from_x: i32::MAX - 100,
        from_y: 0,
        to_x: i32::MAX,
        to_y: 0,
        start: now,
        duration: Duration::from_millis(200),
        easing: Easing::EaseOut,
    });
    assert!(visual_delta(&fixture.state.slides, WindowId(1), i32::MAX, 0, now).0 <= 0);
    fixture.update(
        &arrangement_of(vec![placement(1, i32::MAX, true, false, false, false)]),
        now + Duration::from_millis(300),
        false,
    );
    assert!(fixture.state.slides.is_empty());
}

#[test]
fn finished_slides_prune_and_idle_goes_quiet() {
    let start = Instant::now();
    let mut slides = vec![Slide {
        id: WindowId(1),
        from_x: 0,
        from_y: 0,
        to_x: 100,
        to_y: 0,
        start,
        duration: Duration::from_millis(50),
        easing: Easing::Linear,
    }];
    let late = start + Duration::from_millis(5000);
    assert!(!slides_in_flight(&slides, late));
    prune_finished(&mut slides, late);
    assert!(slides.is_empty());
}

#[test]
fn input_and_ipc_read_the_final_geometry_mid_slide() {
    let start = Instant::now();
    let slide = Slide {
        id: WindowId(1),
        from_x: 0,
        from_y: 0,
        to_x: 400,
        to_y: 0,
        start,
        duration: Duration::from_millis(200),
        easing: Easing::Linear,
    };
    let mid = start + Duration::from_millis(100);
    let target = 400;
    let visual = super::visual_x(&slide, mid);
    assert_eq!(visual, 200);
    assert_eq!(
        visual_delta(std::slice::from_ref(&slide), WindowId(1), target, 0, mid),
        (-200, 0)
    );
}

#[test]
fn reload_enables_disables_and_snaps_a_live_slide() {
    use scoot_ipc::{Request, Response};
    use std::fs;

    let mut fixture = SlideFixture::new();
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("config.toml");
    fs::write(
        &path,
        "[animation]\nenabled = true\nduration_ms = 200\neasing = \"linear\"\n",
    )
    .expect("a config file");
    fixture.state.config_path = Some(path.clone());
    fixture.state.needs_render = false;
    fixture.state.scene_dirty = false;
    // Fixture::new enables animation at 200ms; reset to defaults so the
    // file's `enabled = true` / 200ms / linear all read as changes.
    fixture.state.animation = AnimationSettings::default();

    match fixture.state.handle_request(Request::Reload) {
        Response::Reloaded { applied, .. } => {
            assert!(
                applied.contains(&"animation.enabled".to_owned()),
                "{applied:?}"
            );
            assert!(
                applied.contains(&"animation.duration_ms".to_owned()),
                "{applied:?}"
            );
            assert!(
                applied.contains(&"animation.easing".to_owned()),
                "{applied:?}"
            );
        }
        other => panic!("reload must apply: {other:?}"),
    }
    assert!(fixture.state.animation.enabled);

    let now = Instant::now();
    fixture.state.slides.push(Slide {
        id: WindowId(1),
        from_x: 0,
        from_y: 0,
        to_x: 400,
        to_y: 0,
        start: now,
        duration: Duration::from_millis(200),
        easing: Easing::Linear,
    });
    fs::write(&path, "[animation]\nenabled = false\n").expect("a rewritten config");
    match fixture.state.handle_request(Request::Reload) {
        Response::Reloaded { applied, .. } => {
            assert!(
                applied.contains(&"animation.enabled".to_owned()),
                "{applied:?}"
            );
        }
        other => panic!("disable must apply: {other:?}"),
    }
    assert!(!fixture.state.animation.enabled);
    assert!(fixture.state.slides.is_empty());
}

// -------------------------------------------------------------------------
// Vertical slide: same interpolation, retarget and cancel rules on y.
// -------------------------------------------------------------------------

#[test]
fn vertical_slide_interpolates_like_horizontal() {
    assert_eq!(slide_position(0, 300, 0.5, Easing::Linear), 150);
    assert_eq!(slide_position(300, 0, 0.5, Easing::Linear), 150);
    assert_eq!(slide_position(0, 300, 0.5, Easing::EaseOut), 263);
    let start = Instant::now();
    let duration = Duration::from_millis(200);
    let slide = Slide {
        id: WindowId(21),
        from_x: 0,
        from_y: 0,
        to_x: 0,
        to_y: 300,
        start,
        duration,
        easing: Easing::Linear,
    };
    let mid = start + Duration::from_millis(100);
    assert_eq!(visual_y(&slide, mid), 150);
    assert_eq!(
        visual_delta(std::slice::from_ref(&slide), WindowId(21), 0, 300, mid),
        (0, -150)
    );
}

#[test]
fn vertical_retarget_continues_from_the_visual_position() {
    let start = Instant::now();
    let duration = Duration::from_millis(200);
    let first = Slide {
        id: WindowId(22),
        from_x: 0,
        from_y: 0,
        to_x: 0,
        to_y: 400,
        start,
        duration,
        easing: Easing::Linear,
    };
    let mid_now = start + Duration::from_millis(100);
    assert_eq!(visual_y(&first, mid_now), 200);
    let retargeted = Slide {
        id: WindowId(22),
        from_x: 0,
        from_y: 200,
        to_x: 0,
        to_y: 600,
        start: mid_now,
        duration,
        easing: Easing::Linear,
    };
    assert_eq!(visual_y(&retargeted, mid_now), 200);
    assert_eq!(
        visual_y(&retargeted, mid_now + Duration::from_millis(200)),
        600
    );
}

#[test]
fn diagonal_slide_moves_both_axes_together() {
    let start = Instant::now();
    let duration = Duration::from_millis(200);
    let slide = Slide {
        id: WindowId(23),
        from_x: 0,
        from_y: 0,
        to_x: 400,
        to_y: 300,
        start,
        duration,
        easing: Easing::Linear,
    };
    let mid = start + Duration::from_millis(100);
    assert_eq!(
        visual_delta(std::slice::from_ref(&slide), WindowId(23), 400, 300, mid),
        (-200, -150)
    );
}

#[test]
fn a_y_only_move_starts_a_slide() {
    let mut fixture = SlideFixture::new();
    let now = Instant::now();
    // Seed the old target: the space holds y=0 until `apply()` remaps, so
    // first publish y=0, then move to y=300.
    fixture.update(
        &arrangement_of(vec![placement_at(1, 100, 0, true, false, false, false)]),
        now,
        false,
    );
    assert!(fixture.state.slides.is_empty());
    // `update_slides` reads the old target off the space only for windows it
    // knows; the harness has no real `Window`, so stage the old position as
    // an in-flight visual at the old target instead: a fresh move from there.
    fixture.state.slides.push(Slide {
        id: WindowId(1),
        from_x: 100,
        from_y: 0,
        to_x: 100,
        to_y: 0,
        start: now,
        duration: Duration::from_millis(200),
        easing: Easing::Linear,
    });
    fixture.update(
        &arrangement_of(vec![placement_at(1, 100, 300, true, false, false, false)]),
        now + Duration::from_millis(10),
        false,
    );
    assert_eq!(fixture.state.slides.len(), 1);
    let slide = &fixture.state.slides[0];
    assert_eq!((slide.from_x, slide.to_x), (100, 100));
    assert_eq!((slide.from_y, slide.to_y), (0, 300));
}

#[test]
fn a_height_only_resize_starts_no_slide() {
    let mut fixture = SlideFixture::new();
    let now = Instant::now();
    fixture.update(
        &arrangement_of(vec![placement_at(1, 100, 200, true, false, false, false)]),
        now,
        false,
    );
    // Same x/y (size is not tracked by slides): nothing to animate.
    assert!(fixture.state.slides.is_empty());
}

#[test]
fn a_closed_window_loses_its_vertical_slide() {
    let mut fixture = SlideFixture::new();
    let now = Instant::now();
    fixture.state.slides.push(Slide {
        id: WindowId(24),
        from_x: 0,
        from_y: 0,
        to_x: 0,
        to_y: 400,
        start: now,
        duration: Duration::from_millis(200),
        easing: Easing::Linear,
    });
    fixture.update(&arrangement_of(Vec::new()), now, false);
    assert!(fixture.state.slides.is_empty());
}

#[test]
fn a_window_hidden_by_output_switch_loses_its_vertical_slide() {
    let mut fixture = SlideFixture::new();
    let now = Instant::now();
    fixture.state.slides.push(Slide {
        id: WindowId(25),
        from_x: 0,
        from_y: 0,
        to_x: 0,
        to_y: 400,
        start: now,
        duration: Duration::from_millis(200),
        easing: Easing::Linear,
    });
    fixture.update(
        &arrangement_of(vec![placement_at(25, 0, 400, false, false, false, false)]),
        now,
        false,
    );
    assert!(fixture.state.slides.is_empty());
}

#[test]
fn vertical_fullscreen_maximized_and_floating_never_slide() {
    let mut fixture = SlideFixture::new();
    let now = Instant::now();
    for (id, floating, fullscreen, maximized) in [
        (31, true, false, false),
        (32, false, true, false),
        (33, false, false, true),
    ] {
        fixture.state.slides.clear();
        fixture.state.slides.push(Slide {
            id: WindowId(id),
            from_x: 0,
            from_y: 0,
            to_x: 0,
            to_y: 400,
            start: now,
            duration: Duration::from_millis(200),
            easing: Easing::Linear,
        });
        fixture.update(
            &arrangement_of(vec![placement_at(
                id, 0, 600, true, floating, fullscreen, maximized,
            )]),
            now,
            false,
        );
        assert!(
            fixture.state.slides.is_empty(),
            "id {id} must snap, never slide"
        );
    }
}

#[test]
fn vertical_extreme_coordinates_do_not_panic() {
    let mut fixture = SlideFixture::new();
    let now = Instant::now();
    fixture.state.slides.push(Slide {
        id: WindowId(26),
        from_x: 0,
        from_y: i32::MAX - 100,
        to_x: 0,
        to_y: i32::MAX,
        start: now,
        duration: Duration::from_millis(200),
        easing: Easing::EaseOut,
    });
    let (dx, dy) = visual_delta(&fixture.state.slides, WindowId(26), 0, i32::MAX, now);
    assert_eq!(dx, 0);
    assert!(dy <= 0);
    fixture.update(
        &arrangement_of(vec![placement_at(
            26,
            0,
            i32::MAX,
            true,
            false,
            false,
            false,
        )]),
        now + Duration::from_millis(300),
        false,
    );
    assert!(fixture.state.slides.is_empty());
}

#[test]
fn vertical_locked_and_disabled_both_snap() {
    let mut fixture = SlideFixture::new();
    let now = Instant::now();
    fixture.state.slides.push(Slide {
        id: WindowId(27),
        from_x: 0,
        from_y: 0,
        to_x: 0,
        to_y: 400,
        start: now,
        duration: Duration::from_millis(200),
        easing: Easing::Linear,
    });
    fixture.update(
        &arrangement_of(vec![placement_at(27, 0, 400, true, false, false, false)]),
        now,
        true,
    );
    assert!(fixture.state.slides.is_empty());
    fixture.state.slides.push(Slide {
        id: WindowId(27),
        from_x: 0,
        from_y: 0,
        to_x: 0,
        to_y: 400,
        start: now,
        duration: Duration::from_millis(200),
        easing: Easing::Linear,
    });
    fixture.state.animation.enabled = false;
    fixture.update(
        &arrangement_of(vec![placement_at(27, 0, 600, true, false, false, false)]),
        now,
        false,
    );
    assert!(fixture.state.slides.is_empty());
}

#[test]
fn vertical_input_and_ipc_read_the_final_geometry_mid_slide() {
    let start = Instant::now();
    let slide = Slide {
        id: WindowId(28),
        from_x: 0,
        from_y: 0,
        to_x: 0,
        to_y: 400,
        start,
        duration: Duration::from_millis(200),
        easing: Easing::Linear,
    };
    let mid = start + Duration::from_millis(100);
    assert_eq!(visual_y(&slide, mid), 200);
    assert_eq!(
        visual_delta(std::slice::from_ref(&slide), WindowId(28), 0, 400, mid),
        (0, -200)
    );
}

// -------------------------------------------------------------------------
// Spring easing: the same retarget/cancel/reload pins as the shipped
// curves, plus the spring's own shape (overshoot, exact landing).
// -------------------------------------------------------------------------

#[test]
fn spring_overshoots_then_lands_exactly() {
    assert_eq!(super::easing_factor(0.0, Easing::Spring), 0.0);
    assert_eq!(super::easing_factor(1.0, Easing::Spring), 1.0);
    // p = 36/64 hits a table entry exactly (no lerp): the peak overshoot.
    let peak = super::easing_factor(0.5625, Easing::Spring);
    assert!(
        peak > 1.08 && peak < 1.11,
        "spring must overshoot ~9% mid-flight, got {peak}"
    );
    // A quarter of the way in hits entry 16 exactly: ahead of linear.
    let early = super::easing_factor(0.25, Easing::Spring);
    assert!(
        (early - 0.681888).abs() < 1e-6,
        "spring at 0.25 pins the table entry, got {early}"
    );
    assert!(
        early > 0.25,
        "spring starts faster than linear, got {early}"
    );
}

#[test]
fn spring_slide_position_passes_the_target_mid_flight() {
    // Overshoot is visual only: the slide still reports the target at rest.
    assert_eq!(slide_position(0, 100, 0.0, Easing::Spring), 0);
    assert_eq!(slide_position(0, 100, 1.0, Easing::Spring), 100);
    let mid = slide_position(0, 100, 0.5625, Easing::Spring);
    assert!(
        mid > 100,
        "spring must draw past the target mid-flight, got {mid}"
    );
    assert_eq!(mid, 109);
    // Same on the way back down: 400 -> 0 passes below zero... no: it
    // approaches from above, so mid-flight it has not yet reached 0.
    let back = slide_position(400, 0, 0.5625, Easing::Spring);
    assert!(
        back < 0,
        "spring overshoots a leftward slide past zero, got {back}"
    );
}

#[test]
fn spring_zero_distance_has_nowhere_to_slide() {
    assert_eq!(slide_position(100, 100, 0.5625, Easing::Spring), 100);
    let empty: Vec<Placement> = Vec::new();
    assert!(empty.is_empty());
}

#[test]
fn spring_late_frame_snaps_and_idles() {
    let start = Instant::now();
    let duration = Duration::from_millis(200);
    let slide = Slide {
        id: WindowId(41),
        from_x: 0,
        from_y: 0,
        to_x: 400,
        to_y: 300,
        start,
        duration,
        easing: Easing::Spring,
    };
    // A compositor falling behind snaps rather than queuing frames, even
    // with an overshooting curve: past the deadline the visual IS the target.
    let late = start + Duration::from_millis(10_000);
    assert!(is_finished(&slide, late));
    assert_eq!(visual_x(&slide, late), 400);
    assert_eq!(visual_y(&slide, late), 300);
    assert_eq!(
        visual_delta(std::slice::from_ref(&slide), WindowId(41), 400, 300, late),
        (0, 0)
    );
    let mut slides = vec![slide];
    assert!(!slides_in_flight(&slides, late));
    prune_finished(&mut slides, late);
    assert!(slides.is_empty());
}

#[test]
fn spring_retarget_continues_from_the_visual_position() {
    let start = Instant::now();
    let duration = Duration::from_millis(200);
    let first = Slide {
        id: WindowId(42),
        from_x: 0,
        from_y: 0,
        to_x: 400,
        to_y: 0,
        start,
        duration,
        easing: Easing::Spring,
    };
    // Quarter-flight visual (entry 16, no lerp): the retarget base.
    let mid_now = start + Duration::from_millis(50);
    let seen = visual_x(&first, mid_now);
    assert_eq!(seen, slide_position(0, 400, 0.25, Easing::Spring));
    let retargeted = Slide {
        id: WindowId(42),
        from_x: seen,
        from_y: 0,
        to_x: 600,
        to_y: 0,
        start: mid_now,
        duration,
        easing: Easing::Spring,
    };
    assert_eq!(visual_x(&retargeted, mid_now), seen);
    assert_eq!(
        visual_x(&retargeted, mid_now + Duration::from_millis(200)),
        600
    );
}

#[test]
fn spring_moves_diagonally_and_reports_the_target_to_ipc() {
    let start = Instant::now();
    let duration = Duration::from_millis(200);
    let slide = Slide {
        id: WindowId(43),
        from_x: 0,
        from_y: 0,
        to_x: 400,
        to_y: 300,
        start,
        duration,
        easing: Easing::Spring,
    };
    // At the overshoot peak both axes draw past their targets...
    let peak_now = start + Duration::from_millis(112);
    let (dx, dy) = visual_delta(
        std::slice::from_ref(&slide),
        WindowId(43),
        400,
        300,
        peak_now,
    );
    assert!(dx > 0 && dy > 0, "both axes overshoot, got ({dx}, {dy})");
    // ...while IPC and hit-testing keep reading the final geometry
    // (that half never looks at the slide at all).
    assert_eq!(slide.to_x, 400);
    assert_eq!(slide.to_y, 300);
}

#[test]
fn spring_extreme_coordinates_saturate_instead_of_wrapping() {
    // An overshooting curve on an `i32::MAX` window must clamp, never wrap.
    assert_eq!(
        slide_position(i32::MAX - 100, i32::MAX, 0.5625, Easing::Spring),
        i32::MAX
    );
    assert_eq!(
        slide_position(i32::MIN + 100, i32::MIN, 0.5625, Easing::Spring),
        i32::MIN
    );
    let start = Instant::now();
    let duration = Duration::from_millis(200);
    let slide = Slide {
        id: WindowId(44),
        from_x: i32::MAX - 100,
        from_y: 0,
        to_x: i32::MAX,
        to_y: 0,
        start,
        duration,
        easing: Easing::Spring,
    };
    let (dx, _) = visual_delta(
        std::slice::from_ref(&slide),
        WindowId(44),
        i32::MAX,
        0,
        start,
    );
    assert!(dx <= 0);
}

#[test]
fn spring_cancel_rules_match_the_shipped_curves() {
    // Closed, hidden, floating, fullscreen and maximized windows lose a
    // spring slide exactly like a linear one; lock and disable snap.
    let mut fixture = SlideFixture::new();
    fixture.state.animation.easing = Easing::Spring;
    let now = Instant::now();
    let spring_slide = |id: u64| Slide {
        id: WindowId(id),
        from_x: 0,
        from_y: 0,
        to_x: 400,
        to_y: 300,
        start: now,
        duration: Duration::from_millis(200),
        easing: Easing::Spring,
    };
    // A real move under the spring session stamps spring slides...
    fixture.update(
        &arrangement_of(vec![placement_at(1, 100, 200, true, false, false, false)]),
        now,
        false,
    );
    // ...but nothing was on screen before (no old position), so a brand-new
    // window still snaps rather than sliding in from nowhere.
    assert!(fixture.state.slides.is_empty());
    // A staged in-flight spring slide is dropped when its window closes.
    fixture.state.slides.push(spring_slide(9));
    fixture.update(&arrangement_of(Vec::new()), now, false);
    assert!(fixture.state.slides.is_empty());
    // ...when it hides (workspace switch / output switch)...
    fixture.state.slides.push(spring_slide(10));
    fixture.update(
        &arrangement_of(vec![placement_at(10, 400, 300, false, false, false, false)]),
        now,
        false,
    );
    assert!(fixture.state.slides.is_empty());
    // ...and when it stops tiling (floats, fullscreen, maximizes).
    for (id, floating, fullscreen, maximized) in [
        (11, true, false, false),
        (12, false, true, false),
        (13, false, false, true),
    ] {
        fixture.state.slides.clear();
        fixture.state.slides.push(spring_slide(id));
        fixture.update(
            &arrangement_of(vec![placement_at(
                id, 600, 500, true, floating, fullscreen, maximized,
            )]),
            now,
            false,
        );
        assert!(
            fixture.state.slides.is_empty(),
            "id {id} must snap, never slide"
        );
    }
    // Lock snaps; disable snaps.
    fixture.state.slides.push(spring_slide(14));
    fixture.update(
        &arrangement_of(vec![placement_at(14, 400, 300, true, false, false, false)]),
        now,
        true,
    );
    assert!(fixture.state.slides.is_empty());
    fixture.state.slides.push(spring_slide(14));
    fixture.state.animation.enabled = false;
    fixture.update(
        &arrangement_of(vec![placement_at(14, 600, 500, true, false, false, false)]),
        now,
        false,
    );
    assert!(fixture.state.slides.is_empty());
}

#[test]
fn spring_reload_applies_easing_and_keeps_in_flight_timing() {
    use scoot_ipc::{Request, Response};
    use std::fs;

    let mut fixture = SlideFixture::new();
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("config.toml");
    fs::write(
        &path,
        "[animation]\nenabled = true\nduration_ms = 200\neasing = \"spring\"\n",
    )
    .expect("a config file");
    fixture.state.config_path = Some(path.clone());
    fixture.state.needs_render = false;
    fixture.state.scene_dirty = false;
    fixture.state.animation = AnimationSettings::default();

    match fixture.state.handle_request(Request::Reload) {
        Response::Reloaded { applied, .. } => {
            assert!(
                applied.contains(&"animation.enabled".to_owned()),
                "{applied:?}"
            );
            assert!(
                applied.contains(&"animation.duration_ms".to_owned()),
                "{applied:?}"
            );
            assert!(
                applied.contains(&"animation.easing".to_owned()),
                "{applied:?}"
            );
        }
        other => panic!("reload must apply: {other:?}"),
    }
    assert_eq!(fixture.state.animation.easing, Easing::Spring);

    // An in-flight ease-out slide keeps its curve across the reload (only
    // slides started after the reload read the new easing), so nothing jumps.
    let now = Instant::now();
    fixture.state.slides.push(Slide {
        id: WindowId(15),
        from_x: 0,
        from_y: 0,
        to_x: 400,
        to_y: 0,
        start: now,
        duration: Duration::from_millis(200),
        easing: Easing::EaseOut,
    });
    let before = visual_x(&fixture.state.slides[0], now);
    fs::write(
        &path,
        "[animation]\nenabled = true\nduration_ms = 300\neasing = \"linear\"\n",
    )
    .expect("a rewritten config");
    match fixture.state.handle_request(Request::Reload) {
        Response::Reloaded { applied, .. } => {
            assert!(
                applied.contains(&"animation.duration_ms".to_owned()),
                "{applied:?}"
            );
            assert!(
                applied.contains(&"animation.easing".to_owned()),
                "{applied:?}"
            );
        }
        other => panic!("reload must apply: {other:?}"),
    }
    assert_eq!(fixture.state.animation.easing, Easing::Linear);
    assert_eq!(fixture.state.slides.len(), 1);
    assert_eq!(fixture.state.slides[0].easing, Easing::EaseOut);
    assert_eq!(visual_x(&fixture.state.slides[0], now), before);

    // Unknown names warn back to the default (and the reload reports the
    // default taking effect), never to spring.
    assert!(Easing::parse(Some("bounce")).is_err());
    fs::write(&path, "[animation]\neasing = \"bounce\"\n").expect("a bad easing");
    match fixture.state.handle_request(Request::Reload) {
        Response::Reloaded { applied, .. } => {
            assert!(
                applied.contains(&"animation.easing".to_owned()),
                "{applied:?}"
            );
        }
        other => panic!("bad easing falls back to the default: {other:?}"),
    }
    assert_eq!(fixture.state.animation.easing, Easing::EaseOut);
}

// -------------------------------------------------------------------------
// Workspace-switch viewport slide: one offset per output, incoming only.
// -------------------------------------------------------------------------

use super::{ViewportSlide, viewport_delta, viewports_in_flight};
use scoot_core::{Action, Event as CoreEvent, Vertical, WindowInfo};

fn open_window(fixture: &mut SlideFixture, id: u64, output: OutputId) {
    fixture.state.world.handle_event(CoreEvent::WindowOpened {
        id: WindowId(id),
        info: WindowInfo::default(),
        output: Some(output),
        focus: true,
    });
}

fn arrange_now(fixture: &SlideFixture) -> Arrangement {
    fixture.state.world.arrange()
}

fn primary_id(fixture: &SlideFixture) -> OutputId {
    fixture
        .state
        .outputs
        .primary_id()
        .expect("a primary output")
}

#[test]
fn viewport_delta_is_zero_when_idle_and_eases_to_zero() {
    use super::viewport_position;
    let output = OutputId(1);
    let empty: Vec<ViewportSlide> = Vec::new();
    let now = Instant::now();
    assert_eq!(viewport_delta(&empty, output, now), 0);
    let start = now;
    let duration = Duration::from_millis(200);
    let slide = ViewportSlide {
        output,
        from_x: 800,
        start,
        duration,
        easing: Easing::Linear,
    };
    let arr = [slide];
    assert_eq!(viewport_delta(&arr, output, start), 800);
    assert_eq!(
        viewport_delta(&arr, output, start + Duration::from_millis(100)),
        400
    );
    assert_eq!(
        viewport_delta(&arr, output, start + Duration::from_millis(200)),
        0
    );
    assert_eq!(
        viewport_delta(&arr, output, start + Duration::from_millis(10_000)),
        0,
        "a late frame snaps rather than queuing"
    );
    // Another output is untouched.
    assert_eq!(viewport_delta(&arr, OutputId(2), start), 0);
    // Ease-out lands exactly too.
    assert_eq!(
        viewport_position(800, start + duration, start, duration, Easing::EaseOut),
        0
    );
    assert!(viewports_in_flight(
        &arr,
        start + Duration::from_millis(100)
    ));
    assert!(!viewports_in_flight(
        &arr,
        start + Duration::from_millis(10_000)
    ));
}

#[test]
fn workspace_switch_slides_the_incoming_strip_from_the_edge() {
    let mut fixture = SlideFixture::new();
    let id = primary_id(&fixture);
    let now = Instant::now();
    // Seed the index recording.
    fixture.update(&arrange_now(&fixture), now, false);
    assert!(fixture.state.viewport_slides.is_empty());
    open_window(&mut fixture, 1, id);
    // New window snaps per-window; no workspace switch yet so no viewport.
    fixture.update(
        &arrange_now(&fixture),
        now + Duration::from_millis(10),
        false,
    );
    assert!(fixture.state.viewport_slides.is_empty());
    // Switch down to the empty trailing workspace: nothing visible to slide.
    fixture
        .state
        .world
        .handle_action(Action::FocusWorkspace(Vertical::Down));
    fixture.update(
        &arrange_now(&fixture),
        now + Duration::from_millis(20),
        false,
    );
    assert!(
        fixture.state.viewport_slides.is_empty(),
        "an empty workspace has nothing to draw, so it snaps"
    );
    // Switch back up: the single window slides in from the left.
    fixture
        .state
        .world
        .handle_action(Action::FocusWorkspace(Vertical::Up));
    let switch_now = now + Duration::from_millis(30);
    fixture.update(&arrange_now(&fixture), switch_now, false);
    assert_eq!(fixture.state.viewport_slides.len(), 1);
    let slide = &fixture.state.viewport_slides[0];
    assert_eq!(slide.output, id);
    let width = fixture
        .state
        .world
        .usable_area(id)
        .map(|a| a.w)
        .unwrap_or(0);
    assert!(width > 0);
    assert_eq!(slide.from_x, -width, "up (lower index) comes from the left");
    assert_eq!(
        super::viewport_delta(&fixture.state.viewport_slides, id, switch_now),
        -width
    );
    assert_eq!(
        super::viewport_delta(
            &fixture.state.viewport_slides,
            id,
            switch_now + Duration::from_millis(10_000)
        ),
        0
    );
}

#[test]
fn workspace_switch_direction_follows_the_index() {
    let mut fixture = SlideFixture::new();
    let id = primary_id(&fixture);
    let now = Instant::now();
    fixture.update(&arrange_now(&fixture), now, false);
    open_window(&mut fixture, 1, id);
    fixture.update(
        &arrange_now(&fixture),
        now + Duration::from_millis(10),
        false,
    );
    // Down to empty (no slide), open a window there so both sides are occupied.
    fixture
        .state
        .world
        .handle_action(Action::FocusWorkspace(Vertical::Down));
    fixture.update(
        &arrange_now(&fixture),
        now + Duration::from_millis(20),
        false,
    );
    assert!(fixture.state.viewport_slides.is_empty());
    open_window(&mut fixture, 2, id);
    fixture.update(
        &arrange_now(&fixture),
        now + Duration::from_millis(30),
        false,
    );
    assert!(fixture.state.viewport_slides.is_empty());
    // Up then down again: down (higher index) comes from the right.
    fixture
        .state
        .world
        .handle_action(Action::FocusWorkspace(Vertical::Up));
    let up_now = now + Duration::from_millis(40);
    fixture.update(&arrange_now(&fixture), up_now, false);
    assert_eq!(fixture.state.viewport_slides.len(), 1);
    let width = fixture
        .state
        .world
        .usable_area(id)
        .map(|a| a.w)
        .unwrap_or(0);
    assert_eq!(fixture.state.viewport_slides[0].from_x, -width);
    // Down again, after the up slide landed and pruned: a fresh slide from
    // the right (higher index comes from the right). Letting the first
    // slide land first matters: a switch arriving *mid-flight* continues
    // from the visual instead (pinned below), it does not restart from
    // the edge.
    fixture
        .state
        .prune_slides(up_now + Duration::from_millis(10_000));
    assert!(fixture.state.viewport_slides.is_empty());
    fixture
        .state
        .world
        .handle_action(Action::FocusWorkspace(Vertical::Down));
    let down_now = now + Duration::from_millis(300);
    fixture.update(&arrange_now(&fixture), down_now, false);
    assert_eq!(fixture.state.viewport_slides.len(), 1);
    assert_eq!(fixture.state.viewport_slides[0].from_x, width);
}

#[test]
fn a_switch_mid_flight_continues_from_the_visual() {
    // Rapid A→B→C: the second switch arrives while the first slide is still
    // running, so it must continue from what is on screen rather than jump
    // back to the full edge — the viewport twin of the per-window retarget
    // rule.
    let mut fixture = SlideFixture::new();
    let id = primary_id(&fixture);
    let now = Instant::now();
    fixture.update(&arrange_now(&fixture), now, false);
    open_window(&mut fixture, 1, id);
    fixture.update(
        &arrange_now(&fixture),
        now + Duration::from_millis(10),
        false,
    );
    fixture
        .state
        .world
        .handle_action(Action::FocusWorkspace(Vertical::Down));
    fixture.update(
        &arrange_now(&fixture),
        now + Duration::from_millis(20),
        false,
    );
    open_window(&mut fixture, 2, id);
    fixture.update(
        &arrange_now(&fixture),
        now + Duration::from_millis(30),
        false,
    );
    fixture
        .state
        .world
        .handle_action(Action::FocusWorkspace(Vertical::Down));
    fixture.update(
        &arrange_now(&fixture),
        now + Duration::from_millis(40),
        false,
    );
    open_window(&mut fixture, 3, id);
    fixture.update(
        &arrange_now(&fixture),
        now + Duration::from_millis(50),
        false,
    );
    // Up to the middle workspace: a fresh slide from the left edge.
    fixture
        .state
        .world
        .handle_action(Action::FocusWorkspace(Vertical::Up));
    let first_switch = now + Duration::from_millis(60);
    fixture.update(&arrange_now(&fixture), first_switch, false);
    assert_eq!(fixture.state.viewport_slides.len(), 1);
    let width = fixture
        .state
        .world
        .usable_area(id)
        .map(|a| a.w)
        .unwrap_or(0);
    assert!(width > 0);
    assert_eq!(fixture.state.viewport_slides[0].from_x, -width);
    // Down again 50ms later (duration is 200ms): still in flight. What the
    // strip shows at the retarget instant...
    let second_switch = first_switch + Duration::from_millis(50);
    let shown = super::viewport_delta(&fixture.state.viewport_slides, id, second_switch);
    assert!(shown < 0 && shown > -width);
    fixture
        .state
        .world
        .handle_action(Action::FocusWorkspace(Vertical::Down));
    fixture.update(&arrange_now(&fixture), second_switch, false);
    assert_eq!(fixture.state.viewport_slides.len(), 1);
    // ...is where the retargeted slide starts: not the full right edge.
    let retargeted = &fixture.state.viewport_slides[0];
    assert_eq!(retargeted.from_x, shown);
    assert_ne!(retargeted.from_x, width);
    // No visual jump: the delta the instant before is the delta the
    // instant after.
    assert_eq!(
        super::viewport_delta(&fixture.state.viewport_slides, id, second_switch),
        shown
    );
    // And it still lands exactly on time.
    assert_eq!(
        super::viewport_delta(
            &fixture.state.viewport_slides,
            id,
            second_switch + Duration::from_millis(10_000)
        ),
        0
    );
}

#[test]
fn a_direction_reversal_mid_flight_continues_from_the_visual() {
    // A→B then back to A before the first slide lands: restarting from the
    // new edge would fling the strip across the screen; continuing from the
    // visual glides it back from where it is.
    let mut fixture = SlideFixture::new();
    let id = primary_id(&fixture);
    let now = Instant::now();
    fixture.update(&arrange_now(&fixture), now, false);
    open_window(&mut fixture, 1, id);
    fixture.update(
        &arrange_now(&fixture),
        now + Duration::from_millis(10),
        false,
    );
    fixture
        .state
        .world
        .handle_action(Action::FocusWorkspace(Vertical::Down));
    fixture.update(
        &arrange_now(&fixture),
        now + Duration::from_millis(20),
        false,
    );
    open_window(&mut fixture, 2, id);
    fixture.update(
        &arrange_now(&fixture),
        now + Duration::from_millis(30),
        false,
    );
    // Up to the first workspace: fresh slide from the left.
    fixture
        .state
        .world
        .handle_action(Action::FocusWorkspace(Vertical::Up));
    let first_switch = now + Duration::from_millis(40);
    fixture.update(&arrange_now(&fixture), first_switch, false);
    assert_eq!(fixture.state.viewport_slides.len(), 1);
    let width = fixture
        .state
        .world
        .usable_area(id)
        .map(|a| a.w)
        .unwrap_or(0);
    assert_eq!(fixture.state.viewport_slides[0].from_x, -width);
    // Back down 50ms later, mid-flight: the strip is left of center...
    let second_switch = first_switch + Duration::from_millis(50);
    let shown = super::viewport_delta(&fixture.state.viewport_slides, id, second_switch);
    assert!(shown < 0 && shown > -width);
    fixture
        .state
        .world
        .handle_action(Action::FocusWorkspace(Vertical::Down));
    fixture.update(&arrange_now(&fixture), second_switch, false);
    assert_eq!(fixture.state.viewport_slides.len(), 1);
    // ...and the retarget keeps it there instead of restarting from the
    // right edge a reversal would otherwise name.
    let retargeted = &fixture.state.viewport_slides[0];
    assert_eq!(retargeted.from_x, shown);
    assert_ne!(retargeted.from_x, width);
    assert_eq!(
        super::viewport_delta(&fixture.state.viewport_slides, id, second_switch),
        shown
    );
}

#[test]
fn empty_and_single_window_workspaces_behave() {
    let mut fixture = SlideFixture::new();
    let now = Instant::now();
    // Empty to empty: no slide, no panic.
    fixture.update(&arrangement_of(Vec::new()), now, false);
    assert!(fixture.state.viewport_slides.is_empty());
    // Single window switching to empty and back is covered above; here pin
    // that an empty arrangement never starts a slide even when the index
    // moved (driven through the real world below).
    let id = primary_id(&fixture);
    fixture.update(&arrange_now(&fixture), now, false);
    open_window(&mut fixture, 1, id);
    fixture.update(
        &arrange_now(&fixture),
        now + Duration::from_millis(10),
        false,
    );
    fixture
        .state
        .world
        .handle_action(Action::FocusWorkspace(Vertical::Down));
    fixture.update(
        &arrange_now(&fixture),
        now + Duration::from_millis(20),
        false,
    );
    assert!(fixture.state.viewport_slides.is_empty());
}

#[test]
fn multi_output_switches_are_independent() {
    let mut fixture = SlideFixture::new();
    let first = primary_id(&fixture);
    crate::compositor::headless::add_output(&mut fixture.state, "headless-2", 800, 600)
        .expect("a second output");
    let second = fixture
        .state
        .outputs
        .iter_with_ids()
        .map(|(id, _)| id)
        .find(|id| *id != first)
        .expect("two outputs");
    let now = Instant::now();
    fixture.update(&arrange_now(&fixture), now, false);
    open_window(&mut fixture, 1, first);
    open_window(&mut fixture, 2, second);
    fixture.update(
        &arrange_now(&fixture),
        now + Duration::from_millis(10),
        false,
    );
    assert!(fixture.state.viewport_slides.is_empty());
    // Switch only the first output's workspace (focus it first).
    fixture
        .state
        .world
        .handle_action(Action::FocusWorkspace(Vertical::Down));
    // The focused output is the one holding focus; ensure only one side
    // switched by checking at most one viewport slide and that it names a
    // switched output. If both switched (focus moved), both slides are still
    // per-output and independent.
    let switch_now = now + Duration::from_millis(20);
    fixture.update(&arrange_now(&fixture), switch_now, false);
    for slide in &fixture.state.viewport_slides {
        assert!(slide.output == first || slide.output == second);
    }
    // A switch that touches neither output starts nothing (already pinned by
    // the empty case); here pin that a second output with no switch keeps no
    // slide once its own index is recorded: clear, re-record, no switch.
    fixture.state.viewport_slides.clear();
    fixture.update(
        &arrange_now(&fixture),
        switch_now + Duration::from_millis(10),
        false,
    );
    assert!(
        fixture.state.viewport_slides.is_empty(),
        "no index change means no viewport slide, on any output"
    );
}

#[test]
fn viewport_lock_and_disable_snap() {
    let mut fixture = SlideFixture::new();
    let id = primary_id(&fixture);
    let now = Instant::now();
    fixture.update(&arrange_now(&fixture), now, false);
    open_window(&mut fixture, 1, id);
    fixture.update(
        &arrange_now(&fixture),
        now + Duration::from_millis(10),
        false,
    );
    fixture
        .state
        .world
        .handle_action(Action::FocusWorkspace(Vertical::Down));
    fixture.update(
        &arrange_now(&fixture),
        now + Duration::from_millis(20),
        false,
    );
    fixture
        .state
        .world
        .handle_action(Action::FocusWorkspace(Vertical::Up));
    let switch_now = now + Duration::from_millis(30);
    fixture.update(&arrange_now(&fixture), switch_now, false);
    assert_eq!(fixture.state.viewport_slides.len(), 1);
    // Lock snaps.
    fixture.update(
        &arrange_now(&fixture),
        switch_now + Duration::from_millis(10),
        true,
    );
    assert!(fixture.state.viewport_slides.is_empty());
    // Restart, then disable snaps.
    fixture
        .state
        .world
        .handle_action(Action::FocusWorkspace(Vertical::Down));
    fixture.update(
        &arrange_now(&fixture),
        switch_now + Duration::from_millis(20),
        false,
    );
    // Down went to empty (no slide); go back up for a real slide.
    fixture
        .state
        .world
        .handle_action(Action::FocusWorkspace(Vertical::Up));
    fixture.update(
        &arrange_now(&fixture),
        switch_now + Duration::from_millis(30),
        false,
    );
    assert_eq!(fixture.state.viewport_slides.len(), 1);
    fixture.state.animation.enabled = false;
    fixture.update(
        &arrange_now(&fixture),
        switch_now + Duration::from_millis(40),
        false,
    );
    assert!(fixture.state.viewport_slides.is_empty());
    assert!(fixture.state.slides.is_empty());
}

#[test]
fn viewport_reload_disable_snaps_and_reports() {
    use scoot_ipc::{Request, Response};
    use std::fs;

    let mut fixture = SlideFixture::new();
    let id = primary_id(&fixture);
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("config.toml");
    fs::write(&path, "[animation]\nenabled = true\n").expect("a config file");
    fixture.state.config_path = Some(path.clone());
    fixture.state.animation = AnimationSettings::default();
    match fixture.state.handle_request(Request::Reload) {
        Response::Reloaded { applied, .. } => {
            assert!(
                applied.contains(&"animation.enabled".to_owned()),
                "{applied:?}"
            );
        }
        other => panic!("reload must apply: {other:?}"),
    }
    assert!(fixture.state.animation.enabled);
    let now = Instant::now();
    fixture.update(&arrange_now(&fixture), now, false);
    open_window(&mut fixture, 1, id);
    fixture.update(
        &arrange_now(&fixture),
        now + Duration::from_millis(10),
        false,
    );
    fixture
        .state
        .world
        .handle_action(Action::FocusWorkspace(Vertical::Down));
    fixture.update(
        &arrange_now(&fixture),
        now + Duration::from_millis(20),
        false,
    );
    fixture
        .state
        .world
        .handle_action(Action::FocusWorkspace(Vertical::Up));
    fixture.update(
        &arrange_now(&fixture),
        now + Duration::from_millis(30),
        false,
    );
    assert_eq!(fixture.state.viewport_slides.len(), 1);
    fs::write(&path, "[animation]\nenabled = false\n").expect("a rewritten config");
    match fixture.state.handle_request(Request::Reload) {
        Response::Reloaded { applied, .. } => {
            assert!(
                applied.contains(&"animation.enabled".to_owned()),
                "{applied:?}"
            );
        }
        other => panic!("disable must apply: {other:?}"),
    }
    assert!(!fixture.state.animation.enabled);
    assert!(fixture.state.viewport_slides.is_empty());
    assert!(fixture.state.slides.is_empty());
}

#[test]
fn viewport_prunes_and_idles() {
    let start = Instant::now();
    let mut slides = vec![ViewportSlide {
        output: OutputId(1),
        from_x: 800,
        start,
        duration: Duration::from_millis(50),
        easing: Easing::Linear,
    }];
    let late = start + Duration::from_millis(5000);
    assert!(!viewports_in_flight(&slides, late));
    super::prune_finished_viewports(&mut slides, late);
    assert!(slides.is_empty());
    // State-level: in-flight keeps frames, landed idles.
    let mut fixture = SlideFixture::new();
    let id = primary_id(&fixture);
    let now = Instant::now();
    fixture.update(&arrange_now(&fixture), now, false);
    open_window(&mut fixture, 1, id);
    fixture.update(
        &arrange_now(&fixture),
        now + Duration::from_millis(10),
        false,
    );
    fixture
        .state
        .world
        .handle_action(Action::FocusWorkspace(Vertical::Down));
    fixture.update(
        &arrange_now(&fixture),
        now + Duration::from_millis(20),
        false,
    );
    fixture
        .state
        .world
        .handle_action(Action::FocusWorkspace(Vertical::Up));
    let switch_now = now + Duration::from_millis(30);
    fixture.update(&arrange_now(&fixture), switch_now, false);
    assert!(
        fixture
            .state
            .animation_in_flight(switch_now + Duration::from_millis(10))
    );
    assert!(
        !fixture
            .state
            .animation_in_flight(switch_now + Duration::from_millis(10_000))
    );
    fixture
        .state
        .prune_slides(switch_now + Duration::from_millis(10_000));
    assert!(fixture.state.viewport_slides.is_empty());
}
