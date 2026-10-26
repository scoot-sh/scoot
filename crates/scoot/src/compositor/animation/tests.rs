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
fn easing_parses_the_two_names_and_rejects_the_rest() {
    assert_eq!(Easing::parse(None), Ok(Easing::EaseOut));
    assert_eq!(Easing::parse(Some("ease-out")), Ok(Easing::EaseOut));
    assert_eq!(Easing::parse(Some("linear")), Ok(Easing::Linear));
    assert!(Easing::parse(Some("spring")).is_err());
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
