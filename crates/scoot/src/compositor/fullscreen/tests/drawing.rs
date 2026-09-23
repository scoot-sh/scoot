//! What reaches the framebuffer and where a click lands while a fullscreen
//! window covers an output: the window edge to edge with no ring and no
//! rounded corners, the top layer hidden (drawn, pointed at and typed into
//! nowhere), the overlay layer kept, and the lock screen over all of it.

use scoot_core::{Action, Vertical};

use super::*;

/// Every corner, every edge midpoint and the centre of the canvas.
fn probes() -> [(i32, i32); 9] {
    let far = CANVAS - 1;
    let mid = CANVAS / 2;
    [
        (0, 0),
        (far, 0),
        (0, far),
        (far, far),
        (mid, 0),
        (0, mid),
        (far, mid),
        (mid, far),
        (mid, mid),
    ]
}

fn assert_covered(pixels: &[u8], color: [u8; 4], what: &str) {
    for (x, y) in probes() {
        assert_eq!(
            pixel(pixels, x, y),
            color,
            "{what}: ({x}, {y}) is not the window"
        );
    }
    assert!(
        !test_support::contains(pixels, RING_BGRA),
        "{what}: a focus ring was drawn around a fullscreen window"
    );
}

/// Maps a window, makes it fullscreen, and draws it at the size it was told.
fn fullscreen_window(fixture: &mut Fixture) {
    fixture.map(WINDOW_BGRA);
    fixture.configured(Step::SetFullscreen {
        window: 0,
        output: None,
    });
    fixture.done(Step::Draw {
        window: 0,
        color: WINDOW_BGRA,
    });
}

#[test]
fn a_fullscreen_window_covers_the_output_edge_to_edge_with_no_ring() {
    let mut fixture = Fixture::new();
    fixture.map(OTHER_BGRA);
    fixture.map(WINDOW_BGRA);
    assert_eq!(fixture.rect_of(0).x, GAP);
    let before = fixture.render();
    // The tiled frame has the gap, the ring and the background around it.
    assert_eq!(pixel(&before, 0, 0), BACKGROUND_BGRA);
    assert!(test_support::contains(&before, RING_BGRA));

    fixture.configured(Step::SetFullscreen {
        window: 1,
        output: None,
    });
    fixture.done(Step::Draw {
        window: 1,
        color: WINDOW_BGRA,
    });
    let covered = fixture.render();
    assert_covered(&covered, WINDOW_BGRA, "fullscreen");
    assert!(
        !test_support::contains(&covered, OTHER_BGRA),
        "the other window shows beside a covering one"
    );
}

#[test]
fn rounded_corners_are_off_for_a_fullscreen_window() {
    let mut fixture = Fixture::with_appearance(Appearance {
        corner_radius: 12,
        ..appearance()
    });
    fullscreen_window(&mut fixture);
    assert_covered(&fixture.render(), WINDOW_BGRA, "rounded session");
}

#[test]
fn the_bar_is_hidden_and_the_notification_kept() {
    let mut fixture = Fixture::new();
    fixture.done(Step::CreateLayer(Layer::Bar));
    fixture.done(Step::CreateLayer(Layer::Notification));
    fixture.map(WINDOW_BGRA);
    // The notification reserves nothing but respects the bar's zone (its
    // own zone is 0), so it sits just under the bar.
    let note = (CANVAS - 5, BAR_HEIGHT as i32 + 5);
    let tiled = fixture.render();
    assert_eq!(pixel(&tiled, 5, 5), BAR_BGRA);
    assert_eq!(pixel(&tiled, note.0, note.1), NOTE_BGRA);

    fixture.configured(Step::SetFullscreen {
        window: 0,
        output: None,
    });
    fixture.done(Step::Draw {
        window: 0,
        color: WINDOW_BGRA,
    });
    // It ignores the bar's exclusive zone as well as the gap.
    assert_eq!(fixture.rect_of(0), OUTPUT);
    let covered = fixture.render();
    assert!(
        !test_support::contains(&covered, BAR_BGRA),
        "the top layer was drawn over a fullscreen window"
    );
    assert_eq!(
        pixel(&covered, 5, 5),
        WINDOW_BGRA,
        "the window must reach the top edge the bar reserved"
    );
    assert_eq!(
        pixel(&covered, note.0, note.1),
        NOTE_BGRA,
        "the overlay layer must stay above a fullscreen window"
    );
    // The notification covers one corner; everything else is the window.
    assert_eq!(pixel(&covered, 0, CANVAS - 1), WINDOW_BGRA);
    assert_eq!(pixel(&covered, CANVAS - 1, CANVAS - 1), WINDOW_BGRA);

    // Leaving brings the bar back.
    fixture.configured(Step::UnsetFullscreen { window: 0 });
    fixture.done(Step::Draw {
        window: 0,
        color: WINDOW_BGRA,
    });
    assert_eq!(pixel(&fixture.render(), 5, 5), BAR_BGRA);
}

#[test]
fn focusing_away_or_switching_workspace_brings_the_bar_back() {
    let mut fixture = Fixture::new();
    fixture.done(Step::CreateLayer(Layer::Bar));
    fixture.map(OTHER_BGRA);
    fixture.map(WINDOW_BGRA);
    fixture.configured(Step::SetFullscreen {
        window: 1,
        output: None,
    });
    fixture.done(Step::Draw {
        window: 1,
        color: WINDOW_BGRA,
    });
    assert!(!test_support::contains(&fixture.render(), BAR_BGRA));

    fixture.state.act(Action::FocusWindowId(fixture.id(0)));
    fixture.settle();
    assert_eq!(
        pixel(&fixture.render(), 5, 5),
        BAR_BGRA,
        "focused away, nothing covers the output"
    );
    assert!(fixture.state.world.is_fullscreen(fixture.id(1)));

    fixture.state.act(Action::FocusWindowId(fixture.id(1)));
    fixture.settle();
    assert!(!test_support::contains(&fixture.render(), BAR_BGRA));

    fixture.state.act(Action::FocusWorkspace(Vertical::Down));
    fixture.settle();
    let away = fixture.render();
    assert_eq!(pixel(&away, 5, 5), BAR_BGRA);
    assert!(!test_support::contains(&away, WINDOW_BGRA));

    fixture.state.act(Action::FocusWorkspace(Vertical::Up));
    fixture.settle();
    assert_covered(&fixture.render(), WINDOW_BGRA, "back on its workspace");
}

#[test]
fn a_click_where_the_hidden_bar_was_reaches_the_window() {
    let mut fixture = Fixture::new();
    fixture.done(Step::CreateLayer(Layer::Bar));
    fixture.map(WINDOW_BGRA);
    // Tiled: the top edge is the bar's.
    fixture.click(100.0, 5.0);
    assert_eq!(fixture.pointer(), Some(Entered::Layer(0)));

    fixture.configured(Step::SetFullscreen {
        window: 0,
        output: None,
    });
    fixture.done(Step::Draw {
        window: 0,
        color: WINDOW_BGRA,
    });
    fixture.click(100.0, 6.0);
    assert_eq!(
        fixture.pointer(),
        Some(Entered::Window(0)),
        "an undrawn bar took the pointer over a fullscreen window"
    );
}

#[test]
fn the_notification_still_takes_the_pointer() {
    let mut fixture = Fixture::new();
    fixture.done(Step::CreateLayer(Layer::Notification));
    fullscreen_window(&mut fixture);
    fixture.click(f64::from(CANVAS) - 5.0, 5.0);
    assert_eq!(fixture.pointer(), Some(Entered::Layer(0)));
}

#[test]
fn another_output_s_bar_is_untouched() {
    // Fullscreen is per output: covering the second output leaves the bar
    // on the first one drawn and clickable.
    let mut fixture = Fixture::new();
    let second =
        crate::compositor::headless::add_output(&mut fixture.state, "headless-2", CANVAS, CANVAS)
            .expect("a second output");
    fixture.settle();
    fixture.done(Step::CreateLayer(Layer::Bar)); // on the primary output
    // New windows open on the pointer's output.
    fixture.state.pointer_move(f64::from(CANVAS) + 50.0, 50.0);
    fullscreen_window(&mut fixture);
    assert_eq!(
        fixture.state.world.fullscreen_on(second),
        Some(fixture.id(0))
    );

    let first = fixture.render();
    assert_eq!(
        pixel(&first, 5, 5),
        BAR_BGRA,
        "the first output's bar was hidden"
    );
    let covered = fixture.pixels_of(second);
    assert_covered(&covered, WINDOW_BGRA, "second output");

    fixture.click(100.0, 5.0);
    assert_eq!(fixture.pointer(), Some(Entered::Layer(0)));
}

#[test]
fn the_lock_screen_covers_a_fullscreen_window() {
    let mut fixture = Fixture::new();
    fullscreen_window(&mut fixture);
    fixture.done(Step::LockSession);
    assert!(fixture.state.session_lock.is_locked());
    let locked = fixture.render();
    assert!(
        !test_support::contains(&locked, WINDOW_BGRA),
        "a fullscreen window showed through the lock screen"
    );
    // Still fullscreen underneath, for when the session unlocks.
    assert!(fixture.state.world.is_fullscreen(fixture.id(0)));
}

#[test]
fn a_top_layer_launcher_does_not_take_the_keyboard_from_under_a_fullscreen_window() {
    // Hidden means hidden for the keyboard too: keys typed into a launcher
    // nobody can see would be lost. It gets the keyboard back the moment
    // nothing covers the output.
    let mut fixture = Fixture::new();
    fullscreen_window(&mut fixture);
    let window = fixture.window_surface(0);
    fixture.done(Step::CreateLayer(Layer::Launcher(
        zwlr_layer_shell_v1::Layer::Top,
    )));
    assert_eq!(fixture.keyboard_focus(), Some(window.clone()));
    assert!(!test_support::contains(&fixture.render(), NOTE_BGRA));

    fixture.configured(Step::UnsetFullscreen { window: 0 });
    fixture.settle();
    assert_ne!(
        fixture.keyboard_focus(),
        Some(window),
        "the launcher did not get the keyboard once nothing covered it"
    );
}

#[test]
fn an_overlay_launcher_takes_the_keyboard_over_a_fullscreen_window() {
    let mut fixture = Fixture::new();
    fullscreen_window(&mut fixture);
    let window = fixture.window_surface(0);
    fixture.done(Step::CreateLayer(Layer::Launcher(
        zwlr_layer_shell_v1::Layer::Overlay,
    )));
    assert_ne!(fixture.keyboard_focus(), Some(window));
    assert!(test_support::contains(&fixture.render(), NOTE_BGRA));
}
